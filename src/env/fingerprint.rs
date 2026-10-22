use std::collections::BTreeMap;
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvFingerprint {
    pub env_vars: BTreeMap<String, String>,
    pub toolchain: BTreeMap<String, String>,
    pub os: String,
    pub arch: String,
}

impl EnvFingerprint {
    pub fn collect() -> Self {
        let mut fingerprint = Self {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            ..Default::default()
        };

        // Collect critical environment variables
        let critical_vars = ["PATH", "RUST_VERSION", "NODE_ENV", "LANG", "LC_ALL"];
        for var in critical_vars {
            if let Ok(val) = std::env::var(var) {
                fingerprint.env_vars.insert(var.to_string(), val);
            }
        }

        // 3. Detect toolchain versions
        fingerprint.detect_toolchains();

        fingerprint
    }

    fn detect_toolchains(&mut self) {
        let tools = [
            ("rustc", vec!["--version"]),
            ("node", vec!["--version"]),
            ("python3", vec!["--version"]),
            ("go", vec!["version"]),
        ];

        for (tool, args) in tools {
            if let Ok(output) = Command::new(tool).args(&args).output() {
                if output.status.success() {
                    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    self.toolchain.insert(tool.to_string(), version);
                }
            }
        }
    }

    pub fn hash(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.os.as_bytes());
        hasher.update(self.arch.as_bytes());

        for (k, v) in &self.env_vars {
            hasher.update(k.as_bytes());
            hasher.update(v.as_bytes());
        }

        for (k, v) in &self.toolchain {
            hasher.update(k.as_bytes());
            hasher.update(v.as_bytes());
        }

        hasher.finalize().to_hex().to_string()
    }
}
