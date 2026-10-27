use flate2::read::GzDecoder;
use memobuild::docker::{dag, parser, stage::StageFilesystem};
use memobuild::export::{
    config::OCIConfig,
    manifest::{OCIIndex, OCIManifest},
    oci_exporter::validate_oci_layout,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Output;
use tar::Archive;
use tempfile::TempDir;

fn make_context() -> TempDir {
    let context = TempDir::new().expect("create build context");
    fs::create_dir_all(context.path().join("app/nested")).expect("create context directories");
    fs::write(
        context.path().join("app/nested/message.txt"),
        "hello from scratch\n",
    )
    .expect("write context file");
    fs::write(context.path().join("payload.bin"), [0, 1, 2, 255]).expect("write payload");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            context.path().join("app"),
            fs::Permissions::from_mode(0o750),
        )
        .expect("set source directory permissions");
        fs::set_permissions(
            context.path().join("app/nested"),
            fs::Permissions::from_mode(0o710),
        )
        .expect("set nested directory permissions");
        fs::set_permissions(
            context.path().join("app/nested/message.txt"),
            fs::Permissions::from_mode(0o751),
        )
        .expect("set executable file permissions");
    }
    fs::write(
        context.path().join("Dockerfile"),
        "FROM scratch\n\
         COPY app /opt/app/\n\
         COPY payload.bin /payload.bin\n\
         ENTRYPOINT [\"/opt/app/nested/message.txt\", \"two words\"]\n\
         CMD [\"--mode\", \"fast path\"]\n",
    )
    .expect("write Dockerfile");
    context
}

fn run_build(context: &Path) -> String {
    let cache = TempDir::new().expect("create cache parent");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(cache.path(), fs::Permissions::from_mode(0o700))
            .expect("restrict cache parent permissions");
    }
    let cache_path = cache.path().join("cache");
    let diagnostic = run_build_with_cache(context, &cache_path);
    assert!(
        cache_path.join("build-snapshots-v2/entries").is_dir(),
        "successful build should publish a verified filesystem snapshot"
    );
    diagnostic
}

fn run_build_with_cache(context: &Path, cache_path: &Path) -> String {
    let output = invoke_build(context, cache_path);
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{diagnostic}");
    diagnostic
}

fn invoke_build(context: &Path, cache_path: &Path) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_memobuild"))
        .args(["build", "--file", "Dockerfile", "--reproducible"])
        .current_dir(context)
        .env("MEMOBUILD_CACHE_DIR", cache_path)
        .output()
        .expect("run MemoBuild")
}

#[test]
fn cache_quota_failure_does_not_publish_an_oci_image() {
    let context = make_context();
    let cache_parent = TempDir::new().expect("create cache parent");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(cache_parent.path(), fs::Permissions::from_mode(0o700))
            .expect("restrict cache parent permissions");
    }
    let cache_root = cache_parent.path().join("cache");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_memobuild"))
        .args(["build", "--file", "Dockerfile", "--reproducible"])
        .current_dir(context.path())
        .env("MEMOBUILD_CACHE_DIR", &cache_root)
        .env("MEMOBUILD_CACHE_MAX_BYTES", "128")
        .output()
        .expect("run build with an insufficient cache quota");
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{diagnostic}");
    assert!(diagnostic.contains("quota exceeded"), "{diagnostic}");
    assert!(
        !context.path().join(".memobuild-output").exists(),
        "quota failure must not publish an OCI layout"
    );
}

fn published_layout(context: &Path) -> PathBuf {
    let image_dir = context
        .join(".memobuild-output")
        .join("memobuild-demo-latest");
    let layouts: Vec<_> = fs::read_dir(&image_dir)
        .expect("read image output directory")
        .map(|entry| entry.expect("read image output entry").path())
        .collect();
    assert_eq!(layouts.len(), 1, "one digest-addressed layout is expected");
    layouts.into_iter().next().expect("layout path")
}

fn layer_entries(
    layout: &Path,
    manifest: &OCIManifest,
    expected_diff_ids: &[String],
) -> BTreeMap<String, (u32, Vec<u8>)> {
    let layer = &manifest.layers[0];
    let blob_path = layout
        .join("blobs/sha256")
        .join(layer.digest.strip_prefix("sha256:").expect("sha256 digest"));
    let compressed = fs::read(&blob_path).expect("read compressed layer blob");
    assert_eq!(compressed.len() as u64, layer.size);
    assert_eq!(
        format!("sha256:{}", hex::encode(Sha256::digest(&compressed))),
        layer.digest
    );

    let mut decoder = GzDecoder::new(compressed.as_slice());
    let mut uncompressed = Vec::new();
    decoder
        .read_to_end(&mut uncompressed)
        .expect("decompress layer");
    let diff_id = format!("sha256:{}", hex::encode(Sha256::digest(&uncompressed)));
    assert_eq!(expected_diff_ids, &[diff_id]);

    let mut archive = Archive::new(uncompressed.as_slice());
    let mut entries = BTreeMap::new();
    for entry in archive.entries().expect("read layer tar entries") {
        let mut entry = entry.expect("read tar entry");
        let path = entry
            .path()
            .expect("layer path")
            .to_string_lossy()
            .into_owned();
        let mode = entry.header().mode().expect("tar entry mode");
        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .expect("read layer entry contents");
        entries.insert(path, (mode, contents));
    }
    entries
}

#[test]
fn scratch_copy_build_publishes_actual_valid_oci_layout_and_metadata() {
    let context = make_context();
    let stage = StageFilesystem::new().expect("create scratch stage");
    assert_eq!(
        fs::read_dir(stage.root())
            .expect("read initial scratch root")
            .count(),
        0,
        "FROM scratch must start genuinely empty"
    );
    let workspace = stage.workspace_path().to_path_buf();
    stage.cleanup().expect("clean empty stage");
    assert!(
        !workspace.exists(),
        "stage lifecycle must remove its workspace"
    );

    run_build(context.path());
    let layout = published_layout(context.path());
    let manifest_digest =
        validate_oci_layout(&layout).expect("OCI layout structural and digest validation");
    assert_eq!(
        layout.file_name().and_then(|name| name.to_str()),
        Some(manifest_digest.trim_start_matches("sha256:"))
    );

    let index: OCIIndex =
        serde_json::from_slice(&fs::read(layout.join("index.json")).unwrap()).unwrap();
    let manifest_descriptor = &index.manifests[0];
    assert_eq!(manifest_descriptor.platform.as_ref().unwrap().os, "linux");
    assert_eq!(
        manifest_descriptor.platform.as_ref().unwrap().architecture,
        "amd64"
    );
    let manifest_path = layout
        .join("blobs/sha256")
        .join(manifest_descriptor.digest.trim_start_matches("sha256:"));
    let manifest: OCIManifest = serde_json::from_slice(&fs::read(manifest_path).unwrap()).unwrap();
    let config_path = layout
        .join("blobs/sha256")
        .join(manifest.config.digest.trim_start_matches("sha256:"));
    let config: OCIConfig = serde_json::from_slice(&fs::read(config_path).unwrap()).unwrap();
    assert_eq!(config.os, "linux");
    assert_eq!(config.architecture, "amd64");
    assert_eq!(
        config.config.entrypoint,
        Some(vec![
            "/opt/app/nested/message.txt".to_string(),
            "two words".to_string()
        ])
    );
    assert_eq!(
        config.config.cmd,
        Some(vec!["--mode".to_string(), "fast path".to_string()])
    );
    assert!(config.config.env.is_none());
    assert!(config.config.working_dir.is_none());
    assert_eq!(config.rootfs.diff_ids.len(), manifest.layers.len());

    let entries = layer_entries(&layout, &manifest, &config.rootfs.diff_ids);
    assert_eq!(
        entries["opt/app/nested/message.txt"].1,
        b"hello from scratch\n"
    );
    assert_eq!(entries["payload.bin"].1, [0, 1, 2, 255]);
    #[cfg(unix)]
    {
        assert_eq!(entries["opt"].0, 0o755);
        assert_eq!(entries["opt/app"].0, 0o750);
        assert_eq!(entries["opt/app/nested/message.txt"].0, 0o751);
        assert_eq!(entries["opt/app/nested"].0, 0o710);
    }

    let source = context.path().join("app/nested/message.txt");
    assert_eq!(
        fs::read_to_string(source).unwrap(),
        "hello from scratch\n",
        "build must not modify its context"
    );
}

#[cfg(unix)]
#[test]
fn stage_cleanup_handles_read_only_copied_directories() {
    use std::os::unix::fs::PermissionsExt;

    let context = TempDir::new().expect("create build context");
    let source = context.path().join("restricted/nested");
    fs::create_dir_all(&source).expect("create restricted source");
    fs::write(source.join("payload"), "stage cleanup").expect("write payload");
    fs::set_permissions(
        context.path().join("restricted"),
        fs::Permissions::from_mode(0o550),
    )
    .expect("set source directory permissions");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o500))
        .expect("set nested source permissions");

    let dockerfile = "FROM scratch\nCOPY restricted /restricted/\n";
    let plan = dag::compile_dockerfile(
        parser::parse_dockerfile(dockerfile).expect("parse Dockerfile"),
        context.path().to_path_buf(),
    )
    .expect("compile supported build");
    let stage = StageFilesystem::new().expect("create stage");
    stage
        .apply_copy(&plan.context_root, &plan.copies[0])
        .expect("copy restricted directories");
    let workspace = stage.workspace_path().to_path_buf();

    stage.cleanup().expect("clean restricted stage");
    assert!(
        !workspace.exists(),
        "cleanup must remove nested read-only stage directories"
    );
}

#[test]
fn cli_restores_snapshot_cache_across_builds_and_rejects_corruption() {
    let context = make_context();
    let cache_parent = TempDir::new().expect("create cache parent");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(cache_parent.path(), fs::Permissions::from_mode(0o700))
            .expect("restrict cache parent permissions");
    }
    let cache_root = cache_parent.path().join("cache");
    let first = run_build_with_cache(context.path(), &cache_root);
    assert!(first.contains("filesystem snapshot cache miss"), "{first}");
    let first_layout = published_layout(context.path());
    let first_digest = validate_oci_layout(&first_layout).unwrap();

    fs::remove_dir_all(context.path().join(".memobuild-output"))
        .expect("remove image output to force publication from cached rootfs");
    let second = run_build_with_cache(context.path(), &cache_root);
    assert!(second.contains("filesystem snapshot cache hit"), "{second}");
    let restored_layout = published_layout(context.path());
    assert_eq!(
        validate_oci_layout(&restored_layout).unwrap(),
        first_digest,
        "cache restore must reproduce the same OCI image"
    );

    let entries_dir = cache_root.join("build-snapshots-v2/entries");
    let entry_path = fs::read_dir(&entries_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let entry: Value = serde_json::from_slice(&fs::read(&entry_path).unwrap()).unwrap();
    let blob_path = cache_root
        .join("build-snapshots-v2/blobs/sha256")
        .join(entry["snapshot_digest"].as_str().unwrap());
    let mut bytes = fs::read(&blob_path).unwrap();
    bytes[0] ^= 1;
    fs::write(&blob_path, bytes).expect("corrupt cached snapshot");

    fs::remove_dir_all(context.path().join(".memobuild-output"))
        .expect("remove image output before corrupt-cache attempt");
    let failed = invoke_build(context.path(), &cache_root);
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&failed.stdout),
        String::from_utf8_lossy(&failed.stderr)
    );
    assert!(!failed.status.success(), "corrupt cache must fail closed");
    assert!(
        diagnostic.contains("snapshot cache blob"),
        "corrupt cache must report an integrity error: {diagnostic}"
    );
    assert!(
        !context.path().join(".memobuild-output").exists(),
        "corrupt cache must not publish an image"
    );
}

#[test]
fn reproducible_builds_have_identical_manifest_digest_and_blob_bytes() {
    let first = make_context();
    let second = make_context();
    run_build(first.path());
    run_build(second.path());
    let first_layout = published_layout(first.path());
    let second_layout = published_layout(second.path());
    let first_digest = validate_oci_layout(&first_layout).unwrap();
    let second_digest = validate_oci_layout(&second_layout).unwrap();
    assert_eq!(first_digest, second_digest);

    let shared_files = [
        PathBuf::from("index.json"),
        PathBuf::from("oci-layout"),
        PathBuf::from("blobs/sha256").join(first_digest.trim_start_matches("sha256:")),
    ];
    for relative in shared_files {
        assert_eq!(
            fs::read(first_layout.join(&relative)).unwrap(),
            fs::read(second_layout.join(&relative)).unwrap()
        );
    }
    let first_index: OCIIndex =
        serde_json::from_slice(&fs::read(first_layout.join("index.json")).unwrap()).unwrap();
    let first_manifest_path = first_layout.join("blobs/sha256").join(
        first_index.manifests[0]
            .digest
            .trim_start_matches("sha256:"),
    );
    let manifest: OCIManifest =
        serde_json::from_slice(&fs::read(first_manifest_path).unwrap()).unwrap();
    let second_index: OCIIndex =
        serde_json::from_slice(&fs::read(second_layout.join("index.json")).unwrap()).unwrap();
    let second_manifest_path = second_layout.join("blobs/sha256").join(
        second_index.manifests[0]
            .digest
            .trim_start_matches("sha256:"),
    );
    let second_manifest: OCIManifest =
        serde_json::from_slice(&fs::read(second_manifest_path).unwrap()).unwrap();
    for (first_layer, second_layer) in manifest.layers.iter().zip(&second_manifest.layers) {
        assert_eq!(first_layer.digest, second_layer.digest);
        assert_eq!(
            fs::read(
                first_layout
                    .join("blobs/sha256")
                    .join(first_layer.digest.trim_start_matches("sha256:"))
            )
            .unwrap(),
            fs::read(
                second_layout
                    .join("blobs/sha256")
                    .join(second_layer.digest.trim_start_matches("sha256:"))
            )
            .unwrap()
        );
    }
}

#[test]
fn unsupported_and_invalid_builds_fail_without_publishing_an_image() {
    let context = make_context();
    let invalid_dockerfiles = [
        "FROM scratch\nRUN touch host-marker\n",
        "FROM scratch\nUNKNOWN do-something\n",
        "FROM alpine:latest\n",
        "FROM scratch\nFROM scratch\n",
        "FROM scratch\nCOPY --chmod=755 payload.bin /payload\n",
        "FROM scratch\nCOPY app/nested/* /opt/\n",
        "FROM scratch\nCOPY app/nested/message.txt payload.bin /opt/\n",
        "FROM scratch\nCOPY payload.bin ../../outside\n",
        "FROM scratch\nCOPY absent.txt /absent\n",
        "FROM scratch\nCMD /shell-form\n",
    ];
    for dockerfile in invalid_dockerfiles {
        fs::write(context.path().join("Dockerfile"), dockerfile).unwrap();
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_memobuild"))
            .args(["build", "--file", "Dockerfile"])
            .current_dir(context.path())
            .output()
            .expect("run invalid build");
        assert!(
            !output.status.success(),
            "invalid Dockerfile succeeded: {dockerfile}"
        );
        assert!(
            !context.path().join(".memobuild-output").exists(),
            "invalid Dockerfile published output: {dockerfile}"
        );
        assert!(
            !context.path().join("host-marker").exists(),
            "RUN was executed on the host"
        );
    }
}

#[cfg(unix)]
#[test]
fn symlink_sources_are_rejected_before_stage_or_image_publication() {
    use std::os::unix::fs::symlink;

    let context = TempDir::new().expect("create context");
    let outside = TempDir::new().expect("create outside fixture");
    fs::write(outside.path().join("protected"), "host data").unwrap();
    symlink(
        outside.path().join("protected"),
        context.path().join("escape"),
    )
    .unwrap();
    let dockerfile = "FROM scratch\nCOPY escape /protected\n";
    fs::write(context.path().join("Dockerfile"), dockerfile).unwrap();
    let plan = dag::compile_dockerfile(
        parser::parse_dockerfile(dockerfile).unwrap(),
        context.path().to_path_buf(),
    );
    assert!(plan.unwrap_err().to_string().contains("symlink"));
    assert!(!context.path().join(".memobuild-output").exists());
    assert_eq!(
        fs::read_to_string(outside.path().join("protected")).unwrap(),
        "host data"
    );
}
