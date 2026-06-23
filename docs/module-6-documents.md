# Module 6 Documents

Module 6 implements the document service foundation. The source of truth lives
under `src-tauri/src/documents`.

## Implemented Files

- `models.rs`: document metadata, read/write requests, patch preview/apply reports, index invalidation, artifact write requests, and the `DocumentRepository` trait.
- `service.rs`: `FileDocumentService` for sandboxed file reads/writes, JSON validation, patch preview/apply, artifact persistence, document/chunk/artifact refs, and Git checkpoint integration.

## Contract Rules

- Supported editable formats are Markdown, plain text, and JSON.
- File reads and writes reuse Module 0 `PermissionPolicy` before opening or writing the target path.
- Parent-directory traversal and symlink escape remain rejected by the shared path sandbox.
- Document ids are canonical file paths, so the runtime can pass `document_ref` instead of duplicating large text in workflow state.
- Patch hunks use UTF-8 byte ranges. The service rejects out-of-bounds, overlapping, and non-character-boundary ranges.
- Patch preview returns a compact change window and content hash, not the full rewritten document.
- Saving or patching returns `IndexInvalidation`, allowing the background indexer to perform incremental updates.
- Patch apply can create a Module 5 Git checkpoint after the file write.
- Artifact ids are restricted relative paths and are stored under the configured artifact root.

## Verification

- `cargo fmt`
- `cargo test --quiet`
- `cargo test --features system-keychain --no-run`
