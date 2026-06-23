# Module 8 Knowledge Base

Module 8 implements the long-term knowledge base foundation. The source of
truth lives under `src-tauri/src/knowledge`.

## Implemented Files

- `models.rs`: layered summaries, versioned facts, fact proposals, conflict queue records, approval decisions, health reports, and rebuild reports.
- `traits.rs`: `KnowledgeRepository` contract for replaceable persistence backends.
- `memory.rs`: in-memory repository used by tests and early service integration.
- `service.rs`: proposal decision logic, Auto Mode approval rules, conflict queue construction, and rebuild report helpers.

## Contract Rules

- Every `LayeredSummary` must keep non-empty text, a `source_version`, and at least one `SourceSpan`.
- Every `KnowledgeFact` must keep a stable id, entity, attribute, versioned value, and at least one `SourceSpan`.
- Normal mode keeps unapproved AI fact proposals pending.
- Auto Mode can approve ordinary non-conflicting proposals.
- Conflicting proposals never overwrite existing facts.
- Conflicts are always queued with the proposed fact, source spans, existing fact id, writing reason, and judge status text.
- If independent two-step approval is not available yet, the queue keeps the extraction reason and marks the judge reason as waiting for independent LLM judgment.
- Metadata corruption, index corruption, and Git restore are represented as explicit rebuild reasons.
- Git restore requires both metadata and index rebuild; metadata-only and index-only failures are reported separately.
- Rebuildable indexes are treated as derived state; facts and summaries remain the canonical data.

## Verification

- `cargo fmt`
- `git diff --check`
- `cargo test --quiet`
- `cargo test --features system-keychain --no-run`
