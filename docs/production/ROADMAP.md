# Production roadmap

This roadmap is ordered by the prerequisites of a trustworthy path from Dockerfile to runnable image. It is not a release promise. Effort is relative (`XS` through `XL`); no calendar estimates are implied.

## Priority work items

| ID | Priority / title | Evidence and impact | Prerequisites | Acceptance criteria | Test needs | Effort | Release blocker |
|---|---|---|---|---|---|---|---|
| MB-P0-000 | Record a reproducible baseline | Current tests and docs do not establish a verified real-build path | None | Commit, toolchain, commands, environment, known failures, and expected baseline fixtures are recorded; blocked commands remain marked blocked | Baseline command run and fixture review | XS | Yes |
| MB-P0-001 | Decide and enforce the build isolation boundary | v0.2.0 default RUN used host execution; hardening branch adds a Bubblewrap boundary | Bubblewrap on Linux x86_64; unprivileged user namespaces; explicit limits/network policy | Linux x86_64 CLI defaults to verified Bubblewrap, fails closed if setup is unavailable, limits and timeout are applied, and host execution is explicitly unsafe opt-in. This task does not provide a cgroup/seccomp or aggregate disk quota | S-01, S-03, normal command, exit status, network namespace, timeout and cleanup tests | **Implemented with documented limits on `production/v0.2.x-hardening`** | Yes; `RUN` state capture remains unsupported by MB-P0-002 |
| MB-P0-002 | Implement real staged filesystem state and first OCI image | v0.2.0 had no real rootfs handoff or OCI layer capture; hardening branch now supports a narrow scratch/COPY/command-metadata path | MB-P0-001, MB-P0-003; Linux x86_64; exactly one `FROM scratch` | Supported COPY produces actual rootfs content and a validated, atomically published OCI layout; unsupported syntax fails closed; no RUN is claimed | Empty scratch, COPY contents/modes/boundaries, failure cleanup, config, descriptor/digest/DiffID, reproducibility, runc, and Docker load/run tests | **Implemented for the stated subset; Docker tag lookup behavior remains unresolved** | Yes for general builds |
| MB-P0-003 | Define a strict Dockerfile subset and fail closed | v0.2.0 parser silently dropped malformed syntax and emitted generic no-op nodes | Choose and document base-image scope; now paired with the MB-P0-002 scratch-only build path | Unknown and malformed instructions fail with source lines; recognized unsupported operations fail with specific reasons; multi-stage aliases and external bases are rejected; no unsupported input reaches cache/executor | `tests/dockerfile_strict.rs`, parser/DAG tests, CLI pre-execution rejection, existing end-to-end regression targets | **Implemented on `production/v0.2.x-hardening`; exact executable subset is limited to MB-P0-002** | Yes for general Dockerfile support |
| MB-P0-004 | Define versioned build keys for the supported subset | Legacy `Node::compute_node_key` omits reliable filesystem state; `BuildSnapshotCache` keys normalized compiled operations and every consumed COPY source | MB-P0-002, MB-P0-003 | Supported keys include cache/compiler/snapshot/export schema versions, package version, ordered operations, source content/type/modes, command metadata, target platform, base identity, image reference, and reproducibility setting. General semantic DAG/operation keys remain open | Source/content/mode/order/destination/command/platform/export-setting sensitivity and stable-key tests | **Implemented for the narrow build path; general DAG keying remains** | Yes for incremental/general builds |
| MB-P0-005 | Restore verified local filesystem snapshots | Legacy executor cache hits discard bytes; build-specific cache restores a complete validated tar snapshot into a fresh rootfs | MB-P0-002, narrow MB-P0-004 | Snapshot digest/size, schema, paths, types and normalized metadata are verified before transactional restore; missing/corrupt artifacts fail closed; fresh-process restore reproduces identical tree and OCI digest | Fresh-process restore, tree/mode equality, corruption/traversal rejection, concurrent writes/GC, quota, and subprocess crash-boundary tests | **Implemented for the narrow build path; legacy executor cache remains unsound** | Yes for general executor cache |
| MB-P1-002 | Align build-cache identity, storage bounds, and crash recovery | Previous narrow cache key used SHA-256 while the internal specification requires BLAKE3; storage was unbounded and had no recovery GC | MB-P0-004/005 | Isolated v2 BLAKE3 namespace; explicit byte quota; lock-coordinated safe GC; interrupted publication reopens only as a verified entry or miss | Stable/sensitive key tests; quota/eviction, concurrent restore/GC, orphan cleanup, and subprocess interruption at publication boundaries | **Implemented for the supported scratch/COPY cache; filesystem power-loss guarantees remain unverified** | Yes for trusted cache reuse |
| MB-P0-006 | Complete OCI interoperability beyond the first vertical slice | Docker imported and ran one image by content ID; tag lookup was inconsistent and no standalone OCI validator is installed | MB-P0-002; choose validator/runtime CI prerequisites and broader output scope | Independent OCI validator accepts the layout and runtime reliably uses the intended image reference; preserve exact content/config/digest checks | External validator, reference-tag load/run, failure and platform tests | M | Yes for broader OCI interoperability claims |
| MB-P0-007 | Secure or disable remote execution endpoints | `src/remote_exec/server.rs:18-28`, `worker_server.rs:17-27`, `worker.rs:24-69`; bind-all unauthenticated command dispatch | MB-P0-001; deployment threat model | Endpoints are disabled until authenticated, authorized, TLS-protected, bounded, and isolated; default bind is not public; request/result protocol is validated | Unauthorized request, malformed request, tenant separation, resource/cancel and sandbox tests | L | Yes if remote execution ships |
| MB-P1-001 | Make local CAS durable and self-validating | `src/cache.rs:46-113`; silent index reset, non-atomic writes, no digest verification | MB-P0-004, MB-P0-005 | Versioned index/blob format; atomic publication; size/digest validation on read; corrupt metadata/artifacts rejected; defined cleanup/quota policy | Truncation, stale index, restart, concurrent writer and disk-full tests | M | Yes |
| MB-P1-006 | Reconcile remote cache wire format and trust | `src/remote_cache.rs:157-222`, `src/server/mod.rs:490-506`; compression/key mismatch | MB-P1-001; remote cache auth decision | A fresh client fetches byte-identical verified snapshot; both sides agree on hash bytes/encoding/version; auth/namespace and remote outage behavior documented | Independent-process cache round-trip, tamper, version mismatch, unauthorized and outage tests | L | Yes for shared cache |
| MB-P1-003 | Establish end-to-end correctness regression suite | Current tests mostly inspect structures/counters; no runnable-image check | MB-P0-002 through MB-P0-006 | Automated minimal supported build verifies expected rootfs, cache restore, and runtime; every confirmed semantic defect gets a regression test | Validation plan cases below | L | Yes |
| MB-P1-004 | Define reproducibility contract | `normalize_artifact` is no-op; test compares index only | MB-P0-004, MB-P0-006 | Document controlled inputs and guarantees; repeated clean builds compare all image blobs and unpacked required state | Repeat/fresh-cache/fresh-process tests; metadata variation tests | M | Yes for reproducible claims |
| MB-P1-005 | Correct context and ignore-file semantics | `src/main.rs:163`, `src/core.rs:31-33`, `src/hasher/ignore.rs:8-48` | MB-P0-003, MB-P0-004 | Dockerfile and context resolution are explicit; ignore behavior matches declared compatibility fixtures; traversal/read errors surface | Context path, ignore negation, invalid pattern, symlink and permissions tests | M | Yes |
| MB-P2-001 | Implement registry interoperability | `src/export/registry.rs`, `src/main.rs:257-268,366-373`; tag and pull behavior incomplete | MB-P0-006; credential decision | Pull/push to test registry uses requested reference, validates digest, handles auth challenge and reports failures without leaking credentials | Local registry integration and auth-secret checks | L | No, unless registry is in milestone |
| MB-P2-002 | Replace simulated benchmarks and load tests | `benches/core_bench.rs`; `tests/scalability_test.rs` uses random/sleep simulation | MB-P0-003 through MB-P0-006 | Publish reproducible BuildKit-equivalent workload results with correctness and resource metrics; no claimed advantage without repeated evidence | Benchmark matrix in validation plan | M | No |
| MB-P3-001 | Reassess distributed workers, Kubernetes, dashboard, and generated CI | CLI generation is placeholder; remote routers/scalers lack demonstrated operational behavior | Production local build, cache, and security gates | Each shipped service has documented APIs, auth, persistence, recovery, backpressure, integration tests, and operations | Deployment and failure-recovery tests | XL | No |

## Phases and exit gates

### Phase 0 — Trusted baseline

Complete MB-P0-000. Capture known failures without claiming they passed; do not make full end-to-end correctness a Phase 0 prerequisite.

**Exit gate:** a new contributor can run the documented suite, and every current limitation/failure has a reproducible test or explicit `NOT RUN` reason.

### Phase 1 — Correct local build

MB-P0-001, MB-P0-003, and the minimal MB-P0-002 COPY-to-OCI path are implemented on `production/v0.2.x-hardening`. The path supports only `FROM scratch`, simple context-relative COPY, and JSON-array CMD/ENTRYPOINT; RUN is rejected. The image ran under runc and Docker after OCI-layout import; Docker tag-name lookup is inconsistent and standalone OCI validation remains. Do not imply general Docker compatibility or production readiness.

**Exit gate:** one supported Dockerfile builds in the declared isolation boundary, produces the expected runtime filesystem and command, and unsupported syntax fails explicitly.

### Phase 2 — Incremental correctness

MB-P0-004/005 and MB-P1-002 are implemented for the narrow scratch/COPY result cache: domain-separated versioned BLAKE3 keys cover supported inputs; SHA-256 remains the snapshot-integrity and OCI digest algorithm. A 1 GiB default logical-byte quota (`MEMOBUILD_CACHE_MAX_BYTES`) and lock-coordinated GC bound cooperating local writers; GC removes interrupted temporary/orphan files and deterministically evicts valid entries by lexicographic key when space is required. Subprocess interruption tests cover temp creation, serialization, fsync, blob publication, record publication, and parent-directory sync; recovery yields a verified complete entry or a miss. These are not power-loss tests. General graph semantics and legacy executor-cache behavior remain unresolved. Cache use still assumes a private trusted cache root and cooperating processes.

**Exit gate:** controlled changes invalidate the necessary nodes only; unchanged builds restore identical valid state; corruption and process restart are safe.

### Phase 3 — Production local cache and OCI

Harden local persistence and OCI output (MB-P1-001, MB-P0-006); run independent OCI validation and close Docker tag lookup behavior. The current cache covers a narrow local snapshot format but has no quota, garbage collection, or broader concurrent/crash-recovery policy.

**Exit gate:** OCI-aware validation and runtime smoke test pass; cache survives interruption/restart/concurrency without incomplete publication.

### Phase 4 — Remote cache

Complete MB-P1-006 only after local cache correctness; include authentication, authorization, integrity, namespaces, version compatibility, transfer limits, and outage policy.

**Exit gate:** an independent client reuses valid artifacts; corrupt, incompatible, or unauthorized artifacts are rejected.

### Phase 5 — Performance and safe concurrency

Complete MB-P2-002 after correctness; profile real workloads before optimization. Add parallelism only for proven independent nodes and isolated snapshots.

**Exit gate:** repeated measurements show a defined improvement over modern BuildKit on equivalent outputs while correctness remains identical and resources are reported.

### Phase 6 — Platform expansion

Consider MB-P0-007 as a gate for any remote execution release, then MB-P3-001 for workers, Kubernetes, dashboards, and additional integrations.

**Exit gate:** every exposed service has an explicit security boundary, protocol, recovery strategy, integration suite, and operator documentation.

## First recommended milestone

The first executable slice is one Linux/amd64, single-stage local build from an empty `scratch` base. It supports plain context-relative COPY and JSON-array CMD/ENTRYPOINT, exports actual filesystem content, and uses a build-specific verified local snapshot cache. It does not execute RUN or support an external base. General operation-level graph keys, legacy executor-cache repair, remote cache, and registry resolution remain out of scope. Do not include remote execution, Kubernetes, general extension syntax, or a performance claim.

## Decisions that block implementation

1. Is the build threat model hostile Dockerfiles, trusted developer inputs, or both? Which runtime provides the required isolation and network policy?
2. Is the first base-image scope pinned OCI digest only? Which platforms and registry authentication methods are in scope?
3. Which exact Dockerfile forms are supported initially, including ARG, shell/JSON forms, multi-stage, ADD, and `.dockerignore`?
4. Does cache identity include normalized environment/toolchain by default, and what is the cache namespace/trust model?
5. Which OCI/Docker image and registry operations are release requirements?
6. What reproducibility guarantee is promised, under what controlled inputs, and across which hosts?
