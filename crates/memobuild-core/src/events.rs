use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BuildStatus {
    Queued,
    Building,
    Completed,
    Cached,
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeEvent {
    pub node_id: usize,
    pub name: String,
    pub status: BuildStatus,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BuildEvent {
    BuildStarted {
        total_nodes: usize,
    },
    NodeStarted {
        node_id: usize,
        name: String,
    },
    NodeCompleted {
        node_id: usize,
        name: String,
        duration_ms: u64,
        cache_hit: bool,
    },
    NodeFailed {
        node_id: usize,
        name: String,
        error: String,
    },
    BuildCompleted {
        total_duration_ms: u64,
        cache_hits: usize,
        executed_nodes: usize,
    },
}

pub trait BuildObserver: Send + Sync {
    fn on_event(&self, event: BuildEvent);
}

pub struct NoopObserver;
impl BuildObserver for NoopObserver {
    fn on_event(&self, _event: BuildEvent) {}
}
