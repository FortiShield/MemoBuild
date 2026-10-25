# MemoBuild Repository: Project Structure Analysis & Restructuring Plan

## 1. Executive Assessment

### 1.1 Core Architecture
The root `src/` mixes core build logic, execution, networking, infrastructure integrations, server code, and operational features. The workspace has a single `Cargo.toml` with all dependencies at the root level, making it difficult to reason about boundaries. Key modules span from DAG computation to Kubernetes operators, with no clear ownership model.

### 1.2 Module Overlap (Investigate First)
Several pairs of paths represent overlapping or duplicate responsibilities that need investigation:

| Existing Paths | Investigation Required |
|---|---|
| `src/cache.rs` and `src/cache/` | `cache.rs` is a re-export facade; determine which functions belong to cache backends vs. the facade |
| `src/executor.rs` and `src/execution/` | `executor.rs` re-exports from `execution/executor.rs`; verify this is intentional layering |
| `src/cluster_server.rs` and `src/network/cluster_server.rs` | Two separate servers: regular cache server vs. cluster-aware server; compare routes, listeners, protocols |
| `src/remote_cache.rs` and `src/cache/remote.rs` | Both define `RemoteCache` trait; one may be transport, the other semantics — clarify ownership |
| `src/cache_cluster.rs` and `src/cache/cluster.rs` | Both implement distributed caching; determine which is the canonical implementation |
| `src/secrets.rs` and `src/secrets/` | Trait + implementations; establish one public interface and isolate provider implementations |
| `src/slsa.rs` and `src/slsa/` | Supply-chain verification + in-toto integration; clarify relationship |
| `src/server/` and `src/remote_exec/` | Control-plane server vs. worker execution responsibilities; define distinct boundaries |
| `src/index.js` and `dist/index.js` | Same content in both places; establish source ownership and generation process |

### 1.3 Packaging and Interfaces
Rust, Node.js, Go, TypeScript, and Python components coexist without an obviously unified workspace or package ownership model. The single `Cargo.toml` pulls in dependencies for all integrations (S3, GCS, Redis, Kubernetes, gRPC, TLS, etc.) even when only a subset is needed.

### 1.4 Production Engineering
Deployment manifests, monitoring, security policies, supply-chain verification, and tests exist, but their validation and release relationships need to be made explicit. Dockerfiles at root and `docker/` have inconsistent build contexts. Helm charts, K8s manifests, and monitoring configs need dependency alignment.

## 2. Target Architecture

Organize around six responsibilities with dependencies flowing inward:

- **Core**: build graphs, hashing, reproducibility, shared domain types
- **Execution**: local execution, remote execution, workers, scheduling, sandboxing
- **Platform services**: cache, storage, networking, APIs, observability
- **Security**: authentication, secrets, auditing, artifact verification, SBOM generation
- **Interfaces**: CLI, server, JavaScript bindings, extension UI
- **Operations**: deployment, monitoring, documentation, examples, release automation

## 3. Current Module Overlap Analysis

### 3.1 Cache Module (`src/cache.rs` + `src/cache/`)
- `src/cache.rs` (15 lines): re-exports `CacheCluster, ClusterNode, ClusterStatus, DistributedCache` from `cluster`; `HttpRemoteCache` from `http`; `HybridCache` from `hybrid`; `LocalCache` from `local`; `DatabaseStats, PostgresMetadataStore, ReplicatedMetadataStore` from `metadata`; `RemoteCache, RemoteCacheEntry` from `remote`; `merge_artifact, split_artifact, ArtifactLayer, ArtifactManifest, FileEntry` from `utils`
- `src/cache/`: 7 submodules — `cluster.rs`, `http.rs`, `hybrid.rs`, `local.rs`, `metadata.rs`, `remote.rs`, `utils.rs`
- **Resolution**: `cache.rs` serves as the public facade. Keep it but ensure each submodule has a clear, distinct responsibility. The `hybrid.rs` module likely combines local+remote strategies and needs documentation.

### 3.2 Execution Module (`src/executor.rs` + `src/execution/`)
- `src/executor.rs` (1 line): `pub use crate::execution::executor::*;`
- `src/execution/mod.rs` (2 lines): `pub mod executor; pub use executor::*;`
- `src/execution/executor.rs` (505 lines): `IncrementalExecutor` struct with parallel/sequential execution, cache integration, remote execution, sandboxing
- **Resolution**: This is correct layering. `src/executor.rs` is the public API entry point; `src/execution/` is the internal module. No restructuring needed, but document this as the canonical pattern.

### 3.3 Server Modules (`src/cluster_server.rs` + `src/network/cluster_server.rs`)
- `src/cluster_server.rs` (300 lines): "Regular" cache server with cache endpoints + health. Routes: `/cache/:hash`, `/health`. Uses `distributed_cache` from `cache_cluster`; does NOT have cluster management endpoints.
- `src/network/cluster_server.rs` (283 lines): "Cluster-aware" server with full cluster management + auto-scaling. Additional routes: `/cluster/status`, `/cluster/nodes`, `/scaling/*`. Uses `CacheCluster` from `cache::cluster`; has auto-scaler integration.
- `src/network/mod.rs` (4 lines): re-exports `cluster_server::*`
- **Resolution**: These are intentionally different server types. `src/cluster_server.rs` is a simple standalone cache server; `src/network/cluster_server.rs` is the full cluster node server. Both are valid. Consider renaming `src/cluster_server.rs` to `src/server/simple.rs` or documenting the distinction clearly. The `network/mod.rs` re-export should be reviewed — it currently exposes the cluster server publicly.

### 3.4 Remote Cache Trait (`src/remote_cache.rs` + `src/cache/remote.rs`)
- `src/remote_cache.rs` (341 lines): `RemoteCache` trait + `HttpRemoteCache` struct. Full implementation with retry_with_backoff, TLS config, gzip compression, metrics reporting. Remote cache **client** for communicating with remote cache nodes.
- `src/cache/remote.rs` (35 lines): `RemoteCache` trait + `RemoteCacheEntry` struct. Bare-minimum trait definition. No implementation, no entry struct beyond the entry.
- **Resolution**: `src/cache/remote.rs` defines the **semantic interface** (what operations a cache must support). `src/remote_cache.rs` defines the **transport implementation** (HTTP client with retries, TLS, compression). The `RemoteCache` trait in `cache/remote.rs` should be the canonical interface; `remote_cache.rs::HttpRemoteCache` implements it. Consider making `cache/remote.rs` the trait definition and having `remote_cache.rs` re-export or implement it, or consolidating so one owns the trait.

### 3.5 Distributed Cache Implementations (`src/cache_cluster.rs` + `src/cache/cluster.rs`)
- `src/cache_cluster.rs` (534 lines): `DistributedCache` struct that implements `RemoteCache`. Includes metrics integration, `cache_span!` macros, replication tasks with timeouts. Depends on `crate::metrics::SharedMetrics`.
- `src/cache/cluster.rs` (482 lines): `DistributedCache` struct that implements `RemoteCache`. Similar replication logic but without metrics integration. Simpler, no span macros.
- **Resolution**: `src/cache_cluster.rs` is the more complete implementation with observability. `src/cache/cluster.rs` appears to be a lighter version. Consider merging into a single canonical implementation in `memobuild-cache` crate, or keeping `cache/cluster.rs` as a no-features backend and `cache_cluster.rs` as the full-featured one.

### 3.6 Secrets Module (`src/secrets.rs` + `src/secrets/`)
- `src/secrets.rs` (188 lines): `SecretProvider` trait + `EnvSecretProvider`, `VaultSecretProvider`, `AwsKmsProvider`. Factory function `create_secret_provider()`. Defaults to env vars.
- `src/secrets/` (1 file: `aws.rs`): 62 lines — partial `AwsSecretProvider` implementation (only struct, no `SecretProvider` trait implementation — the `AwsKmsProvider` in `secrets.rs` already covers KMS).
- **Resolution**: `src/secrets.rs` is the canonical module. `src/secrets/aws.rs` has a standalone struct that overlaps with `AwsKmsProvider` in `secrets.rs`. Either remove `src/secrets/aws.rs` and move its contents into `secrets.rs`, or ensure it re-exports from `secrets.rs`.

### 3.7 SLSA/Provenance Module (`src/slsa.rs` + `src/slsa/`)
- `src/slsa.rs` (257 lines): `ProvenanceGenerator` struct, SLSA provenance generation, in-toto DSSE signing, `cli` submodule, `InvocationParams`, `EnvironmentVariable`, `InputArtifact`, `Provenance`, `Predicate`, `BuildDefinition`, `RunDetails`, `Builder`, `ProvenanceMetadata`, `Material`, `Product`, `Attestation`, `Signature` structs. `cli::generate_cmd` and `cli::verify_cmd`.
- `src/slsa/` (1 file: `in_toto.rs`): `sign_dsse`, `verify_dsse`, `create_statement` — in-toto DSSE signature utilities used by `slsa.rs`.
- **Resolution**: `src/slsa.rs` is the canonical module; `src/slsa/in_toto.rs` is a sub-module. No conflict, but ensure `in_toto.rs` functions are re-exported or imported cleanly from `slsa.rs`.

### 3.8 Server vs Remote Execution (`src/server/` + `src/remote_exec/`)
- `src/server/mod.rs` (758 lines): HTTP server with axum — cache endpoints, WS, metrics, gc, analytics. Control-plane API server.
- `src/remote_exec/`: RE API compatibility layer with gRPC services (`reapi.rs`), execution client (`client.rs`), scheduler (`scheduler.rs`), gRPC server (`grpc_server.rs`), worker model (`worker.rs`, `worker_pool.rs`, `worker_server.rs`).
- **Resolution**: Clear separation: `src/server/` = control-plane HTTP API; `src/remote_exec/` = worker execution plane (RE API compatibility + local sandbox execution). Both are needed. No restructuring needed, but document the control-plane/worker boundary.

### 3.9 JavaScript Entry Points (`src/index.js` + `dist/index.js`)
- Both files contain identical content: `console.log('🚀 Hello from MemoBuild engine!'); console.log('✨ MemoBuild detected a change!');`
- **Resolution**: One is the source, the other is generated output. Determine which is which based on the Node.js build pipeline. If `dist/index.js` is generated during `npm build`, it should not be tracked in version control (or at least be clearly marked as generated). If both are hand-edited, consolidate to a single source.

## 4. Recommended Target Directory Structure

```
memobuild/
├── Cargo.toml          — workspace manifest
├── Cargo.lock          — Rust dependency lockfile
├── package.json        — root JS tooling, if needed
├── build.rs            — retain only if needed by root package
├── memobuild-core/     — Graph, hashing, Git inputs, reproducibility, shared types
├── memobuild-engine/   — Build orchestration, execution lifecycle, task planning
├── memobuild-cache/    — Local, remote, hybrid, Redis, cluster caching
├── memobuild-remote/   — Workers, scheduler, gRPC, REAPI, routing
├── memobuild-container/ — Docker parsing, OCI export, sandbox integrations
├── memobuild-server/   — API, authentication, server storage, WebSocket endpoints
├── memobuild-security/ — Audit, SBOM, SLSA/in-toto, policy verification
├── memobuild-observability/ — Metrics, tracing, logging, dashboard integration
├── memobuild-operator/ — Kubernetes CRDs and controller logic
├── memobuild-cli/      — CLI and executable entry point
├── proto/memobuild/v1/ — versioned protocol definitions
├── tests/              — cross-crate integration tests
├── benches/            — performance benchmarks
├── examples/           — supported integration examples
├── extension/          — extension UI and backend
├   ├── ui/             — JavaScript/TypeScript frontend
├   └── backend/        — Go backend
├── deploy/             — Kubernetes manifests and policies
├   ├── k8s/            — CRDs, StatefulSets, Services
├   ├── helm/           — Helm charts
├   └── prometheus/     — alert rules, ServiceMonitors
├── monitoring/         — Grafana dashboards, recording rules
├── charts/             — Helm charts (if separate from deploy/)
├── scripts/            — bootstrap, CI, release and maintenance
├── docs/               — architecture, guides, ADRs, reference
├   ├── architecture/
├   ├── getting-started/
├   ├── guides/
├   ├── reference/
│     ├── cli.md
│     ├── configuration.md
│     └── api/
├   ├── operations/
│     ├── deployment.md
│     ├── scaling.md
│     ├── monitoring.md
│     └── troubleshooting.md
├   ├── security/
├   ├── adr/
│   └── releases/
├── docker/             — shared Docker build assets
├── dist/               — generated release artifacts
└── tests/              — cross-crate integration tests
```

## 5. Phased Implementation Plan

### Phase 01: Inventory & Baseline
- Inspect `Cargo.toml`, `package.json`, `build.rs`, and all module declarations
- Record public APIs, binary targets, feature flags, and dependency versions
- Run existing test suite and record baseline failures
- Map CI, Docker, Helm, and release references to repository paths
- **Deliverable**: architecture inventory and dependency map

### Phase 02: Stabilize Module Boundaries
- Resolve ambiguous module ownership (the overlaps documented above)
- Introduce consistent public facades and error types
- Separate configuration, domain types, and infrastructure adapters
- Add characterization tests before moving behavior
- **Deliverable**: cleaner module boundaries without a major workspace conversion

### Phase 03: Extract Foundational Crates
- Extract `memobuild-core`: graph, hashing, reproducibility, domain types
- Extract `memobuild-cache`: cache abstractions and implementations (resolve `cache/remote.rs` vs `remote_cache.rs` overlap)
- Extract `memobuild-engine`: build planning and lifecycle after dependency graph is clear
- Preserve old imports using re-exports
- **Deliverable**: first working Cargo workspace with incremental crate extraction

### Phase 04: Isolate Platform Services
- Separate `memobuild-remote`: distributed scheduling, workers, routing, REAPI
- Define `memobuild-server`: API and service composition (split from current server crate if tightly coupled)
- Define `memobuild-container`: container and artifact integrations
- Keep protocol definitions versioned under `proto/`
- **Deliverable**: clearly separated local and distributed execution paths

### Phase 05: Unify Deployment and Packaging
- Align Dockerfiles, Compose, Helm charts, and Kubernetes manifests
- Verify monitoring dashboards and alert rules
- Establish generated-artifact and release-package conventions
- Separate extension packaging from Rust package releases
- **Deliverable**: reproducible build and deployment workflows

### Phase 06: Enforce Architecture and Release Gates
- Add crate-level dependency checks and linting
- Run unit, integration, reproducibility, and scalability tests
- Add dependency auditing and security policy validation
- Publish architecture diagrams and migration decisions through ADRs
- **Deliverable**: a maintainable architecture with automated regression protection

## 6. Testing and CI Gates

| Gate | Validation |
|---|---|
| Rust compilation | `cargo check --workspace --all-targets` |
| Rust tests | `cargo test --workspace` |
| Linting | `cargo clippy --workspace --all-targets -- -D warnings` |
| Formatting | `cargo fmt --all -- --check` |
| JavaScript | Package-specific tests, build, and type checking |
| Go | `go test ./...` from each Go module |
| Shell scripts | ShellCheck and existing bootstrap tests |
| Containers | Build every supported Docker target and Compose service |
| Kubernetes | Helm lint/template validation and schema/policy checks |
| Compatibility | Existing integration tests and documented public APIs |
| Performance | Compare `benches/core_bench.rs` and distributed scalability tests against baseline |

## 7. Key Structural Decisions

### 7.1 Cache Trait Unification
The `RemoteCache` trait exists in two places:
- `src/cache/remote.rs` — minimal semantic interface
- `src/remote_cache.rs` — full transport implementation with HttpClient, retries, TLS, compression

**Action**: Make `src/cache/remote.rs` the canonical trait definition. Have `src/remote_cache.rs::HttpRemoteCache` implement that trait. Remove the duplicate trait definition from `remote_cache.rs` or have it re-export from `cache/remote.rs`.

### 7.2 Server Duality
`src/cluster_server.rs` (simple cache server) and `src/network/cluster_server.rs` (cluster-aware server) serve different purposes but share substantial structural similarity.

**Action**: Document the distinction clearly. Consider merging common infrastructure (state management, metrics, auth) into a shared base, but keep the endpoint surfaces separate. The `network/mod.rs` re-export should be reviewed — it currently exposes the cluster server API publicly.

### 7.3 JS Entry Points
Determine whether `src/index.js` or `dist/index.js` is the source of truth. If `dist/index.js` is generated, add it to `.gitignore` and update the build pipeline. If neither is generated, consolidate to a single location.

### 7.4 Secrets Interface
`src/secrets/aws.rs` has a struct that partially overlaps `AwsKmsProvider` in `secrets.rs`.

**Action**: Either remove `src/secrets/aws.rs` and integrate its functionality into `secrets.rs`, or make it a re-export of the provider from `secrets.rs`.

## 8. Immediate Next Steps

1. **Establish the baseline**: Run `cargo check --workspace` and `cargo test --workspace` to record current state
2. **Resolve cache trait duplication**: Unify `RemoteCache` between `src/cache/remote.rs` and `src/remote_cache.rs`
3. **Document server duality**: Clarify `src/cluster_server.rs` vs `src/network/cluster_server.rs` responsibilities
4. **Consolidate JS entry points**: Decide on `src/index.js` vs `dist/index.js` ownership
5. **Audit secrets**: Clean up `src/secrets/aws.rs` overlap
6. **Create workspace scaffold**: Begin Phase 02 with modular crate extraction