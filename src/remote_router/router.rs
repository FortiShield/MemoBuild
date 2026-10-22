use std::sync::Arc;

use anyhow::Result;

use crate::dashboard::BuildEvent;
use crate::remote_router::region::RegionNode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingStrategy {
    LowestLatency,
    GeoHash,
    RoundRobin,
    Sticky,
}

pub struct CacheRouter {
    pub(crate) regions: Vec<Arc<RegionNode>>,
    pub(crate) strategy: RoutingStrategy,
}

impl CacheRouter {
    pub fn new(regions: Vec<Arc<RegionNode>>, strategy: RoutingStrategy) -> Self {
        Self { regions, strategy }
    }

    // --- Core Routing Logic ---

    pub async fn has(&self, hash: &str) -> Result<bool> {
        let nodes = self.select_nodes_for_read(hash).await;
        for node in nodes {
            if let Ok(true) = node.client.has(hash).await {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn get(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        let nodes = self.select_nodes_for_read(hash).await;
        for node in nodes {
            match node.client.get(hash).await {
                Ok(Some(data)) => {
                    println!("🌐 [Router] Cache HIT from region: {}", node.name);
                    return Ok(Some(data));
                }
                _ => continue,
            }
        }
        Ok(None)
    }

    pub async fn put(&self, hash: &str, data: &[u8]) -> Result<()> {
        let primary = self.select_primary_for_write(hash).await;
        primary.client.put(hash, data).await?;

        let replicas = self.select_replicas_for_write(hash, &primary.name).await;
        for replica in replicas {
            let h = hash.to_string();
            let d = data.to_vec();
            let c = replica.client.clone();
            tokio::spawn(async move {
                let _ = c.put(&h, &d).await;
            });
        }
        Ok(())
    }

    // --- Layer Operations ---

    pub async fn has_layer(&self, hash: &str) -> Result<bool> {
        let nodes = self.select_nodes_for_read(hash).await;
        for node in nodes {
            if let Ok(true) = node.client.has_layer(hash).await {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn get_layer(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        let nodes = self.select_nodes_for_read(hash).await;
        for node in nodes {
            if let Ok(Some(data)) = node.client.get_layer(hash).await {
                return Ok(Some(data));
            }
        }
        Ok(None)
    }

    pub async fn put_layer(&self, hash: &str, data: &[u8]) -> Result<()> {
        let primary = self.select_primary_for_write(hash).await;
        primary.client.put_layer(hash, data).await?;

        let replicas = self.select_replicas_for_write(hash, &primary.name).await;
        for replica in replicas {
            let h = hash.to_string();
            let d = data.to_vec();
            let c = replica.client.clone();
            tokio::spawn(async move {
                let _ = c.put_layer(&h, &d).await;
            });
        }
        Ok(())
    }

    pub async fn get_node_layers(&self, hash: &str) -> Result<Option<Vec<String>>> {
        let nodes = self.select_nodes_for_read(hash).await;
        for node in nodes {
            if let Ok(Some(layers)) = node.client.get_node_layers(hash).await {
                return Ok(Some(layers));
            }
        }
        Ok(None)
    }

    pub async fn register_node_layers(
        &self,
        hash: &str,
        layers: &[String],
        total_size: u64,
    ) -> Result<()> {
        let primary = self.select_primary_for_write(hash).await;
        primary
            .client
            .register_node_layers(hash, layers, total_size)
            .await?;

        let replicas = self.select_replicas_for_write(hash, &primary.name).await;
        for replica in replicas {
            let h = hash.to_string();
            let l = layers.to_vec();
            let s = total_size;
            let c = replica.client.clone();
            tokio::spawn(async move {
                let _ = c.register_node_layers(&h, &l, s).await;
            });
        }
        Ok(())
    }

    // --- Analytics / Events ---

    pub async fn report_build_event(&self, event: BuildEvent) -> Result<()> {
        // Broadcast analytics to all regions or just primary?
        // Usually, a central analytics service is better, but here we broadcast to primary
        let primary = self.select_primary_for_write("").await;
        primary.client.report_build_event(event).await
    }

    pub async fn report_dag(&self, dag: &crate::graph::BuildGraph) -> Result<()> {
        let primary = self.select_primary_for_write("").await;
        primary.client.report_dag(dag).await
    }

    pub async fn report_analytics(&self, dirty: u32, cached: u32, duration_ms: u64) -> Result<()> {
        let primary = self.select_primary_for_write("").await;
        primary
            .client
            .report_analytics(dirty, cached, duration_ms)
            .await
    }

    // --- Selection Logic ---

    async fn select_nodes_for_read(&self, _hash: &str) -> Vec<Arc<RegionNode>> {
        let mut healthy_nodes = Vec::new();
        for region in &self.regions {
            if region.health.read().await.healthy {
                healthy_nodes.push(region.clone());
            }
        }

        // Strategy: Lowest Latency (sort by health stats)
        if self.strategy == RoutingStrategy::LowestLatency {
            // For better performance, we'd pre-sort this in the health service,
            // but for MVP we just return healthy ones in configured order.
        }

        healthy_nodes
    }

    async fn select_primary_for_write(&self, _hash: &str) -> Arc<RegionNode> {
        for region in &self.regions {
            if region.health.read().await.healthy {
                return region.clone();
            }
        }
        self.regions[0].clone()
    }

    async fn select_replicas_for_write(
        &self,
        _hash: &str,
        primary_name: &str,
    ) -> Vec<Arc<RegionNode>> {
        self.regions
            .iter()
            .filter(|r| r.name != primary_name)
            .cloned()
            .collect()
    }
}
