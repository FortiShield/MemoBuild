use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::remote_exec::{ActionRequest, ActionResult, RemoteExecutor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulingStrategy {
    LeastLoaded,
    Random,
    RoundRobin,
    DataLocality,
}

pub struct Scheduler {
    workers: Vec<Arc<dyn RemoteExecutor>>,
    strategy: SchedulingStrategy,
    next_worker: Mutex<usize>,
}

impl Scheduler {
    pub fn new(workers: Vec<Arc<dyn RemoteExecutor>>, strategy: SchedulingStrategy) -> Self {
        Self {
            workers,
            strategy,
            next_worker: Mutex::new(0),
        }
    }

    async fn select_worker(&self, action: &ActionRequest) -> Result<Arc<dyn RemoteExecutor>> {
        if self.workers.is_empty() {
            return Err(anyhow!("No available workers in the build farm"));
        }

        match self.strategy {
            SchedulingStrategy::Random => {
                use rand::Rng;
                let idx = rand::thread_rng().gen_range(0..self.workers.len());
                Ok(self.workers[idx].clone())
            }
            SchedulingStrategy::RoundRobin => {
                let mut next = self.next_worker.lock().await;
                let worker = self.workers[*next].clone();
                *next = (*next + 1) % self.workers.len();
                Ok(worker)
            }
            SchedulingStrategy::LeastLoaded => {
                // For MVP, we fallback to RoundRobin unless we track load
                let mut next = self.next_worker.lock().await;
                let worker = self.workers[*next].clone();
                *next = (*next + 1) % self.workers.len();
                Ok(worker)
            }
            SchedulingStrategy::DataLocality => {
                // Consistent hashing based on input root digest
                let hash_val = blake3::hash(action.input_root_digest.hash.as_bytes());
                let bytes = hash_val.as_bytes();
                let idx = (bytes[0] as usize + ((bytes[1] as usize) << 8)) % self.workers.len();
                Ok(self.workers[idx].clone())
            }
        }
    }
}

#[async_trait]
impl RemoteExecutor for Scheduler {
    async fn execute(&self, action: ActionRequest) -> Result<ActionResult> {
        let worker = self.select_worker(&action).await?;
        worker.execute(action).await
    }
}
