use memobuild::docker::{
    build_cache::BuildSnapshotCache,
    dag::{self, CompiledBuild},
    parser,
    stage::StageFilesystem,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tempfile::TempDir;

fn make_plan(context: &Path, dockerfile: &str) -> CompiledBuild {
    fs::write(context.join("Dockerfile"), dockerfile).expect("write Dockerfile");
    dag::compile_dockerfile(
        parser::parse_dockerfile(dockerfile).expect("parse Dockerfile"),
        context.to_path_buf(),
    )
    .expect("compile Dockerfile")
}

fn fixture() -> (TempDir, CompiledBuild) {
    let context = TempDir::new().expect("create build context");
    fs::create_dir_all(context.path().join("app/nested")).expect("create source directories");
    fs::write(
        context.path().join("app/nested/data.txt"),
        "snapshot payload\n",
    )
    .expect("write source file");
    let plan = make_plan(
        context.path(),
        "FROM scratch\nCOPY app /app/\nENTRYPOINT [\"/app/nested/data.txt\"]\n",
    );
    (context, plan)
}

fn private_cache_root(parent: &TempDir) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o700))
            .expect("restrict cache fixture permissions");
    }
    parent.path().to_path_buf()
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

#[test]
fn cache_key_tracks_content_metadata_destination_and_command_config() {
    use std::os::unix::fs::PermissionsExt;

    let (context, plan) = fixture();
    let first = BuildSnapshotCache::key_for_build(&plan).expect("compute first key");
    assert_eq!(
        first,
        BuildSnapshotCache::key_for_build(&plan).expect("compute stable key")
    );
    assert!(first.starts_with("v2:blake3:"));

    fs::write(
        context.path().join("app/nested/data.txt"),
        "changed payload\n",
    )
    .expect("change source content");
    assert_ne!(
        first,
        BuildSnapshotCache::key_for_build(&plan).expect("compute content key")
    );

    let changed_content = BuildSnapshotCache::key_for_build(&plan).unwrap();
    fs::set_permissions(
        context.path().join("app/nested/data.txt"),
        fs::Permissions::from_mode(0o700),
    )
    .expect("change source mode");
    assert_ne!(
        changed_content,
        BuildSnapshotCache::key_for_build(&plan).expect("compute metadata key")
    );

    let alternate = make_plan(
        context.path(),
        "FROM scratch\nCOPY app /different/\nENTRYPOINT [\"/app/nested/data.txt\"]\n",
    );
    assert_ne!(
        BuildSnapshotCache::key_for_build(&plan).unwrap(),
        BuildSnapshotCache::key_for_build(&alternate).unwrap()
    );

    let alternate = make_plan(
        context.path(),
        "FROM scratch\nCOPY app /app/\nENTRYPOINT [\"/app/nested/data.txt\", \"arg\"]\n",
    );
    assert_ne!(
        BuildSnapshotCache::key_for_build(&plan).unwrap(),
        BuildSnapshotCache::key_for_build(&alternate).unwrap()
    );
}

#[test]
fn v2_namespace_does_not_interpret_or_modify_v1_sha256_entries() {
    let (context, plan) = fixture();
    let parent = TempDir::new().unwrap();
    let root = private_cache_root(&parent);
    let legacy = root.join("build-snapshots-v1/entries/legacy.json");
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(&legacy, b"legacy sha256 entry").unwrap();

    let cache = BuildSnapshotCache::new(&root).unwrap();
    assert!(cache.cache_directory().ends_with("build-snapshots-v2"));
    assert_eq!(fs::read(&legacy).unwrap(), b"legacy sha256 entry");
    let target = TempDir::new().unwrap();
    assert!(!cache.restore(&plan, target.path()).unwrap());
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
    drop(context);
}

#[test]
fn cache_key_tracks_target_export_mode_and_compiled_instruction_order() {
    use memobuild::docker::build_cache::BuildCacheKeyOptions;

    let (context, plan) = fixture();
    let default_options = BuildCacheKeyOptions::default();
    let key = BuildSnapshotCache::key_for_build_with_options(&plan, &default_options).unwrap();

    let mut options = default_options.clone();
    options.reproducible = true;
    assert_ne!(
        key,
        BuildSnapshotCache::key_for_build_with_options(&plan, &options).unwrap()
    );

    let mut options = default_options.clone();
    options.target_architecture = "arm64".to_string();
    assert_ne!(
        key,
        BuildSnapshotCache::key_for_build_with_options(&plan, &options).unwrap()
    );

    let mut options = default_options.clone();
    options.target_os = "freebsd".to_string();
    assert_ne!(
        key,
        BuildSnapshotCache::key_for_build_with_options(&plan, &options).unwrap()
    );

    let mut options = default_options.clone();
    options.image_reference = "memobuild-demo:other".to_string();
    assert_ne!(
        key,
        BuildSnapshotCache::key_for_build_with_options(&plan, &options).unwrap()
    );

    fs::write(context.path().join("second.txt"), "second\n").unwrap();
    let first_order = make_plan(
        context.path(),
        "FROM scratch\nCOPY app /app/\nCOPY second.txt /second.txt\n",
    );
    let reverse_order = make_plan(
        context.path(),
        "FROM scratch\nCOPY second.txt /second.txt\nCOPY app /app/\n",
    );
    assert_ne!(
        BuildSnapshotCache::key_for_build(&first_order).unwrap(),
        BuildSnapshotCache::key_for_build(&reverse_order).unwrap()
    );

    let changed_context_key = BuildSnapshotCache::key_for_build(&plan).unwrap();
    let missing = context.path().join("app/nested/data.txt");
    fs::remove_file(&missing).unwrap();
    assert_ne!(
        changed_context_key,
        BuildSnapshotCache::key_for_build(&plan).unwrap()
    );
}

#[test]
fn cache_restores_complete_snapshot_in_a_fresh_process() {
    let (context, plan) = fixture();
    let cache_root = TempDir::new().expect("create cache root");
    let cache_root = private_cache_root(&cache_root);
    let restored_root = TempDir::new().expect("create restore target");
    let cache = BuildSnapshotCache::new(&cache_root).expect("create snapshot cache");

    let stage = StageFilesystem::new().expect("create source stage");
    for operation in &plan.copies {
        stage
            .apply_copy(&plan.context_root, operation)
            .expect("apply COPY");
    }
    let expected = fs::read(stage.root().join("app/nested/data.txt")).unwrap();
    #[cfg(unix)]
    let expected_modes = [
        mode(&stage.root().join("app")),
        mode(&stage.root().join("app/nested")),
        mode(&stage.root().join("app/nested/data.txt")),
    ];
    let key = cache.store(&plan, stage.root()).expect("store snapshot");
    assert!(key.starts_with("v2:blake3:"));
    let (record_path, blob_path) = stored_snapshot_digest(&cache, &key);
    let record: Value = serde_json::from_slice(&fs::read(record_path).unwrap()).unwrap();
    assert_eq!(record["format_version"], 2);
    assert_eq!(
        record["snapshot_digest"].as_str().unwrap(),
        hex::encode(Sha256::digest(fs::read(blob_path).unwrap()))
    );
    stage.cleanup().expect("clean source stage");

    let child = Command::new(std::env::current_exe().expect("test executable path"))
        .arg("--exact")
        .arg("cache_restore_child_helper")
        .arg("--nocapture")
        .env("MEMOBUILD_CACHE_CHILD_CONTEXT", context.path())
        .env("MEMOBUILD_CACHE_CHILD_ROOT", restored_root.path())
        .env("MEMOBUILD_CACHE_CHILD_CACHE", &cache_root)
        .output()
        .expect("spawn fresh test process");
    assert!(
        child.status.success(),
        "{}{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert_eq!(
        fs::read(restored_root.path().join("app/nested/data.txt")).unwrap(),
        expected
    );
    #[cfg(unix)]
    {
        assert_eq!(mode(&restored_root.path().join("app")), expected_modes[0]);
        assert_eq!(
            mode(&restored_root.path().join("app/nested")),
            expected_modes[1]
        );
        assert_eq!(
            mode(&restored_root.path().join("app/nested/data.txt")),
            expected_modes[2]
        );
    }
}

#[test]
fn cache_restore_child_helper() {
    let (Ok(context), Ok(root), Ok(cache_root)) = (
        std::env::var("MEMOBUILD_CACHE_CHILD_CONTEXT"),
        std::env::var("MEMOBUILD_CACHE_CHILD_ROOT"),
        std::env::var("MEMOBUILD_CACHE_CHILD_CACHE"),
    ) else {
        return;
    };
    let context = PathBuf::from(context);
    let plan = make_plan(
        &context,
        "FROM scratch\nCOPY app /app/\nENTRYPOINT [\"/app/nested/data.txt\"]\n",
    );
    let cache = BuildSnapshotCache::new(cache_root).expect("open snapshot cache in child");
    assert!(
        cache
            .restore(&plan, &PathBuf::from(root))
            .expect("restore snapshot in child"),
        "fresh process must find the stored snapshot"
    );
}

#[test]
fn cache_store_child_helper() {
    let (Ok(context), Ok(cache_root)) = (
        std::env::var("MEMOBUILD_CACHE_CHILD_CONTEXT"),
        std::env::var("MEMOBUILD_CACHE_CHILD_CACHE"),
    ) else {
        return;
    };
    let context = PathBuf::from(context);
    let plan = make_plan(
        &context,
        "FROM scratch\nCOPY app /app/\nENTRYPOINT [\"/app/nested/data.txt\"]\n",
    );
    let stage = StageFilesystem::new().expect("create child stage");
    for operation in &plan.copies {
        stage
            .apply_copy(&plan.context_root, operation)
            .expect("apply COPY in child process");
    }
    BuildSnapshotCache::new(cache_root)
        .expect("open snapshot cache in child")
        .store(&plan, stage.root())
        .expect("store snapshot in child");
}

fn stored_snapshot_digest(cache: &BuildSnapshotCache, key: &str) -> (PathBuf, PathBuf) {
    let entry = cache
        .cache_directory()
        .join("entries")
        .join(format!("{}.json", key.replace(':', "-")));
    let value: Value = serde_json::from_slice(&fs::read(&entry).unwrap()).unwrap();
    let digest = value["snapshot_digest"].as_str().unwrap();
    let blob = cache.cache_directory().join("blobs/sha256").join(digest);
    (entry, blob)
}

#[test]
fn missing_or_corrupt_snapshot_blobs_fail_closed() {
    for corrupt in [false, true] {
        let (context, plan) = fixture();
        let cache_root = TempDir::new().expect("create cache root");
        let cache_root = private_cache_root(&cache_root);
        let cache = BuildSnapshotCache::new(&cache_root).expect("create cache");
        let stage = StageFilesystem::new().expect("create source stage");
        for operation in &plan.copies {
            stage
                .apply_copy(&plan.context_root, operation)
                .expect("apply COPY");
        }
        let key = cache.store(&plan, stage.root()).expect("store snapshot");
        stage.cleanup().expect("clean source stage");
        let (_entry, blob) = stored_snapshot_digest(&cache, &key);
        if corrupt {
            fs::write(&blob, b"corrupted snapshot").expect("corrupt blob");
        } else {
            fs::remove_file(&blob).expect("remove snapshot blob");
        }

        let target = TempDir::new().expect("create restore target");
        let error = cache
            .restore(&plan, target.path())
            .expect_err("bad snapshot must not be restored");
        let message = format!("{error:#}");
        if corrupt {
            assert!(
                message.contains("size mismatch") || message.contains("digest mismatch"),
                "{message}"
            );
        } else {
            assert!(message.contains("missing blob"), "{message}");
        }
        assert_eq!(
            fs::read_dir(target.path()).unwrap().count(),
            0,
            "failed validation must not mutate the restore target"
        );
        drop(context);
    }
}

#[test]
fn absent_snapshot_is_a_cache_miss_without_mutating_rootfs() {
    let (_context, plan) = fixture();
    let cache_root = TempDir::new().expect("create cache root");
    let cache_root = private_cache_root(&cache_root);
    let target = TempDir::new().expect("create restore target");
    let cache = BuildSnapshotCache::new(&cache_root).expect("create cache");
    assert!(!cache.restore(&plan, target.path()).unwrap());
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn cache_statistics_count_only_verified_restores_and_completed_writes() {
    let (_context, plan) = fixture();
    let cache_root = TempDir::new().unwrap();
    let cache_root = private_cache_root(&cache_root);
    let cache = BuildSnapshotCache::new(&cache_root).unwrap();
    let cold_stage = StageFilesystem::new().unwrap();
    assert!(!cache.restore(&plan, cold_stage.root()).unwrap());
    for operation in &plan.copies {
        cold_stage
            .apply_copy(&plan.context_root, operation)
            .unwrap();
    }
    let cold_signature = filesystem_signature(cold_stage.root());
    cache.store(&plan, cold_stage.root()).unwrap();
    cold_stage.cleanup().unwrap();

    let restored_stage = StageFilesystem::new().unwrap();
    assert!(cache.restore(&plan, restored_stage.root()).unwrap());
    assert_eq!(filesystem_signature(restored_stage.root()), cold_signature);
    restored_stage.cleanup().unwrap();

    let stats = cache.try_statistics().unwrap();
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.key_mismatches, 0);
    assert_eq!(stats.snapshot_verification_failures, 0);
    assert_eq!(stats.restored_bytes, b"snapshot payload\n".len() as u64);
    assert_eq!(stats.snapshot_writes, 1);
    assert_eq!(stats.evictions, 0);
    assert_eq!(stats.rebuilds_after_cache_failure, 0);
    assert!(stats.storage_usage_bytes <= stats.storage_limit_bytes);
}

#[test]
fn quota_refusal_leaves_no_partial_snapshot_or_entry() {
    let (_context, plan) = fixture();
    let cache_parent = TempDir::new().unwrap();
    let cache_root = private_cache_root(&cache_parent);
    let cache = BuildSnapshotCache::with_limit(&cache_root, 2048).unwrap();
    let stage = StageFilesystem::new().unwrap();
    fs::write(stage.root().join("large"), vec![0x5a; 16 * 1024]).unwrap();

    let error = cache
        .store(&plan, stage.root())
        .expect_err("oversized snapshot must respect quota");
    assert!(
        format!("{error:#}").contains("quota exceeded"),
        "unexpected quota error: {error:#}"
    );
    let entries = cache.cache_directory().join("entries");
    let blobs = cache.cache_directory().join("blobs/sha256");
    assert_eq!(fs::read_dir(entries).unwrap().count(), 0);
    assert_eq!(fs::read_dir(blobs).unwrap().count(), 0);
    let stats = cache.statistics();
    assert!(stats.quota_refusals > 0);
    assert!(stats.storage_usage_bytes <= stats.storage_limit_bytes);
}

#[test]
fn quota_pressure_evicts_deterministically_and_keeps_new_entry_restorable() {
    let (context, first_plan) = fixture();
    let cache_parent = TempDir::new().unwrap();
    let cache_root = private_cache_root(&cache_parent);
    let cache = BuildSnapshotCache::with_limit(&cache_root, 1_400_000).unwrap();
    let first_stage = StageFilesystem::new().unwrap();
    fs::write(first_stage.root().join("large"), vec![0x11; 900_000]).unwrap();
    let first_key = cache.store(&first_plan, first_stage.root()).unwrap();

    let second_plan = make_plan(
        context.path(),
        "FROM scratch\nCOPY app /app/\nCMD [\"different\"]\n",
    );
    let second_stage = StageFilesystem::new().unwrap();
    fs::write(second_stage.root().join("large"), vec![0x22; 900_000]).unwrap();
    let second_key = cache.store(&second_plan, second_stage.root()).unwrap();

    assert_ne!(first_key, second_key);
    assert!(cache.statistics().evictions > 0);
    assert!(cache.statistics().storage_usage_bytes <= 1_400_000);
    let restored = TempDir::new().unwrap();
    assert!(cache.restore(&second_plan, restored.path()).unwrap());
    assert_eq!(
        fs::read(restored.path().join("large")).unwrap(),
        vec![0x22; 900_000]
    );
}

#[test]
fn cache_reopen_removes_interrupted_temps_and_unreferenced_blobs() {
    let (context, plan) = fixture();
    let cache_parent = TempDir::new().unwrap();
    let cache_root = private_cache_root(&cache_parent);
    let cache = BuildSnapshotCache::new(&cache_root).unwrap();
    let stage = StageFilesystem::new().unwrap();
    for operation in &plan.copies {
        stage.apply_copy(&plan.context_root, operation).unwrap();
    }
    cache.store(&plan, stage.root()).unwrap();
    let entries = cache.cache_directory().join("entries");
    let blobs = cache.cache_directory().join("blobs/sha256");
    fs::write(entries.join(".tmp-interrupted-record"), b"partial").unwrap();
    fs::write(blobs.join(".tmp-interrupted-snapshot"), b"partial").unwrap();
    fs::write(blobs.join("a".repeat(64)), b"orphan").unwrap();
    drop(cache);

    let reopened = BuildSnapshotCache::new(&cache_root).unwrap();
    assert_eq!(reopened.statistics().orphan_files_removed, 3);
    assert!(!entries.join(".tmp-interrupted-record").exists());
    assert!(!blobs.join(".tmp-interrupted-snapshot").exists());
    assert!(!blobs.join("a".repeat(64)).exists());
    let restored = TempDir::new().unwrap();
    assert!(reopened.restore(&plan, restored.path()).unwrap());
    let restored_again = TempDir::new().unwrap();
    assert!(reopened.restore(&plan, restored_again.path()).unwrap());
    drop(context);
}

#[cfg(unix)]
fn filesystem_signature(root: &Path) -> std::collections::BTreeMap<PathBuf, (bool, u32, Vec<u8>)> {
    use std::os::unix::fs::PermissionsExt;
    let mut signature = std::collections::BTreeMap::new();
    for item in walkdir::WalkDir::new(root).min_depth(1) {
        let item = item.unwrap();
        let path = item.path().strip_prefix(root).unwrap().to_path_buf();
        let metadata = fs::symlink_metadata(item.path()).unwrap();
        let bytes = if metadata.is_file() {
            fs::read(item.path()).unwrap()
        } else {
            Vec::new()
        };
        signature.insert(
            path,
            (
                metadata.is_dir(),
                metadata.permissions().mode() & 0o7777,
                bytes,
            ),
        );
    }
    signature
}

#[test]
fn concurrent_identical_builds_publish_one_consistent_cache_entry() {
    let (_context, plan) = fixture();
    let cache_root = TempDir::new().unwrap();
    let cache_root = private_cache_root(&cache_root);
    let cache = Arc::new(BuildSnapshotCache::new(&cache_root).unwrap());
    let first_root = TempDir::new().unwrap();
    let second_root = TempDir::new().unwrap();
    for root in [first_root.path(), second_root.path()] {
        fs::create_dir_all(root.join("app/nested")).unwrap();
        fs::write(root.join("app/nested/data.txt"), b"snapshot payload\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for directory in [root.join("app"), root.join("app/nested")] {
                fs::set_permissions(directory, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
    }
    let first_plan = plan.clone();
    let second_plan = plan.clone();
    let first_path = first_root.path().to_path_buf();
    let second_path = second_root.path().to_path_buf();
    let first_cache = Arc::clone(&cache);
    let second_cache = Arc::clone(&cache);
    let (first_key, second_key) = std::thread::scope(|scope| {
        let first = scope.spawn(move || first_cache.store(&first_plan, &first_path));
        let second = scope.spawn(move || second_cache.store(&second_plan, &second_path));
        (
            first.join().unwrap().unwrap(),
            second.join().unwrap().unwrap(),
        )
    });
    assert_eq!(first_key, second_key);

    let restore_root = TempDir::new().unwrap();
    assert!(cache.restore(&plan, restore_root.path()).unwrap());
    assert_eq!(
        fs::read(restore_root.path().join("app/nested/data.txt")).unwrap(),
        b"snapshot payload\n"
    );
    assert_eq!(cache.statistics().snapshot_writes, 2);
}

#[test]
fn concurrent_process_handles_can_initialize_a_fresh_cache_root() {
    let cache_parent = TempDir::new().unwrap();
    let cache_root = private_cache_root(&cache_parent).join("fresh-cache");
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..8 {
            let root = cache_root.clone();
            workers.push(scope.spawn(move || BuildSnapshotCache::new(root).unwrap()));
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });
    assert!(cache_root.join("build-snapshots-v2/entries").is_dir());
    assert!(cache_root.join("build-snapshots-v2/blobs/sha256").is_dir());
}

#[test]
fn concurrent_processes_publish_a_restorable_snapshot() {
    let (context, plan) = fixture();
    let cache_root = TempDir::new().unwrap();
    let cache_root = private_cache_root(&cache_root);
    let cache = BuildSnapshotCache::new(&cache_root).unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut children = Vec::new();
    for _ in 0..2 {
        children.push(
            Command::new(&executable)
                .arg("--exact")
                .arg("cache_store_child_helper")
                .arg("--nocapture")
                .env("MEMOBUILD_CACHE_CHILD_CONTEXT", context.path())
                .env("MEMOBUILD_CACHE_CHILD_CACHE", &cache_root)
                .spawn()
                .unwrap(),
        );
    }
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(
        fs::read_dir(cache.cache_directory().join("entries"))
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        fs::read_dir(cache.cache_directory().join("blobs/sha256"))
            .unwrap()
            .count(),
        1
    );
    let restore_root = TempDir::new().unwrap();
    assert!(cache.restore(&plan, restore_root.path()).unwrap());
    assert_eq!(
        fs::read(restore_root.path().join("app/nested/data.txt")).unwrap(),
        b"snapshot payload\n"
    );
}

#[test]
fn garbage_collection_serializes_with_concurrent_restores() {
    let (_context, plan) = fixture();
    let cache_parent = TempDir::new().unwrap();
    let cache_root = private_cache_root(&cache_parent);
    let cache = Arc::new(BuildSnapshotCache::new(&cache_root).unwrap());
    let stage = StageFilesystem::new().unwrap();
    for operation in &plan.copies {
        stage.apply_copy(&plan.context_root, operation).unwrap();
    }
    cache.store(&plan, stage.root()).unwrap();
    let collector = Arc::clone(&cache);
    let restore_cache = Arc::clone(&cache);
    let (gc_result, restore_result) = std::thread::scope(|scope| {
        let gc = scope.spawn(move || {
            for _ in 0..8 {
                collector.collect_garbage().unwrap();
            }
        });
        let restore = scope.spawn(move || {
            for _ in 0..8 {
                let destination = TempDir::new().unwrap();
                assert!(restore_cache.restore(&plan, destination.path()).unwrap());
                assert_eq!(
                    fs::read(destination.path().join("app/nested/data.txt")).unwrap(),
                    b"snapshot payload\n"
                );
            }
        });
        (gc.join(), restore.join())
    });
    gc_result.unwrap();
    restore_result.unwrap();
}

#[cfg(unix)]
#[test]
fn failed_snapshot_write_does_not_publish_cache_entry_or_blob() {
    use std::os::unix::fs::symlink;

    let (_context, plan) = fixture();
    let cache_root = TempDir::new().unwrap();
    let cache_root = private_cache_root(&cache_root);
    let cache = BuildSnapshotCache::new(&cache_root).unwrap();
    let stage = StageFilesystem::new().unwrap();
    symlink("/tmp", stage.root().join("unexpected-link")).unwrap();
    assert!(cache.store(&plan, stage.root()).is_err());
    assert_eq!(
        fs::read_dir(cache.cache_directory().join("entries"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        fs::read_dir(cache.cache_directory().join("blobs/sha256"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn unsupported_record_schema_is_rejected_and_counted() {
    let (_context, plan) = fixture();
    let cache_root = TempDir::new().unwrap();
    let cache_root = private_cache_root(&cache_root);
    let cache = BuildSnapshotCache::new(&cache_root).unwrap();
    let stage = StageFilesystem::new().unwrap();
    for operation in &plan.copies {
        stage.apply_copy(&plan.context_root, operation).unwrap();
    }
    let key = cache.store(&plan, stage.root()).unwrap();
    stage.cleanup().unwrap();
    let entry_path = cache
        .cache_directory()
        .join("entries")
        .join(format!("{}.json", key.replace(':', "-")));
    let mut entry: Value = serde_json::from_slice(&fs::read(&entry_path).unwrap()).unwrap();
    entry["format_version"] = Value::from(99);
    fs::write(&entry_path, serde_json::to_vec(&entry).unwrap()).unwrap();
    let restore_root = TempDir::new().unwrap();
    assert!(cache.restore(&plan, restore_root.path()).is_err());
    assert_eq!(fs::read_dir(restore_root.path()).unwrap().count(), 0);

    entry["format_version"] = Value::from(2);
    entry["build_key"] = Value::String(format!("v2:blake3:{}", "a".repeat(64)));
    fs::write(&entry_path, serde_json::to_vec(&entry).unwrap()).unwrap();
    assert!(cache.restore(&plan, restore_root.path()).is_err());
    let stats = cache.statistics();
    assert_eq!(stats.key_mismatches, 1);
    assert_eq!(stats.snapshot_verification_failures, 2);
    assert_eq!(stats.hits, 0);
    assert_eq!(fs::read_dir(restore_root.path()).unwrap().count(), 0);
}

#[test]
fn truncated_record_fails_closed_and_becomes_a_miss_after_reopen() {
    let (context, plan) = fixture();
    let cache_parent = TempDir::new().unwrap();
    let cache_root = private_cache_root(&cache_parent);
    let cache = BuildSnapshotCache::new(&cache_root).unwrap();
    let stage = StageFilesystem::new().unwrap();
    for operation in &plan.copies {
        stage.apply_copy(&plan.context_root, operation).unwrap();
    }
    let key = cache.store(&plan, stage.root()).unwrap();
    let entry = cache
        .cache_directory()
        .join("entries")
        .join(format!("{}.json", key.replace(':', "-")));
    fs::write(&entry, b"{\"format_version\":2").unwrap();
    let target = TempDir::new().unwrap();
    let error = cache.restore(&plan, target.path()).unwrap_err();
    assert!(
        format!("{error:#}").contains("malformed"),
        "unexpected truncated-record error: {error:#}"
    );
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);

    drop(cache);
    let reopened = BuildSnapshotCache::new(&cache_root).unwrap();
    let target = TempDir::new().unwrap();
    assert!(!reopened.restore(&plan, target.path()).unwrap());
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
    drop(context);
}

#[cfg(unix)]
#[test]
fn cache_root_symlinks_are_rejected_before_creating_cache_state() {
    use std::os::unix::fs::symlink;
    use std::os::unix::fs::PermissionsExt;

    let cache_parent = TempDir::new().unwrap();
    fs::set_permissions(cache_parent.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let outside = TempDir::new().unwrap();
    let cache_link = cache_parent.path().join("cache-link");
    symlink(outside.path(), &cache_link).unwrap();
    assert!(BuildSnapshotCache::new(&cache_link).is_err());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn group_or_world_writable_cache_roots_are_rejected() {
    use std::os::unix::fs::PermissionsExt;

    let cache_root = TempDir::new().unwrap();
    fs::set_permissions(cache_root.path(), fs::Permissions::from_mode(0o777)).unwrap();
    assert!(BuildSnapshotCache::new(cache_root.path()).is_err());
    assert!(!cache_root.path().join("build-snapshots-v2").exists());
}

#[cfg(unix)]
#[test]
fn group_or_world_writable_cache_ancestors_are_rejected_before_writes() {
    use std::os::unix::fs::PermissionsExt;

    let parent = TempDir::new().unwrap();
    fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o777)).unwrap();
    let cache_root = parent.path().join("cache");
    assert!(BuildSnapshotCache::new(&cache_root).is_err());
    assert!(!cache_root.exists());
}

#[cfg(unix)]
#[test]
fn digest_consistent_snapshot_with_traversal_path_is_rejected() {
    use std::io::Cursor;
    use tar::{Builder, EntryType, Header};

    let (context, plan) = fixture();
    let cache_root = TempDir::new().unwrap();
    let cache_root = private_cache_root(&cache_root);
    let cache = BuildSnapshotCache::new(&cache_root).unwrap();
    let stage = StageFilesystem::new().unwrap();
    for operation in &plan.copies {
        stage.apply_copy(&plan.context_root, operation).unwrap();
    }
    let key = cache.store(&plan, stage.root()).unwrap();
    stage.cleanup().unwrap();

    let mut malicious_tar = Vec::new();
    {
        let mut archive = Builder::new(Cursor::new(&mut malicious_tar));
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Regular);
        header.set_size(4);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        let name = header.as_mut_bytes();
        name[..100].fill(0);
        name[..9].copy_from_slice(b"../escape");
        header.set_cksum();
        archive.append(&header, Cursor::new(b"evil")).unwrap();
        archive.finish().unwrap();
    }
    let digest = hex::encode(Sha256::digest(&malicious_tar));
    let entry_path = cache
        .cache_directory()
        .join("entries")
        .join(format!("{}.json", key.replace(':', "-")));
    let mut entry: Value = serde_json::from_slice(&fs::read(&entry_path).unwrap()).unwrap();
    let original_blob_path = cache
        .cache_directory()
        .join("blobs/sha256")
        .join(entry["snapshot_digest"].as_str().unwrap());
    let malicious_blob_path = cache.cache_directory().join("blobs/sha256").join(&digest);
    fs::write(&malicious_blob_path, &malicious_tar).unwrap();
    fs::remove_file(original_blob_path).unwrap();
    entry["snapshot_digest"] = Value::String(digest.clone());
    entry["snapshot_size"] = Value::from(malicious_tar.len() as u64);
    fs::write(&entry_path, serde_json::to_vec(&entry).unwrap()).unwrap();

    let restore_root = TempDir::new().unwrap();
    let error = cache.restore(&plan, restore_root.path()).unwrap_err();
    assert!(
        format!("{error:#}").contains("unsafe path"),
        "unexpected traversal diagnostic: {error:#}"
    );
    assert!(!context.path().parent().unwrap().join("escape").exists());
    assert_eq!(fs::read_dir(restore_root.path()).unwrap().count(), 0);
    assert_eq!(cache.statistics().hits, 0);
}

#[cfg(unix)]
#[test]
fn digest_consistent_symlink_snapshot_is_rejected() {
    use std::io::Cursor;
    use tar::{Builder, EntryType, Header};

    let (context, plan) = fixture();
    let cache_root = TempDir::new().unwrap();
    let cache_root = private_cache_root(&cache_root);
    let cache = BuildSnapshotCache::new(&cache_root).unwrap();
    let stage = StageFilesystem::new().unwrap();
    for operation in &plan.copies {
        stage.apply_copy(&plan.context_root, operation).unwrap();
    }
    let key = cache.store(&plan, stage.root()).unwrap();
    stage.cleanup().unwrap();

    let mut malicious_tar = Vec::new();
    {
        let mut archive = Builder::new(Cursor::new(&mut malicious_tar));
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_link_name("../../outside").unwrap();
        header.set_cksum();
        archive
            .append_data(&mut header, "escape-link", Cursor::new([]))
            .unwrap();
        archive.finish().unwrap();
    }
    let digest = hex::encode(Sha256::digest(&malicious_tar));
    let entry_path = cache
        .cache_directory()
        .join("entries")
        .join(format!("{}.json", key.replace(':', "-")));
    let mut entry: Value = serde_json::from_slice(&fs::read(&entry_path).unwrap()).unwrap();
    let original_digest = entry["snapshot_digest"].as_str().unwrap().to_string();
    fs::write(
        cache.cache_directory().join("blobs/sha256").join(&digest),
        &malicious_tar,
    )
    .unwrap();
    fs::remove_file(
        cache
            .cache_directory()
            .join("blobs/sha256")
            .join(original_digest),
    )
    .unwrap();
    entry["snapshot_digest"] = Value::String(digest);
    entry["snapshot_size"] = Value::from(malicious_tar.len() as u64);
    fs::write(&entry_path, serde_json::to_vec(&entry).unwrap()).unwrap();

    let restore_root = TempDir::new().unwrap();
    let error = cache.restore(&plan, restore_root.path()).unwrap_err();
    assert!(
        format!("{error:#}").contains("unsupported tar entry type"),
        "unexpected symlink diagnostic: {error:#}"
    );
    assert_eq!(fs::read_dir(restore_root.path()).unwrap().count(), 0);
    assert!(!context.path().parent().unwrap().join("outside").exists());
}
