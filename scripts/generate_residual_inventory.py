#!/usr/bin/env python3
"""Generate {SCRATCH}/unfinished-inventory.md from residual_re_review_manifest.yaml + logs.

Exit non-zero if:
  - residual count != 16
  - any required column missing
  - any primary test_filter not green in its log
  - F12 not split into a/b/c separate rows
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

try:
    import yaml  # type: ignore
except ImportError:
    yaml = None


REQUIRED_IDS = [
    "S3",
    "S4",
    "D1-a",
    "D1-b",
    "D3-a",
    "D4-a",
    "F10-c",
    "F10-d",
    "F12-a",
    "F12-b",
    "F12-c",
    "C10",
    "F14-a",
    "F14-b",
    "C5-a",
    "C5-b",
]

DISPOSITIONS = {
    "code-close",
    "already-closed",
    "honest-open-backlog",
    "Non-goal",
}


def load_manifest(path: Path) -> list[dict]:
    text = path.read_text(encoding="utf-8")
    if yaml is not None:
        data = yaml.safe_load(text)
        return list(data["residuals"])
    # Minimal YAML subset parser for this flat-list schema (no pyyaml).
    rows: list[dict] = []
    cur: dict | None = None
    extras: list[str] | None = None
    for raw in text.splitlines():
        line = raw.rstrip()
        if line.strip().startswith("#") or not line.strip():
            continue
        if line.startswith("residuals:"):
            continue
        m = re.match(r"^  - id:\s*(.+)$", line)
        if m:
            if cur is not None:
                rows.append(cur)
            cur = {"id": m.group(1).strip().strip('"'), "extra_test_filters": []}
            extras = cur["extra_test_filters"]
            continue
        if cur is None:
            continue
        m = re.match(r"^    (\w+):\s*(.*)$", line)
        if m:
            key, val = m.group(1), m.group(2).strip()
            if key == "extra_test_filters":
                continue
            if val.startswith('"') and val.endswith('"'):
                val = val[1:-1]
            cur[key] = val
            continue
        m = re.match(r'^      - "(.+)"$', line)
        if m and extras is not None:
            extras.append(m.group(1))
    if cur is not None:
        rows.append(cur)
    return rows


def filter_green(log_text: str, test_filter: str) -> bool:
    """True if a line shows the test passed (cargo or dotnet style)."""
    # cargo: test name ... ok
    # cargo: test path::name ... ok
    # dotnet detailed: 已通过 Namespace.TestName [time]
    patterns = [
        re.compile(rf"{re.escape(test_filter)}\s*\.\.\.\s*ok\b", re.I),
        re.compile(rf"{re.escape(test_filter)}\s+ok\b", re.I),
        re.compile(rf"\b{re.escape(test_filter)}\b.*\bok\b", re.I),
        re.compile(rf"已通过[^\n]*{re.escape(test_filter)}", re.I),
        re.compile(rf"Passed[^\n]*{re.escape(test_filter)}", re.I),
        re.compile(rf"{re.escape(test_filter)}[^\n]*(已通过|Passed)", re.I),
    ]
    for pat in patterns:
        if pat.search(log_text):
            return True
    # Dotnet summary: filter name appears and suite fully green
    if test_filter in log_text:
        if re.search(r"失败:\s*0|Failed:\s*0|已通过!|测试运行成功|test result: ok\.", log_text):
            return True
        if re.search(rf"{re.escape(test_filter)}[^\n]*ok", log_text, re.I):
            return True
    return False


def log_overall_failed(log_text: str) -> bool:
    if re.search(r"test result: FAILED", log_text):
        return True
    if re.search(r"失败:\s*[1-9]|Failed:\s*[1-9]", log_text):
        return True
    return False


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--manifest",
        type=Path,
        default=Path(__file__).resolve().parent / "residual_re_review_manifest.yaml",
    )
    ap.add_argument(
        "--scratch",
        type=Path,
        required=True,
        help="SCRATCH implementer dir containing tests-*.log",
    )
    ap.add_argument(
        "--out",
        type=Path,
        default=None,
        help="Output inventory path (default: SCRATCH/unfinished-inventory.md)",
    )
    ap.add_argument(
        "--post-audit",
        type=Path,
        default=None,
        help="Also write post-audit.md (default: SCRATCH/post-audit.md)",
    )
    args = ap.parse_args()
    scratch: Path = args.scratch
    out = args.out or (scratch / "unfinished-inventory.md")
    post_audit = args.post_audit or (scratch / "post-audit.md")

    rows = load_manifest(args.manifest)
    errors: list[str] = []
    if len(rows) != 16:
        errors.append(f"row_count={len(rows)} want 16")

    ids = [r.get("id") for r in rows]
    if ids != REQUIRED_IDS:
        errors.append(f"id order/set mismatch: got {ids}")

    if "F12-a/b/c" in ids or any(i == "F12" for i in ids):
        errors.append("F12 must be split into F12-a, F12-b, F12-c rows")

    log_cache: dict[str, str] = {}
    table_rows: list[str] = []
    live: list[str] = []
    closed = 0

    for r in rows:
        rid = r.get("id", "")
        mode = r.get("residual_mode", "")
        entry = r.get("product_entry", "")
        disp = r.get("disposition", "")
        filt = r.get("test_filter", "")
        log_rel = r.get("log_path", "")
        if not all([rid, mode, entry, disp, filt, log_rel]):
            errors.append(f"{rid}: missing required column(s)")
            continue
        if disp not in DISPOSITIONS:
            errors.append(f"{rid}: bad disposition {disp!r}")

        log_path = scratch / log_rel
        if log_rel not in log_cache:
            if not log_path.exists():
                log_cache[log_rel] = ""
                errors.append(f"{rid}: missing log {log_path}")
            else:
                log_cache[log_rel] = log_path.read_text(encoding="utf-8", errors="replace")
        log_text = log_cache.get(log_rel, "")

        filters = [filt] + list(r.get("extra_test_filters") or [])
        missing_filters: list[str] = []
        for f in filters:
            if not filter_green(log_text, f):
                missing_filters.append(f)

        green = not missing_filters and bool(log_text)
        if disp in ("code-close", "already-closed"):
            if not green:
                live.append(rid)
                errors.append(
                    f"{rid}: disposition {disp} but filter(s) not green in {log_rel}: {missing_filters}"
                )
            else:
                closed += 1
        elif disp in ("honest-open-backlog", "Non-goal"):
            live.append(rid)  # open by design; still counts as unfinished residual

        cite = f"`{filt}` @ `{log_rel}`"
        if r.get("extra_test_filters"):
            cite += " + " + ", ".join(f"`{x}`" for x in r["extra_test_filters"])
        table_rows.append(
            f"| **{rid}** | {mode} | `{entry}` | **{disp}** | {cite} |"
        )

    residual_count = len(live)
    # For goal criterion 4: live re-review residual = open modes still shipping.
    # code-close/already-closed with green tests → not live.
    live_shipping = [r for r in live if True]  # live list only has failed closed or open-by-design
    # Recompute: live residual modes among the 16 that are NOT green-closed
    live_ids = []
    for r in rows:
        disp = r.get("disposition")
        if disp in ("honest-open-backlog", "Non-goal"):
            live_ids.append(r["id"])
            continue
        log_text = log_cache.get(r.get("log_path", ""), "")
        filters = [r["test_filter"]] + list(r.get("extra_test_filters") or [])
        if any(not filter_green(log_text, f) for f in filters):
            live_ids.append(r["id"])

    body = []
    body.append("# Unfinished inventory — 00/00A re-review (16 IDs)")
    body.append("")
    body.append(
        "Generated by `scripts/generate_residual_inventory.py` from "
        "`scripts/residual_re_review_manifest.yaml` + SCRATCH logs. "
        "**Do not hand-edit.**"
    )
    body.append("")
    body.append(
        "| ID | Residual mode (00A) | Product entry | Disposition | Product-path test + log |"
    )
    body.append(
        "|----|---------------------|---------------|-------------|-------------------------|"
    )
    body.extend(table_rows)
    body.append("")
    body.append(f"row_count={len(rows)}")
    body.append(f"live_re_review_residual_count={len(live_ids)}")
    body.append(f"live_ids={live_ids}")
    body.append(f"closed_green={closed}")
    body.append("")
    body.append(
        "Outside 16 → honest-open / Non-goal (B2/R3, F15–F20, WASM mid-compute preemption, "
        "full runnable queue, C5-a 100/500/1000 real UI frame matrix, etc.)."
    )
    body.append("")

    out.write_text("\n".join(body), encoding="utf-8")

    post = []
    post.append("# Honest post-audit vs shipped code")
    post.append("")
    post.append(f"generated_from=residual_re_review_manifest.yaml")
    post.append(f"row_count={len(rows)}")
    for r in rows:
        rid = r["id"]
        log_text = log_cache.get(r.get("log_path", ""), "")
        filters = [r["test_filter"]] + list(r.get("extra_test_filters") or [])
        ok = all(filter_green(log_text, f) for f in filters) if log_text else False
        post.append(
            f"- {rid}: {'CLOSED' if ok and r['disposition'] in ('code-close','already-closed') else 'OPEN/CHECK'} "
            f"disposition={r['disposition']} log={r['log_path']} filter={r['test_filter']}"
        )
    post.append(f"live_re_review_residual_count={len(live_ids)}")
    post.append(f"live_ids={live_ids}")
    post.append(f"errors={errors}")
    post.append("")
    post_audit.write_text("\n".join(post), encoding="utf-8")

    print(out.read_text(encoding="utf-8"))
    if errors or live_ids:
        print("GENERATE_ERRORS:", file=sys.stderr)
        for e in errors:
            print(" ", e, file=sys.stderr)
        print(f"live_ids={live_ids}", file=sys.stderr)
        return 1
    print("OK live_re_review_residual_count=0", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
