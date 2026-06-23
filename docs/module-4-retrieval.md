# Module 4 Retrieval

Module 4 implements the retrieval backend contract layer. The source of truth
lives under `src-tauri/src/retrieval`.

## Implemented Files

- `models.rs`: chunk records, retrieval results, vector/full-text/hybrid requests, store health, and rebuild reports.
- `traits.rs`: `VectorStore`, `FullTextStore`, `ResultReranker`, and `HybridSearch`.
- `memory.rs`: in-memory vector and full-text stores used for tests and early integration.
- `hybrid.rs`: hybrid search engine that merges vector and full-text results by chunk id.
- `reranker.rs`: score-based reranker and adapter for Module 3 `RerankerProvider`.
- `sidecar.rs`: Qdrant sidecar supervisor contract, port selection, health status, crash marking, restart, and TCP health wait.

## Contract Rules

- Retrieval results must include `chunk_id`, `document_id`, snippet, score, source, and optional source spans/metadata.
- Vector and full-text stores support upsert, document deletion, search, health checks, rebuild-required marking, and rebuild reports.
- Qdrant and Tantivy indexes are treated as rebuildable data, not the only source of truth.
- Hybrid search can combine vector and full-text results, de-duplicate by `chunk_id`, and optionally pass candidates through a reranker.
- Qdrant sidecar lifecycle is represented separately from `VectorStore`: start, stop, restart, crash marking, port conflict handling, endpoint reporting, and health checks.
- If Qdrant cannot bind the requested port, the supervisor selects an available port and reports degraded status with a reason.
- If a spawned sidecar does not become reachable during startup, health is degraded rather than silently treated as healthy.

## Current Backend Scope

The current implementation intentionally avoids binding upper modules to a
specific Qdrant or Tantivy client. Real Qdrant/Tantivy adapters can implement
the same traits later while Module 8 and Module 9 integrate against the stable
interfaces.

## Verification

- `cargo fmt`
- `cargo test --quiet`
- `cargo test --features system-keychain --no-run`
