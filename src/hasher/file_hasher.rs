use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use anyhow::{Context, Result};
use blake3::Hasher;
use rayon::prelude::*;

use crate::hasher::ignore::IgnoreRules;
use crate::hasher::walker::walk_dir;

/// Chunk size for large-file streaming hashing (64 KB — BLAKE3 optimal)
const CHUNK_SIZE: usize = 64 * 1024;

/// Hash a single file using BLAKE3, reading in 64 KB chunks.
pub fn hash_file(path: &Path) -> Result<String> {
    let file = File::open(path)
        .with_context(|| format!("Cannot open file for hashing: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Hasher::new();
    let mut buf = vec![0u8; CHUNK_SIZE];

    loop {
        let n = reader
            .read(&mut buf)
            .with_context(|| format!("Read error on: {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

/// Hash a directory tree recursively using Rayon for parallel execution.
pub fn hash_dir(root: &Path, ignore: &IgnoreRules) -> Result<String> {
    let files = walk_dir(root, ignore);

    // Fix 2: Parallel hashing of file contents using Rayon
    let results: Result<Vec<(String, String)>> = files
        .par_iter()
        .map(|abs_path| {
            let rel = abs_path.strip_prefix(root).unwrap_or(abs_path.as_path());
            let rel_path_str = rel.to_string_lossy().to_string();
            let file_hash = hash_file(abs_path)?;
            Ok((rel_path_str, file_hash))
        })
        .collect();

    let mut top_hasher = Hasher::new();
    for (rel_path, file_hash) in results? {
        top_hasher.update(rel_path.as_bytes());
        top_hasher.update(file_hash.as_bytes());
    }

    Ok(top_hasher.finalize().to_hex().to_string())
}

/// Dispatch: hash a file or a directory, respecting ignore rules.
pub fn hash_path(path: &Path, ignore: &IgnoreRules) -> Result<String> {
    if path.is_dir() {
        hash_dir(path, ignore)
    } else if path.is_file() {
        hash_file(path)
    } else {
        let mut hasher = Hasher::new();
        hasher.update(path.to_string_lossy().as_bytes());
        Ok(hasher.finalize().to_hex().to_string())
    }
}
