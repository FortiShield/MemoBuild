use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::cache::RemoteCache;
use crate::dashboard::BuildEvent;

pub mod health;
pub mod region;
pub mod router;

pub use region::RegionNode;
pub use router::{CacheRouter, RoutingStrategy};

/// A Meta-Cache that routes requests to multiple regional backends
pub struct RouterRemoteCache {
    pub router: Arc<router::CacheRouter>,
}

#[async_trait]
impl RemoteCache for RouterRemoteCache {
    async fn has(&self, hash: &str) -> Result<bool> {
        self.router.has(hash).await
    }

    async fn get(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        self.router.get(hash).await
    }

    async fn put(&self, hash: &str, data: &[u8]) -> Result<()> {
        self.router.put(hash, data).await
    }

    async fn has_layer(&self, hash: &str) -> Result<bool> {
        self.router.has_layer(hash).await
    }

    async fn get_layer(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        self.router.get_layer(hash).await
    }

    async fn put_layer(&self, hash: &str, data: &[u8]) -> Result<()> {
        self.router.put_layer(hash, data).await
    }

    async fn get_node_layers(&self, hash: &str) -> Result<Option<Vec<String>>> {
        self.router.get_node_layers(hash).await
    }

    async fn register_node_layers(
        &self,
        hash: &str,
        layers: &[String],
        total_size: u64,
    ) -> Result<()> {
        self.router
            .register_node_layers(hash, layers, total_size)
            .await
    }

    async fn report_build_event(&self, event: BuildEvent) -> Result<()> {
        self.router.report_build_event(event).await
    }

    async fn report_dag(&self, dag: &crate::graph::BuildGraph) -> Result<()> {
        self.router.report_dag(dag).await
    }

    async fn report_analytics(&self, dirty: u32, cached: u32, duration_ms: u64) -> Result<()> {
        self.router
            .report_analytics(dirty, cached, duration_ms)
            .await
    }
}
