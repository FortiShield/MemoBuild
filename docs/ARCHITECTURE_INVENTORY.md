# Architecture inventory

This is a repository-grounded starting point for the incremental restructuring
described in [Architecture](ARCHITECTURE.md). It records current ownership and
build surfaces so future moves can be based on actual dependencies rather than
directory names alone. It does not prescribe a bulk file move.

## Current package and build surfaces

### Rust

- The root `Cargo.toml` defines one `memobuild` package (version `0.2.0`,
  edition 2021, MIT), not a Cargo workspace. `Cargo.lock` is the Rust lockfile.
- The package exposes a library (`src/lib.rs`) and a binary (`src/main.rs`),
  plus three examples, six integration-test targets, and the `core_bench`
  benchmark.
- The root library publicly exposes the majority of its top-level modules.
  Those modules currently share one crate and can freely depend on one another;
  the source tree alone does not enforce the proposed inward dependency model.
- `build.rs` always compiles `proto/memobuild/v1/execution.proto` and writes a
  descriptor set. The default feature is `remote-exec`; `containerd` and
  `remote-exec` enable optional tonic/prost dependencies. The `server` feature
  is currently empty.
- Dependencies cover graph and hashing, async execution and HTTP/gRPC,
  SQLite/PostgreSQL, Redis, S3/GCS, OCI/containerd, Kubernetes, tracing and
  metrics, CLI, and security-related integrations. These are all currently
  declared by the same package.

### Other language/package surfaces

- `extension/backend/` is a Go module; `extension/ui/` is a separate Node
  package. The example applications under `examples/` also include Go and
  Node packages.
- The root `package.json` is named `memobuild-demo`; its build script copies
  `src/index.js` to `dist/index.js`. The two JavaScript files currently have
  identical contents.
- Protocol definitions live under `proto/`. Deployment and operations assets
  already have top-level homes under `deploy/`, `charts/`, `monitoring/`, and
  `scripts/`.

## Existing module ownership

| Responsibility | Current paths | Current boundary |
|---|---|---|
| Build graph, hashing, inputs | `src/graph.rs`, `src/core.rs`, `src/hasher/`, `src/git.rs`, `src/env/` | Core concepts are spread across root modules and submodules. |
| Docker parsing and build graph creation | `src/docker/` | Distinct parser, extension, and graph-building modules. |
| Build execution and sandboxing | `src/executor.rs`, `src/execution/`, `src/sandbox/`, `src/remote_exec/` | Local/incremental and remote execution concerns are visible, but executor ownership is duplicated (see below). |
| Cache | `src/cache.rs`, `src/cache/`, `src/cache_cluster.rs`, `src/cache_redis.rs`, `src/remote_cache.rs` | A public cache facade coexists with separate root-level cache APIs and implementations. |
| Server and platform services | `src/server/`, `src/network/`, `src/cluster_server.rs`, `src/storage/`, `src/remote_router/` | API/cache server, cluster services, storage adapters, and routing are separate paths within the same crate. |
| Security and provenance | `src/auth.rs`, `src/secrets.rs`, `src/audit.rs`, `src/verify.rs`, `src/sbom.rs`, `src/slsa.rs` | Related capabilities are spread across public root modules; some have nested implementation modules. |
| Kubernetes operator | `src/operator/` | Operator logic and CRD types are grouped, but remain in the application crate. |

This is a path-based inventory, not a dependency graph. The next extraction
step should use actual imports and tests to define crate boundaries.

## Overlaps and ownership findings

| Paths | Verified finding | Safe next step |
|---|---|---|
| `src/cache.rs` and `src/cache/` | `cache.rs` is a facade: it declares the child modules and re-exports their public types. | Retain this shape; it is not itself a duplicate implementation. |
| `src/executor.rs` and `src/execution/executor.rs` | Resolved: the nested module owns the implementation; the root module re-exports it for callers using the existing `executor` API. The consolidated implementation retains the root tracing span and the nested remote input-manifest upload. | Keep the root re-export as a compatibility facade; characterize behavior before future executor changes. |
| `src/cluster_server.rs` and `src/network/cluster_server.rs` | Both define `ClusterServer`/`ClusterAppState` and implement matching cache, cluster, scaling, and health routes. They import different cache and scaling APIs. The CLI starts the root-level server. | Compare the full handlers and callers before consolidating; avoid deleting either based on naming alone. |
| `src/cache_cluster.rs` and `src/cache/cluster.rs` | Both define cache cluster types and consistent-hash behavior, but depend on different `RemoteCache` traits (`crate::remote_cache` versus `crate::cache::remote`). The root-level variant is wired into the CLI. | Identify the canonical cache contract and capture both callers before moving the implementation. |
| `src/remote_cache.rs` and `src/cache/remote.rs` | Each declares a distinct public `RemoteCache` trait and `RemoteCacheEntry`. The root trait includes build-event/DAG reporting; the nested trait is used by the cache facade and includes artifact/layer operations. | Treat these as overlapping, incompatible APIs until their consumers and required capabilities are mapped. |
| `src/secrets.rs` and `src/secrets/` | The file is the parent module and contains the provider trait/implementations; `secrets/aws.rs` is a child module with a currently empty `kms` namespace. | Keep the parent/child namespace; clarify provider ownership when implementing or extracting adapters. |
| `src/slsa.rs` and `src/slsa/` | The root module owns provenance types and declares `in_toto`; the child uses the parent `Signature` type. | Treat as a parent API plus helper, not a duplicate based on the paths alone. |
| `src/server/` and `src/remote_exec/` | The former contains the cache/API server and storage/metadata surfaces; the latter contains worker, scheduler, and remote-execution protocol services. | Preserve the control-plane/worker distinction and define explicit shared contracts before extraction. |
| `src/index.js` and `dist/index.js` | Contents match; the root npm build script copies source to `dist/`. | Keep the generation relationship explicit and confirm publishing/consumption before changing tracked output. |

## CI, deployment, and packaging references

- `.github/workflows/ci.yml` runs Rust formatting, Clippy with all targets and
  features, tests with all features, Cargo audit, a multi-platform Rust build,
  and ShellCheck/bootstrap tests. The test workflow does not run Node or Go
  package tests.
- `.github/workflows/docker.yml` builds `Dockerfile.server` from the repository
  root and runs Trivy image scanning on non-PR events. `docker-compose.prod.yml`
  also names `Dockerfile.server`.
- `.github/workflows/release.yml` builds and publishes the root Cargo binary
  and crate. The release workflow is separate from the extension release
  workflow.
- `Dockerfile.farm`, `Dockerfile.sample`, and `Dockerfile.server` are tracked.
  Keep the primary root Dockerfile and update every build context/reference
  before considering specialized Dockerfile moves.
- `output.txt` is tracked and is named as an output by
  `examples/build_farm_demo.rs`. `test_dockerfile` is tracked; a repository
  content search found no references to that filename. Neither file should be
  removed without a separate usage/fixture review.

## Migration implications

1. Do not convert to a multi-crate workspace by moving files first. The public
   root library API and the remaining overlapping cache/server contracts make
   that a compatibility-sensitive change.
2. Continue stabilizing cache and server API ownership;
   root-level modules are already consumed by the CLI and tests, while the
   nested cache API is consumed by the facade modules.
3. Add characterization tests for behavior that differs between parallel
   implementations. Then extract foundational graph/hash contracts and cache
   interfaces before execution and infrastructure adapters.
4. Preserve the current root package as a compatibility facade during
   incremental crate extraction. Keep deployment, protocol, extension, and
   example package release surfaces independent unless an explicit packaging
   requirement justifies changing them.

## Baseline commands

The existing CI commands are the reference baseline:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features -- --nocapture
cargo audit
./scripts/lint_shellcheck --ci
./scripts/tests/bootstrap_test.sh
```

Run package-specific Go, Node, Docker, Helm, and Kubernetes checks separately;
they are not covered by the root Cargo package.
