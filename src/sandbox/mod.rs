use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::graph::Node;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SandboxKind {
    Local,
    Containerd,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourceLimits {
    pub cpu_shares: Option<u64>,
    pub memory_mb: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SandboxEnv {
    pub workspace_dir: std::path::PathBuf,
    pub env_vars: HashMap<String, String>,
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
    async fn cleanup(&self, env: &SandboxEnv) -> Result<()>;
}

#[cfg(feature = "containerd")]
pub mod containerd;
pub mod local;
pub mod spec;
