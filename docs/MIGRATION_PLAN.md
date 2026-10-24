# MemoBuild Cargo Workspace Migration Plan

## Principles

1. Inventory before moving.
2. Preserve the current root package and public interfaces until explicitly reviewed.
3. Extract one capability at a time.
4. Add regression tests before changing ownership.
5. Keep non-Rust applications and deployment assets out of Cargo-specific refactors.
6. Every migration change must be reviewable and revertible.

## Stage 0 — Baseline (no source moves)

- [ ] Run `scripts/audit-rust-layout.sh`.
- [ ] Save current test, build, benchmark, and CI results.
- [ ] Review package metadata, features, build script, binary targets, and public API.
- [ ] Identify whether the root package must remain a package in the workspace.
- [ ] Record required feature flags and platform-specific build behavior.
- [ ] Review generated code and `include!` / `env!` / `OUT_DIR` usage.

**Gate:** baseline report reviewed and migration owner approves the proposed crate map.

## Stage 1 — Integrate workspace metadata

- [ ] Merge the workspace configuration from `Cargo.workspace.fragment.toml` into the existing root manifest.
- [ ] Keep the existing `[package]` and targets unless a separately reviewed decision says otherwise.
- [ ] Add only crate members that are ready and present.
- [ ] Run `cargo metadata --format-version 1`.
- [ ] Confirm lockfile behavior and CI commands.
- [ ] Do not change edition, features, or dependency versions as incidental cleanup.

**Gate:** Cargo metadata succeeds and the pre-existing root package still builds.

## Stage 2 — Extract low-level core

- [ ] Map graph, hashing, Git input, reproducibility, and shared type ownership.
- [ ] Move only reviewed modules into `memobuild-core`.
- [ ] Replace implicit cross-module access with explicit public APIs.
- [ ] Add compatibility re-exports where needed.
- [ ] Run targeted tests and the full workspace checks.

**Gate:** core crate has no dependency on server, CLI, Kubernetes, or concrete network clients.

## Stage 3 — Cache and execution contracts

- [ ] Define cache contracts and error semantics.
- [ ] Extract cache backends without changing cache-key semantics.
- [ ] Define engine execution interfaces.
- [ ] Test local and remote cache failure behavior.
- [ ] Verify cache correctness and reproducibility.

**Gate:** cache and engine tests pass; benchmark results are compared with baseline.

## Stage 4 — Remote execution and adapters

- [ ] Extract scheduler, workers, gRPC, REAPI, and remote routing in small increments.
- [ ] Keep protocol definitions versioned under `proto/`.
- [ ] Separate container and storage adapters from core domain logic.
- [ ] Test cancellation, retries, timeouts, worker failure, and recovery.

**Gate:** distributed integration and failure-injection tests pass.

## Stage 5 — Server, operator, security, observability, CLI

- [ ] Extract these capabilities only where boundaries are stable.
- [ ] Keep Kubernetes-specific dependencies outside core.
- [ ] Preserve command behavior and exit codes.
- [ ] Verify API and protocol compatibility.
- [ ] Validate observability and security policy enforcement.

**Gate:** API, CLI, security, and deployment validations pass.

## Stage 6 — Enforce and document

- [ ] Review and update `architecture/dependency-boundaries.json`.
- [ ] Run the boundary checker in CI only after policy review.
- [ ] Document workspace commands and crate ownership.
- [ ] Update architecture diagrams and relevant ADRs.
- [ ] Verify release packaging and non-Rust build workflows.

**Gate:** all required CI checks pass and migration decisions are documented.

## Rollback strategy

- Keep each extraction in a focused commit or pull request.
- Do not combine directory moves with broad formatting or dependency upgrades.
- Preserve the prior implementation until the replacement passes equivalent tests.
- Revert the individual extraction if the baseline or compatibility gate fails.
