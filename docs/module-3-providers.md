# Module 3 Providers

Module 3 implements the provider abstraction layer. The source of truth lives
under `src-tauri/src/providers`.

## Implemented Files

- `models.rs`: standard LLM, embedding, rerank, and search request/response models.
- `traits.rs`: `LlmProvider`, `EmbeddingProvider`, `RerankerProvider`, and `SearchProvider`.
- `registry.rs`: runtime registry for independently switching providers by id.
- `protocol.rs`: protocol classification and tool-use envelope mapping for OpenAI-compatible, Anthropic, and Gemini families.
- `executor.rs`: provider execution wrapper that records costs through Module 2.

## Contract Rules

- OpenAI-compatible providers use the OpenAI protocol family and require `base_url`.
- Anthropic and Gemini tool-use envelopes are represented distinctly at the provider layer.
- Provider implementations return normalized responses; downstream modules should not branch on raw vendor payloads.
- Provider calls with `cost_usd` are recorded to `CostLedger` with workflow/run/node/tool-call metadata.
- Timeout and retry settings are part of `ProviderCallContext`; concrete HTTP clients in later modules must honor them.

## Verification

- `cargo fmt`
- `cargo test`
- `cargo test --features system-keychain --no-run`

