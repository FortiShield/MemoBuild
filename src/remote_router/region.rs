use std::sync::Arc;
use std::time::Instant;

use tokio::sync::RwLock;

use crate::remote_cache::RemoteCache;

#[derive(Debug, Clone)]
pub struct HealthStatus {
    pub latency_ms: u64,
    pub last_seen: Instant,
    pub healthy: bool,
}

impl Default for HealthStatus {
    fn default() -> Self {
        Self {
            latency_ms: 9999,
            last_seen: Instant::now(),
            healthy: true,
        }
    }
}

pub struct RegionNode {
    pub name: String,
    pub endpoint: String,
    pub priority: u8,
    pub weight: u8,
    pub client: Arc<dyn RemoteCache>,
    pub health: Arc<RwLock<HealthStatus>>,
}

impl RegionNode {
    pub fn new(name: &str, endpoint: &str, client: Arc<dyn RemoteCache>) -> Self {
        Self {
            name: name.to_string(),
            endpoint: endpoint.to_string(),
            priority: 1,
            weight: 10,
            client,
            health: Arc::new(RwLock::new(HealthStatus::default())),
        }
    }
}
