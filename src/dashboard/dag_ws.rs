use std::sync::Arc;

use tokio::sync::broadcast;

use crate::dashboard::metrics::{BuildEvent, BuildObserver};
use crate::remote_cache::RemoteCache;

pub struct BroadcastObserver {
    tx: broadcast::Sender<BuildEvent>,
}

impl BroadcastObserver {
    pub fn new(tx: broadcast::Sender<BuildEvent>) -> Self {
        Self { tx }
    }
}

impl BuildObserver for BroadcastObserver {
    fn on_event(&self, event: BuildEvent) {
        let _ = self.tx.send(event);
    }
}

pub struct RemoteObserver {
    remote: Arc<dyn RemoteCache>,
}

impl RemoteObserver {
    pub fn new(remote: Arc<dyn RemoteCache>) -> Self {
        Self { remote }
    }
}

impl BuildObserver for RemoteObserver {
    fn on_event(&self, event: BuildEvent) {
        let remote = self.remote.clone();
        tokio::spawn(async move {
            let _ = remote.report_build_event(event).await;
        });
    }
}
