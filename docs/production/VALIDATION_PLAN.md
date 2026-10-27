# Validation plan

## MB-P0-002A OCI interoperability and MB-P0-004/005 snapshot cache follow-up

This follow-up retained the dirty `production/v0.2.x-hardening` worktree at `c9689e8c719a01b6671f57209107dbefd826a93f` and did not commit or push. Docker image store was confirmed empty before import testing. The task-owned target `/tmp/memobuild-p0-002a-target.5cBNjs` was created after checking `/tmp` space and removed after final validation.

| Command / check | Result | Evidence |
|---|---|---|
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-002a-target cargo test --test dockerfile_strict -- --nocapture` | **PASS: 14 tests** | Strict supported subset, unknown/malformed/unsupported instructions, stage/base policy, COPY path/symlink errors, missing-source diagnostic, and reject-before-execution. |
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-002a-target.5cBNjs cargo test --test build_snapshot_cache --test staged_oci_build --test dockerfile_strict --test sandbox_isolation -- --nocapture` | **PASS: 31 tests** | Current focused run: snapshot key sensitivity, fresh-process restore, content/mode equality, corrupted/missing blob rejection; deterministic implicit directory modes; scratch/COPY OCI output, reproducibility, cache hit/miss and publication failure; strict syntax rejection; MB-P0-001 isolation regressions. |
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-002a-target.5cBNjs cargo test --all-features -- --nocapture` | **PASS: 102 tests; 0 failed** | All unit, integration, and doc-test targets completed after the final stage-permission change. |
| `cargo fmt --all -- --check`; `git diff --check` | **PASS** | Formatting and whitespace checks exited 0 after the final source/test/doc edits. |
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-002a-target.5cBNjs cargo clippy --all-targets --all-features -- -D warnings` | **FAIL: 17 baseline diagnostics** | Sixteen `clippy::double_must_use` diagnostics and one `clippy::useless_borrows_in_formatting` in `src/export/registry.rs`; no diagnostic points to the new build-cache code. |
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-002a-target.5cBNjs cargo clippy --all-targets --all-features -- -D warnings -A clippy::double_must_use -A clippy::useless_borrows_in_formatting` | **PASS** | All targets/features lint clean when only the two known baseline lint categories are allowed. |
| In-process OCI layout validation (`validate_oci_layout`) | **PASS** | Layout/index/manifest/config descriptors, platform, blob sizes/digests, media type, DiffID, and layer entries validated. |
| Rootless `runc` smoke using an extracted generated rootfs | **PASS** | Static executable printed `OCI-RUNTIME-SMOKE-OK`; runc 1.5.2, Linux x86_64. |
| Standalone OCI validator (`umoci`/`skopeo`) | **NOT RUN** | Neither executable is installed. The runtime smoke used `runc`; internal OCI descriptor validation is not represented as an external validator. |
| Docker OCI-layout archive import, image inspection, and run | **PASS by content ID; tag lookup caveat** | A final post-cache build was imported from an OCI-layout archive. Docker inspection by returned image ID showed `linux/amd64`, `ENTRYPOINT ["/hello"]`, and `CMD ["from-cmd"]`; `docker run --platform linux/amd64 <image-id>` executed the image's own config and printed `OCI-CACHE-PATH-SMOKE-OK`. Earlier tag-based inspect/run was inconsistent despite `docker load` listing the tag. The task-created image and fixture were removed. |

The Docker runtime consumed the imported image's own configuration; unlike the earlier runc smoke, this demonstrates that configured `ENTRYPOINT` and `CMD` work through an independent importer/runtime path. It does not resolve the daemon's tag-name lookup inconsistency or prove broad OCI compatibility. The build-specific local cache uses versioned SHA-256 keys over ordered COPY inputs/destinations, source content/type/mode, command metadata, platform, and base identity. Snapshot tar blobs are content-addressed and size/digest verified before transactional restore. Legacy executor and remote caches remain unused and unsound.

## MB-P0-004/005 cache integrity verification update

This verification started from `production/v0.2.x-hardening` at `c9689e8c719a01b6671f57209107dbefd826a93f` with the intentional hardening worktree already dirty. Existing changes, including the untracked audit documents and `node_modules/`/`pnpm-lock.yaml`, were preserved. `/workspaces` had 9.5 GiB available and the existing `target/` occupied 3.8 GiB, so validation reused that target instead of creating a second large build tree.

| Command / check | Result | Evidence |
|---|---|---|
| `cargo test --test build_snapshot_cache -- --nocapture` | **PASS: 17 tests** | Final focused cache run includes rejection of group/world-writable cache ancestors before creating cache state. |
| `cargo test --all-features -- --nocapture` | **PASS: 115 tests; 0 failed** | Final all-target run, including cache, parser/compiler, staging/OCI, and isolation integration coverage. |
| `cargo fmt --all -- --check`; `git diff --check` | **PASS** | Final formatting and whitespace checks. |
| `cargo clippy --all-targets --all-features -- -D warnings` | **FAIL: 17 existing diagnostics** | Sixteen `clippy::double_must_use` async-trait-expanded diagnostics and one `clippy::useless_borrows_in_formatting` in `src/export/registry.rs`; none point to the cache changes. |
| `cargo clippy --all-targets --all-features -- -D warnings -A clippy::double_must_use -A clippy::useless_borrows_in_formatting` | **PASS** | Final all-target lint run passes when only the two documented baseline categories are allowed. |
| Snapshot/OCI structural validation | **PASS** | Cache archive schema/path/type/mode/owner/time checks; verified blob size/SHA-256; cold/hit filesystem tree equivalence; CLI cache-hit OCI digest equality; existing `validate_oci_layout` descriptor, blob, platform, DiffID, and config tests pass. |
| External OCI validator/runtime smoke | **NOT RUN for this verification** | `runc` is installed; `umoci` and `skopeo` are not. This cache-only change did not rerun Docker import/runtime. The separate Docker import and run evidence above remains from the preceding MB-P0-002A check. |

The cache key now covers MemoBuild package/compiler and explicit cache/compiler/snapshot/export schema versions; normalized compiled operation order; supported COPY semantics and source path/content/type/size/permission metadata; base and target platform; command metadata; reproducibility mode; and image reference. Snapshot v1 is deterministic uncompressed tar, preserving directory layout and permission bits while normalizing owner IDs/timestamps to zero. Cache stats are per cache instance/process; evictions and rebuild-after-error remain zero because eviction is not implemented and corruption fails closed rather than falling back to a rebuild. Same-key concurrent identical writers are tested in-process and across child processes; separate-process restore is tested. A `/dev/full` fixture verifies tar serialization propagates `ENOSPC`, and failed serialization does not publish cache entries/blobs; the actual cache filesystem was not filled. Abrupt process termination at every fsync boundary, quotas/GC, and adversarial writers controlling both cache records and blobs remain unverified or unsupported. This paragraph records the MB-P0-004/005 implementation before the MB-P1-002 update below.

## MB-P1-002 cache identity, quota, garbage collection, and crash recovery

Starting state was `production/v0.2.x-hardening` at `c9689e8c719a01b6671f57209107dbefd826a93f`, with the dirty hardening worktree, audit files, `node_modules/`, and `pnpm-lock.yaml` preserved. Before this change, the full all-features suite passed **115 tests**. Strict Clippy failed on **17 baseline diagnostics** (16 `clippy::double_must_use` from async-trait expansions and one `clippy::useless_borrows_in_formatting` in `src/export/registry.rs`); the check allowing only those two categories passed. BLAKE3 was already a direct dependency; `fs2` was added as the cross-platform advisory file-lock dependency and recorded in `Cargo.lock`.

| Command / check | Result | Evidence |
|---|---|---|
| `cargo test --lib --test build_snapshot_cache --test staged_oci_build -- --nocapture` | **PASS: 55 tests** | Cache unit/integration tests plus staged OCI build tests; includes BLAKE3 versioning, quota refusal/eviction, cache reopen cleanup, concurrent GC/restore and fresh-root initialization, truncated records, eight subprocess interruption points, and quota refusal with no OCI publication. |
| `cargo test --all-features -- --nocapture` | **PASS: 125 tests; 0 failed** | Final all-features unit, integration, and doc-test targets. |
| `cargo fmt --all -- --check`; `git diff --check` | **PASS** | Both final checks completed successfully. |
| `cargo clippy --all-targets --all-features -- -D warnings` | **FAIL: 17 pre-existing diagnostics** | Same 16 async-trait `double_must_use` diagnostics and `useless_borrows_in_formatting` in `src/export/registry.rs`; no cache-specific lint remains. |
| `cargo clippy --all-targets --all-features -- -D warnings -A clippy::double_must_use -A clippy::useless_borrows_in_formatting` | **PASS** | All targets/features pass with only the two existing diagnostic categories allowed. |
| Internal snapshot/OCI validation | **PASS** | Tests verify BLAKE3 cache-key identity versus SHA-256 snapshot integrity, OCI cold/cache-hit digest equivalence, no accepted partial entry, and no image publication on quota failure. |

The build-result key is domain-separated BLAKE3 with prefix `v2:blake3:` and namespace `build-snapshots-v2`; v1 SHA-256 entries are not loaded or migrated. Snapshot integrity remains explicitly SHA-256, separate from both internal key identity and OCI descriptor/DiffID hashing. `MEMOBUILD_CACHE_MAX_BYTES` configures the logical-byte limit (default **1 GiB**). Accounting includes all regular files in the v2 namespace: records, snapshots, in-flight temporary files, and orphan blobs. It excludes directory/inode and allocated-block overhead. Serialization is bounded while writing, including its temporary snapshot; publication renames do not create a second logical copy. An `fs2` advisory file lock serializes cooperating process restore, publication, and GC. GC removes temporary/orphan files, then deterministically evicts lexicographically ordered valid entries only when space is required; if no safe candidate fits, the cache write fails explicitly. The build does not publish an OCI layout after that failure.

Subprocess crash tests exit without destructors after snapshot-temp creation, tar serialization, snapshot fsync, blob rename, blob-directory fsync, record fsync, record rename, and record-directory fsync. Reopening yields either a restorable complete record or a miss; repeated restore succeeds for surviving entries. This verifies process interruption, not power loss or filesystem-specific durability guarantees. `fs2` supplies platform-specific advisory locks; validation was run on Linux, and Windows/macOS locking behavior was not exercised here. Storage accounting is logical file length, not physical disk blocks, so external filesystem quotas and `ENOSPC` remain possible. A cache owner or hostile same-UID process can still rewrite both records and blobs; the cache is not an authenticated trust boundary. Legacy executor/remote caches and unsupported Dockerfile behavior remain unchanged.

## MB-P0-003 parser/compiler validation before MB-P0-002 on `production/v0.2.x-hardening`

The starting revision was `c9689e8c719a01b6671f57209107dbefd826a93f` (`v0.2.0-dirty`) on `production/v0.2.x-hardening`, with pre-existing uncommitted MB-P0-001 changes and untracked `docs/production/` audit files. These were retained. Before using a separate Cargo target, available storage was checked: `/tmp` had 37 GiB free and the existing task target occupied 2.0 GiB.

| Command | Result | Evidence |
|---|---|---|
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-003-target cargo test --test dockerfile_strict -- --nocapture` | **PASS: 14 tests** | Exact scratch declaration; unknown/malformed/recognized-but-unsupported syntax; line/column and stable diagnostics; image policy; aliases/stages; all recognized instruction families explicitly rejected; COPY traversal/missing/symlink escape; valid COPY source rejected until staged semantics; CLI rejection before cache and host execution; custom extension rejection. |
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-003-target cargo test --test executor_coverage_test -- --nocapture` | **PASS: 15 tests (also in all-feature run)** | Updated parser/compiler-specific legacy target passes; assertions no longer treat multi-stage/RUN graphs as proven supported. |
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-003-target cargo test --test e2e_test -- --nocapture` | **PASS: 3 tests (also in all-feature run)** | Scratch compiler, known multi-stage example rejection, and CMD rejection. |
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-003-target cargo test --test reproducibility_test -- --nocapture` | **PASS: 2 tests (also in all-feature run)** | External image rejection and stable scratch compiler identity; no longer claims the fake exporter is a reproducible image build. |
| `cargo fmt --all -- --check` and `git diff --check` | **PASS** | Both final checks exited 0. |
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-003-target cargo test --all-features -- --nocapture` | **PASS: 88 tests, 0 failed** | All unit, integration, and documentation tests completed, including 14 strict parser/compiler tests and 6 sandbox-isolation tests. |
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-003-target cargo clippy --all-targets --all-features -- -D warnings` | **FAIL: 17 existing diagnostics** | Current diagnostics are the same existing `double_must_use` async-trait expansions and `useless_borrows_in_formatting` in `src/export/registry.rs` documented under MB-P0-001; no diagnostics point to the MB-P0-003 changes. |
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-003-target cargo clippy --all-targets --all-features -- -D warnings -A clippy::double_must_use -A clippy::useless_borrows_in_formatting` | **PASS** | All targets/features pass with only the two known baseline lint categories allowed. |
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-003-target cargo run --all-features --example dag_verification` | **PASS** | Example compiles/runs, validates `FROM scratch`, and clearly reports that execution awaits MB-P0-002. |

The first all-feature attempts found stale assumptions in the API example, a test expecting the old external-base diagnostic, and an MB-P0-001 CLI test expecting Bubblewrap dispatch for a now-unsupported `RUN`. These were updated to match the strict compiler contract; the final all-feature run above passed. The direct sandbox S-03 test continues to verify missing-runtime fail-closed behavior.

**Historical result at completion of MB-P0-003, before MB-P0-002:** that test matrix verified a compiler declaration, not a runnable build; the CLI rejected even `FROM scratch` before cache initialization. MB-P0-002's later implementation and current validation are recorded above. The external digest-reference test verifies rejection, not digest parsing/verification: no external reference form is currently accepted.

## MB-P0-001 validation on `production/v0.2.x-hardening`

The following MB-P0-001 results were actual runs on Linux x86_64 with Bubblewrap 0.9.0, after confirming available storage (`/tmp`: 37 GiB free when the focused test ran). The original CLI test that exercised default backend dispatch was superseded by MB-P0-003's stricter rule: unsupported RUN is rejected before backend dispatch. The current test retains that ordering guarantee; direct S-03 still exercises missing Bubblewrap at the sandbox interface.

| Command | Result | Evidence |
|---|---|---|
| `cargo fmt --all -- --check` | **PASS** | Exit code 0 after changes |
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-001-target cargo test --test sandbox_isolation -- --nocapture` | **PASS: 6 tests at MB-P0-001; current 6-test target passed in the final all-features run** | S-01 protected host-file write, S-03 missing-runtime fail-closed behavior, CLI behavior (since MB-P0-003, strict RUN rejection before backend dispatch), staged-context command plus exit-code preservation and isolated network namespace, timeout/process/workspace cleanup, and surfaced cleanup failure. |
| Initial focused `cargo test --test sandbox_isolation -- --nocapture` attempts | **FAIL, then fixed** | Early assertions exposed a missing sandbox-local `/tmp` parent and insufficiently specific missing-runtime diagnostics. A later read-only-context fixture exposed cleanup permissions preventing temporary-directory removal. Each issue was fixed and the final focused suite passes. |
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-001-target cargo test --all-features -- --nocapture` | **PASS: 74 tests** | All unit, integration, and doc-test targets completed; zero failures. |
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-001-target cargo clippy --all-targets --all-features -- -D warnings` | **BLOCKED by existing lint failures** | Exit code 101: 17 total diagnostics in existing `double_must_use` async-trait expansions and a `useless_borrows_in_formatting` call in `src/export/registry.rs`. The changed executor's `let_and_return` diagnostic from the first run was fixed. |
| `CARGO_TARGET_DIR=/tmp/memobuild-p0-001-target cargo clippy --all-targets --all-features -- -D warnings -A clippy::double_must_use -A clippy::useless_borrows_in_formatting` | **PASS** | All targets and features lint clean with the two existing repository lint categories explicitly allowed. |

The S-01 test specifically attempts a write to a protected host file under `/tmp`; it does not cover every path traversal, symlink, or kernel escape. The network check compares network namespace IDs and does not constitute an external connectivity penetration test. The timeout test verifies a late filesystem side effect does not occur and the temporary workspace is removed. Aggregate writable-workspace disk usage, cgroup limits, seccomp, and syscall filtering remain uncovered and are not claimed.

## Original v0.2.0 validation status

| Command | Result | Evidence / next action |
|---|---|---|
| `cargo fmt --all -- --check` | **PASS** | Ran at the audited checkout; exit code 0 |
| `CARGO_TARGET_DIR=/tmp/memobuild-validation-target MEMOBUILD_CACHE_DIR=/tmp/memobuild-validation-cache cargo test --all-features --test cache_and_core_test -- --nocapture` | **PASS: 14 tests** | Ran in isolated temporary directories; completed in 1.24s. This target's cache round-trip test asserts only that get/put return `Ok`, not byte equality or artifact restoration, so it does not establish cache correctness. |
| `cargo test --all-features -- --nocapture` | **BLOCKED before tests ran** | Cargo compiled dependencies but linking failed with `ld terminated with signal 7 [Bus error]` for `executor_coverage_test` and `load_test_runner`. Immediately afterward, `/workspaces` was 100% full (108 MB available) and `target/` occupied 14 GB. Re-run after providing sufficient workspace storage. No full-suite test result is reported. |
| `cargo tree --depth 1` | **PASS** | Direct dependency inventory completed; toolchain observed as `rustc 1.99.1`, `cargo 1.99.1`. |
| `cargo clippy --all-targets --all-features -- -D warnings` | **NOT RUN** | CI command; run after build storage is available |
| `cargo audit` | **NOT RUN** | CI command; run with a current advisory database |
| OCI layout validation and runtime smoke test | **NOT RUN** | No OCI-aware validator/runtime command is configured in the observed test suite |
| BuildKit comparison | **NOT RUN** | Current benchmark is not an equivalent build workload |

Recommended repeatable baseline sequence:

```sh
cargo fmt --all -- --check
cargo test --all-features -- --nocapture
cargo clippy --all-targets --all-features -- -D warnings
cargo audit
```

The focused test was run with a separate target directory because the workspace filesystem filled during full-suite linking. The focused test is useful only as a smoke check; the broader test matrix below remains necessary.

Record tool versions, host platform, commit SHA, exit status, and logs for each run. Do not infer a successful suite from CI configuration or compilation of dependencies.

## Correctness and incremental test matrix

Each row should become an automated integration test with a disposable context, isolated cache directory, and observable command side effect (for example, a counter file inside the build root).

| ID | Setup and action | Expected result | Bug class |
|---|---|---|---|
| B-01 | Minimal supported Dockerfile with pinned base and one command; build once | Success only for documented syntax; rootfs contains exact expected files | Missing/incomplete build path |
| B-02 | Inspect exported image and unpack layers | Actual build filesystem is present, not metadata markers | Placeholder export |
| B-03 | Build using `WORKDIR`, `ENV`, and `COPY`; command reads copied file and writes output | Paths, values, and resulting files match declared semantics | State propagation |
| B-04 | Include an unsupported instruction and malformed supported instruction | Build fails before execution with instruction and line diagnostic | Silent misinterpretation |
| B-05 | Multi-step build where each RUN consumes previous output | Each step sees the complete preceding rootfs | Missing stage state |
| B-06 | Force a command failure midway through a build | Nonzero result; no partial cache entry or final image is published | Failure atomicity |
| I-01 | Build twice without changing inputs; second command increments an execution counter if run | Second build skips valid operation and restores identical files/metadata | False cache hit / no restore |
| I-02 | Change one copied source file | Only dependent operations and descendants rerun; final output matches clean build | Missed invalidation |
| I-03 | Change an unrelated context file not consumed by any instruction | Unrelated operation remains reusable when the declared semantics permit | Over-invalidation |
| I-04 | Change dependency manifest or lockfile | Dependency-install step and descendants rerun | Missing dependency input |
| I-05 | Change ENV or ARG value consumed by a command | Relevant operation key changes; unrelated independent work remains reusable | Incomplete environment key |
| I-06 | Change a parent stage/base image digest | Every dependent stage invalidates; independent stage can remain cached | Stage graph error |
| I-07 | Rename a file, alter mode, and replace regular file with symlink in separate runs | Fingerprint changes exactly according to documented COPY semantics | Incomplete metadata fingerprint |
| I-08 | Introduce a cycle into graph input or parser-generated graph | Graph validation rejects it without recursion overflow | Cycle handling |

## Cache integrity and service tests

| ID | Setup and action | Expected result | Bug class |
|---|---|---|---|
| C-01 | Populate local cache, restart process, rebuild from same cache | Artifact is found and restores exact output | Persistence/recovery |
| C-02 | Truncate or bit-flip an artifact | Read rejects it; corrupted bytes are never used as output | Missing integrity verification |
| C-03 | Leave index entry but delete blob; also leave blob with malformed index | Missing/corrupt data is treated as cache miss or explicit integrity error, never a false hit | Stale metadata |
| C-04 | Kill writer during publication and run concurrent readers/writers | Readers see old complete or new complete artifact, never partial data | Non-atomic publication/race |
| C-05 | Populate cache from independent client A and restore on client B using only remote server | Exact bytes and metadata restored; request counters alone are insufficient | Remote protocol mismatch |
| C-06 | Remote unavailable, rejects version, or returns corrupt body | Behavior matches documented fallback/fail policy; error surfaced | Silent remote failure |
| C-07 | Submit an untrusted manifest with `../` and absolute paths to a worker | Rejected before filesystem write | Archive/path traversal |
| C-08 | Send unauthenticated cache/worker requests from an unauthorized client | Rejected; no artifact read/write or command execution | Missing authorization/tenant boundary |

## OCI, reproducibility, and security tests

| ID | Setup and action | Expected result | Bug class |
|---|---|---|---|
| O-01 | Generate OCI layout; validate with an OCI-aware validator | Layout descriptors, media types, blob paths, sizes, and required fields validate | Structurally invalid layout |
| O-02 | Recompute each compressed blob digest and uncompressed tar DiffID | Both exactly match manifest/config descriptors | Digest convention mismatch |
| O-03 | Load/pull image into a supported OCI runtime and start configured command | Runtime sees expected rootfs, metadata, and command | Non-runnable image |
| O-04 | Repeat clean build under controlled identical inputs; compare index, manifest, config, every layer, and unpacked rootfs | All outputs match the stated reproducibility contract | Partial reproducibility |
| S-01 | Attempt writes via `..`, symlink escapes, absolute paths, and host-sensitive paths | No write/read escapes the enforced build root | Host filesystem exposure |
| S-02 | Use credentials in supported secret path and inspect logs, cache, image, and metadata | Secret absent from logs and exported/cache artifacts | Credential leakage |
| S-03 | Run oversized context, long process, excessive output, and cancellation cases | Configured limits are enforced and cleanup completes | Denial of service/resource leak |
| S-04 | Send malformed/oversized unauthenticated remote requests and test cross-tenant access | Requests rejected within resource bounds; tenant data inaccessible | Remote service abuse |

## Benchmark methodology

Benchmark only after B-01 through B-06 and the core cache/OCI tests pass. Compare MemoBuild to a current BuildKit version, not an assumed legacy linear builder. Use identical pinned inputs, Dockerfile semantics, image outputs, machine/OS, CPU and memory limits, network conditions, and cache state. Warmups and repeated runs must be reported separately; do not mix cold and warm cache results.

| Scenario | Workload change |
|---|---|
| H-01 | First build with empty cache |
| H-02 | Rebuild with no source changes |
| H-03 | Modify one application source file |
| H-04 | Modify dependency manifest or lockfile |
| H-05 | Modify a shared build input |
| H-06 | Modify an unrelated context file |
| H-07 | Rebuild from populated local cache |
| H-08 | Rebuild from populated remote cache |
| H-09 | Concurrent builds sharing immutable cache content |
| H-10 | Large context hashing and artifact restoration |

Report median and spread across repeated runs for wall time, CPU time, peak memory, bytes hashed/read/written, cache hits/misses, invalidated operations, and network bytes. Independently compare final unpacked rootfs and image descriptor digests. Record runtime/build-tool versions, cache cleanup method, workload repository/revision, hardware, and exact commands. A tiny directory-hash microbenchmark is not evidence of build-system performance.

## Release gate

A release candidate for the first production milestone must pass all correctness, incremental, cache, OCI, and security tests applicable to its explicitly documented instruction subset and threat model. Any unsupported behavior must fail clearly. Remote cache and distributed execution tests become mandatory only if those features are enabled or advertised in that release.
