use crate::graph::{Node, NodeKind};
use crate::sandbox::{ExecResult, Sandbox, SandboxEnv};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const PROFILE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_TMPFS_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ADDRESS_SPACE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_PROCESSES: u64 = 256;
const MAX_OPEN_FILES: u64 = 1024;

/// Linux x86_64 command execution in a Bubblewrap namespace and filesystem sandbox.
pub struct BubblewrapSandbox {
    context_dir: PathBuf,
    runtime: Option<PathBuf>,
    timeout: Duration,
}

impl BubblewrapSandbox {
    pub fn new(context_dir: PathBuf) -> Self {
        Self {
            context_dir,
            runtime: find_runtime(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_runtime(context_dir: PathBuf, runtime: impl Into<PathBuf>) -> Self {
        Self {
            context_dir,
            runtime: Some(runtime.into()),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn runtime_path(&self) -> Result<&Path> {
        let runtime = self.runtime.as_deref().ok_or_else(|| {
            anyhow!(
                "Bubblewrap is required for isolated build execution but was not found in PATH. \
                 Install bubblewrap and enable unprivileged user namespaces; no host-shell fallback is available."
            )
        })?;
        if !runtime.is_file() {
            bail!(
                "Bubblewrap is required for isolated build execution but the configured executable \
                 does not exist: {}. No host-shell fallback is available.",
                runtime.display()
            );
        }
        Ok(runtime)
    }

    async fn verify_profile(&self, workspace_dir: &Path) -> Result<()> {
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = workspace_dir;
            bail!("Bubblewrap isolation currently supports Linux x86_64 only");
        }

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let runtime = self.runtime_path()?;
            let version = Command::new(runtime)
                .arg("--version")
                .output()
                .await
                .with_context(|| format!("Failed to run Bubblewrap at {}", runtime.display()))?;
            let version_text = String::from_utf8_lossy(&version.stdout);
            if !version.status.success() || !version_text.starts_with("bubblewrap ") {
                bail!(
                    "The configured isolation runtime is not a working Bubblewrap executable: {}",
                    String::from_utf8_lossy(&version.stderr).trim()
                );
            }

            let mut command = Command::new(runtime);
            self.configure_command(&mut command, workspace_dir, &HashMap::new(), "/bin/true")?;
            command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            set_resource_limits(&mut command)?;
            let mut child = command
                .spawn()
                .context("Failed to initialize the Bubblewrap isolation profile")?;
            let status = match timeout(PROFILE_TIMEOUT, child.wait()).await {
                Ok(status) => status.context("Failed waiting for Bubblewrap profile check")?,
                Err(_) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    bail!("Bubblewrap isolation profile initialization timed out");
                }
            };
            if !status.success() {
                let mut diagnostic = Vec::new();
                if let Some(mut stderr) = child.stderr.take() {
                    stderr.read_to_end(&mut diagnostic).await.ok();
                }
                bail!(
                    "Bubblewrap could not establish the required isolated namespaces and mounts: {}",
                    String::from_utf8_lossy(&diagnostic).trim()
                );
            }
            Ok(())
        }
    }

    #[cfg(target_os = "linux")]
    fn configure_command(
        &self,
        command: &mut Command,
        workspace_dir: &Path,
        env_vars: &HashMap<String, String>,
        shell_command: &str,
    ) -> Result<()> {
        let _ = self.runtime_path()?;
        command
            .arg("--unshare-all")
            .arg("--unshare-user")
            .arg("--disable-userns")
            .arg("--assert-userns-disabled")
            .arg("--die-with-parent")
            .arg("--new-session")
            .arg("--clearenv");

        for path in ["/usr", "/bin", "/sbin", "/lib", "/lib64"] {
            if Path::new(path).exists() {
                command.args(["--ro-bind", path, path]);
            }
        }

        command.args(["--dev", "/dev", "--proc", "/proc"]);
        let tmpfs_size = MAX_TMPFS_BYTES.to_string();
        for path in ["/tmp", "/var", "/run", "/home", "/root"] {
            command.args(["--size", &tmpfs_size, "--tmpfs", path]);
        }
        command.arg("--dir").arg("/etc");

        for path in ["/etc/ld.so.cache", "/etc/ssl/certs"] {
            if Path::new(path).exists() {
                command.args(["--ro-bind", path, path]);
            }
        }

        command
            .args(["--dir", "/workspace"])
            .args(["--bind", &workspace_dir.to_string_lossy(), "/workspace"])
            .args(["--chdir", "/workspace"])
            .args(["--setenv", "HOME", "/tmp"])
            .args(["--setenv", "TMPDIR", "/tmp"])
            .args([
                "--setenv",
                "PATH",
                "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            ])
            .args(["--setenv", "USER", "builder"])
            .args(["--setenv", "LOGNAME", "builder"]);
        for (key, value) in env_vars {
            if key.is_empty()
                || key.contains('=')
                || key.as_bytes().contains(&0)
                || value.as_bytes().contains(&0)
            {
                bail!("Invalid environment variable supplied to isolated command: {key:?}");
            }
            command.args(["--setenv", key, value]);
        }
        command
            .arg("--cap-drop")
            .arg("ALL")
            .arg("--")
            .arg("/bin/sh")
            .arg("-c")
            .arg(shell_command);
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    fn configure_command(
        &self,
        _command: &mut Command,
        _workspace_dir: &Path,
        _env_vars: &HashMap<String, String>,
        _shell_command: &str,
    ) -> Result<()> {
        bail!("Bubblewrap isolation currently supports Linux x86_64 only")
    }

    fn copy_context(&self, destination: &Path) -> Result<()> {
        if !self.context_dir.is_dir() {
            bail!(
                "Build context directory does not exist: {}",
                self.context_dir.display()
            );
        }

        let mut directories = Vec::new();
        for entry in walkdir::WalkDir::new(&self.context_dir)
            .follow_links(false)
            .sort_by_file_name()
        {
            let entry = entry?;
            let relative = entry
                .path()
                .strip_prefix(&self.context_dir)
                .context("Build context entry escaped its root")?;
            if relative.as_os_str().is_empty() {
                continue;
            }
            let target = destination.join(relative);
            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&target)?;
                directories.push((target, entry.path().to_path_buf()));
            } else if entry.file_type().is_file() {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(entry.path(), &target)?;
                set_context_permissions(&target, entry.path())?;
            } else if entry.file_type().is_symlink() {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let link = std::fs::read_link(entry.path())?;
                #[cfg(unix)]
                std::os::unix::fs::symlink(link, &target)?;
                #[cfg(not(unix))]
                bail!("Symbolic links in build context are unsupported on this platform");
            } else {
                bail!(
                    "Unsupported special file in build context: {}",
                    entry.path().display()
                );
            }
        }
        for (target, source) in directories.into_iter().rev() {
            set_context_permissions(&target, &source)?;
        }
        Ok(())
    }
}

#[async_trait]
impl Sandbox for BubblewrapSandbox {
    async fn prepare(&self, node: &Node) -> Result<SandboxEnv> {
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = node;
            bail!("Bubblewrap isolation currently supports Linux x86_64 only");
        }

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            self.runtime_path()?;
            let temp_dir = TempDir::new().context("Failed to create isolated build directory")?;
            let workspace_dir = temp_dir.path().join("workspace");
            std::fs::create_dir(&workspace_dir)?;
            self.copy_context(&workspace_dir)
                .context("Failed to stage build context for isolation")?;
            set_owner_writable(&workspace_dir)?;
            self.verify_profile(&workspace_dir).await?;
            Ok(SandboxEnv {
                workspace_dir,
                env_vars: node.env.clone(),
                temp_dir: Some(temp_dir),
            })
        }
    }

    async fn execute(&self, env: &SandboxEnv, node: &Node) -> Result<ExecResult> {
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (env, node);
            bail!("Bubblewrap isolation currently supports Linux x86_64 only");
        }

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let command_text = match &node.kind {
                NodeKind::Run => node.content.as_str(),
                NodeKind::RunExtend { command, .. } => command.as_str(),
                NodeKind::CustomHook { hook_name, params } => {
                    let mut command = hook_name.clone();
                    for param in params {
                        command.push(' ');
                        command.push_str(param);
                    }
                    return self.execute_command(env, &command).await;
                }
                _ => {
                    return Ok(ExecResult {
                        exit_code: 0,
                        stdout: format!("Artifact for {}", node.name).into_bytes(),
                        stderr: Vec::new(),
                    })
                }
            };
            self.execute_command(env, command_text).await
        }
    }

    async fn cleanup(&self, env: SandboxEnv) -> Result<()> {
        if let Some(temp_dir) = env.temp_dir {
            make_workspace_removable(&env.workspace_dir)
                .context("Failed to prepare isolated workspace for removal")?;
            temp_dir
                .close()
                .context("Failed to remove isolated build directory")?;
        }
        Ok(())
    }
}

impl BubblewrapSandbox {
    #[cfg(target_os = "linux")]
    async fn execute_command(&self, env: &SandboxEnv, shell_command: &str) -> Result<ExecResult> {
        let mut command = Command::new(self.runtime_path()?);
        self.configure_command(
            &mut command,
            &env.workspace_dir,
            &env.env_vars,
            shell_command,
        )?;
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        set_resource_limits(&mut command)?;

        let mut child = command
            .spawn()
            .context("Failed to start isolated Bubblewrap command")?;
        let stdout = child.stdout.take().expect("stdout was configured as piped");
        let stderr = child.stderr.take().expect("stderr was configured as piped");
        let stdout_task = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stdout
                .take((MAX_OUTPUT_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .await?;
            Ok::<_, std::io::Error>(bytes)
        });
        let stderr_task = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stderr
                .take((MAX_OUTPUT_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .await?;
            Ok::<_, std::io::Error>(bytes)
        });

        let status = match timeout(self.timeout, child.wait()).await {
            Ok(status) => status.context("Failed while waiting for isolated build command")?,
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                bail!(
                    "Isolated build command exceeded timeout of {} seconds",
                    self.timeout.as_secs()
                );
            }
        };
        let stdout = stdout_task
            .await
            .context("Failed joining command stdout reader")??;
        let stderr = stderr_task
            .await
            .context("Failed joining command stderr reader")??;
        if stdout.len() > MAX_OUTPUT_BYTES || stderr.len() > MAX_OUTPUT_BYTES {
            bail!(
                "Isolated build command output exceeded the {} MiB limit",
                MAX_OUTPUT_BYTES / (1024 * 1024)
            );
        }

        use std::os::unix::process::ExitStatusExt;
        let exit_code = status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
        Ok(ExecResult {
            exit_code,
            stdout,
            stderr,
        })
    }

    #[cfg(not(target_os = "linux"))]
    async fn execute_command(&self, _env: &SandboxEnv, _shell_command: &str) -> Result<ExecResult> {
        bail!("Bubblewrap isolation currently supports Linux x86_64 only")
    }
}

fn find_runtime() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join("bwrap"))
        .find(|path| path.is_file())
}

#[cfg(target_os = "linux")]
fn set_resource_limits(command: &mut Command) -> Result<()> {
    use std::os::unix::process::CommandExt;

    unsafe {
        command.as_std_mut().pre_exec(|| {
            set_limit(libc::RLIMIT_CORE, 0)?;
            set_limit(libc::RLIMIT_CPU, 30 * 60)?;
            set_limit(libc::RLIMIT_FSIZE, MAX_FILE_BYTES)?;
            set_limit(libc::RLIMIT_NOFILE, MAX_OPEN_FILES)?;
            set_limit(libc::RLIMIT_NPROC, MAX_PROCESSES)?;
            set_limit(libc::RLIMIT_AS, MAX_ADDRESS_SPACE_BYTES)?;
            Ok(())
        });
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn set_resource_limits(_command: &mut Command) -> Result<()> {
    bail!("Bubblewrap resource limits currently support Linux only")
}

#[cfg(target_os = "linux")]
fn set_limit(resource: libc::__rlimit_resource_t, value: u64) -> std::io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value as libc::rlim_t,
        rlim_max: value as libc::rlim_t,
    };
    let result = unsafe { libc::setrlimit(resource, &limit) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn set_context_permissions(target: &Path, source: &Path) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let mode = std::fs::metadata(source)?.mode() & 0o777;
    std::fs::set_permissions(target, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_context_permissions(_target: &Path, _source: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_writable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o700);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(unix)]
fn make_workspace_removable(path: &Path) -> Result<()> {
    for entry in walkdir::WalkDir::new(path)
        .follow_links(false)
        .contents_first(true)
    {
        let entry = entry?;
        if entry.file_type().is_dir() {
            set_owner_writable(entry.path())?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn make_workspace_removable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_writable(_path: &Path) -> Result<()> {
    Ok(())
}
