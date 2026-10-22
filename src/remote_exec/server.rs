use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Extension, Json, Router};

use crate::remote_exec::scheduler::Scheduler;
use crate::remote_exec::{ActionRequest, ActionResult};

pub struct ExecutionServer {
    scheduler: Arc<Scheduler>,
}

impl ExecutionServer {
    pub fn new(scheduler: Arc<Scheduler>) -> Self {
        Self { scheduler }
    }

    pub async fn start(self, port: u16) -> Result<()> {
        let app = Router::new()
            .route("/execute", post(handle_execute))
            .layer(Extension(self.scheduler));

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        println!("🚀 Remote Execution Scheduler listening on {}", addr);

        axum::Server::bind(&addr)
            .serve(app.into_make_service())
            .await
            .map_err(|e| anyhow::anyhow!("Server error: {}", e))
    }
}

async fn handle_execute(
    Extension(scheduler): Extension<Arc<Scheduler>>,
    Json(action): Json<ActionRequest>,
) -> Result<Json<ActionResult>, (StatusCode, String)> {
    use crate::remote_exec::RemoteExecutor;

    match scheduler.execute(action).await {
        Ok(result) => Ok(Json(result)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}
