use crate::export::{
    config::{self, ImageConfigMetadata, OCIConfig},
    layer::{self, LayerInfo, OCI_GZIP_LAYER_MEDIA_TYPE},
    manifest::{OCIDescriptor, OCIIndex, OCIManifest, OCIPlatform},
    utils,
};
use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};
use tar::Archive;

const OCI_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const OCI_CONFIG_MEDIA_TYPE: &str = "application/vnd.oci.image.config.v1+json";
const OCI_INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";

pub struct OciExporter {
    output_dir: PathBuf,
}

impl OciExporter {
    pub fn new<P: AsRef<Path>>(output_dir: P) -> Self {
        Self {
            output_dir: output_dir.as_ref().to_path_buf(),
        }
    }

    pub fn write_layout(
        &self,
        stage_root: &Path,
        image_metadata: &ImageConfigMetadata,
        reference_name: &str,
        reproducible: bool,
    ) -> Result<String> {
        fs::create_dir_all(&self.output_dir)?;
        let blobs_dir = self.output_dir.join("blobs").join("sha256");
        fs::create_dir_all(&blobs_dir)?;

        let layer = layer::create_filesystem_layer(stage_root, &self.output_dir)?;
        let oci_config =
            config::create_config(std::slice::from_ref(&layer), image_metadata, reproducible);
        let config_json = serde_json::to_vec(&oci_config)?;
        let config_descriptor = write_blob(&blobs_dir, OCI_CONFIG_MEDIA_TYPE, &config_json)?;

        let manifest = OCIManifest {
            schema_version: 2,
            media_type: OCI_MANIFEST_MEDIA_TYPE.to_string(),
            config: config_descriptor,
            layers: vec![layer_descriptor(&layer)],
        };
        let manifest_json = serde_json::to_vec(&manifest)?;
        let manifest_digest = format!("sha256:{}", utils::sha256_bytes(&manifest_json));
        let manifest_descriptor = OCIDescriptor {
            media_type: OCI_MANIFEST_MEDIA_TYPE.to_string(),
            digest: manifest_digest.clone(),
            size: manifest_json.len() as u64,
            platform: None,
            annotations: None,
        };
        fs::write(blob_path(&blobs_dir, &manifest_digest)?, &manifest_json)?;

        let mut annotations = BTreeMap::new();
        annotations.insert(
            "org.opencontainers.image.ref.name".to_string(),
            reference_name.to_string(),
        );
        let index = OCIIndex {
            schema_version: 2,
            media_type: Some(OCI_INDEX_MEDIA_TYPE.to_string()),
            manifests: vec![OCIDescriptor {
                media_type: manifest_descriptor.media_type,
                digest: manifest_descriptor.digest,
                size: manifest_descriptor.size,
                platform: Some(OCIPlatform {
                    architecture: "amd64".to_string(),
                    os: "linux".to_string(),
                }),
                annotations: Some(annotations),
            }],
        };
        fs::write(
            self.output_dir.join("index.json"),
            serde_json::to_vec(&index)?,
        )?;
        fs::write(
            self.output_dir.join("oci-layout"),
            br#"{"imageLayoutVersion":"1.0.0"}"#,
        )?;

        let verified_digest = validate_oci_layout(&self.output_dir)?;
        if verified_digest != manifest_digest {
            bail!("OCI layout validation returned an unexpected manifest digest");
        }
        Ok(manifest_digest)
    }
}

fn layer_descriptor(layer: &LayerInfo) -> OCIDescriptor {
    OCIDescriptor {
        media_type: layer.media_type.clone(),
        digest: layer.digest.clone(),
        size: layer.size,
        platform: None,
        annotations: None,
    }
}

fn write_blob(blobs_dir: &Path, media_type: &str, bytes: &[u8]) -> Result<OCIDescriptor> {
    let digest = format!("sha256:{}", utils::sha256_bytes(bytes));
    fs::write(blob_path(blobs_dir, &digest)?, bytes)?;
    Ok(OCIDescriptor {
        media_type: media_type.to_string(),
        digest,
        size: bytes.len() as u64,
        platform: None,
        annotations: None,
    })
}

pub fn validate_oci_layout(layout_dir: &Path) -> Result<String> {
    let layout: Value = serde_json::from_slice(
        &fs::read(layout_dir.join("oci-layout")).context("OCI layout declaration is missing")?,
    )?;
    if layout["imageLayoutVersion"] != "1.0.0" {
        bail!("Unsupported OCI image layout version");
    }

    let index: OCIIndex = serde_json::from_slice(
        &fs::read(layout_dir.join("index.json")).context("OCI index.json is missing")?,
    )?;
    if index.schema_version != 2 || index.media_type.as_deref() != Some(OCI_INDEX_MEDIA_TYPE) {
        bail!("OCI index has an unsupported schema or media type");
    }
    if index.manifests.len() != 1 {
        bail!("This first-milestone OCI layout must contain exactly one image manifest");
    }
    let descriptor = &index.manifests[0];
    if descriptor.media_type != OCI_MANIFEST_MEDIA_TYPE
        || descriptor.platform.as_ref()
            != Some(&OCIPlatform {
                architecture: "amd64".to_string(),
                os: "linux".to_string(),
            })
    {
        bail!("OCI index descriptor must identify a Linux/amd64 image manifest");
    }
    let manifest_bytes = read_verified_blob(layout_dir, descriptor)?;
    let manifest: OCIManifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest.schema_version != 2 || manifest.media_type != OCI_MANIFEST_MEDIA_TYPE {
        bail!("OCI image manifest has an unsupported schema or media type");
    }
    if manifest.config.media_type != OCI_CONFIG_MEDIA_TYPE {
        bail!("OCI image config descriptor has an unexpected media type");
    }
    let config_bytes = read_verified_blob(layout_dir, &manifest.config)?;
    let image_config: OCIConfig = serde_json::from_slice(&config_bytes)?;
    if image_config.os != "linux" || image_config.architecture != "amd64" {
        bail!("OCI image config platform must be Linux/amd64");
    }
    if image_config.rootfs.fs_type != "layers"
        || image_config.rootfs.diff_ids.len() != manifest.layers.len()
    {
        bail!("OCI config rootfs DiffIDs do not match manifest layer count");
    }

    for (layer_descriptor, diff_id) in manifest.layers.iter().zip(&image_config.rootfs.diff_ids) {
        if layer_descriptor.media_type != OCI_GZIP_LAYER_MEDIA_TYPE {
            bail!(
                "Unsupported OCI layer media type '{}'",
                layer_descriptor.media_type
            );
        }
        let layer_path = verified_blob_path(layout_dir, layer_descriptor)?;
        let mut decoder = GzDecoder::new(File::open(&layer_path)?);
        let actual_diff_id = format!("sha256:{}", layer::sha256_reader(&mut decoder)?);
        if &actual_diff_id != diff_id {
            bail!("OCI layer DiffID does not match the uncompressed tar payload");
        }

        let decoder = GzDecoder::new(File::open(&layer_path)?);
        let mut archive = Archive::new(decoder);
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.into_owned();
            if path.is_absolute()
                || path.components().any(|component| {
                    matches!(
                        component,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                })
            {
                bail!("OCI layer contains a path outside the image rootfs");
            }
            let entry_type = entry.header().entry_type();
            if !entry_type.is_file() && !entry_type.is_dir() {
                bail!("OCI layer contains an unsupported filesystem entry type");
            }
            io::copy(&mut entry, &mut io::sink())?;
        }
    }
    Ok(descriptor.digest.clone())
}

fn read_verified_blob(layout_dir: &Path, descriptor: &OCIDescriptor) -> Result<Vec<u8>> {
    let path = verified_blob_path(layout_dir, descriptor)?;
    fs::read(path).with_context(|| format!("Failed to read OCI blob {}", descriptor.digest))
}

fn verified_blob_path(layout_dir: &Path, descriptor: &OCIDescriptor) -> Result<PathBuf> {
    let path = blob_path(&layout_dir.join("blobs").join("sha256"), &descriptor.digest)?;
    let metadata = fs::metadata(&path)
        .with_context(|| format!("OCI blob is missing: {}", descriptor.digest))?;
    if metadata.len() != descriptor.size {
        bail!(
            "OCI blob size does not match descriptor {}",
            descriptor.digest
        );
    }
    if utils::sha256_file(&path)? != descriptor.digest.trim_start_matches("sha256:") {
        bail!(
            "OCI blob digest does not match descriptor {}",
            descriptor.digest
        );
    }
    Ok(path)
}

fn blob_path(blobs_dir: &Path, digest: &str) -> Result<PathBuf> {
    let hex = digest
        .strip_prefix("sha256:")
        .context("OCI blob digest must use sha256")?;
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("Invalid OCI SHA-256 digest '{digest}'");
    }
    Ok(blobs_dir.join(hex))
}
