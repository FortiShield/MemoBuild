# Proposed Crate Ownership

These are intended boundaries, not a claim that the current source already fits them.

| Crate | Owns | Avoid |
|---|---|---|
| `memobuild-core` | Domain types, graph contracts, hashing/reproducibility primitives | HTTP, Kubernetes, CLI parsing |
| `memobuild-engine` | Build planning and orchestration | UI and transport-specific logic |
| `memobuild-cache` | Cache policy, cache interfaces, backends | CLI argument parsing |
| `memobuild-remote` | Scheduler, workers, remote execution protocol adapters | Dashboard presentation |
| `memobuild-container` | Container parsing, sandbox and OCI integration | Owning all orchestration |
| `memobuild-server` | API and service composition | Reimplementing core algorithms |
| `memobuild-security` | Policy, provenance, artifact verification | Deployment-specific business logic |
| `memobuild-observability` | Instrumentation and telemetry integration | Domain ownership |
| `memobuild-operator` | Kubernetes controllers and CRDs | Kubernetes dependencies in core |
| `memobuild-cli` | CLI entry point and command wiring | Duplicating engine algorithms |

## Dependency policy

- Dependencies point toward stable domain contracts where practical.
- Core must not depend on server, CLI, operator, or concrete infrastructure adapters.
- Infrastructure adapters should be passed into services through explicit interfaces where this improves testability.
- Avoid splitting a crate solely to match a directory name; split when it creates a useful API and dependency boundary.
- Revisit this map after inspecting the real `Cargo.toml`, feature graph, and module visibility.
