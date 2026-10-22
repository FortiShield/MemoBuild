use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Extension, Json, Router};

use crate::remote_exec::worker::WorkerNode;
use crate::remote_exec::{ActionRequest, ActionResult};

pub struct WorkerServer {
    worker: Arc<WorkerNode>,
}

impl WorkerServer {
    pub fn new(worker: Arc<WorkerNode>) -> Self {
        Self { worker }
    }

    pub async fn start(self, port: u16) -> Result<()> {
        let worker_id = self.worker.id.clone();
        let app = Router::new()
            .route("/execute", post(handle_worker_execute))
            .layer(Extension(self.worker));

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        println!("👷 Worker Node {} listening on {}", worker_id, addr);

        axum::Server::bind(&addr)
            .serve(app.into_make_service())
            .await
            .map_err(|e| anyhow::anyhow!("Worker server error: {}", e))
    }
}

async fn handle_worker_execute(
    Extension(worker): Extension<Arc<WorkerNode>>,
    Json(action): Json<ActionRequest>,
) -> Result<Json<ActionResult>, (StatusCode, String)> {
    use crate::remote_exec::RemoteExecutor;

    match worker.execute(action).await {
        Ok(result) => Ok(Json(result)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}
