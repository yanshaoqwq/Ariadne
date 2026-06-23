# Module 2 Costs And Budgets

Module 2 implements the cost tracking and budget-control foundation. The source
of truth lives under `src-tauri/src/costs`.

## Implemented Files

- `models.rs`: cost categories, new/recorded cost records, cost queries, token usage.
- `pricing.rs`: token-cost estimation from provider model pricing.
- `budget.rs`: single-call, daily, monthly, high-cost confirmation, and Auto Mode preauthorization decisions.
- `ledger.rs`: SQLite-backed cost ledger using `costs.db`.

## Runtime Semantics

- All cost records include category, timestamp, amount, and optional provider/model/workflow/run/node/tool-call metadata.
- Tool-use costs can be recorded by `tool_call_id`.
- Over-budget decisions return `RunControl::Pause`.
- Normal mode high-cost operations return `RequireConfirmation` with `RunControl::Pause`.
- Auto Mode allows calls inside preauthorized budget and pauses when preauthorization is exceeded.

## Storage

`SqliteCostLedger::open(project_root)` stores data in `costs.db`.
`SqliteCostLedger::open_in_memory()` supports tests.

The SQLite schema has an idempotent migration path and stores:

- `schema_migrations`
- `cost_events`

## Verification

- `cargo fmt`
- `cargo test`
- `cargo test --features system-keychain --no-run`

