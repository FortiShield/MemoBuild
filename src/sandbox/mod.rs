use crate::graph::Node;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tempfile::TempDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SandboxKind {
    Bubblewrap,
    Local,
    Containerd,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourceLimits {
    pub cpu_shares: Option<u64>,
    pub memory_mb: Option<u64>,
}

#[derive(Debug)]
pub struct SandboxEnv {
    pub workspace_dir: std::path::PathBuf,
    pub env_vars: HashMap<String, String>,
    pub(crate) temp_dir: Option<TempDir>,
}

impl SandboxEnv {
    pub fn new(workspace_dir: std::path::PathBuf, env_vars: HashMap<String, String>) -> Self {
        Self {
            workspace_dir,
            env_vars,
            temp_dir: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[async_trait]
pub trait Sandbox: Send + Sync {
    async fn prepare(&self, node: &Node) -> Result<SandboxEnv>;
    async fn execute(&self, env: &SandboxEnv, node: &Node) -> Result<ExecResult>;
    async fn cleanup(&self, env: SandboxEnv) -> Result<()>;
}

pub mod bubblewrap;
#[cfg(feature = "containerd")]
pub mod containerd;
#[cfg(feature = "unsafe-local-execution")]
pub mod local;
pub mod spec;
