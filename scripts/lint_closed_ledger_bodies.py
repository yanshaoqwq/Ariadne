#!/usr/bin/env python3
"""Lint closed review-ledger sections for unhistorized residual bodies.

For each `### ID` section whose `**状态：**` contains `已验证关闭`, fail if:
  - a heading matches `**用户影响**` or `**改善原则**` without `（历史）`
  - a non-history/non-closed-structure line contains open-work present-tense phrases

Exit non-zero and print `ID:line:heading_or_snippet` for every hit.
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

SECTION_RE = re.compile(r"^###\s+((?:[A-Z]\d+(?:-[a-z])?|[WU]\d+|C\d+))\b")
STATUS_RE = re.compile(r"^\*\*状态：")
CLOSED_STATUS_RE = re.compile(r"已验证关闭")
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
    r"(未见|仍未|尚未|完全没有|零引用|读屏可能|建立.*测试|可能只|"
    r"定义 compact|增加.*验收|必须|不得继续|"
    r"没有 MaxHeight|没有.*绑定|硬编码为|被硬编码|"
    r"始终使用|只切换|不读取|不使用|不会重算|"
    r"用户无法|用户不能|用户会误以为)"
)


def lint_file(path: Path) -> list[tuple[str, int, str]]:
    lines = path.read_text(encoding="utf-8").splitlines()
    hits: list[tuple[str, int, str]] = []
    current_id: str | None = None
    closed = False
    in_history = False

    for i, line in enumerate(lines, 1):
        m = SECTION_RE.match(line)
        if m:
            current_id = m.group(1)
            closed = False
            in_history = False
            continue

        if current_id is None:
            continue

        if STATUS_RE.match(line):
            closed = bool(CLOSED_STATUS_RE.search(line))
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

    return hits


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "paths",
        nargs="*",
        default=[
            "项目检验报告/发布前全量代码审查/08-工作区画布交互与视觉.md",
            "项目检验报告/发布前全量代码审查/11-桌面通用体验本地化与无障碍.md",
        ],
    )
    parser.add_argument("--write-evidence", default="", help="Write open_partial_hits evidence file")
    args = parser.parse_args()

    all_hits: list[tuple[str, str, int, str]] = []
    for p in args.paths:
        path = Path(p)
        if not path.is_file():
            print(f"MISSING {path}", file=sys.stderr)
            return 2
        for sid, line_no, msg in lint_file(path):
            all_hits.append((str(path), sid, line_no, msg))
            print(f"{sid}:{line_no}:{msg}  ({path})")

    if args.write_evidence:
        out = Path(args.write_evidence)
        out.parent.mkdir(parents=True, exist_ok=True)
        with out.open("w", encoding="utf-8") as f:
            f.write(f"open_partial_hits={len(all_hits)}\n")
            f.write("# closed-section unhistorized residual bodies (lint_closed_ledger_bodies.py)\n")
            for path, sid, line_no, msg in all_hits:
                f.write(f"OPEN {path}:{line_no}: [{sid}] {msg}\n")

    print(f"open_partial_hits={len(all_hits)}")
    return 1 if all_hits else 0


if __name__ == "__main__":
    sys.exit(main())
