use crate::docker::dag::{CompiledBuild, CopyOperation};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tar::{Builder, EntryType, Header};
use tempfile::{NamedTempFile, TempDir};

const FORMAT_VERSION: u32 = 2;
const COMPILER_SCHEMA_VERSION: u32 = 1;
const SNAPSHOT_FORMAT_VERSION: u32 = 1;
const EXPORT_FORMAT_VERSION: u32 = 1;
const KEY_PREFIX: &str = "v2:blake3:";
const CACHE_NAMESPACE: &str = "build-snapshots-v2";
pub const DEFAULT_CACHE_LIMIT_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuildCacheKeyOptions {
    pub target_os: String,
    pub target_architecture: String,
    pub reproducible: bool,
    pub image_reference: String,
}

impl Default for BuildCacheKeyOptions {
    fn default() -> Self {
        Self {
            target_os: "linux".to_string(),
            target_architecture: "amd64".to_string(),
            reproducible: false,
            image_reference: "memobuild-demo:latest".to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
struct BuildKeyInput<'a> {
    cache_format_version: u32,
    compiler_version: &'static str,
    compiler_schema_version: u32,
    snapshot_format_version: u32,
    export_format_version: u32,
    target_os: &'a str,
    target_architecture: &'a str,
    reproducible: bool,
    image_reference: &'a str,
    base: &'static str,
    operations: Vec<CompiledOperationKey>,
    copies: Vec<CopyKeyInput>,
    cmd: &'a Option<Vec<String>>,
    entrypoint: &'a Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct CompiledOperationKey {
    kind: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct CopyKeyInput {
    source: String,
    destination: String,
    destination_is_directory: bool,
    entries: Vec<SourceEntry>,
}

#[derive(Debug, Serialize)]
struct SourceEntry {
    path: String,
    kind: &'static str,
    mode: u32,
    size: u64,
    content_blake3: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SnapshotRecord {
    format_version: u32,
    build_key: String,
    snapshot_digest: String,
    snapshot_size: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStatistics {
    pub hits: u64,
    pub misses: u64,
    pub key_mismatches: u64,
    pub snapshot_verification_failures: u64,
    pub restored_bytes: u64,
    pub snapshot_writes: u64,
    pub evictions: u64,
    pub rebuilds_after_cache_failure: u64,
    pub quota_refusals: u64,
    pub storage_usage_bytes: u64,
    pub storage_limit_bytes: u64,
    pub garbage_collection_failures: u64,
    pub orphan_files_removed: u64,
}

#[derive(Default)]
struct CacheCounters {
    hits: AtomicU64,
    misses: AtomicU64,
    key_mismatches: AtomicU64,
    snapshot_verification_failures: AtomicU64,
    restored_bytes: AtomicU64,
    snapshot_writes: AtomicU64,
    quota_refusals: AtomicU64,
    storage_usage_bytes: AtomicU64,
    evictions: AtomicU64,
    garbage_collection_failures: AtomicU64,
    orphan_files_removed: AtomicU64,
}

pub struct BuildSnapshotCache {
    root: PathBuf,
    entries: PathBuf,
    blobs: PathBuf,
    lock_path: PathBuf,
    max_bytes: u64,
    counters: CacheCounters,
}

impl BuildSnapshotCache {
    pub fn new(cache_root: impl AsRef<Path>) -> Result<Self> {
        Self::with_limit(cache_root, DEFAULT_CACHE_LIMIT_BYTES)
    }

    pub fn with_limit(cache_root: impl AsRef<Path>, max_bytes: u64) -> Result<Self> {
        let requested_root = cache_root.as_ref();
        let cache_root = if requested_root.is_absolute() {
            requested_root.to_path_buf()
        } else {
            std::env::current_dir()?.join(requested_root)
        };
        create_directory_chain_without_symlinks(&cache_root)?;
        let root = cache_root.join(CACHE_NAMESPACE);
        let entries = root.join("entries");
        let blobs = root.join("blobs").join("sha256");
        create_cache_directory(&root)?;
        create_cache_directory(&entries)?;
        create_cache_directory(&root.join("blobs"))?;
        create_cache_directory(&blobs)?;
        let cache = Self {
            lock_path: root.join(".lock"),
            root,
            entries,
            blobs,
            max_bytes,
            counters: CacheCounters::default(),
        };
        cache.collect_garbage()?;
        cache.refresh_usage()?;
        Ok(cache)
    }

    pub fn key_for_build(plan: &CompiledBuild) -> Result<String> {
        Self::key_for_build_with_options(plan, &BuildCacheKeyOptions::default())
    }

    pub fn key_for_build_with_options(
        plan: &CompiledBuild,
        options: &BuildCacheKeyOptions,
    ) -> Result<String> {
        let context_root = fs::canonicalize(&plan.context_root).with_context(|| {
            format!(
                "Failed to resolve build context {} while computing cache key",
                plan.context_root.display()
            )
        })?;
        let copies = plan
            .copies
            .iter()
            .map(|operation| copy_key_input(&context_root, operation))
            .collect::<Result<Vec<_>>>()?;
        let operations = plan
            .graph
            .nodes
            .iter()
            .map(|node| CompiledOperationKey {
                kind: node.name.clone(),
                content: node.content.clone(),
            })
            .collect();
        let input = BuildKeyInput {
            cache_format_version: FORMAT_VERSION,
            compiler_version: env!("CARGO_PKG_VERSION"),
            compiler_schema_version: COMPILER_SCHEMA_VERSION,
            snapshot_format_version: SNAPSHOT_FORMAT_VERSION,
            export_format_version: EXPORT_FORMAT_VERSION,
            target_os: &options.target_os,
            target_architecture: &options.target_architecture,
            reproducible: options.reproducible,
            image_reference: &options.image_reference,
            base: "scratch",
            operations,
            copies,
            cmd: &plan.cmd,
            entrypoint: &plan.entrypoint,
        };
        let serialized = serde_json::to_vec(&input)?;
        let mut hasher = blake3::Hasher::new_derive_key("MemoBuild build-result cache key v2");
        hasher.update(&serialized);
        Ok(format!("{KEY_PREFIX}{}", hasher.finalize().to_hex()))
    }

    pub fn restore(&self, plan: &CompiledBuild, rootfs: &Path) -> Result<bool> {
        self.restore_with_options(plan, rootfs, &BuildCacheKeyOptions::default())
    }

    pub fn restore_with_options(
        &self,
        plan: &CompiledBuild,
        rootfs: &Path,
        options: &BuildCacheKeyOptions,
    ) -> Result<bool> {
        let _lock = self.acquire_lock()?;
        let build_key = Self::key_for_build_with_options(plan, options)?;
        let entry_path = self.entry_path(&build_key)?;
        let record_bytes = match read_regular_file_if_present(&entry_path) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                self.counters.misses.fetch_add(1, Ordering::Relaxed);
                return Ok(false);
            }
            Err(error) => {
                self.counters
                    .snapshot_verification_failures
                    .fetch_add(1, Ordering::Relaxed);
                return Err(error);
            }
        };
        let record: SnapshotRecord = match serde_json::from_slice(&record_bytes)
            .context("Build snapshot cache entry is malformed")
        {
            Ok(record) => record,
            Err(error) => {
                self.counters
                    .snapshot_verification_failures
                    .fetch_add(1, Ordering::Relaxed);
                return Err(error);
            }
        };
        if record.format_version != FORMAT_VERSION {
            self.counters
                .snapshot_verification_failures
                .fetch_add(1, Ordering::Relaxed);
            bail!("Build snapshot cache entry has an unsupported format version");
        }
        if record.build_key != build_key {
            self.counters.key_mismatches.fetch_add(1, Ordering::Relaxed);
            self.counters
                .snapshot_verification_failures
                .fetch_add(1, Ordering::Relaxed);
            bail!("Build snapshot cache entry has a key mismatch");
        }
        let blob_path = match self.blob_path(&record.snapshot_digest) {
            Ok(path) => path,
            Err(error) => {
                self.counters
                    .snapshot_verification_failures
                    .fetch_add(1, Ordering::Relaxed);
                return Err(error);
            }
        };
        let restored_bytes = (|| {
            let mut snapshot = open_regular_file(&blob_path).with_context(|| {
                format!(
                    "Build snapshot cache entry references missing blob or unsafe cache object {}",
                    record.snapshot_digest
                )
            })?;
            if snapshot.metadata()?.len() != record.snapshot_size {
                bail!(
                    "Build snapshot cache blob size mismatch for {}",
                    record.snapshot_digest
                );
            }
            let actual_digest = sha256_reader(&mut snapshot)?;
            if actual_digest != record.snapshot_digest {
                bail!(
                    "Build snapshot cache blob digest mismatch for {}",
                    record.snapshot_digest
                );
            }
            snapshot.seek(SeekFrom::Start(0))?;
            restore_snapshot(snapshot, rootfs)
        })();
        let restored_bytes = match restored_bytes {
            Ok(bytes) => bytes,
            Err(error) => {
                self.counters
                    .snapshot_verification_failures
                    .fetch_add(1, Ordering::Relaxed);
                return Err(error);
            }
        };
        self.counters.hits.fetch_add(1, Ordering::Relaxed);
        self.counters
            .restored_bytes
            .fetch_add(restored_bytes, Ordering::Relaxed);
        Ok(true)
    }

    pub fn store(&self, plan: &CompiledBuild, rootfs: &Path) -> Result<String> {
        self.store_with_options(plan, rootfs, &BuildCacheKeyOptions::default())
    }

    pub fn store_with_options(
        &self,
        plan: &CompiledBuild,
        rootfs: &Path,
        options: &BuildCacheKeyOptions,
    ) -> Result<String> {
        let _lock = self.acquire_lock()?;
        let build_key = Self::key_for_build_with_options(plan, options)?;
        self.collect_garbage_locked(None)?;
        let (snapshot_digest, snapshot_size) = self.store_snapshot(rootfs)?;

        let after_key = Self::key_for_build_with_options(plan, options)?;
        if after_key != build_key {
            bail!("Build context changed while snapshotting; cache entry was not published");
        }

        let record = SnapshotRecord {
            format_version: FORMAT_VERSION,
            build_key: build_key.clone(),
            snapshot_digest: snapshot_digest.clone(),
            snapshot_size,
        };
        let bytes = serde_json::to_vec(&record)?;
        self.make_space_for(bytes.len() as u64, Some(&build_key), Some(&snapshot_digest))?;
        let entry_path = self.entry_path(&build_key)?;
        let mut temporary = NamedTempFile::new_in(&self.entries)
            .context("Failed to create temporary build cache entry")?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        cache_test_crash_point("entry_file_synced");
        match temporary.persist_noclobber(&entry_path) {
            Ok(_) => {}
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                let existing = read_regular_file_if_present(&entry_path)?
                    .context("Concurrent cache entry disappeared during publication")?;
                if existing != bytes {
                    bail!("Concurrent build produced a different snapshot for the same cache key");
                }
            }
            Err(error) => {
                return Err(error.error)
                    .with_context(|| format!("Failed to publish build cache entry {build_key}"));
            }
        }
        cache_test_crash_point("entry_published");
        sync_directory(&self.entries)?;
        cache_test_crash_point("entry_directory_synced");
        self.counters
            .snapshot_writes
            .fetch_add(1, Ordering::Relaxed);
        self.refresh_usage()?;
        Ok(build_key)
    }

    fn store_snapshot(&self, rootfs: &Path) -> Result<(String, u64)> {
        let temporary = loop {
            let current_usage = self.refresh_usage()?;
            let available = self.max_bytes.saturating_sub(current_usage);
            let mut temporary = NamedTempFile::new_in(&self.blobs)
                .context("Failed to create temporary filesystem snapshot")?;
            cache_test_crash_point("snapshot_temp_created");
            let mut limited = QuotaWriter::new(temporary.as_file_mut(), available);
            match write_snapshot_tar(rootfs, &mut limited) {
                Ok(()) => {
                    cache_test_crash_point("snapshot_serialized");
                    break temporary;
                }
                Err(error) if is_cache_quota_error(&error) => {
                    self.counters.quota_refusals.fetch_add(1, Ordering::Relaxed);
                    drop(temporary);
                    if !self.evict_one_entry(None)? {
                        return Err(error).context(format!(
                            "Build snapshot cache quota exceeded ({} bytes configured)",
                            self.max_bytes
                        ));
                    }
                }
                Err(error) => return Err(error),
            }
        };
        temporary.as_file().sync_all()?;
        cache_test_crash_point("snapshot_file_synced");
        let snapshot_size = temporary.as_file().metadata()?.len();
        let snapshot_digest = sha256_file(temporary.path())?;
        let mut verification_file = open_regular_file(temporary.path())?;
        if sha256_reader(&mut verification_file)? != snapshot_digest {
            bail!("Newly written build snapshot failed digest verification");
        }
        verification_file.seek(SeekFrom::Start(0))?;
        validate_snapshot_archive(&mut verification_file)?;
        let blob_path = self.blob_path(&snapshot_digest)?;
        let newly_published = match temporary.persist_noclobber(&blob_path) {
            Ok(_) => true,
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                let existing = fs::symlink_metadata(&blob_path)?;
                if !existing.is_file()
                    || existing.file_type().is_symlink()
                    || existing.len() != snapshot_size
                    || sha256_file(&blob_path)? != snapshot_digest
                {
                    bail!(
                        "Existing build snapshot blob failed integrity checks: {}",
                        blob_path.display()
                    );
                }
                false
            }
            Err(error) => {
                return Err(error.error)
                    .with_context(|| format!("Failed to publish snapshot blob {snapshot_digest}"));
            }
        };
        if newly_published {
            cache_test_crash_point("blob_published");
        }
        sync_directory(&self.blobs)?;
        if newly_published {
            cache_test_crash_point("blob_directory_synced");
        }
        self.refresh_usage()?;
        Ok((snapshot_digest, snapshot_size))
    }

    fn entry_path(&self, build_key: &str) -> Result<PathBuf> {
        validate_build_key(build_key)?;
        Ok(self
            .entries
            .join(format!("{}.json", build_key.replace(':', "-"))))
    }

    fn blob_path(&self, digest: &str) -> Result<PathBuf> {
        validate_digest(digest)?;
        Ok(self.blobs.join(digest))
    }

    fn acquire_lock(&self) -> Result<CacheLock> {
        CacheLock::acquire(&self.lock_path)
    }

    fn refresh_usage(&self) -> Result<u64> {
        let usage = directory_usage(&self.root)?;
        self.counters
            .storage_usage_bytes
            .store(usage, Ordering::Relaxed);
        Ok(usage)
    }

    fn make_space_for(
        &self,
        required: u64,
        protected_key: Option<&str>,
        protected_digest: Option<&str>,
    ) -> Result<()> {
        self.collect_garbage_locked(protected_digest)?;
        loop {
            let usage = self.refresh_usage()?;
            if usage
                .checked_add(required)
                .is_some_and(|total| total <= self.max_bytes)
            {
                return Ok(());
            }
            if !self.evict_one_entry(protected_key)? {
                self.counters.quota_refusals.fetch_add(1, Ordering::Relaxed);
                bail!(
                    "Build snapshot cache quota exceeded: {usage} bytes used, \
                     {required} additional bytes required, {} byte limit",
                    self.max_bytes
                );
            }
        }
    }

    fn collect_garbage_locked(&self, protected_digest: Option<&str>) -> Result<()> {
        let result = self.collect_garbage_inner(protected_digest);
        if result.is_err() {
            self.counters
                .garbage_collection_failures
                .fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    fn collect_garbage_inner(&self, protected_digest: Option<&str>) -> Result<()> {
        let mut referenced = HashSet::new();

        for item in fs::read_dir(&self.entries)? {
            let path = item?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                bail!(
                    "Unexpected non-regular object in cache entries: {}",
                    path.display()
                );
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if name.starts_with(".tmp") {
                fs::remove_file(&path)?;
                self.counters
                    .orphan_files_removed
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let entry_bytes = match read_regular_file_if_present(&path)? {
                Some(bytes) => bytes,
                None => continue,
            };
            let record = serde_json::from_slice::<SnapshotRecord>(&entry_bytes);
            let valid = record.ok().filter(|record| {
                record.format_version == FORMAT_VERSION
                    && validate_build_key(&record.build_key).is_ok()
                    && validate_digest(&record.snapshot_digest).is_ok()
                    && self
                        .entry_path(&record.build_key)
                        .is_ok_and(|expected| expected == path)
            });
            if let Some(record) = valid {
                referenced.insert(record.snapshot_digest.clone());
            } else {
                fs::remove_file(&path)?;
                self.counters
                    .orphan_files_removed
                    .fetch_add(1, Ordering::Relaxed);
            }
        }

        for item in fs::read_dir(&self.blobs)? {
            let path = item?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                bail!(
                    "Unexpected non-regular object in cache blobs: {}",
                    path.display()
                );
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if name.starts_with(".tmp")
                || validate_digest(name).is_err()
                || (!referenced.contains(name) && Some(name) != protected_digest)
            {
                fs::remove_file(&path)?;
                self.counters
                    .orphan_files_removed
                    .fetch_add(1, Ordering::Relaxed);
            }
        }

        sync_directory(&self.entries)?;
        sync_directory(&self.blobs)?;

        Ok(())
    }

    fn evict_one_entry(&self, protected_key: Option<&str>) -> Result<bool> {
        let mut candidates = fs::read_dir(&self.entries)?
            .map(|item| item.map(|item| item.path()))
            .collect::<io::Result<Vec<_>>>()?;
        candidates.sort();
        for path in candidates {
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                bail!(
                    "Unexpected non-regular object in cache entries: {}",
                    path.display()
                );
            }
            let Some(bytes) = read_regular_file_if_present(&path)? else {
                continue;
            };
            let Ok(record) = serde_json::from_slice::<SnapshotRecord>(&bytes) else {
                continue;
            };
            if record.format_version != FORMAT_VERSION
                || validate_build_key(&record.build_key).is_err()
                || self.entry_path(&record.build_key)? != path
                || protected_key == Some(record.build_key.as_str())
            {
                continue;
            }
            fs::remove_file(&path)?;
            self.counters.evictions.fetch_add(1, Ordering::Relaxed);
            self.collect_garbage_inner(None)?;
            sync_directory(&self.entries)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn cache_directory(&self) -> &Path {
        &self.root
    }

    pub fn collect_garbage(&self) -> Result<()> {
        let _lock = self.acquire_lock()?;
        self.make_space_for(0, None, None)
    }

    pub fn statistics(&self) -> CacheStatistics {
        CacheStatistics {
            hits: self.counters.hits.load(Ordering::Relaxed),
            misses: self.counters.misses.load(Ordering::Relaxed),
            key_mismatches: self.counters.key_mismatches.load(Ordering::Relaxed),
            snapshot_verification_failures: self
                .counters
                .snapshot_verification_failures
                .load(Ordering::Relaxed),
            restored_bytes: self.counters.restored_bytes.load(Ordering::Relaxed),
            snapshot_writes: self.counters.snapshot_writes.load(Ordering::Relaxed),
            evictions: self.counters.evictions.load(Ordering::Relaxed),
            rebuilds_after_cache_failure: 0,
            quota_refusals: self.counters.quota_refusals.load(Ordering::Relaxed),
            storage_usage_bytes: self.counters.storage_usage_bytes.load(Ordering::Relaxed),
            storage_limit_bytes: self.max_bytes,
            garbage_collection_failures: self
                .counters
                .garbage_collection_failures
                .load(Ordering::Relaxed),
            orphan_files_removed: self.counters.orphan_files_removed.load(Ordering::Relaxed),
        }
    }

    pub fn try_statistics(&self) -> Result<CacheStatistics> {
        let _lock = self.acquire_lock()?;
        self.refresh_usage()?;
        Ok(self.statistics())
    }
}

#[derive(Debug)]
struct CacheQuotaExceeded;

impl std::fmt::Display for CacheQuotaExceeded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("cache quota exceeded while serializing snapshot")
    }
}

impl std::error::Error for CacheQuotaExceeded {}

fn is_cache_quota_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<CacheQuotaExceeded>().is_some()
            || cause
                .downcast_ref::<io::Error>()
                .and_then(io::Error::get_ref)
                .and_then(|source| source.downcast_ref::<CacheQuotaExceeded>())
                .is_some()
    })
}

struct QuotaWriter<'a> {
    file: &'a mut File,
    remaining: u64,
}

impl<'a> QuotaWriter<'a> {
    fn new(file: &'a mut File, remaining: u64) -> Self {
        Self { file, remaining }
    }
}

impl Write for QuotaWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::other(CacheQuotaExceeded));
        }
        let remaining = usize::try_from(self.remaining).unwrap_or(usize::MAX);
        let allowed = bytes.len().min(remaining);
        let written = self.file.write(&bytes[..allowed])?;
        self.remaining -= written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

struct CacheLock {
    file: File,
}

impl CacheLock {
    fn acquire(path: &Path) -> Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
            options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        #[cfg(windows)]
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if metadata.file_type().is_symlink() {
                bail!("Cache coordination lock must not be a symlink");
            }
        }
        let file = options.open(path).with_context(|| {
            format!("Failed opening cache coordination lock {}", path.display())
        })?;
        if !file.metadata()?.is_file() {
            bail!("Cache coordination lock is not a regular file");
        }
        fs2::FileExt::lock_exclusive(&file).context("Failed acquiring cache coordination lock")?;
        Ok(Self { file })
    }
}

impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

fn directory_usage(root: &Path) -> Result<u64> {
    let mut total = 0u64;
    for item in walkdir::WalkDir::new(root).follow_links(false) {
        let item = item.context("Failed scanning snapshot cache storage usage")?;
        let metadata = fs::symlink_metadata(item.path())?;
        if metadata.file_type().is_symlink() {
            bail!(
                "Symlink found inside build snapshot cache: {}",
                item.path().display()
            );
        }
        if metadata.is_file() {
            total = total
                .checked_add(metadata.len())
                .context("Build snapshot cache usage overflow")?;
        } else if !metadata.is_dir() {
            bail!(
                "Unsupported object found inside build snapshot cache: {}",
                item.path().display()
            );
        }
    }
    Ok(total)
}

#[cfg(test)]
fn cache_test_crash_point(point: &str) {
    if std::env::var("MEMOBUILD_CACHE_TEST_CRASH").as_deref() == Ok(point) {
        std::process::exit(86);
    }
}

#[cfg(not(test))]
fn cache_test_crash_point(_point: &str) {}

fn copy_key_input(context_root: &Path, operation: &CopyOperation) -> Result<CopyKeyInput> {
    if operation.source.is_absolute()
        || operation.source.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("Cache key COPY source must be relative to the build context");
    }
    let source_path = context_root.join(&operation.source);
    reject_source_symlinks(context_root, &operation.source)?;
    let canonical = fs::canonicalize(&source_path).with_context(|| {
        format!(
            "COPY source '{}' is missing while computing cache key",
            operation.source.display()
        )
    })?;
    if !canonical.starts_with(context_root) {
        bail!(
            "COPY source '{}' resolves outside the build context",
            operation.source.display()
        );
    }
    let mut entries = Vec::new();
    let metadata = fs::symlink_metadata(&canonical)?;
    if metadata.file_type().is_symlink() {
        bail!("Symlink COPY sources are unsupported by the build cache");
    }
    append_source_entry(
        &canonical,
        &operation.source,
        &operation.source,
        &metadata,
        &mut entries,
    )?;
    if metadata.is_dir() {
        for entry in walkdir::WalkDir::new(&canonical)
            .min_depth(1)
            .follow_links(false)
        {
            let entry = entry.context("Failed to enumerate COPY source for cache key")?;
            let path = entry.path();
            let relative_to_context = path.strip_prefix(context_root)?;
            let relative_to_source = path.strip_prefix(&canonical)?;
            let metadata = fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink() {
                bail!("Symlink COPY sources are unsupported by the build cache");
            }
            append_source_entry(
                path,
                relative_to_context,
                relative_to_source,
                &metadata,
                &mut entries,
            )?;
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(CopyKeyInput {
        source: operation.source.to_string_lossy().into_owned(),
        destination: operation.destination.to_string_lossy().into_owned(),
        destination_is_directory: operation.destination_is_directory,
        entries,
    })
}

fn append_source_entry(
    path: &Path,
    relative_to_context: &Path,
    relative_to_source: &Path,
    metadata: &fs::Metadata,
    entries: &mut Vec<SourceEntry>,
) -> Result<()> {
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o7777
    };
    #[cfg(not(unix))]
    let mode = if metadata.permissions().readonly() {
        0o444
    } else {
        0o644
    };

    if metadata.is_file() {
        entries.push(SourceEntry {
            path: path_key(relative_to_context),
            kind: "file",
            mode,
            size: metadata.len(),
            content_blake3: Some(blake3_file(path)?),
        });
    } else if metadata.is_dir() {
        entries.push(SourceEntry {
            path: path_key(relative_to_context),
            kind: "directory",
            mode,
            size: 0,
            content_blake3: None,
        });
    } else {
        bail!(
            "Unsupported COPY source entry while computing cache key: {}",
            relative_to_source.display()
        );
    }
    Ok(())
}

fn reject_source_symlinks(context_root: &Path, source: &Path) -> Result<()> {
    let mut current = context_root.to_path_buf();
    for component in source.components() {
        if let Component::Normal(name) = component {
            current.push(name);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    bail!(
                        "Symlink COPY source '{}' is unsupported by the build cache",
                        source.display()
                    );
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn path_key(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    hex::encode(path.as_os_str().as_bytes())
}

#[cfg(not(unix))]
fn path_key(path: &Path) -> String {
    hex::encode(path.to_string_lossy().as_bytes())
}

fn write_snapshot_tar(rootfs: &Path, output: &mut impl Write) -> Result<()> {
    let root = fs::canonicalize(rootfs)
        .with_context(|| format!("Failed to resolve stage rootfs {}", rootfs.display()))?;
    if !root.is_dir() {
        bail!("Stage rootfs is not a directory: {}", root.display());
    }
    let mut paths = walkdir::WalkDir::new(&root)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .map(|entry| entry.map(|entry| entry.into_path()))
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to enumerate stage rootfs for cache snapshot")?;
    paths.sort();

    let mut archive = Builder::new(output);
    archive.mode(tar::HeaderMode::Deterministic);
    for path in paths {
        let relative = path.strip_prefix(&root)?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!("Cannot cache stage symlink {}", relative.display());
        }
        let mode = filesystem_mode(&metadata);
        if metadata.is_dir() {
            let mut header = snapshot_header(0, mode, EntryType::Directory)?;
            archive.append_data(&mut header, relative, io::empty())?;
        } else if metadata.is_file() {
            let mut header = snapshot_header(metadata.len(), mode, EntryType::Regular)?;
            archive.append_data(&mut header, relative, File::open(&path)?)?;
        } else {
            bail!(
                "Cannot cache unsupported stage entry {}",
                relative.display()
            );
        }
    }
    archive.finish()?;
    Ok(())
}

fn snapshot_header(size: u64, mode: u32, entry_type: EntryType) -> Result<Header> {
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

fn restore_snapshot(mut snapshot: File, destination: &Path) -> Result<u64> {
    validate_snapshot_archive(&mut snapshot)?;
    snapshot.seek(SeekFrom::Start(0))?;
    let destination_metadata = fs::symlink_metadata(destination)
        .with_context(|| format!("Failed to inspect restore root {}", destination.display()))?;
    if destination_metadata.file_type().is_symlink() || !destination_metadata.is_dir() {
        bail!("Build snapshot restore target must be a real directory");
    }
    if fs::read_dir(destination)?.next().is_some() {
        bail!("Build snapshot restore target must be empty");
    }
    let parent = destination
        .parent()
        .context("Build snapshot restore target has no parent")?;
    let transaction = TempDir::new_in(parent).context("Failed to create restore transaction")?;
    let extracted = transaction.path().join("rootfs");
    fs::create_dir(&extracted)?;
    let restore_result = (|| {
        let restored_bytes = extract_snapshot(snapshot, &extracted)?;
        let previous = transaction.path().join("previous-rootfs");
        fs::rename(destination, &previous)
            .context("Failed to stage empty restore target for atomic replacement")?;
        if let Err(error) = fs::rename(&extracted, destination) {
            let rollback = fs::rename(&previous, destination);
            return match rollback {
                Ok(()) => Err(error).context("Failed to install restored filesystem snapshot"),
                Err(rollback_error) => bail!(
                    "Failed to install restored filesystem snapshot: {error}; \
                     restoring the original empty rootfs also failed: {rollback_error}"
                ),
            };
        }
        Ok(restored_bytes)
    })();
    let cleanup_result = cleanup_restore_transaction(transaction);
    match (restore_result, cleanup_result) {
        (Ok(bytes), Ok(())) => Ok(bytes),
        (Err(restore_error), Ok(())) => Err(restore_error),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
        (Err(restore_error), Err(cleanup_error)) => bail!(
            "{restore_error:#}; additionally, failed cleaning restore transaction: \
             {cleanup_error:#}"
        ),
    }
}

fn cleanup_restore_transaction(transaction: TempDir) -> Result<()> {
    let permissions = make_removable(transaction.path());
    let removal = transaction
        .close()
        .context("Failed to remove cache restore transaction");
    match (permissions, removal) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(permission_error), Ok(())) => Err(permission_error),
        (Ok(()), Err(removal_error)) => Err(removal_error),
        (Err(permission_error), Err(removal_error)) => bail!(
            "Failed making restore transaction removable: {permission_error:#}; \
             removal also failed: {removal_error:#}"
        ),
    }
}

fn make_removable(directory: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(directory)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("Restore transaction contains an unexpected non-directory path");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            directory,
            fs::Permissions::from_mode(metadata.permissions().mode() | 0o700),
        )?;
    }
    #[cfg(not(unix))]
    {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(false);
        fs::set_permissions(directory, permissions)?;
    }
    for child in fs::read_dir(directory)? {
        let child = child?.path();
        let metadata = fs::symlink_metadata(&child)?;
        if metadata.file_type().is_symlink() {
            bail!(
                "Restore transaction contains unexpected symlink {}",
                child.display()
            );
        }
        if metadata.is_dir() {
            make_removable(&child)?;
        }
    }
    Ok(())
}

fn extract_snapshot(snapshot: File, root: &Path) -> Result<u64> {
    let mut archive = tar::Archive::new(snapshot);
    let mut seen = HashSet::new();
    let mut directory_modes = Vec::new();
    let mut restored_bytes = 0u64;
    for item in archive.entries()? {
        let mut entry = item?;
        let path = entry.path()?.into_owned();
        validate_snapshot_path(&path)?;
        if !seen.insert(path.clone()) {
            bail!("Build snapshot contains duplicate path {}", path.display());
        }
        let destination = root.join(&path);
        if !destination.starts_with(root) {
            bail!("Build snapshot path escapes restore root");
        }
        ensure_no_symlink_components(root, &destination)?;
        let entry_type = entry.header().entry_type();
        let mode = entry.header().mode()?;
        validate_snapshot_metadata(&entry, mode)?;
        if entry_type.is_dir() {
            fs::create_dir_all(&destination)?;
            directory_modes.push((destination, mode));
        } else if entry_type.is_file() {
            let parent = destination
                .parent()
                .context("Snapshot file has no parent")?;
            fs::create_dir_all(parent)?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination)
                .with_context(|| {
                    format!("Failed to create restored file {}", destination.display())
                })?;
            let expected_size = entry.size();
            let copied = io::copy(&mut entry, &mut file)?;
            if copied != expected_size {
                bail!(
                    "Build snapshot file {} has an unexpected content size",
                    path.display()
                );
            }
            restored_bytes = restored_bytes
                .checked_add(copied)
                .context("Restored snapshot byte count overflowed")?;
            file.sync_all()?;
            set_path_mode(&destination, mode)?;
        } else {
            bail!(
                "Build snapshot contains unsupported tar entry type for {}",
                path.display()
            );
        }
    }
    for (directory, mode) in directory_modes.into_iter().rev() {
        set_path_mode(&directory, mode)?;
    }
    Ok(restored_bytes)
}

fn validate_snapshot_archive(snapshot: &mut File) -> Result<()> {
    snapshot.seek(SeekFrom::Start(0))?;
    let mut archive = tar::Archive::new(snapshot);
    let mut kinds = BTreeMap::new();
    let mut previous_path: Option<PathBuf> = None;
    for item in archive.entries()? {
        let mut entry = item?;
        let path = entry.path()?.into_owned();
        validate_snapshot_path(&path)?;
        if previous_path
            .as_ref()
            .is_some_and(|previous| previous >= &path)
        {
            bail!("Build snapshot paths are duplicated or not deterministically ordered");
        }
        previous_path = Some(path.clone());

        let entry_type = entry.header().entry_type();
        let mode = entry.header().mode()?;
        validate_snapshot_metadata(&entry, mode)?;
        let is_directory = if entry_type.is_dir() {
            if entry.size() != 0 {
                bail!("Build snapshot directory {} contains data", path.display());
            }
            true
        } else if entry_type.is_file() {
            let expected_size = entry.size();
            let actual_size = io::copy(&mut entry, &mut io::sink())?;
            if actual_size != expected_size {
                bail!(
                    "Build snapshot file {} has an unexpected content size",
                    path.display()
                );
            }
            false
        } else {
            bail!(
                "Build snapshot contains unsupported tar entry type for {}",
                path.display()
            );
        };
        if kinds.insert(path.clone(), is_directory).is_some() {
            bail!("Build snapshot contains duplicate path {}", path.display());
        }
    }
    for path in kinds.keys() {
        let mut parent = path.parent();
        while let Some(parent_path) = parent {
            if parent_path.as_os_str().is_empty() {
                break;
            }
            match kinds.get(parent_path) {
                Some(true) => {}
                Some(false) => bail!(
                    "Build snapshot path {} has a non-directory parent {}",
                    path.display(),
                    parent_path.display()
                ),
                None => bail!(
                    "Build snapshot path {} is missing parent directory {}",
                    path.display(),
                    parent_path.display()
                ),
            }
            parent = parent_path.parent();
        }
    }
    Ok(())
}

fn validate_snapshot_metadata<R: Read>(entry: &tar::Entry<'_, R>, mode: u32) -> Result<()> {
    if mode & !0o7777 != 0
        || entry.header().uid()? != 0
        || entry.header().gid()? != 0
        || entry.header().mtime()? != 0
    {
        bail!("Build snapshot contains unsupported or non-normalized metadata");
    }
    Ok(())
}

fn validate_snapshot_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!(
            "Build snapshot contains an unsafe path '{}'",
            path.display()
        );
    }
    Ok(())
}

fn ensure_no_symlink_components(root: &Path, destination: &Path) -> Result<()> {
    let relative = destination.strip_prefix(root)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        if let Component::Normal(name) = component {
            current.push(name);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    bail!(
                        "Build snapshot path traverses symlink {}",
                        current.display()
                    );
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

fn read_regular_file_if_present(path: &Path) -> Result<Option<Vec<u8>>> {
    match open_regular_file(path) {
        Ok(mut file) => {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            Ok(Some(bytes))
        }
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|e| e.kind() == io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn open_regular_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .with_context(|| format!("Failed to open regular cache file {}", path.display()))?;
    if !file.metadata()?.is_file() {
        bail!(
            "Build snapshot cache path is not a regular file: {}",
            path.display()
        );
    }
    Ok(file)
}

fn create_cache_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            bail!(
                "Build snapshot cache path is not a directory: {}",
                path.display()
            );
        }
        Ok(_) => validate_cache_directory_permissions(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match create_private_directory(path) {
                Ok(()) => set_private_cache_permissions(path)?,
                Err(create_error) if create_error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(create_error) => {
                    return Err(create_error).with_context(|| {
                        format!(
                            "Failed to create build snapshot cache directory {}",
                            path.display()
                        )
                    });
                }
            }
            validate_cache_directory_permissions(path)
        }
        Err(error) => Err(error.into()),
    }
}

fn create_directory_chain_without_symlinks(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => bail!(
                "Build cache root must not contain parent traversal: {}",
                path.display()
            ),
            Component::Normal(name) => {
                current.push(name);
                match fs::symlink_metadata(&current) {
                    Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                        bail!(
                            "Build cache root contains a symlink or non-directory component: {}",
                            current.display()
                        );
                    }
                    Ok(metadata) => validate_cache_ancestor_permissions(&current, &metadata)?,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        match create_private_directory(&current) {
                            Ok(()) => set_private_cache_permissions(&current)?,
                            Err(create_error)
                                if create_error.kind() == io::ErrorKind::AlreadyExists => {}
                            Err(create_error) => {
                                return Err(create_error).with_context(|| {
                                    format!(
                                        "Failed to create build cache directory {}",
                                        current.display()
                                    )
                                });
                            }
                        }
                        let metadata = fs::symlink_metadata(&current)?;
                        if metadata.file_type().is_symlink() || !metadata.is_dir() {
                            bail!(
                                "Build cache root contains a symlink or non-directory component: {}",
                                current.display()
                            );
                        }
                        validate_cache_directory_permissions(&current)?;
                    }
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!(
                                "Failed to inspect build cache directory {}",
                                current.display()
                            )
                        });
                    }
                }
            }
        }
    }
    validate_cache_directory_permissions(path)
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(path)
    }
}

fn set_private_cache_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).with_context(|| {
            format!(
                "Failed to restrict build cache directory permissions for {}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn validate_cache_directory_permissions(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "Build cache path is not a real directory: {}",
            path.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode() & 0o7777;
        if mode & 0o022 != 0 {
            bail!(
                "Build cache directory must not be writable by group or other users \
                 (mode {mode:o}): {}",
                path.display(),
            );
        }
    }
    Ok(())
}

fn validate_cache_ancestor_permissions(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode() & 0o7777;
        if mode & 0o022 != 0 && mode & 0o1000 == 0 {
            bail!(
                "Build cache ancestor must not be writable by group or other users \
                 unless sticky: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn validate_build_key(key: &str) -> Result<()> {
    let digest = key
        .strip_prefix(KEY_PREFIX)
        .context("Build cache key has an unsupported format")?;
    validate_hex(digest, 64).context("Build cache key must contain a BLAKE3 digest")
}

fn validate_digest(digest: &str) -> Result<()> {
    validate_hex(digest, 64).context("Snapshot digest must be a SHA-256 hex value")
}

fn validate_hex(value: &str, length: usize) -> Result<()> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("Invalid SHA-256 hex value");
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = open_regular_file(path)?;
    sha256_reader(&mut file)
}

fn sha256_reader(reader: &mut impl Read) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn blake3_file(path: &Path) -> Result<String> {
    let mut file = open_regular_file(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn filesystem_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn filesystem_mode(metadata: &fs::Metadata) -> u32 {
    if metadata.permissions().readonly() {
        0o444
    } else {
        0o644
    }
}

#[cfg(unix)]
fn set_path_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_path_mode(path: &Path, mode: u32) -> Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_readonly(mode & 0o222 == 0);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        validate_build_key, validate_snapshot_path, write_snapshot_tar, BuildSnapshotCache,
        KEY_PREFIX,
    };
    use crate::docker::{dag, parser};
    use std::fs::{self, OpenOptions};
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    #[test]
    fn snapshot_paths_must_be_relative_normal_components() {
        for path in ["../escape", "/absolute", "a/../../b", ""] {
            assert!(validate_snapshot_path(Path::new(path)).is_err(), "{path}");
        }
        assert!(validate_snapshot_path(Path::new("nested/file")).is_ok());
    }

    #[test]
    fn cache_keys_are_versioned_blake3_values() {
        let key = format!("{KEY_PREFIX}{}", "a".repeat(64));
        assert!(validate_build_key(&key).is_ok());
        assert!(validate_build_key(&format!("v1:sha256:{}", "a".repeat(64))).is_err());
        assert!(validate_build_key("sha256:abcd").is_err());
        assert!(validate_build_key("../outside").is_err());
    }

    fn cache_plan(context: &Path) -> crate::docker::dag::CompiledBuild {
        let dockerfile = "FROM scratch\nCOPY payload /payload\n";
        fs::write(context.join("Dockerfile"), dockerfile).unwrap();
        fs::write(context.join("payload"), b"crash-recovery-payload").unwrap();
        dag::compile_dockerfile(
            parser::parse_dockerfile(dockerfile).unwrap(),
            context.to_path_buf(),
        )
        .unwrap()
    }

    #[test]
    fn cache_crash_child_helper() {
        let (Ok(context), Ok(cache_root), Ok(rootfs)) = (
            std::env::var("MEMOBUILD_CACHE_TEST_CONTEXT"),
            std::env::var("MEMOBUILD_CACHE_TEST_ROOT"),
            std::env::var("MEMOBUILD_CACHE_TEST_STAGE"),
        ) else {
            return;
        };
        let context = Path::new(&context);
        let plan = cache_plan(context);
        let cache = BuildSnapshotCache::new(cache_root).unwrap();
        cache.store(&plan, Path::new(&rootfs)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn interrupted_publication_recovers_to_complete_entry_or_miss() {
        use std::os::unix::fs::PermissionsExt;

        let points = [
            "snapshot_temp_created",
            "snapshot_serialized",
            "snapshot_file_synced",
            "blob_published",
            "blob_directory_synced",
            "entry_file_synced",
            "entry_published",
            "entry_directory_synced",
        ];
        for point in points {
            let workspace = TempDir::new().unwrap();
            let context = workspace.path().join("context");
            let cache_root = workspace.path().join("cache");
            let stage = workspace.path().join("stage");
            fs::set_permissions(workspace.path(), fs::Permissions::from_mode(0o700)).unwrap();
            fs::create_dir(&context).unwrap();
            fs::create_dir(&cache_root).unwrap();
            fs::create_dir(&stage).unwrap();
            fs::set_permissions(&cache_root, fs::Permissions::from_mode(0o700)).unwrap();
            let plan = cache_plan(&context);
            fs::write(stage.join("payload"), b"crash-recovery-payload").unwrap();

            let output = Command::new(std::env::current_exe().unwrap())
                .arg("cache_crash_child_helper")
                .arg("--nocapture")
                .env("MEMOBUILD_CACHE_TEST_CONTEXT", &context)
                .env("MEMOBUILD_CACHE_TEST_ROOT", &cache_root)
                .env("MEMOBUILD_CACHE_TEST_STAGE", &stage)
                .env("MEMOBUILD_CACHE_TEST_CRASH", point)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(86),
                "fault point {point}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );

            let recovered = BuildSnapshotCache::new(&cache_root).unwrap();
            let destination = TempDir::new().unwrap();
            let hit = recovered.restore(&plan, destination.path()).unwrap();
            let should_hit = matches!(point, "entry_published" | "entry_directory_synced");
            assert_eq!(hit, should_hit, "fault point {point}");
            if hit {
                assert_eq!(
                    fs::read(destination.path().join("payload")).unwrap(),
                    b"crash-recovery-payload"
                );
                let repeated = TempDir::new().unwrap();
                assert!(recovered.restore(&plan, repeated.path()).unwrap());
            } else {
                assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 0);
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn snapshot_writer_propagates_no_space_errors() {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("payload"), vec![0; 4096]).unwrap();
        let mut no_space = OpenOptions::new().write(true).open("/dev/full").unwrap();
        let error = write_snapshot_tar(root.path(), &mut no_space).unwrap_err();
        assert!(
            error.chain().any(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .and_then(std::io::Error::raw_os_error)
                    == Some(libc::ENOSPC)
            }),
            "expected an ENOSPC write error, got {error:#}"
        );
    }
}
