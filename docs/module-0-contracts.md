# Module 0 Contracts

This document records the first implemented contract layer for Literature Agent.
The Rust source of truth lives under `src-tauri/src/core`.

## Implemented Files

- `ports.rs`: typed port definitions, reference-style `PortValue`, text ranges, required port validation.
- `workflow.rs`: workflow, node, edge, run status, `RunControl`, and bounded `LoopPolicy`.
- `artifacts.rs`: artifact descriptors and document patch metadata.
- `registry.rs`: node, skill, and provider registries with duplicate/missing entry errors.
- `events.rs`: event envelope and core runtime event variants.
- `errors.rs`: shared error model used by all core modules.
- `resources.rs`: resource limits, resource pool leases, and cancellation token.
- `permissions.rs`: Auto Mode state, node approval policy, hard permission evaluation.

## Contract Rules

- Large text should move through `DocumentRef`, `ChunkRef`, or `ArtifactRef` instead of inline copies.
- `Pause` and `Stop` are distinct: pause preserves resumable run state, stop terminates the run while preserving completed outputs.
- Loops must be bounded by iteration count, timeout, and a non-null stop condition. Budget limits are optional but validated when present.
- Auto Mode only skips ordinary human confirmation when the node approval policy allows it. It does not bypass network, file, WASM, key, or budget restrictions.
- Registries are keyed by stable type/id strings so later modules can add implementations without changing the core contract.

