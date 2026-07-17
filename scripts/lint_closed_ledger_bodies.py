#!/usr/bin/env python3
"""Lint closed review-ledger sections and their evidence coverage.

For each `### ID` section whose status declares the item closed, fail if:
  - a heading matches `**用户影响**` or `**改善原则**` without `（历史）`
  - a non-history/non-closed-structure line contains open-work present-tense phrases
  - the authoritative closed set, category statuses, and 00A coverage matrix disagree

Exit non-zero and print `ID:line:heading_or_snippet` for every hit.
"""
from __future__ import annotations

import argparse
from collections import Counter
import re
import sys
from pathlib import Path

SECTION_RE = re.compile(r"^###\s+((?:[A-Z]\d+(?:-[a-z])?|[WU]\d+|C\d+))\b")
THIRD_LEVEL_HEADING_RE = re.compile(r"^###\s+")
STATUS_RE = re.compile(r"^\*\*状态(?:更新)?[：:]")
CLOSED_STATUS_RE = re.compile(r"已(?:验证|结构)?关闭|已修复并验证|已优化并验证关闭|已收口")
# Headings that must be historized under closed sections
BAD_HEADING_RE = re.compile(
    r"^\*\*(用户影响|视觉影响|视觉与认知影响|审美问题|审美与体验问题|改善原则|修复原则)\*\*"
)
HISTORIZED_HEADING_RE = re.compile(r"（历史）|\(历史\)")
# Blocks that are allowed to contain residual-looking language
SAFE_BLOCK_RE = re.compile(
    r"已关闭结构|修复前|历史状态|历史）|诚实边界|Non-goal|不得写回|"
    r"验证：|合同：|门禁|状态："
)
# Present-tense "bug still open" phrases outside history blocks
OPEN_PHRASE_RE = re.compile(
    r"(仍未|尚未|仍缺|仍然缺少|未完成|未实现|完全没有|没有任何|零引用|"
    r"不能.*关闭|不提升为.*关闭|用户无法|用户不能|用户会误以为)"
)
ID_RE = re.compile(r"\b[A-Z]\d+(?:-[a-z])?\b")
LEDGER_ROOT = Path("项目检验报告/发布前全量代码审查")
AUTHORITATIVE_LEDGER = LEDGER_ROOT / "00-已实现改善与验证记录.md"
COVERAGE_LEDGER = LEDGER_ROOT / "00A-已修复项结构合规复核.md"
COVERAGE_START = "<!-- closed-ledger-coverage:start -->"
COVERAGE_END = "<!-- closed-ledger-coverage:end -->"


def lint_file(path: Path) -> tuple[list[tuple[str, int, str]], set[str]]:
    lines = path.read_text(encoding="utf-8").splitlines()
    hits: list[tuple[str, int, str]] = []
    current_id: str | None = None
    closed = False
    in_history = False
    closed_ids: set[str] = set()

    for i, line in enumerate(lines, 1):
        m = SECTION_RE.match(line)
        if m:
            current_id = m.group(1)
            closed = False
            in_history = False
            continue
        if THIRD_LEVEL_HEADING_RE.match(line):
            current_id = None
            closed = False
            in_history = False
            continue

        if current_id is None:
            continue

        if STATUS_RE.match(line):
            closed = bool(CLOSED_STATUS_RE.search(line))
            if closed:
                closed_ids.add(current_id)
            in_history = False
            continue

        if not closed:
            continue

        # History / closed-structure block markers
        if re.search(r"修复前|历史状态|用户影响（历史）|改善原则（历史）|视觉影响（历史）", line):
            in_history = True
            continue
        if re.search(r"已关闭结构|诚实边界", line):
            in_history = False
            continue

        # Bad headings under closed sections
        hm = BAD_HEADING_RE.match(line.strip())
        if hm:
            if not HISTORIZED_HEADING_RE.search(line):
                hits.append((current_id, i, f"heading **{hm.group(1)}** not labeled 历史"))
            else:
                in_history = True
            continue

        if in_history:
            continue
        if SAFE_BLOCK_RE.search(line):
            continue
        if not line.strip() or line.strip().startswith(">") or line.strip().startswith("#"):
            continue
        if line.strip().startswith("- ") or line.strip().startswith("*"):
            # bullet under current block
            if OPEN_PHRASE_RE.search(line) and len(line.strip()) > 12:
                hits.append((current_id, i, line.strip()[:120]))
            continue
        if OPEN_PHRASE_RE.search(line) and len(line.strip()) > 12:
            hits.append((current_id, i, line.strip()[:120]))

    return hits, closed_ids


def authoritative_closed_ids() -> set[str]:
    lines = AUTHORITATIVE_LEDGER.read_text(encoding="utf-8").splitlines()
    source_lines = [
        line
        for line in lines
        if line.startswith("- 原始问题链已关闭")
        or line.startswith("- 2026-07-13 复审 16 后缀链")
    ]
    if len(source_lines) != 2:
        raise ValueError("authoritative closed-id bullets are missing or ambiguous")
    ids = set(ID_RE.findall("\n".join(source_lines)))
    ids.discard("B3")
    return ids


def coverage_matrix_ids() -> Counter[str]:
    lines = COVERAGE_LEDGER.read_text(encoding="utf-8").splitlines()
    try:
        start = lines.index(COVERAGE_START)
        end = lines.index(COVERAGE_END, start + 1)
    except ValueError as error:
        raise ValueError("00A closed-ledger coverage markers are missing") from error
    ids: Counter[str] = Counter()
    for line in lines[start + 1 : end]:
        if not line.startswith("|"):
            continue
        first_cell = line.split("|", 2)[1].strip()
        if first_cell in {"", "关闭 ID（精确集合）"} or set(first_cell) <= {"-"}:
            continue
        ids.update(ID_RE.findall(first_cell))
    return ids


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "paths",
        nargs="*",
        default=[str(AUTHORITATIVE_LEDGER)]
        + [
            str(path)
            for path in sorted(LEDGER_ROOT.glob("[01][0-9]-*.md"))
            if path.name[:2] in {f"{index:02d}" for index in range(1, 12)}
        ],
    )
    parser.add_argument("--write-evidence", default="", help="Write open_partial_hits evidence file")
    args = parser.parse_args()

    all_hits: list[tuple[str, str, int, str]] = []
    category_closed_ids: set[str] = set()
    for p in args.paths:
        path = Path(p)
        if not path.is_file():
            print(f"MISSING {path}", file=sys.stderr)
            return 2
        hits, closed_ids = lint_file(path)
        category_closed_ids.update(closed_ids)
        for sid, line_no, msg in hits:
            all_hits.append((str(path), sid, line_no, msg))
            print(f"{sid}:{line_no}:{msg}  ({path})")

    try:
        expected_ids = authoritative_closed_ids()
        coverage_counts = coverage_matrix_ids()
    except (OSError, ValueError) as error:
        print(f"COVERAGE_ERROR {error}", file=sys.stderr)
        return 2

    coverage_ids = set(coverage_counts)
    coverage_missing = sorted(expected_ids - coverage_ids)
    coverage_unexpected = sorted(coverage_ids - expected_ids)
    coverage_duplicate = sorted(sid for sid, count in coverage_counts.items() if count != 1)
    category_unledgered = sorted(category_closed_ids - expected_ids)

    if args.write_evidence:
        out = Path(args.write_evidence)
        out.parent.mkdir(parents=True, exist_ok=True)
        with out.open("w", encoding="utf-8") as f:
            f.write(f"open_partial_hits={len(all_hits)}\n")
            f.write("# closed-section unhistorized residual bodies (lint_closed_ledger_bodies.py)\n")
            for path, sid, line_no, msg in all_hits:
                f.write(f"OPEN {path}:{line_no}: [{sid}] {msg}\n")
            f.write(f"coverage_missing={','.join(coverage_missing)}\n")
            f.write(f"coverage_unexpected={','.join(coverage_unexpected)}\n")
            f.write(f"coverage_duplicate={','.join(coverage_duplicate)}\n")
            f.write(f"category_unledgered={','.join(category_unledgered)}\n")

    print(f"open_partial_hits={len(all_hits)}")
    print(f"coverage_expected={len(expected_ids)} coverage_mapped={len(coverage_ids)}")
    print(f"coverage_missing={','.join(coverage_missing)}")
    print(f"coverage_unexpected={','.join(coverage_unexpected)}")
    print(f"coverage_duplicate={','.join(coverage_duplicate)}")
    print(f"category_unledgered={','.join(category_unledgered)}")
    return 1 if any(
        [
            all_hits,
            coverage_missing,
            coverage_unexpected,
            coverage_duplicate,
            category_unledgered,
        ]
    ) else 0


if __name__ == "__main__":
    sys.exit(main())
