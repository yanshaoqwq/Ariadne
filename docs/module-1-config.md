# Module 1 Configuration

Module 1 implements project configuration, schema migration, and secret-reference
contracts. The source of truth lives under `src-tauri/src/config`.

## Configuration Layout

`ConfigStore::load_or_create` creates and loads the planned split YAML layout:

- `.config/app.yaml`
- `.config/providers.yaml`
- `.config/permissions.yaml`
- `.config/rag.yaml`
- `.config/workflow.yaml`
- `.config/git.yaml`
- `.config/auto_mode.yaml`

Each file carries `schema_version`.

## Implemented Contracts

- `ProjectConfig` aggregates all split config files.
- Provider config supports OpenAI, Anthropic, Gemini, OpenAI-compatible, local, and search/reranker/embedding capabilities through Module 0 provider types.
- OpenAI-compatible providers require `base_url`.
- Provider API keys are represented as `SecretRef { key_id }`; raw secret values are not serialized into project config.
- `SecretStore` abstracts key storage.
- `MemorySecretStore` supports tests and development.
- `SystemKeychainSecretStore` is available behind the `system-keychain` feature and uses the `keyring` crate.
- `migrate_all` is idempotent and upgrades missing `schema_version` to the current version.
- Auto Mode config is stored separately from permission switches.

## Verification

- `cargo fmt`
- `cargo test`
- `cargo test --features system-keychain --no-run`

