use std::process::Command;

use anyhow::{Context, Result};

/// Fetch the latest commit hash (HEAD) for a remote Git repository.
/// Uses `git ls-remote` which is very fast and doesn't require cloning.
pub fn get_remote_head_hash(url: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["ls-remote", url, "HEAD"])
        .output()
        .with_context(|| format!("Failed to run git ls-remote for {}", url))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git ls-remote failed for {}: {}", url, err);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let hash = stdout
        .split_whitespace()
        .next()
        .context("Could not parse hash from git ls-remote output")?;

    Ok(hash.to_string())
}

/// Clone a repository into a target directory.
pub fn clone_repo(url: &str, target_dir: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["clone", "--depth", "1", url, target_dir])
        .status()
        .with_context(|| format!("Failed to start git clone for {}", url))?;

    if !status.success() {
        anyhow::bail!("git clone failed for {}", url);
    }

    Ok(())
}
