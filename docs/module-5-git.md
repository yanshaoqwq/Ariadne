# Module 5 Git

Module 5 implements the Git service foundation. The source of truth lives under
`src-tauri/src/git`.

## Implemented Files

- `models.rs`: health reports, commit summaries, archive points, checkpoints, restore reports, and branch graph nodes.
- `service.rs`: `GitService` with serialized Git operations through a process-local mutex.

## Contract Rules

- Git write operations are globally serialized inside `GitService`.
- `init_repository` is idempotent.
- Named archive points create commits with `Archive: <name>` by default.
- Node checkpoints create commits with `Checkpoint: node <node_id>` by default.
- Restore never rewrites the current branch. It requires a clean worktree and checks out the target commit into a new branch.
- Restore reports mark indexes and runtime bindings as requiring rebuild/rebind.
- Health checks detect non-repositories and repositories without commits. They do not promise automatic repair of corrupted Git data.
- Backup/reinitialize support is intentionally conservative: the service exposes the backup directory name and can re-run `git init`, while destructive backup/removal decisions remain an explicit upper-layer action.

## Verification

- `cargo fmt`
- `cargo test --quiet`
- `cargo test --features system-keychain --no-run`
