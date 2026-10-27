use crate::export::layer::LayerInfo;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageConfigMetadata {
    pub cmd: Option<Vec<String>>,
    pub entrypoint: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OCIConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    pub architecture: String,
    pub os: String,
    pub config: OCIImageConfig,
    pub rootfs: OCIRootFS,
    pub history: Vec<OCIHistory>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OCIImageConfig {
    #[serde(rename = "Env", skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<String>>,
    #[serde(rename = "Cmd", skip_serializing_if = "Option::is_none")]
    pub cmd: Option<Vec<String>>,
    #[serde(rename = "Entrypoint", skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<Vec<String>>,
    #[serde(rename = "WorkingDir", skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OCIRootFS {
    #[serde(rename = "type")]
    pub fs_type: String,
    pub diff_ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OCIHistory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    pub created_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub empty_layer: Option<bool>,
}

pub fn create_config(
    layers: &[LayerInfo],
    image_metadata: &ImageConfigMetadata,
    reproducible: bool,
) -> OCIConfig {
    let timestamp = if reproducible {
        "1970-01-01T00:00:00Z".to_string()
    } else {
        Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
    };

    OCIConfig {
        created: Some(timestamp.clone()),
        architecture: "amd64".to_string(),
        os: "linux".to_string(),
        config: OCIImageConfig {
            env: None,
            cmd: image_metadata.cmd.clone(),
            entrypoint: image_metadata.entrypoint.clone(),
            working_dir: None,
        },
        rootfs: OCIRootFS {
            fs_type: "layers".to_string(),
            diff_ids: layers.iter().map(|layer| layer.diff_id.clone()).collect(),
        },
        history: layers
            .iter()
            .map(|_| OCIHistory {
                created: Some(timestamp.clone()),
                created_by: "MemoBuild: staged filesystem layer".to_string(),
                empty_layer: Some(false),
            })
            .collect(),
    }
}
