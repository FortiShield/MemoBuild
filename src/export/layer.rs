use crate::export::utils::sha256_file;
use anyhow::{bail, Context, Result};
use flate2::{Compression, GzBuilder};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use tar::{Builder, EntryType, Header};
use tempfile::NamedTempFile;

pub const OCI_GZIP_LAYER_MEDIA_TYPE: &str = "application/vnd.oci.image.layer.v1.tar+gzip";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerInfo {
    pub digest: String,
    pub size: u64,
    pub diff_id: String,
    pub media_type: String,
}

pub fn create_filesystem_layer(stage_root: &Path, output_dir: &Path) -> Result<LayerInfo> {
    let blobs_dir = output_dir.join("blobs").join("sha256");
    fs::create_dir_all(&blobs_dir)?;
    let uncompressed = NamedTempFile::new_in(output_dir)
        .context("Failed to allocate temporary uncompressed OCI layer")?;
    write_layer_tar(stage_root, uncompressed.path())?;
    let diff_id = format!("sha256:{}", sha256_file(uncompressed.path())?);

    let mut compressed = NamedTempFile::new_in(&blobs_dir)
        .context("Failed to allocate temporary compressed OCI layer")?;
    {
        let target = compressed.as_file_mut();
        let mut encoder = GzBuilder::new()
            .mtime(0)
            .operating_system(255)
            .write(target, Compression::default());
        let mut source = File::open(uncompressed.path())?;
        io::copy(&mut source, &mut encoder)?;
        encoder.finish()?.flush()?;
    }
    let digest_hex = sha256_file(compressed.path())?;
    let digest = format!("sha256:{digest_hex}");
    let blob_path = blobs_dir.join(&digest_hex);
    let size = fs::metadata(compressed.path())?.len();
    compressed.persist(&blob_path)?;

    Ok(LayerInfo {
        digest,
        size,
        diff_id,
        media_type: OCI_GZIP_LAYER_MEDIA_TYPE.to_string(),
    })
}

fn write_layer_tar(stage_root: &Path, output_path: &Path) -> Result<()> {
    let root = fs::canonicalize(stage_root)
        .with_context(|| format!("Failed to resolve staged rootfs {}", stage_root.display()))?;
    if !root.is_dir() {
        bail!("Staged rootfs is not a directory: {}", root.display());
    }

    let output = File::create(output_path)?;
    let mut archive = Builder::new(output);
    archive.mode(tar::HeaderMode::Deterministic);
    let mut entries: Vec<PathBuf> = walkdir::WalkDir::new(&root)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .map(|entry| entry.map(|entry| entry.into_path()))
        .collect::<std::result::Result<_, _>>()
        .context("Failed to enumerate staged rootfs for OCI layer")?;
    entries.sort();

    for path in entries {
        let relative = path.strip_prefix(&root)?;
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("Failed to inspect staged path {}", path.display()))?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            bail!(
                "Staged rootfs contains unsupported symlink {}",
                relative.display()
            );
        }
        let mode = file_mode(&metadata);
        if metadata.is_dir() {
            let mut header = layer_header(0, mode, EntryType::Directory)?;
            archive.append_data(&mut header, relative, io::empty())?;
        } else if metadata.is_file() {
            let mut header = layer_header(metadata.len(), mode, EntryType::Regular)?;
            archive.append_data(&mut header, relative, File::open(&path)?)?;
        } else {
            bail!(
                "Staged rootfs contains unsupported file type at {}",
                relative.display()
            );
        }
    }
    archive.finish()?;
    archive.into_inner()?.sync_all()?;
    Ok(())
}

fn layer_header(size: u64, mode: u32, entry_type: EntryType) -> Result<Header> {
    let mut header = Header::new_gnu();
    header.set_entry_type(entry_type);
    header.set_size(size);
    header.set_mode(mode);
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    header.set_username("")?;
    header.set_groupname("")?;
    header.set_cksum();
    Ok(header)
}

#[cfg(unix)]
fn file_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn file_mode(metadata: &fs::Metadata) -> u32 {
    if metadata.permissions().readonly() {
        0o444
    } else {
        0o644
    }
}

pub(crate) fn sha256_reader(reader: &mut impl Read) -> Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}
