use memobuild::graph::{Node, NodeKind, NodeMetadata};
use memobuild::sandbox::bubblewrap::BubblewrapSandbox;
use memobuild::sandbox::Sandbox;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;

fn node(command: &str) -> Node {
    Node {
        id: 0,
        name: "isolation-test".to_string(),
        kind: NodeKind::Run,
        content: command.to_string(),
        hash: "isolation-test".to_string(),
        dirty: true,
        deps: Vec::new(),
        source_path: None,
        env: HashMap::new(),
        cache_hit: false,
        metadata: NodeMetadata::default(),
    }
}

fn sandbox(context: &Path) -> BubblewrapSandbox {
    BubblewrapSandbox::new(context.to_path_buf())
}

fn platform_supported() -> bool {
    cfg!(all(target_os = "linux", target_arch = "x86_64"))
}

fn runtime_available() -> bool {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|path| path.join("bwrap"))
        .any(|path| path.is_file())
}

async fn skip_if_platform_prerequisite_missing(context: &Path) -> bool {
    if !platform_supported() {
        eprintln!("SKIP: Bubblewrap security tests require Linux x86_64");
        return true;
    }
    if !runtime_available() {
        eprintln!("SKIP: Bubblewrap security tests require bubblewrap in PATH");
        return true;
    }
    let sandbox = sandbox(context);
    let node = node("true");
    let result = sandbox.prepare(&node).await;
    match result {
        Ok(env) => {
            sandbox
                .cleanup(env)
                .await
                .expect("cleanup after prerequisite probe");
            false
        }
        Err(error) => {
            let message = format!("{error:#}");
            if message.contains(
                "Bubblewrap could not establish the required isolated namespaces and mounts:",
            ) && (message.contains("Operation not permitted")
                || message.contains("Permission denied")
                || message.contains("No permissions to create a new namespace"))
            {
                eprintln!("SKIP: Bubblewrap namespace prerequisite unavailable: {message}");
                true
            } else {
                panic!("Bubblewrap is present but isolation initialization failed: {message}");
            }
        }
    }
}

fn test_context() -> (TempDir, PathBuf) {
    let context = TempDir::new().expect("create test context");
    let path = context.path().to_path_buf();
    (context, path)
}

#[tokio::test]
async fn s01_build_command_cannot_modify_protected_host_file() {
    let (_context, context_path) = test_context();
    if skip_if_platform_prerequisite_missing(&context_path).await {
        return;
    }
    let protected_dir = TempDir::new().expect("create protected fixture");
    let protected_file = protected_dir.path().join("protected.txt");
    std::fs::write(&protected_file, "host-original").expect("write protected fixture");
    let quoted_path = format!("'{}'", protected_file.display());
    let quoted_parent = format!(
        "'{}'",
        protected_file.parent().expect("protected parent").display()
    );
    let command = format!(
        "mkdir -p {quoted_parent}; \
         printf 'workspace-output' > /workspace/output.txt; \
         printf 'sandbox-write' > {quoted_path}"
    );
    let sandbox = sandbox(&context_path);
    let env = sandbox
        .prepare(&node(&command))
        .await
        .expect("prepare sandbox");
    let result = sandbox
        .execute(&env, &node(&command))
        .await
        .expect("execute");
    assert_eq!(
        result.exit_code,
        0,
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&protected_file).expect("read protected fixture"),
        "host-original"
    );
    assert_eq!(
        std::fs::read_to_string(env.workspace_dir.join("output.txt"))
            .expect("read workspace output"),
        "workspace-output"
    );
    sandbox.cleanup(env).await.expect("cleanup sandbox");
}

#[tokio::test]
async fn s03_missing_isolation_runtime_fails_without_host_shell_fallback() {
    let (_context, context_path) = test_context();
    let marker = context_path.join("host-shell-was-run");
    let sandbox =
        BubblewrapSandbox::with_runtime(context_path, "/definitely/missing/memobuild-bwrap");
    let result = sandbox
        .prepare(&node(&format!("touch '{}'", marker.display())))
        .await;
    assert!(result.is_err(), "missing Bubblewrap must fail closed");
    assert!(!marker.exists(), "host shell must not run as a fallback");
    let message = format!("{:#}", result.expect_err("expected initialization failure"));
    if !platform_supported() {
        assert!(message.contains("Linux x86_64"));
        return;
    }
    assert!(
        message.contains("Bubblewrap is required"),
        "expected actionable runtime error, got: {message}"
    );
}

#[test]
fn cli_rejects_unimplemented_run_before_backend_dispatch() {
    let (context, context_path) = test_context();
    let cache = TempDir::new().expect("create isolated cache");
    let marker = context_path.join("host-shell-was-run");
    std::fs::write(
        context_path.join("Dockerfile"),
        "FROM scratch\nRUN touch host-shell-was-run\n",
    )
    .expect("write Dockerfile");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_memobuild"))
        .arg("build")
        .current_dir(&context)
        .env("MEMOBUILD_CACHE_DIR", cache.path())
        .output()
        .expect("run CLI");
    assert!(!output.status.success(), "unsupported RUN must be rejected");
    assert!(
        !marker.exists(),
        "CLI must not execute an unsupported RUN command"
    );
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        diagnostic.contains("RUN is recognized but unsupported"),
        "expected strict compiler rejection before backend dispatch, got: {diagnostic}"
    );
}

#[tokio::test]
async fn isolated_command_succeeds_and_preserves_nonzero_exit_status() {
    let (_context, context_path) = test_context();
    if skip_if_platform_prerequisite_missing(&context_path).await {
        return;
    }
    std::fs::write(context_path.join("input.txt"), "context-input").expect("write context fixture");
    let readonly_dir = context_path.join("readonly");
    std::fs::create_dir(&readonly_dir).expect("create read-only context directory");
    std::fs::write(readonly_dir.join("input.txt"), "readonly-context")
        .expect("write nested context fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&readonly_dir, std::fs::Permissions::from_mode(0o555))
            .expect("make nested context directory read-only");
    }
    let sandbox = sandbox(&context_path);
    let env = sandbox
        .prepare(&node("cat readonly/input.txt"))
        .await
        .expect("prepare");
    let success = sandbox
        .execute(&env, &node("cat readonly/input.txt"))
        .await
        .expect("execute");
    assert_eq!(success.exit_code, 0);
    assert_eq!(success.stdout, b"readonly-context");
    let host_network_namespace =
        std::fs::read_link("/proc/self/ns/net").expect("read host network namespace");
    let isolated = sandbox
        .execute(&env, &node("readlink /proc/self/ns/net"))
        .await
        .expect("read isolated network namespace");
    assert_eq!(isolated.exit_code, 0);
    assert_ne!(
        String::from_utf8_lossy(&isolated.stdout).trim(),
        host_network_namespace.to_string_lossy()
    );
    sandbox.cleanup(env).await.expect("cleanup");

    let env = sandbox.prepare(&node("exit 37")).await.expect("prepare");
    let failure = sandbox
        .execute(&env, &node("exit 37"))
        .await
        .expect("execute");
    assert_eq!(failure.exit_code, 37);
    sandbox.cleanup(env).await.expect("cleanup");
}

#[tokio::test]
async fn timed_out_command_is_killed_and_sandbox_is_removed() {
    let (_context, context_path) = test_context();
    if skip_if_platform_prerequisite_missing(&context_path).await {
        return;
    }
    let sandbox = sandbox(&context_path).with_timeout(Duration::from_millis(150));
    let env = sandbox
        .prepare(&node("sleep 1; touch late"))
        .await
        .expect("prepare");
    let workspace = env.workspace_dir.clone();
    let result = sandbox.execute(&env, &node("sleep 1; touch late")).await;
    assert!(result.is_err(), "command must hit its configured timeout");
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        !workspace.join("late").exists(),
        "timed-out command must not remain alive"
    );
    sandbox.cleanup(env).await.expect("cleanup after timeout");
    assert!(!workspace.exists(), "sandbox workspace must be removed");
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[tokio::test]
async fn cleanup_failures_are_returned_and_do_not_hide_temp_directory() {
    use std::os::unix::fs::PermissionsExt;

    let (_context, context_path) = test_context();
    if skip_if_platform_prerequisite_missing(&context_path).await {
        return;
    }
    let sandbox = sandbox(&context_path);
    let env = sandbox.prepare(&node("true")).await.expect("prepare");
    let temp_root = env
        .workspace_dir
        .parent()
        .expect("temporary workspace parent")
        .to_path_buf();
    std::fs::set_permissions(&temp_root, std::fs::Permissions::from_mode(0o000))
        .expect("make cleanup fail");
    let cleanup_result = sandbox.cleanup(env).await;
    std::fs::set_permissions(&temp_root, std::fs::Permissions::from_mode(0o700))
        .expect("restore temporary directory permissions");
    if temp_root.exists() {
        std::fs::remove_dir_all(&temp_root).expect("remove restored temporary directory");
    }
    assert!(
        cleanup_result.is_err(),
        "cleanup must surface filesystem removal errors"
    );
}
