# Implementation audit

## Baseline and method

| Item | Finding |
|---|---|
| Repository | `FortiShield/MemoBuild`, root `/workspaces/MemoBuild` |
| Analyzed revision | Tag `v0.2.0`, commit `c9689e8c719a01b6671f57209107dbefd826a93f`; `HEAD` exactly matches the tag |
| Branch and worktree | Detached `HEAD` (`git status --short --branch` showed `## HEAD (no branch)`); clean at audit start |
| Available tags | `v0.1.0` at `f00171c6cc310665940afceee4b2e7ee4e853f1a`; `v0.2.0` at the analyzed commit |
| Rust package | One package, `memobuild 0.2.0`, edition 2021; no workspace declaration or `rust-version` in `Cargo.toml` |
| Declared MSRV | README badge says Rust 1.75+, but Cargo does not declare or test that minimum |
| Toolchain observed | `rustc 1.99.1`, `cargo 1.99.1`; `cargo tree --depth 1` completed and showed the direct package dependencies |
| Build features | Default empty; optional `server`, `containerd`, and `remote-exec` features in `Cargo.toml` |
| Validation | `cargo fmt --all -- --check` passed; a focused all-feature cache/core integration target passed 14 tests. The full `cargo test --all-features -- --nocapture` did not reach test execution: linker terminated with Bus error while the workspace filesystem was full (108 MB available afterward; `target/` was 14 GB). See the [original v0.2.0 validation status](./VALIDATION_PLAN.md#original-v020-validation-status) and the separate [MB-P0-001 validation record](./VALIDATION_PLAN.md#mb-p0-001-validation-on-productionv02x-hardening). |

No build script is declared. The direct dependencies include BLAKE3, `petgraph`, Tokio, Axum, Reqwest, SQLite, tar/gzip, Rayon, and `oci-spec`; however, `BuildGraph` is implemented as a `Vec<Node>` with hand-written traversal rather than using `petgraph` (`src/graph.rs`). Runtime environment variables read in Rust are `MEMOBUILD_CACHE_DIR`, `MEMOBUILD_JSON_LOGS`, `MEMOBUILD_WEBHOOK`, `MEMOBUILD_REGISTRY`, `MEMOBUILD_REPO`, and `MEMOBUILD_TOKEN`, plus the `MEMOBUILD_REMOTE_URL` client setting; the hash fingerprint reads `PATH`, `RUST_VERSION`, `NODE_ENV`, `LANG`, and `LC_ALL` (`src/main.rs`, `src/cache.rs`, `src/env/fingerprint.rs`). `SECURITY.md` documents TLS and rate-limit environment variables that were not found in the Rust implementation. CI in `.github/workflows/ci.yml` runs formatting, all-target/all-feature Clippy and tests, `cargo audit`, and release builds on Linux, macOS, and Windows. It uses the stable Rust toolchain rather than validating the README's 1.75+ minimum. The workflow installs `protoc` for its Rust jobs. There are Rust integration tests and one Criterion directory-hashing benchmark. No real-image runtime validation or BuildKit comparison is present in the inspected CI and tests.

The Rust release workflow (`.github/workflows/release.yml`) creates a GitHub release, builds Linux x86_64 and macOS x86_64/ARM64 assets, and publishes to crates.io. It does not build a Windows release asset, sign artifacts, or generate provenance/SBOM. The release is created before the build/publish jobs complete. `SECURITY.md` says dependencies are pinned exactly in `Cargo.toml`, but the manifest uses broad semver requirements (for example, `blake3 = "1"`); `Cargo.lock` records the repository's resolved versions, not exact published library dependency constraints. At the audit baseline, `SECURITY.md` also incorrectly described containerd isolation and CAS verification. Its execution claims are updated for MB-P0-001; cache-read verification remains unimplemented. Treat baseline claims as historical, not as evidence for current cache integrity.

## Executive assessment

**Original v0.2.0 status: BROKEN as an end-to-end container build engine.** The audited tag had useful component scaffolding—BLAKE3 file hashing, a graph model, a disk cache, HTTP cache interfaces, a local HTTP server, and OCI-shaped JSON/tar output—but the CLI did not carry real container filesystem state through the pipeline. Its default executor ran `RUN` commands through the host shell; cache hits did not restore files; and the exporter emitted synthetic marker files with an incorrect DiffID. The current hardening branch adds the narrow MB-P0-002 path described below; it does not repair the general cache/executor or establish general Docker compatibility.

The untrusted-build boundary is also inadequate: default local execution is not isolated. Separate remote execution server types bind to all interfaces and accept requests without authentication; the CLI scheduler/worker commands are placeholders rather than a configured deployment path. Do not expose these endpoints or run untrusted Dockerfiles against the default executor.

## Implementation inventory

| Subsystem | Entry points and principal types | Tests / evidence | Current status |
|---|---|---|---|
| CLI and build orchestration | `src/main.rs`: `main`, `run_build`, `run_graph`, `run_explain_cache`, `pull_base_images` | CLI routes through parser, graph, core, executor, and exporter; `pull_base_images` only prints image names | **PARTIAL** |
| Dockerfile parser | `src/docker/parser.rs`: `Instruction`, `parse_dockerfile` | `tests/executor_coverage_test.rs::test_dockerfile_parsing_simple` checks instruction count only | **BROKEN** for general Dockerfile semantics |
| Graph compiler | `src/docker/dag.rs`: `build_graph_from_instructions`; `src/graph.rs`: `Node`, `NodeKind`, `BuildGraph` | `tests/e2e_test.rs::test_dag_linking`, `test_parallel_levels`; several executor tests only inspect manually constructed graphs | **PARTIAL**; edges are mostly source-order edges, not semantic independence |
| Fingerprinting | `src/core.rs`; `src/hasher/{file_hasher,walker,ignore}.rs`; `src/env/fingerprint.rs` | Hash tests exercise simple file contents and glob examples | **PARTIAL**; correct content hashing is not wired into final node keys |
| Execution and filesystem state | `src/executor.rs`; `src/sandbox/{local,containerd,spec}.rs` | `tests/executor_coverage_test.rs` uses placeholder structural checks | **BROKEN** as isolated container execution or stateful build |
| Local/remote cache | `src/cache.rs`; `src/cache_utils.rs`; `src/remote_cache.rs`; `src/server/{mod,storage,metadata}.rs` | Put/get test checks only that calls return `Ok`; remote E2E asserts a cache-related counter | **PARTIAL** locally; **BROKEN** as reliable artifact restoration / remote reuse |
| OCI output and registry | `src/export/{layer,config,manifest,oci_exporter,registry}.rs` | `tests/reproducibility_test.rs` compares `index.json` only | **BROKEN** as a runnable image; registry protocol is incomplete |
| Remote execution | `src/remote_exec/{client,scheduler,server,worker,worker_server}.rs`; `src/remote_router/` | No demonstrated end-to-end worker integration in the CLI; server handlers are unauthenticated | **PARTIAL** scaffolding; **UNSAFE** if exposed |
| Optimizer / dependency analyzer | `src/ai/{ast,optimizer,ci}.rs` | Heuristic tests do not verify semantic dependency analysis | **PARTIAL**; comments call heuristics “AI/ML” but code is regex/string based |
| Reproducibility | `src/reproducible/normalize.rs`; `src/export/config.rs` | Index-only repeated export test | **PARTIAL**; `normalize_artifact` is a no-op |
| Monitoring and platform features | `src/dashboard/`, `src/loadtest.rs`, `src/remote_router/`, CLI `GenerateCi` / `GenerateK8s` | Load tests use random simulated outcomes and sleeps; CI/Kubernetes commands print success rather than emit configurations | **MISSING** as production integrations |

## Actual v0.2.0 build path

```text
CLI run_build(context_dir, Dockerfile argument)
  -> read Dockerfile relative to process current directory
  -> parse lines into Instruction values
  -> build_graph_from_instructions
  -> heuristic AI/dependency pass
  -> detect_changes (loads ignore rules from current directory)
  -> propagate dirty flags; compute composite hashes; propagate manifests
  -> IncrementalExecutor levels
       -> cache get: hit => mark clean and return bytes, with no filesystem restore
       -> miss => LocalSandbox runs supported RUN command through host sh/cmd
                  in workspace_dir; cache stores stdout bytes, not filesystem diff
  -> export_image: one synthetic marker-file tar layer per graph node
  -> optional registry push
```

Key evidence: `src/main.rs:147-255`; `src/core.rs:31-131`; `src/executor.rs:74-125,316-442`; `src/sandbox/local.rs:26-84`; `src/export/mod.rs:11-24`; `src/export/layer.rs:17-66`.

The graph is not the branchable semantic graph described in the whitepaper. `FROM` nodes have no parents; most other instructions depend on instruction `i - 1`; `RUN` may add COPY edges if the command text contains a source string. A later `FROM` does not reset stage state. For ordinary single-stage inputs this tends to form a chain; multiple `FROM` instructions become separate roots, but subsequent operations still depend on whichever instruction immediately preceded them. `BuildGraph::topological_order` does not detect cycles; it silently ignores already-visited edges.

```text
Typical parsed list: FROM -> WORKDIR -> COPY -> RUN -> COPY -> RUN
Actual edges:        root    i-1       i-1    i-1   i-1    i-1
                     + command-string COPY-source heuristic for RUN
```

## Subsystem findings

### Dockerfile parsing and context — v0.2.0 baseline

At the audited v0.2.0 revision, `parse_dockerfile` processed physical lines independently, split on whitespace, and constructed raw variants. It did not validate instruction grammar. It dropped `FROM` aliases/options and extra COPY arguments; malformed recognized instructions could disappear. Unknown and unimplemented instructions became `Instruction::Other`, and non-runnable graph nodes returned synthetic success. This could silently report a build as successful while omitting requested behavior.

### Strict parser/compiler — `production/v0.2.x-hardening`

`parse_dockerfile` returns located instructions or an error, preserves physical source line/column, rejects unknown keywords, empty/malformed arguments, malformed JSON command forms, unsupported parser directives, and backslash continuations. It recognizes Docker standard directives and legacy MemoBuild extension words explicitly; it never produces a generic `Other` no-op. The compiler accepts exactly one first-line `FROM scratch`, simple one-source context-relative `COPY`, and JSON-array `CMD`/`ENTRYPOINT`. External registry/local refs, digest refs, aliases, platform options, multiple stages, and every other instruction are rejected with a source-line diagnostic. COPY sources receive lexical traversal, existence, and symlink-boundary checks before staging.

The CLI compiles the Dockerfile before any cache lookup or command execution. Unsupported `RUN` is rejected before execution. For the supported path it uses `BuildSnapshotCache` only; the legacy executor/cache are not initialized. On a miss, COPY operations populate an owned temporary scratch rootfs and a verified snapshot is published; on a hit, the complete snapshot is verified and restored before OCI preparation. Stage cleanup succeeds before publishing the validated OCI candidate. `run_graph` and cache explanation also use the strict parser/compiler. `--pull` fails rather than pretending to fetch images.

Parser/compiler capability is deliberately distinct from production build support. The exact executable subset is `FROM scratch`, plain single-source context-relative COPY of regular files/directories, and JSON-array CMD/ENTRYPOINT. WORKDIR, RUN, ENV, ADD, ARG, USER, SHELL, LABEL, EXPOSE, VOLUME, HEALTHCHECK, STOPSIGNAL, ONBUILD, MAINTAINER, and custom GIT/RUN_EXTEND/COPY_EXTEND/HOOK are recognized and explicitly rejected at compilation. Unknown keywords fail at parsing. See the instruction-by-instruction matrix and base policy in [FEATURE_MATRIX.md](./FEATURE_MATRIX.md#mb-p0-003-and-mb-p0-002-instruction-capability-matrix).

The current base policy is one first `FROM scratch` only. It is an empty immutable base identity, not a registry/local image. External images require digest-only OCI references, verified manifest/layer bytes, explicit Linux/amd64 selection, rootfs unpacking, and digest/platform inputs in cache keys before acceptance. The repository has no complete resolver/unpacker/credential provider. RUN is unsupported; the separate Bubblewrap sandbox disables network access for its isolated execution API.

### MB-P0-002 staged filesystem and OCI path — `production/v0.2.x-hardening`

The supported CLI path creates a genuinely empty `StageFilesystem` in an owned temporary workspace and applies validated COPY operations to its rootfs. Sources must remain inside the canonical build context; `..`, absolute paths, symlinks anywhere in the copied tree, wildcards, special files, multiple sources, and flags are rejected. Destinations normalize within the rootfs. Regular-file and directory contents and supported permission bits are copied; stage cleanup makes nested read-only directories removable and is required to succeed before image publication. No Dockerfile command is run.

The exporter emits one deterministically ordered tar/gzip layer with normalized owner IDs and timestamps. Its layer digest covers compressed bytes; its DiffID covers the uncompressed tar stream. Config, manifest, index, platform, descriptor sizes/digests, and layer paths are internally validated. JSON-array command metadata is represented as argument arrays, with omitted fields omitted. `--reproducible` fixes image timestamps; the repeat-build test compares the manifest digest and image blobs for equivalent inputs. A statically linked executable ran from a generated scratch rootfs under runc 1.5.2. Docker loaded an archived OCI layout; inspecting the returned content ID showed `linux/amd64`, the expected `ENTRYPOINT`/`CMD` and rootfs layer, and `docker run` by image ID printed the expected output. Docker tag-based inspect/run was inconsistent even though `docker load` reported the tag; that reference-name behavior remains unresolved. No standalone `umoci`/`skopeo` validator is installed.

The narrow build path uses `BuildSnapshotCache` (`src/docker/build_cache.rs`), separate from the legacy general executor cache. Namespace `build-snapshots-v2` and key prefix `v2:blake3:` separate the current build-key algorithm from legacy v1 SHA-256 entries; no implicit migration or reinterpretation is attempted. The domain-separated BLAKE3 key hashes deterministic JSON containing cache/compiler/snapshot/export schema versions, MemoBuild package version, target OS/architecture, reproducibility setting, image reference, ordered normalized compiled operations, ordered COPY destinations and semantics, source paths/content BLAKE3/type/size/permission bits, and JSON command metadata. Source mtimes and ownership are excluded; owners and timestamps in snapshots are normalized to zero. Snapshot format v1 remains deterministic uncompressed tar of regular files and directories, ordered by path, preserving permission bits while rejecting symlinks, special files, unsafe or duplicate paths, non-normalized owner/time metadata, and malformed hierarchy. Snapshot blobs retain SHA-256 size/digest verification; this integrity digest is deliberately distinct from BLAKE3 build identity and OCI SHA-256 descriptors/DiffIDs. A cross-platform advisory file lock (`fs2`) on a cache-root lock file serializes cooperating process restores, publications, and GC so a blob cannot be removed during restore. Cache writes default to a 1 GiB logical-byte quota, configurable with `MEMOBUILD_CACHE_MAX_BYTES`; accounting includes every regular file under the v2 namespace (records, blobs, temporary files, and orphans), but excludes directory/inode and filesystem block-allocation overhead. Snapshot serialization is quota-limited while writing; atomic rename does not add a second copy. If space is needed, GC first removes temporary/orphan/corrupt records and unreferenced blobs, then evicts valid entries in lexicographic entry-key order. It never follows symlinks; quota refusal is explicit when no safe candidate remains. Publication uses temp files, file fsync, no-clobber atomic publication, and parent-directory fsync. On reopen, GC removes interrupted temporary files and orphan blobs; crash-boundary subprocess tests establish only a complete restorable entry or a miss. Restore still validates the complete archive before transactional extraction into an empty stage. Statistics include quota refusals, storage usage/limit, evictions, GC failures, and orphan cleanup. These guarantees assume private storage and cooperating MemoBuild writers; an actor controlling the cache can forge metadata and blobs. Power-loss behavior on every filesystem, hostile same-UID writers, quota accounting of allocated blocks, and legacy executor/remote cache correctness remain outside verified coverage.

The first target is Linux x86_64 (`linux/amd64`). This is a minimal COPY-to-OCI path, not a complete Docker-compatible engine. Docker imported and ran a static OCI image by content ID using the image's own command config; a tag-name lookup inconsistency remains and broad OCI portability is not established. External registry/base-image resolution and RUN-to-stage state capture remain open. The legacy graph/executor and remote caches remain unsound outside this build-specific cache.

### Dependency graph and cache keys

Most edges are source-order edges. The only extra RUN dependencies are selected by substring matching against prior COPY sources (`src/docker/dag.rs:74-92`); this is neither a complete model of file inputs nor a safe basis for concurrent filesystem mutation. The `parallelizable` field is a metadata flag. The executor partitions nodes at the same graph level, but these edges usually serialize a single-stage build; changing a RUN node's flag does not remove its dependency edges.

`Node::compute_node_key` hashes the node kind, instruction content, node-local env map, optional context/source hash, sorted dependency hashes, selected metadata, and a process environment fingerprint (`src/graph.rs:78-130`). However, `detect_changes` computes a filesystem hash into a local variable and does not assign it to `metadata.source_content_hash`; `compute_composite_hashes` then uses that metadata field, which defaults to `None` (`src/core.rs:36-75,118-131`; `src/graph.rs:66,107`). Thus COPY filesystem changes are not reliably included in the composite cache key. The node-local environment map is set only on an ENV node; it is not propagated to following RUN nodes, and WORKDIR state is not part of a later command's execution environment.

The source-order edge from an earlier stage is not reset at a later `FROM`, stage aliases are discarded, and `COPY --from` is parsed as an ordinary COPY source. Graph construction and cache correctness for multi-stage builds are therefore **BROKEN**.

### Filesystem fingerprinting

`hash_file` streams content in 64 KiB reads; `hash_dir` parallelizes file hashing and adds each relative filename and content digest (`src/hasher/file_hasher.rs:11-57`). The walker sorts returned paths, but suppresses traversal errors and follows no symlinks (`src/hasher/walker.rs:8-29`). Directory fingerprints omit directories, symlinks, file types beyond regular-file inclusion, modes, ownership, and other metadata that can affect COPY or command output. `hash_path` of a standalone file hashes contents only; missing paths hash their textual path instead of returning an error. `EnvFingerprint` includes only a fixed set of environment variables and detected tool versions (`src/env/fingerprint.rs:15-53`), even though the local child process inherits the rest of the host environment.

Ignore rules use basic `glob::Pattern` matching rather than Docker-compatible `.dockerignore` semantics; malformed patterns and file-read errors are silently discarded (`src/hasher/ignore.rs:8-48`). The root used by core change detection is the process working directory, not necessarily the build context. Concatenated path/hash components have no explicit length or type framing, and the node key similarly concatenates components without versioned domain separation.

`ArtifactManifest::from_dir` reads each file fully into memory and preserves walk order; its JSON hash therefore has no explicit canonical sort (`src/cache_utils.rs:27-66`). Its `reconstruct` method joins manifest-supplied paths into a destination without rejecting absolute paths or `..` (`src/cache_utils.rs:68-93`), making validation essential before any untrusted manifest reaches a worker.

### Execution and isolation — v0.2.0 baseline

At the audited `v0.2.0` revision, the default CLI selected `LocalSandbox` (`src/main.rs`); it invoked `sh -c` or `cmd /C` in the context directory, with no container, namespace, resource control, network policy, filesystem confinement, timeout, or cancellation (`src/sandbox/local.rs`). The command inherited the host process environment and overlaid only the node's environment map. A Dockerfile RUN could read or write host files with the invoking user's privileges. ENV/WORKDIR/COPY/FROM state was not realized as a staged container root filesystem; most non-RUN nodes were synthetic success.

The `v0.2.0` `ContainerdSandbox` created a temporary directory but did not start or wait for a task, mount a base filesystem, or capture a filesystem diff; it returned a simulated result (`src/sandbox/containerd.rs`).

### MB-P0-001 implementation on `production/v0.2.x-hardening`

The CLI and `IncrementalExecutor::new` now select `BubblewrapSandbox` by default. The only supported isolated target is Linux x86_64 with Bubblewrap (`bwrap`) installed and working unprivileged user namespaces. Initialization verifies the Bubblewrap executable and launches the required profile before allowing a build command to run; a missing runtime, unsupported platform, or namespace/mount failure returns an error. There is no automatic host-shell fallback. The existing local host-shell implementation is compiled only with `unsafe-local-execution` and must be explicitly selected with `--sandbox local`; selecting it emits a warning. `--sandbox containerd` now fails with an explanation that the existing containerd backend is simulated.

Each invocation copies the declared context into a newly owned temporary workspace. Bubblewrap gives the command a new user, PID, IPC, UTS, and network namespace; disables further user namespaces; starts a new session; sets `--die-with-parent`; and drops all capabilities. The only writable host bind is the staged workspace at `/workspace`. Host program/library directories and selected system certificate/cache paths are read-only. `/tmp`, `/var`, `/run`, `/home`, and `/root` are isolated tmpfs mounts capped at 256 MiB apiece, and ambient environment variables are removed before a small explicit environment is added. The bwrap child receives `RLIMIT_CORE=0`, `RLIMIT_CPU=1800s`, `RLIMIT_AS=8 GiB`, `RLIMIT_FSIZE=4 GiB`, `RLIMIT_NPROC=256`, and `RLIMIT_NOFILE=1024`; each captured output stream is capped at 16 MiB. The default wall-clock timeout is 30 minutes. Timeout kills and waits for Bubblewrap; Bubblewrap's parent-death behavior terminates its namespace processes. Temporary directories are owned by `TempDir`, and execution/cleanup errors are surfaced rather than skipping cleanup.

Focused integration tests on this Linux x86_64 host with Bubblewrap 0.9.0 passed: S-01 attempted to write a protected host `/tmp` file while confirming the workspace was writable, S-03 configured a nonexistent runtime and confirmed initialization failed without running a host-shell marker, a CLI invocation with Bubblewrap removed from `PATH` failed without creating a host-shell marker, a supported shell command read staged context successfully, a command exit code of 37 was preserved, the command network-namespace identifier differed from the host's, a timed-out child did not write its late marker and left no sandbox directory, and a forced temporary-directory cleanup failure was returned and then cleaned up by the test. These tests do not prove resistance to every kernel/runtime exploit or every filesystem escape.

This is an execution boundary, not a complete build engine. It stages the context rather than a parent image/rootfs; changes inside the workspace are discarded during cleanup and are not restored from cache or emitted as OCI layers. The writable staged workspace has no aggregate disk quota, and RLIMITs are process limits rather than cgroup isolation. Bubblewrap and the host kernel remain trusted; no seccomp policy, cgroup CPU/memory/PID controller, disk quota, or syscall filter is installed. Network isolation is namespace-level and has not been validated against every kernel network configuration. macOS, Windows, and non-x86_64 Linux builds fail closed and cannot execute commands through this backend. Remote execution remains subject to the separate remote-service trust and authentication gaps recorded elsewhere.

**Updated guidance:** the default CLI now fails closed when this documented Bubblewrap prerequisite is unavailable. Do not enable `unsafe-local-execution` for untrusted build inputs, and do not infer staged filesystem, cache-restoration, Docker compatibility, or runnable OCI guarantees from MB-P0-001.

### Cache and content-addressed storage

The legacy `LocalCache` keeps an in-memory map of index metadata and stores artifact bytes in files; there is no separate L1 artifact byte cache. Its index is JSON at `~/.memobuild/cache/index.json` or `MEMOBUILD_CACHE_DIR`. Invalid JSON silently becomes an empty index; artifact writes and index replacement are non-atomic; reads do not verify digest or stored size; `exists` checks only index membership, not artifact availability (`src/cache.rs:46-113`). It has no format version, quota, or garbage collector. The build-specific `BuildSnapshotCache` uses the isolated `build-snapshots-v2` namespace and is used only by the supported scratch/COPY path; these controls do not repair the legacy cache.

On any cache hit, the executor discards returned bytes and returns `(false, true)` without restoring an output snapshot (`src/executor.rs:330-334`). On a miss it stores command stdout; non-runnable nodes store empty data (`src/executor.rs:414-442`). Cache-hit counters therefore do not establish correctness or useful filesystem reuse.

Remote cache protocols are inconsistent: the HTTP client compresses payloads before PUT (`src/remote_cache.rs:157-179,207-222`), while server handlers verify BLAKE3 over the received body against the URL key (`src/server/mod.rs:490-506,602-618`). In the ordinary build path, the key is a composite node key, not necessarily the BLAKE3 of artifact bytes. Layer keys are computed over uncompressed chunks and then those chunks are gzip-compressed before PUT. The client also decompresses GET responses. This protocol cannot be treated as verified remote artifact reuse. The server's BLAKE3 check is a useful integrity control for the exact wire body, but does not repair the key/encoding mismatch or validate client-side reads. The first-level `has/get/put` calls use retries and timeouts, but layered operations do not consistently apply either (`src/remote_cache.rs:94-239`). During execution, cache-read errors are logged and the build proceeds; cache-write errors are logged and ignored (`src/executor.rs:330-333,442-443`). Manifest-upload tasks in `run_build` are spawned without awaiting their results (`src/main.rs:192-202`).

### OCI and registry

At v0.2.0, `create_layer_tar` wrote marker files and calculated an invalid DiffID. This is historical baseline behavior, not the current hardening exporter. The OCI Image Specification defines DiffID as the digest over the uncompressed layer tar ([config.md](https://github.com/opencontainers/image-spec/blob/main/config.md#layer-diffid)); the layer media type identifies the gzip-compressed tar payload ([layer.md](https://github.com/opencontainers/image-spec/blob/main/layer.md)).

The v0.2.0 config hard-coded `linux/amd64`, `Cmd: ["/bin/sh"]`, and `/` as working directory; those defaults did not reflect Dockerfile semantics. On the hardening branch, the narrow builder emits actual regular-file/directory entries and content into one deterministic gzip tar layer, normalizes ownership/timestamps, hashes compressed bytes for the descriptor and uncompressed tar bytes for the DiffID, sets `rootfs.diff_ids`, propagates JSON command metadata, and validates the OCI layout/descriptors/platform before publication. Integration tests inspect archive contents, descriptor sizes/digests, DiffIDs, config, and equivalent reproducible builds. External OCI validation/runtime execution remains a separate requirement and must not be inferred from the internal structural validator.

The registry client supports token bearer headers only, lacks a full challenge/auth flow, uploads every manifest to the `latest` tag regardless of the requested tag, and `--pull` does not acquire base images (`src/export/registry.rs:20-29,230-249`; `src/main.rs:366-373`). The CLI does not expose an output image name. Do not claim registry interoperability or runnable OCI images.

### Reproducibility

The exporter fixes gzip header mtime and sets tar metadata to zero, but hashes only synthetic marker content. `normalize_artifact` returns its input unchanged (`src/reproducible/normalize.rs:33-37`). The existing reproducibility test compares only `index.json` (`tests/reproducibility_test.rs:36-66`), not all blobs, rootfs content, runtime behavior, or fresh-machine builds. The test does not verify reproducibility of real build outputs.

### Remote services and security boundaries

The cache HTTP server binds `127.0.0.1` and has no authentication (`src/server/mod.rs:72-95`). The remote execution scheduler and worker server bind `0.0.0.0` and accept JSON execution requests without authentication (`src/remote_exec/server.rs:18-28`; `src/remote_exec/worker_server.rs:17-27`). `WorkerNode::execute` turns a request into a shell command for its configured sandbox and may reconstruct an input manifest (`src/remote_exec/worker.rs:24-69`). The CLI's `start_scheduler` and `start_worker` currently print startup messages and return without instantiating these servers (`src/main.rs:401-427`), so those CLI features are not operational either.

Remote cache server storage and SQLite metadata persist locally, but writes are not atomically published; an existing blob is assumed correct without re-reading it (`src/server/storage.rs:26-57`). The cache server has no auth, tenant namespace, request quotas, transfer-size policy, or explicit authorization. Loopback binding limits default exposure but is not a substitute for authentication if deployment changes the bind address or proxy topology.

Credentials are accepted as a registry token environment variable and attached as bearer auth (`src/main.rs:257-268`; `src/export/registry.rs:25-29`). There is no dedicated secret mount model. Build arguments and secret redaction are not implemented. Supply-chain evidence such as SBOM, provenance, dependency policy, or signed releases was not established by inspected code.

### Tests, benchmarks, and documentation claims

- `tests/cache_and_core_test.rs` tests a put/get call returns `Ok`, but does not assert the bytes equal the input; node-key tests assert a nonempty prefilled string rather than executing key generation.
- `tests/executor_coverage_test.rs` includes tests that explicitly assert placeholder graph states without invoking `propagate_dirty`, executor execution, or cycle rejection. `test_multi_stage_build_structure` only checks that at least two `FROM` instructions were parsed.
- `tests/e2e_test.rs::test_end_to_end_build_with_remote_cache` checks that some graph node is clean after the second executor call; executor marks nodes clean after both cache misses and hits. It does not verify remote upload, command skipping, artifact contents, or restoration.
- `tests/reproducibility_test.rs` compares only index JSON. `tests/scalability_test.rs` simulates cache outcomes with random values and sleeps; `benches/core_bench.rs` hashes 100 tiny files and does not measure build invalidation, restoration, or comparison to BuildKit.
- README and `docs/VISION.md` claims such as L1/L2/L3 working tiers, safe OCI output, rename detection, and selective incremental recomputation are not demonstrated by the implementation. README also refers to `src/oci/mod.rs`, which is absent; the implementation is in `src/export/`.

## Competitive baseline

The project vision contrasts MemoBuild with a linear Docker build. That is not a fair modern baseline: Docker's official [BuildKit documentation](https://docs.docker.com/build/buildkit/) describes a concurrent build graph solver, parallel independent stages, incremental context transfer, and content-addressable graph/cache behavior. Docker's official [cache backend documentation](https://docs.docker.com/build/cache/backends/) describes local and external cache import/export, including registry and CI backends. These capabilities are not evidence that MemoBuild lacks any potential differentiator, but they mean a performance or feature advantage must be demonstrated with equivalent contexts, outputs, cache states, and hardware.

No competitive advantage is verified. Potential finer-grained dependency tracking or efficient snapshots remain hypotheses until correctness is established and the controlled benchmark matrix in [the validation plan](./VALIDATION_PLAN.md#benchmark-methodology) is run.
