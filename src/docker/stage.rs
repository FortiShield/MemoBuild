use crate::docker::dag::CopyOperation;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Component, Path, PathBuf};
use tempfile::TempDir;

pub struct StageFilesystem {
    workspace: Option<TempDir>,
    root: PathBuf,
}

impl StageFilesystem {
    pub fn new() -> Result<Self> {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            bail!("The first staged OCI build target requires Linux x86_64 (linux/amd64)");
        }
        let workspace =
            tempfile::tempdir().context("Failed to allocate scratch stage workspace")?;
        let root = workspace.path().join("rootfs");
        fs::create_dir(&root).context("Failed to create empty scratch root filesystem")?;
        Ok(Self {
            workspace: Some(workspace),
            root,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn workspace_path(&self) -> &Path {
        self.workspace
            .as_ref()
            .expect("stage workspace remains owned until cleanup")
            .path()
    }

    pub fn cleanup(mut self) -> Result<()> {
        let workspace = self
            .workspace
            .take()
            .context("Stage workspace was already cleaned up")?;
        let permissions = make_tree_removable(&self.root);
        let removal = workspace
            .close()
            .context("Failed to remove temporary stage filesystem");
        match (permissions, removal) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(permission_error), Ok(())) => Err(permission_error),
            (Ok(()), Err(removal_error)) => Err(removal_error),
            (Err(permission_error), Err(removal_error)) => bail!(
                "Failed making stage removable: {permission_error:#}; \
                 stage cleanup also failed: {removal_error:#}"
            ),
        }
    }

    pub fn apply_copy(&self, context_root: &Path, operation: &CopyOperation) -> Result<()> {
        let source_path = resolve_source(context_root, operation)?;
        let destination_path = self.root.join(&operation.destination);
        ensure_stage_path(&self.root, &destination_path)?;

        let source_metadata = fs::symlink_metadata(&source_path).with_context(|| {
            format!(
                "Dockerfile line {}: failed to inspect COPY source '{}'",
                operation.source_line,
                operation.source.display()
            )
        })?;
        let mut temporarily_writable =
            make_parent_directories_writable(&self.root, &destination_path)?;

        let result = (|| {
            if source_metadata.is_file() {
                let destination_is_dir = if operation.destination_is_directory {
                    true
                } else {
                    match fs::symlink_metadata(&destination_path) {
                        Ok(metadata) => metadata.is_dir(),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                        Err(error) => {
                            return Err(error).with_context(|| {
                                format!(
                                    "Dockerfile line {}: failed to inspect COPY destination",
                                    operation.source_line
                                )
                            });
                        }
                    }
                };
                let target = if destination_is_dir {
                    let basename = operation.source.file_name().ok_or_else(|| {
                        anyhow::anyhow!(
                            "Dockerfile line {}: COPY file source has no filename",
                            operation.source_line
                        )
                    })?;
                    destination_path.join(basename)
                } else {
                    destination_path.clone()
                };
                ensure_stage_path(&self.root, &target)?;
                if let Some(parent) = target.parent() {
                    create_stage_directories(&self.root, parent)?;
                }

                copy_file(&source_path, &target, operation.source_line)
            } else if source_metadata.is_dir() {
                copy_directory_contents(
                    &self.root,
                    &source_path,
                    &destination_path,
                    operation.source_line,
                )
            } else {
                bail!(
                    "Dockerfile line {}: COPY supports regular files and directories only",
                    operation.source_line
                );
            }
        })();

        let restore_result = restore_directory_permissions(&mut temporarily_writable);
        match (result, restore_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(operation_error), Ok(())) => Err(operation_error),
            (Ok(()), Err(restore_error)) => Err(restore_error),
            (Err(operation_error), Err(restore_error)) => bail!(
                "{operation_error:#}; additionally, failed restoring stage directory permissions: \
                 {restore_error:#}"
            ),
        }
    }
}

impl Drop for StageFilesystem {
    fn drop(&mut self) {
        if self.workspace.is_some() {
            let _ = make_tree_removable(&self.root);
        }
    }
}

fn make_tree_removable(root: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(root)
        .with_context(|| format!("Failed to inspect temporary stage root {}", root.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "Temporary stage root is not a directory: {}",
            root.display()
        );
    }
    make_directory_removable(root)
}

fn make_directory_removable(directory: &Path) -> Result<()> {
    let mut permissions = fs::metadata(directory)?.permissions();
    set_owner_write(&mut permissions);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(permissions.mode() | 0o700);
    }
    fs::set_permissions(directory, permissions).with_context(|| {
        format!(
            "Failed to make stage directory {} removable",
            directory.display()
        )
    })?;

    for entry in fs::read_dir(directory).with_context(|| {
        format!(
            "Failed to read stage directory {} during cleanup",
            directory.display()
        )
    })? {
        let entry = entry?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child)?;
        if metadata.file_type().is_symlink() {
            bail!(
                "Temporary stage unexpectedly contains symlink {}",
                child.display()
            );
        }
        if metadata.is_dir() {
            make_directory_removable(&child)?;
        }
    }
    Ok(())
}

fn resolve_source(context_root: &Path, operation: &CopyOperation) -> Result<PathBuf> {
    if operation.source.is_absolute()
        || operation.source.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!(
            "Dockerfile line {}: COPY source '{}' must remain inside the build context",
            operation.source_line,
            operation.source.display()
        );
    }

    let candidate = context_root.join(&operation.source);
    reject_source_symlink_components(context_root, &operation.source, operation.source_line)?;
    let canonical = fs::canonicalize(&candidate).with_context(|| {
        format!(
            "Dockerfile line {}: COPY source '{}' does not exist in the build context",
            operation.source_line,
            operation.source.display()
        )
    })?;
    if !canonical.starts_with(context_root) {
        bail!(
            "Dockerfile line {}: COPY source '{}' resolves outside the build context",
            operation.source_line,
            operation.source.display()
        );
    }
    validate_source_tree(&canonical, operation.source_line)?;
    Ok(canonical)
}

fn reject_source_symlink_components(
    context_root: &Path,
    relative: &Path,
    line: usize,
) -> Result<()> {
    let mut current = context_root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::CurDir => continue,
            Component::Normal(name) => current.push(name),
            _ => continue,
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!(
                    "Dockerfile line {line}: COPY source '{}' traverses a symlink; \
                     symlink sources are unsupported",
                    relative.display()
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Dockerfile line {line}: failed to inspect COPY source")
                });
            }
        }
    }
    Ok(())
}

fn validate_source_tree(source: &Path, line: usize) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("Dockerfile line {line}: failed to inspect COPY source"))?;
    if !metadata.is_file() && !metadata.is_dir() {
        bail!("Dockerfile line {line}: COPY supports regular files and directories only");
    }
    if metadata.is_dir() {
        for entry in walkdir::WalkDir::new(source).follow_links(false) {
            let entry = entry.with_context(|| {
                format!("Dockerfile line {line}: failed to inspect COPY directory")
            })?;
            let kind = entry.file_type();
            if kind.is_symlink() {
                bail!(
                    "Dockerfile line {line}: COPY source '{}' contains a symlink; \
                     symlink sources are unsupported",
                    entry.path().display()
                );
            }
            if !kind.is_file() && !kind.is_dir() {
                bail!("Dockerfile line {line}: COPY supports regular files and directories only");
            }
        }
    }
    Ok(())
}

fn ensure_stage_path(root: &Path, path: &Path) -> Result<()> {
    if !path.starts_with(root) {
        bail!("COPY destination escapes the image filesystem root");
    }
    let relative = path
        .strip_prefix(root)
        .context("COPY destination is outside the image filesystem root")?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(name) => current.push(name),
            Component::CurDir => {}
            _ => bail!("COPY destination is not a normalized image path"),
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!(
                    "COPY destination '{}' traverses a staged symlink",
                    current.display()
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Failed to inspect image destination {}", current.display())
                });
            }
        }
    }
    Ok(())
}

fn make_parent_directories_writable(
    root: &Path,
    destination: &Path,
) -> Result<Vec<(PathBuf, fs::Permissions)>> {
    let parent = if destination == root {
        root
    } else {
        destination.parent().unwrap_or(root)
    };
    let relative = parent.strip_prefix(root)?;
    let mut current = root.to_path_buf();
    let mut originals = Vec::new();
    for component in relative.components() {
        if let Component::Normal(name) = component {
            current.push(name);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.is_dir() => {
                    originals.push((current.clone(), metadata.permissions()));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("Failed to inspect stage directory {}", current.display())
                    });
                }
            }
        }
    }
    for (path, permissions) in &originals {
        let mut writable = permissions.clone();
        set_owner_write(&mut writable);
        if let Err(error) = fs::set_permissions(path, writable) {
            let mut restore = originals
                .iter()
                .take_while(|(candidate, _)| candidate != path)
                .cloned()
                .collect();
            let restore_error = restore_directory_permissions(&mut restore);
            return match restore_error {
                Ok(()) => Err(error).with_context(|| {
                    format!("Failed to make stage directory {} writable", path.display())
                }),
                Err(restore_error) => bail!(
                    "Failed to make stage directory {} writable: {error}; \
                     also failed restoring permissions: {restore_error:#}",
                    path.display()
                ),
            };
        }
    }
    Ok(originals)
}

fn create_stage_directories(root: &Path, path: &Path) -> Result<()> {
    let relative = path.strip_prefix(root).with_context(|| {
        format!(
            "Stage directory {} is outside rootfs {}",
            path.display(),
            root.display()
        )
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            bail!("Stage directory path is not normalized");
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => bail!(
                "Stage path component is not a directory: {}",
                current.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).with_context(|| {
                    format!("Failed to create image directory {}", current.display())
                })?;
                set_default_directory_permissions(&current)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_default_directory_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_default_directory_permissions(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn restore_directory_permissions(originals: &mut Vec<(PathBuf, fs::Permissions)>) -> Result<()> {
    let mut errors = Vec::new();
    for (path, permissions) in originals.drain(..).rev() {
        if let Err(error) = fs::set_permissions(&path, permissions) {
            errors.push(format!("{}: {error}", path.display()));
        }
    }
    if !errors.is_empty() {
        bail!("{}", errors.join("; "));
    }
    Ok(())
}

fn set_owner_write(permissions: &mut fs::Permissions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(permissions.mode() | 0o700);
    }
    #[cfg(not(unix))]
    permissions.set_readonly(false);
}

fn copy_directory_contents(
    root: &Path,
    source: &Path,
    destination: &Path,
    line: usize,
) -> Result<()> {
    let source_permissions = fs::metadata(source)
        .with_context(|| format!("Dockerfile line {line}: failed to inspect COPY directory"))?
        .permissions();
    let existing_permissions = match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("COPY destination '{}' is a symlink", destination.display());
        }
        Ok(metadata) if !metadata.is_dir() => {
            bail!(
                "Dockerfile line {line}: COPY directory destination '{}' is not a directory",
                destination.display()
            );
        }
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Dockerfile line {line}: failed to inspect COPY destination '{}'",
                    destination.display()
                )
            })
        }
    };
    if let Some(permissions) = existing_permissions.as_ref() {
        let mut writable = permissions.clone();
        set_owner_write(&mut writable);
        fs::set_permissions(destination, writable).with_context(|| {
            format!(
                "Dockerfile line {line}: failed to make image directory '{}' writable",
                destination.display()
            )
        })?;
    }
    let result = (|| {
        create_stage_directories(root, destination).with_context(|| {
            format!(
                "Dockerfile line {line}: failed to create image directory '{}'",
                destination.display()
            )
        })?;

        let mut children: Vec<_> = fs::read_dir(source)
            .with_context(|| format!("Dockerfile line {line}: failed to read COPY directory"))?
            .collect::<std::io::Result<Vec<_>>>()?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let child_source = child.path();
            let child_target = destination.join(child.file_name());
            let metadata = fs::symlink_metadata(&child_source)?;
            if metadata.file_type().is_symlink() {
                bail!(
                    "Dockerfile line {line}: COPY source '{}' contains a symlink; \
                     symlink sources are unsupported",
                    child_source.display()
                );
            }
            if metadata.is_dir() {
                copy_directory_contents(root, &child_source, &child_target, line)?;
                fs::set_permissions(&child_target, metadata.permissions())?;
            } else if metadata.is_file() {
                copy_file(&child_source, &child_target, line)?;
            } else {
                bail!("Dockerfile line {line}: COPY supports regular files and directories only");
            }
        }
        Ok(())
    })();

    let restore_result = match existing_permissions {
        Some(permissions) => fs::set_permissions(destination, permissions).with_context(|| {
            format!(
                "Dockerfile line {line}: failed restoring image directory '{}' permissions",
                destination.display()
            )
        }),
        None => fs::set_permissions(destination, source_permissions).with_context(|| {
            format!(
                "Dockerfile line {line}: failed to preserve copied directory '{}' permissions",
                destination.display()
            )
        }),
    };
    match (result, restore_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(operation_error), Ok(())) => Err(operation_error),
        (Ok(()), Err(restore_error)) => Err(restore_error),
        (Err(operation_error), Err(restore_error)) => bail!(
            "{operation_error:#}; additionally, failed restoring image directory permissions: \
             {restore_error:#}"
        ),
    }
}

fn copy_file(source: &Path, destination: &Path, line: usize) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(destination) {
        if metadata.file_type().is_symlink() {
            bail!("Dockerfile line {line}: COPY destination is a staged symlink");
        }
        if metadata.is_dir() {
            bail!(
                "Dockerfile line {line}: COPY file destination '{}' is a directory",
                destination.display()
            );
        }
    }
    fs::copy(source, destination).with_context(|| {
        format!(
            "Dockerfile line {line}: failed to copy '{}' into the image filesystem",
            source.display()
        )
    })?;
    fs::set_permissions(destination, fs::metadata(source)?.permissions())?;
    Ok(())
}
