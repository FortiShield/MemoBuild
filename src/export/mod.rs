pub mod config;
pub mod layer;
pub mod manifest;
pub mod oci_exporter;
pub mod registry;
pub mod utils;

pub use oci_exporter::OciExporter;

use crate::export::config::ImageConfigMetadata;
use crate::export::oci_exporter::validate_oci_layout;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::{Builder, TempDir};

pub struct PreparedImage {
    candidate: TempDir,
    destination: PathBuf,
    manifest_digest: String,
}

impl PreparedImage {
    pub fn publish(self) -> Result<PathBuf> {
        match fs::symlink_metadata(&self.destination) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                bail!(
                    "Refusing to reuse non-directory or symlink OCI output {}",
                    self.destination.display()
                );
            }
            Ok(_) => {
                let existing_digest =
                    validate_oci_layout(&self.destination).with_context(|| {
                        format!(
                            "Existing OCI layout is invalid: {}",
                            self.destination.display()
                        )
                    })?;
                if existing_digest != self.manifest_digest {
                    bail!("Existing OCI layout does not match the newly built image digest");
                }
                return Ok(self.destination.clone());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Failed to inspect OCI output path {}",
                        self.destination.display()
                    )
                });
            }
        }
        fs::rename(self.candidate.path(), &self.destination).with_context(|| {
            format!(
                "Failed to atomically publish OCI image layout {}",
                self.destination.display()
            )
        })?;
        Ok(self.destination.clone())
    }
}

pub fn prepare_image(
    stage_root: &Path,
    image_metadata: &ImageConfigMetadata,
    output_root: &Path,
    image_name: &str,
    reproducible: bool,
) -> Result<PreparedImage> {
    ensure_supported_platform()?;
    create_output_directory(output_root)?;

    let image_dir = output_root.join(image_directory_name(image_name));
    create_output_directory(&image_dir)?;
    let candidate = Builder::new()
        .prefix(".oci-candidate-")
        .tempdir_in(&image_dir)
        .with_context(|| {
            format!(
                "Failed to create temporary OCI output in {}",
                image_dir.display()
            )
        })?;
    let exporter = OciExporter::new(candidate.path());
    let manifest_digest =
        exporter.write_layout(stage_root, image_metadata, image_name, reproducible)?;
    let destination = image_dir.join(
        manifest_digest
            .strip_prefix("sha256:")
            .context("Exporter returned a malformed manifest digest")?,
    );

    Ok(PreparedImage {
        candidate,
        destination,
        manifest_digest,
    })
}

pub fn export_image(
    stage_root: &Path,
    image_metadata: &ImageConfigMetadata,
    output_root: &Path,
    image_name: &str,
    reproducible: bool,
) -> Result<PathBuf> {
    prepare_image(
        stage_root,
        image_metadata,
        output_root,
        image_name,
        reproducible,
    )?
    .publish()
}

fn ensure_supported_platform() -> Result<()> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        bail!("The first OCI build target requires a Linux x86_64 host (linux/amd64)");
    }
    Ok(())
}

fn create_output_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "Refusing to write OCI output through symlink {}",
                path.display()
            );
        }
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => bail!("OCI output path is not a directory: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(path)
            .with_context(|| format!("Failed to create OCI output directory {}", path.display())),
        Err(error) => Err(error)
            .with_context(|| format!("Failed to inspect OCI output directory {}", path.display())),
    }
}

fn image_directory_name(image_name: &str) -> String {
    image_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect()
}
