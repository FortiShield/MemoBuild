use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::remote_router::region::RegionNode;

pub async fn start_health_service(regions: Vec<Arc<RegionNode>>) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));

    loop {
        interval.tick().await;

        for region in &regions {
            let region_clone = region.clone();
            tokio::spawn(async move {
                let start = Instant::now();
                // Simple health check: check if we can reach the server
                let healthy = region_clone.client.has("ping").await.is_ok();

                let latency = start.elapsed().as_millis() as u64;

                let mut health = region_clone.health.write().await;
                health.healthy = healthy;
                health.latency_ms = latency;
                health.last_seen = Instant::now();
            });
        }
    }
}
