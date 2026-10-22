use std::io::Read;

use anyhow::Result;
use flate2::write::GzEncoder;
use flate2::Compression;
use tar::{Archive, Builder, Header};

pub fn create_reproducible_tar<R: Read>(source: R) -> Result<Vec<u8>> {
    let mut archive = Archive::new(source);

    // Use fixed compression level and no timestamp in gzip header
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());

    {
        let mut builder = Builder::new(&mut encoder);

        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.to_path_buf();

            let mut header = Header::new_gnu();
            header.set_path(&path)?;
            header.set_size(entry.header().size()?);
            header.set_mode(entry.header().mode()?);

            // Normalize metadata
            header.set_mtime(0);
            header.set_uid(0);
            header.set_gid(0);
            header.set_cksum();

            let mut content = Vec::new();
            entry.read_to_end(&mut content)?;

            builder.append(&header, &content[..])?;
        }

        builder.finish()?;
    }

    Ok(encoder.finish()?)
}

pub fn normalize_artifact(data: Vec<u8>) -> Result<Vec<u8>> {
    // If it's a tar/gz, we can re-pack it deterministically
    // For now, let's assume artifacts are blobs.
    Ok(data)
}
