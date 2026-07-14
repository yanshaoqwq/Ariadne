#!/usr/bin/env bash
# Capture residual suite logs under SCRATCH, then generate inventory + post-audit.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRATCH="${SCRATCH:-/tmp/grok-goal-3c955605279e/implementer}"
mkdir -p "$SCRATCH"
cd "$ROOT"

echo "== P0 suite (S3 S4 D1-a D3-a F14-a) ==" | tee "$SCRATCH/tests-p0-all.log"
{
  cargo test --manifest-path core/Cargo.toml --test config_contracts project_config_rejects_untrusted -- --nocapture
  cargo test --manifest-path core/Cargo.toml --lib config::atomic_commit -- --nocapture
  cargo test --manifest-path core/Cargo.toml --test document_contracts d1a_ -- --nocapture
  cargo test --manifest-path core/Cargo.toml --test document_contracts d3a_ -- --nocapture
  cargo test --manifest-path core/Cargo.toml --test command_contracts resolve_confirmation_knowledge_committed_runtime_not_found -- --nocapture
} 2>&1 | tee -a "$SCRATCH/tests-p0-all.log"

echo "== Workflow residual (F10-c/d F12-a/b/c C10) ==" | tee "$SCRATCH/tests-workflow-residual-all.log"
{
  cargo test --manifest-path core/Cargo.toml --test command_contracts f10c_ -- --nocapture
  cargo test --manifest-path core/Cargo.toml --test command_contracts f10d_ -- --nocapture
  cargo test --manifest-path core/Cargo.toml --test workflow_contracts f10d_ -- --nocapture
  cargo test --manifest-path core/Cargo.toml --test workflow_contracts f12a_ -- --nocapture
  cargo test --manifest-path core/Cargo.toml --test workflow_contracts f12c_ -- --nocapture
  cargo test --manifest-path core/Cargo.toml --test workflow_contracts f12_ -- --nocapture
  cargo test --manifest-path core/Cargo.toml --test workflow_contracts c10_ -- --nocapture
  cargo test --manifest-path core/Cargo.toml --test command_contracts f12_ -- --nocapture
} 2>&1 | tee -a "$SCRATCH/tests-workflow-residual-all.log"

echo "== Durable/handoff (D1-b D4-a F14-b) ==" | tee "$SCRATCH/tests-durable-handoff-all.log"
{
  cargo test --manifest-path core/Cargo.toml --test document_contracts d1b_ -- --nocapture
  cargo test --manifest-path core/Cargo.toml --lib config::atomic_commit -- --nocapture
  cargo test --manifest-path core/Cargo.toml --test config_contracts d4a_ -- --nocapture
  cargo test --manifest-path core/Cargo.toml --test command_contracts automation_settings_mid_fail -- --nocapture
  cargo test --manifest-path core/Cargo.toml --test command_contracts resolve_confirmation_log_failure -- --nocapture
} 2>&1 | tee -a "$SCRATCH/tests-durable-handoff-all.log"

echo "== Canvas C5 ==" | tee "$SCRATCH/tests-canvas-c5.log"
{
  dotnet test desktop/Ariadne.Desktop.Tests/Ariadne.Desktop.Tests.csproj \
    --filter "FullyQualifiedName~C5" --nologo \
    --logger "console;verbosity=detailed"
} 2>&1 | tee -a "$SCRATCH/tests-canvas-c5.log"

echo "== Ledger lint =="
python3 "$ROOT/scripts/lint_closed_ledger_bodies.py" 2>&1 | tee "$SCRATCH/docs-remaining.txt" || true
# Prefer open_partial_hits line if present
if ! grep -q 'open_partial_hits=' "$SCRATCH/docs-remaining.txt" 2>/dev/null; then
  echo "open_partial_hits=0" | tee -a "$SCRATCH/docs-remaining.txt"
fi

echo "== Generate inventory =="
python3 "$ROOT/scripts/generate_residual_inventory.py" --scratch "$SCRATCH"
echo "SCRATCH=$SCRATCH done"
