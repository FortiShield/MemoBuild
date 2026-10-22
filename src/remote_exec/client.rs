use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;

use crate::remote_exec::{ActionRequest, ActionResult, RemoteExecutor};

pub struct RemoteExecClient {
    pub endpoint: String,
    pub client: Client,
}

impl RemoteExecClient {
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            client: Client::new(),
        }
    }
}

#[async_trait]
impl RemoteExecutor for RemoteExecClient {
    async fn execute(&self, action: ActionRequest) -> Result<ActionResult> {
        let url = format!("{}/execute", self.endpoint);
        println!("📡 [Client] Dispatching action to build farm: {}", url);

        let response = self
            .client
            .post(&url)
            .json(&action)
            .send()
            .await
            .context("Failed to send execution request to build farm")?;

        if !response.status().is_success() {
            let status = response.status();
            let err_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Build farm execution failed: {} - {}", status, err_text);
        }

        let result = response
            .json::<ActionResult>()
            .await
            .context("Failed to parse ActionResult from build farm")?;

        Ok(result)
    }
}
