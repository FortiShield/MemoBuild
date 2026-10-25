# MemoBuild — Engineering TODO

> **Objective:** Improve architecture, modularity, security, performance, developer experience, and production readiness while preserving existing functionality and compatibility.

**Project:** MemoBuild  
**Focus:** Distributed build system, remote execution, caching, container integration, and platform operations.

## Priority Legend

- **P0 — Critical:** Correctness, security, build integrity, and release blockers.
- **P1 — High:** Architecture, reliability, and essential production capabilities.
- **P2 — Medium:** Maintainability, performance, developer experience, and automation.
- **P3 — Future:** Advanced optimization and optional capabilities.

Task completion must be based on verified implementation and passing validation, not merely code changes.

---

## Phase 1: Repository Audit and Baseline

**Priority: P0**

- [ ] Inspect `Cargo.toml`, `Cargo.lock`, `build.rs`, and `package.json`.
- [ ] Inventory Rust modules, public APIs, binary targets, features, and dependencies.
- [ ] Identify overlapping modules and duplicate implementations.
- [ ] Document current crate and module dependency relationships.
- [ ] Identify generated files, temporary outputs, and tracked build artifacts.
- [ ] Audit Dockerfiles, Compose configurations, Helm charts, and Kubernetes manifests.
- [ ] Inventory CI workflows and release procedures.
- [ ] Run existing Rust, JavaScript, Go, shell, and integration tests.
- [ ] Record baseline build failures, test failures, and known limitations.
- [ ] Establish performance baselines using existing benchmarks.
- [ ] Create an architecture decision record for the restructuring strategy.

**Acceptance criteria**

- [ ] Current architecture is documented.
- [ ] Existing build and test commands are reproducible.
- [ ] Breaking-change risks and migration dependencies are recorded.
- [ ] No source files have been removed without dependency verification.

## Phase 2: Module Ownership and Internal Refactoring

**Priority: P0–P1**

- [ ] Audit `src/cache.rs` and `src/cache/`.
- [ ] Audit `src/executor.rs` and `src/execution/`.
- [ ] Audit `src/cluster_server.rs` and `src/network/cluster_server.rs`.
- [ ] Audit `src/remote_cache.rs` and `src/cache/remote.rs`.
- [ ] Audit `src/cache_cluster.rs` and `src/cache/cluster.rs`.
- [ ] Audit `src/secrets.rs` and `src/secrets/`.
- [ ] Audit `src/slsa.rs` and `src/slsa/`.
- [ ] Clarify the responsibilities of `src/server/` and `src/remote_exec/`.
- [ ] Establish a single documented public interface for each capability.
- [ ] Separate domain logic from infrastructure integrations.
- [ ] Standardize error handling and configuration ownership.
- [ ] Add characterization tests before changing module behavior.
- [ ] Preserve existing public APIs where practical.

**Acceptance criteria**

- [ ] Every major capability has a clearly documented owner.
- [ ] Duplicate implementations are consolidated or their distinct responsibilities are documented.
- [ ] Existing functionality remains covered by tests.
- [ ] Module boundaries do not introduce unnecessary circular dependencies.

## Phase 3: Cargo Workspace Migration

**Priority: P1**

- [ ] Define the Cargo workspace and dependency policy.
- [ ] Establish shared package metadata and dependency conventions.
- [ ] Extract `memobuild-core`.
  - [ ] Build graph and domain types.
  - [ ] File hashing and input discovery.
  - [ ] Reproducibility and normalization.
- [ ] Extract `memobuild-cache`.
  - [ ] Local cache.
  - [ ] Remote cache.
  - [ ] Hybrid cache.
  - [ ] Cluster cache and backend adapters.
- [ ] Extract `memobuild-engine`.
  - [ ] Build planning and orchestration.
  - [ ] Execution lifecycle.
  - [ ] Core integration contracts.
- [ ] Extract `memobuild-container`.
  - [ ] Dockerfile parsing.
  - [ ] Container execution integration.
  - [ ] OCI artifact export.
- [ ] Extract `memobuild-remote`.
  - [ ] Scheduler and worker pool.
  - [ ] Remote execution protocol.
  - [ ] gRPC server and client.
  - [ ] Remote routing and regional execution.
- [ ] Extract `memobuild-server`.
  - [ ] API and service composition.
  - [ ] Server metadata and storage.
  - [ ] Authentication and request handling.
- [ ] Evaluate extraction of `memobuild-security`.
- [ ] Evaluate extraction of `memobuild-observability`.
- [ ] Evaluate extraction of `memobuild-operator`.
- [ ] Establish `memobuild-cli` as the CLI entry point.
- [ ] Preserve the root package or compatibility facade when required.
- [ ] Migrate one crate at a time.
- [ ] Validate dependency direction after every extraction.

**Acceptance criteria**

- [ ] The workspace builds successfully.
- [ ] All required targets are included in validation.
- [ ] Existing integration tests pass or have documented, approved exceptions.
- [ ] Crate dependencies follow the agreed architecture.
- [ ] Public API changes are documented and versioned appropriately.

## Phase 4: Build Engine and Distributed Execution

**Priority: P1**

- [ ] Define clear contracts for local and remote execution.
- [ ] Validate build graph integrity and dependency resolution.
- [ ] Verify deterministic input hashing and cache-key generation.
- [ ] Strengthen reproducible build guarantees.
- [ ] Implement or verify execution cancellation and timeout handling.
- [ ] Review retry policies and idempotency.
- [ ] Verify worker registration, health checks, and lifecycle management.
- [ ] Validate scheduler behavior under worker failures.
- [ ] Test remote execution failure recovery.
- [ ] Test remote input handling and artifact delivery.
- [ ] Verify resource limits and execution isolation.
- [ ] Test concurrent execution and backpressure.
- [ ] Document distributed execution protocols and compatibility.
- [ ] Benchmark local versus remote execution overhead.

**Acceptance criteria**

- [ ] Build graph execution is deterministic where required.
- [ ] Worker failures do not silently produce successful build results.
- [ ] Cancellation and timeout behavior are tested.
- [ ] Distributed execution passes integration and scalability tests.

## Phase 5: Cache Correctness and Performance

**Priority: P0–P1**

- [ ] Audit cache-key correctness and invalidation behavior.
- [ ] Verify cache isolation between projects and trust boundaries.
- [ ] Validate cache integrity before artifact reuse.
- [ ] Test concurrent cache reads and writes.
- [ ] Review Redis and remote backend failure handling.
- [ ] Define behavior when remote cache services are unavailable.
- [ ] Validate metadata consistency and expiration policies.
- [ ] Measure local, remote, and hybrid cache hit rates.
- [ ] Benchmark cache latency and throughput.
- [ ] Identify memory and storage growth patterns.
- [ ] Test recovery from partial writes and interrupted operations.
- [ ] Document cache configuration and tuning recommendations.

**Acceptance criteria**

- [ ] Cache hits cannot bypass required correctness checks.
- [ ] Cache failures have defined fallback behavior.
- [ ] Concurrent access is covered by tests.
- [ ] Performance changes are measured against the baseline.

## Phase 6: Security and Software Supply Chain

**Priority: P0**

- [ ] Review authentication and authorization boundaries.
- [ ] Audit secret storage, retrieval, and log redaction.
- [ ] Validate TLS configuration and certificate lifecycle.
- [ ] Review sandbox isolation and container privileges.
- [ ] Review remote worker trust and network access controls.
- [ ] Validate artifact provenance and signature verification.
- [ ] Review SLSA and in-toto implementation.
- [ ] Validate SBOM generation and completeness.
- [ ] Review Sigstore policy enforcement.
- [ ] Audit dependency vulnerabilities and license compliance.
- [ ] Prevent untrusted cache artifacts from being treated as trusted.
- [ ] Test authorization for sensitive server and cluster operations.
- [ ] Review Kubernetes RBAC, secrets, and network policies.
- [ ] Add security regression tests for identified risks.
- [ ] Document threat models and security response procedures.

**Acceptance criteria**

- [ ] Critical security findings are resolved or explicitly tracked with mitigations.
- [ ] Authentication and authorization are tested.
- [ ] Artifact verification failures are handled safely.
- [ ] Security policies are enforced in automated validation.

## Phase 7: API, Protocols, and Developer Interfaces

**Priority: P1–P2**

- [ ] Review the OpenAPI contract and API versioning.
- [ ] Review `proto/memobuild/v1/execution.proto`.
- [ ] Define compatibility rules for protocol evolution.
- [ ] Validate generated code and client compatibility.
- [ ] Document authentication, errors, and retry behavior.
- [ ] Audit JavaScript package entry points and `dist/index.js`.
- [ ] Establish source-to-distribution build conventions.
- [ ] Validate CLI commands and exit-code semantics.
- [ ] Keep CLI documentation synchronized with implementation.
- [ ] Add integration tests for supported language clients.
- [ ] Document API deprecation and migration policies.

**Acceptance criteria**

- [ ] Public interfaces have documented compatibility guarantees.
- [ ] Generated artifacts are reproducible.
- [ ] API and protocol changes are tested before release.

## Phase 8: Containers and Deployment

**Priority: P1**

- [ ] Audit `Dockerfile`, `Dockerfile.farm`, `Dockerfile.sample`, and `Dockerfile.server`.
- [ ] Standardize image naming, build arguments, and build contexts.
- [ ] Optimize multi-stage builds where appropriate.
- [ ] Review image size and unnecessary dependencies.
- [ ] Validate non-root execution where supported.
- [ ] Audit `docker-compose.prod.yml` and extension Compose configuration.
- [ ] Validate Helm chart templates and default values.
- [ ] Validate Kubernetes CRDs and manifests.
- [ ] Review deployment probes, resource requests, and resource limits.
- [ ] Validate PodDisruptionBudget and autoscaling configurations.
- [ ] Verify persistent storage and data migration requirements.
- [ ] Review certificate and secret provisioning.
- [ ] Validate production startup and shutdown behavior.
- [ ] Document rollback and recovery procedures.

**Acceptance criteria**

- [ ] Supported container images build successfully.
- [ ] Helm templates render with documented configurations.
- [ ] Deployment resources pass appropriate validation.
- [ ] Production startup and rollback procedures are tested.

## Phase 9: Observability and Operations

**Priority: P1–P2**

- [ ] Standardize structured logging.
- [ ] Define metrics for builds, workers, caches, and remote execution.
- [ ] Add or verify distributed tracing across service boundaries.
- [ ] Validate Grafana dashboards and Prometheus data sources.
- [ ] Review alert thresholds and alert routing.
- [ ] Add visibility into queue depth and worker saturation.
- [ ] Monitor build latency and cache performance.
- [ ] Define service-level indicators and objectives.
- [ ] Document health endpoints and diagnostic procedures.
- [ ] Test behavior during dependency outages.
- [ ] Establish operational runbooks.

**Acceptance criteria**

- [ ] Key system health and performance indicators are observable.
- [ ] Alerts identify actionable operational problems.
- [ ] Troubleshooting procedures cover common distributed failures.

## Phase 10: Tests, CI, and Quality Gates

**Priority: P1**

- [ ] Organize unit tests by crate and domain.
- [ ] Keep cross-component tests in `tests/`.
- [ ] Add dedicated test-support utilities where useful.
- [ ] Run Rust formatting and linting in CI.
- [ ] Add JavaScript build and type-checking validation.
- [ ] Add Go module tests.
- [ ] Validate shell scripts with ShellCheck.
- [ ] Validate Docker and Compose builds.
- [ ] Validate Helm charts and Kubernetes resources.
- [ ] Add reproducibility regression tests.
- [ ] Add cache correctness and concurrency tests.
- [ ] Add remote execution failure-injection tests.
- [ ] Add scalability and load-test workflows.
- [ ] Establish a documented CI test matrix.
- [ ] Define branch protection and release gating requirements.

**Acceptance criteria**

- [ ] Required checks run consistently in CI.
- [ ] Failures identify the responsible component.
- [ ] Regression tests protect previously fixed defects.
- [ ] Performance tests have documented baselines and tolerances.

## Phase 11: Documentation and Examples

**Priority: P2**

- [ ] Establish a canonical documentation landing page.
- [ ] Organize architecture documentation and diagrams.
- [ ] Consolidate overlapping performance and scaling guides.
- [ ] Organize configuration, API, CLI, and deployment references.
- [ ] Organize ADRs into a consistent directory.
- [ ] Preserve historical release notes and completion records.
- [ ] Validate internal documentation links.
- [ ] Audit all example projects for correctness.
- [ ] Separate user-facing examples from internal test utilities.
- [ ] Add prerequisites and expected outputs to each example.
- [ ] Document local development and workspace setup.
- [ ] Document release, upgrade, and migration procedures.

**Acceptance criteria**

- [ ] New contributors can build and test the project from the documentation.
- [ ] Examples run using documented instructions.
- [ ] Architecture decisions are discoverable and current.

## Phase 12: Repository Hygiene and Release Engineering

**Priority: P2**

- [ ] Review `output.txt` and determine whether it should be tracked.
- [ ] Review `test_dockerfile` and clarify its purpose.
- [ ] Define generated-artifact policies for `dist/` and build outputs.
- [ ] Standardize repository ignore rules.
- [ ] Review dependency versioning and lockfile policies.
- [ ] Standardize semantic versioning and release notes.
- [ ] Automate release artifact creation and checksums.
- [ ] Verify published package contents.
- [ ] Establish release rollback procedures.
- [ ] Document supported platforms and compatibility.
- [ ] Audit repository permissions and release credentials.

**Acceptance criteria**

- [ ] Generated artifacts have an explicit ownership policy.
- [ ] Release packages contain the intended files.
- [ ] Release procedures are repeatable and documented.

## Phase 13: Future Enhancements

**Priority: P3**

- [ ] Evaluate advanced remote execution scheduling strategies.
- [ ] Investigate cross-region cache optimization.
- [ ] Explore predictive build scheduling based on workload history.
- [ ] Evaluate adaptive worker scaling.
- [ ] Improve resource-aware task placement.
- [ ] Explore incremental graph recomputation.
- [ ] Investigate advanced cache compression and storage tiering.
- [ ] Improve build analytics and performance diagnostics.
- [ ] Evaluate additional storage and execution backends.
- [ ] Establish benchmarks for experimental optimization features.

**Acceptance criteria**

- [ ] Each enhancement has a proposal, measurable success criteria, and benchmark evidence.
- [ ] Experimental features do not compromise correctness or security.

---

## Recommended Execution Order

| Order | Workstream | Completion gate |
|---|---|---|
| 1 | Repository audit | Verified baseline and dependency map |
| 2 | Module ownership | Clear responsibilities and regression tests |
| 3 | Core and cache extraction | Stable foundational crates |
| 4 | Engine and remote execution | Verified local and distributed execution |
| 5 | Security and reliability | Required security and failure-handling tests |
| 6 | API and deployment | Validated interfaces and deployment assets |
| 7 | CI and observability | Automated quality and operational gates |
| 8 | Documentation and release | Reproducible contributor and release workflows |
| 9 | Advanced optimization | Measured improvements |

## Project Rules

1. Do not perform a full rewrite solely to achieve a new directory layout.
2. Do not remove overlapping modules until their callers and behavior have been verified.
3. Preserve public API compatibility unless a documented breaking change is approved.
4. Introduce architecture changes through small, reviewable pull requests.
5. Run relevant tests after every structural migration.
6. Do not claim a task is complete without verifiable evidence.
7. Track unresolved risks explicitly rather than hiding them behind successful compilation.
8. Update architecture documentation whenever a significant boundary changes.

## Definition of Done

MemoBuild's restructuring is complete when the repository has clear module ownership, an enforceable dependency architecture, reliable local and distributed execution, verified security controls, reproducible builds, automated quality gates, validated deployment assets, and maintainable documentation.

**Immediate next action:** Complete Phase 1 and publish the verified architecture inventory before beginning large-scale file movement.
