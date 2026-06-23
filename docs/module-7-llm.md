# Module 7 LLM

Module 7 implements the LLM orchestration foundation. The source of truth lives
under `src-tauri/src/llm`.

## Implemented Files

- `models.rs`: LLM service config, run request/report, stream event contract, tool executor contract, tool execution context/output, and audit events.
- `service.rs`: `LlmService` for basic generation, tool-use loops, cancellation checks, timeout checks, token limits, budget decisions, audit logs, and cost ledger integration through `ProviderExecutor`.

## Contract Rules

- LLM calls go through Module 3 providers and Module 3 `ProviderExecutor`.
- Provider-reported costs are still written to Module 2 cost ledger.
- Basic generation performs a single provider call and returns an auditable report.
- Tool-use generation loops until the model returns no tool calls or a hard limit is reached.
- Tool calls are routed through `ToolExecutor`, which Module 9 RAG tools and Module 10 Skill executors can implement later.
- Assistant tool-call messages and tool-result messages are appended to the next model request.
- Tool-use loops are protected by max rounds, overall timeout, cancellation token, token limit, and budget checks.
- High-cost normal-mode calls return `CoreError::Paused`, preserving the existing run-control semantics.
- Stream support exposes `LlmStreamEvent` and calls providers with `stream=true`; the current synchronous provider contract is adapted into Started/Delta/ToolCall/Finished events. Provider-level token deltas will be connected when protocol adapters are implemented.
- Audit logs record provider response, tool call request, tool completion, costs, tokens, and final run control without storing secrets or full large context.

## Verification

- `cargo fmt`
- `git diff --check`
- `cargo test --quiet`
- `cargo test --features system-keychain --no-run`
