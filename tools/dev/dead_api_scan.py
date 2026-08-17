#!/usr/bin/env python3
"""U116：扫描 core 的公开 API，找出「生产零引用」的项。

方法与审查报告一致：枚举 `core/src` 下的 `pub` 项，再分别统计它在
**生产代码**（src，剔除 `#[cfg(test)]` 块）与**测试代码**（src 内测试块 +
core/tests）里的引用数。

关键是把结果分成两桶：
  - dead      —— 生产 0 引用、测试也 0 引用：纯死代码
  - test-only —— 生产 0 引用、但测试有引用：**危险的那类**
                （实现完整 + 有覆盖 + 生产零调用者，看起来健康。U108/U114/U117 全是这形状）

桌面端不直接调 Rust（走 IPC/JSON），故不计入引用路径。

⚠️ **本脚本的输出是线索，不是施工清单。删任何一项之前必须两步复核**：
  1. 单独 `grep -rw <name> src/ tests/`（grep 不受本脚本的语料处理偏差影响）
  2. 删完跑 `cargo check --all-targets`，让编译器当最终裁判

原因是这类扫描的**误报方向有害**：漏报只是少清一点，误报会让人删掉在用的东西。
实际踩过的两个坑（都已修，留在这里防止重犯）：
  - **原始字符串**：`r#"..."#` 若不先按相同数量的 `#` 配对剔除，普通字符串正则会从
    里面的第一个 `"` 一路吞到很远的另一个 `"`，把中间真实代码整段抹成空白。
    实测在 `frontend/service.rs` 上抹掉 50.3% 的非空白字符（修好后 14.0%），
    导致有 4 处真实引用的 `WorksTreeNodeKind` 被误判成死代码。
  - **注释与再导出**：反过来是**漏报**方向——只在文档注释里被提到、或只被 `mod.rs`
    再导出的项，若不剔除会永远显示成「活的」，扫描会自我掩盖掉近一半问题。

还有一类本脚本**判不出来**、只能人工看的：刻意的双入口。例如 `initialize_project`
（供测试/迁移/非桌面入口）与生产用的 `publish_initialized_project`（原子发布），
`save_knowledge` 与带幂等回执的 `save_knowledge_with_operation`——生产只用后者是正确的。
这类项应保留并在原处写明理由，否则下一轮扫描又会把它当死代码。
"""

import re
import sys
from collections import defaultdict
from pathlib import Path

CORE = Path(__file__).resolve().parent.parent
SRC = CORE / "src"
TESTS = CORE / "tests"


def split_test_blocks(text):
    """返回 (生产代码, 测试代码)。按 `#[cfg(test)]` 后的花括号配对切分。"""
    prod, test = [], []
    lines = text.splitlines(keepends=True)
    i = 0
    while i < len(lines):
        if "#[cfg(test)]" in lines[i]:
            depth, started = 0, False
            while i < len(lines):
                test.append(lines[i])
                depth += lines[i].count("{") - lines[i].count("}")
                if "{" in lines[i]:
                    started = True
                i += 1
                if started and depth <= 0:
                    break
        else:
            prod.append(lines[i])
            i += 1
    return "".join(prod), "".join(test)


def strip_noise(text):
    """把注释与字符串字面量替换成等长空白，避免把「仅在注释里被提到」误判成被引用。

    **保持长度与换行**：后面要按行号排除「声明处自身」，若这里压缩了字符，
    行号会整体错位，排除就会打偏。所以只做同长替换，不做删除。

    这一步不做不行：`rag/` 里大量 API 只在文档注释里出现，不剔除就会被算成活的，
    整个扫描会系统性漏报——正是 U116 这类问题最容易自我掩盖的地方。
    """

    def blank(match):
        return re.sub(r"[^\n]", " ", match.group(0))

    text = re.sub(r"/\*.*?\*/", blank, text, flags=re.S)
    text = re.sub(r"//[^\n]*", blank, text)
    # 原始字符串 r"..." / r#"..."# 必须先处理，且要按**相同数量的 #** 收尾。
    # 否则普通字符串正则会从 r#" 里的第一个 " 一路吞到很远的另一个 "，
    # 把中间的真实代码整段抹成空白——实测在 frontend/service.rs 上抹掉了 50% 的
    # 非空白字符，导致 WorksTreeNodeKind 这类**有真实引用**的类型被误判成死代码。
    # 误报比漏报危险得多：漏报只是少清一点，误报会让人删掉在用的东西。
    #
    # `(?<![A-Za-z0-9_])` 不可省（2026-08-08 修）：没有它，**以 r 结尾的普通字符串**
    # 会被误当成原始字符串的开头。实测 `"reranker",` 里的 `r",` 让正则一路吞到
    # 2414 字符之外的下一个引号，把 `retrieval/project.rs` 中间整段代码抹白，
    # 于是 `index_configuration_revision`、`without_vector` 这些**有真实生产调用者**
    # 的函数被报成纯死代码。与上面那条是同一类错误的另一个实例：
    # 判断原始字符串的起点时，必须确认 `r` 不是某个标识符的尾字符。
    text = re.sub(r'(?<![A-Za-z0-9_])r(#*)"(?:(?!"\1).)*"\1', blank, text, flags=re.S)
    text = re.sub(r'"(?:[^"\\\n]|\\.)*"', blank, text)
    return text


def strip_reexports(text):
    """把 `pub use` / `use` 行替换成等长空白（同样保持行号）。

    再导出与导入都不是「使用」——`pub use foo::Bar` 只是把 Bar 摆到另一个路径上，
    它自己不构成调用者。不剔除的话，凡是被 mod.rs 再导出的项都永远显示为「活的」。
    """
    out = []
    for line in text.splitlines(keepends=True):
        if re.match(r"\s*(pub\s+)?use\s", line):
            out.append(re.sub(r"[^\n]", " ", line))
        else:
            out.append(line)
    return "".join(out)


# 只收「有名字、可被外部调用」的公开项。`pub use`/`pub mod` 是再导出，不算 API 本体。
DECL = re.compile(
    r"^\s*pub(?:\s*\([^)]*\))?\s+"
    r"(?:async\s+)?(?:unsafe\s+)?(?:extern\s+\"[^\"]+\"\s+)?"
    r"(fn|struct|enum|trait|type|const|static)\s+"
    r"([A-Za-z_][A-Za-z0-9_]*)",
    re.M,
)

# 派生/属性宏会生成大量对字段名的引用，单纯计数会把 serde 字段全判成"活的"。
# 这里只统计标识符出现，故对字段不做判定——扫描范围限定在上面 DECL 的 7 类。


# ---------------------------------------------------------------------------
# U116 判定结论（2026-07-31）：以下项**生产零引用是正常的**，不要当死代码删。
#
# 每次扫描都会重新把它们列出来，逐轮人工复判既浪费时间又容易判错，
# 故在此固化结论。新增条目请一并写明归类理由。
# ---------------------------------------------------------------------------
EXPECTED_TEST_ONLY = {
    # 一、显式测试钩子：名字里自带 for_test / fail_after，用于注入故障时序。
    "build_step_instruction_for_test",
    "commit_general_settings_files_with_fail_after",
    "execute_project_search_node_for_test_fixture",
    "save_chapter_knowledge_with_operation_fail_after",
    # 二、简化重载：生产走的是更完整的同族变体，简化版留给测试与非项目场景。
    #     删掉不会让产品少任何能力，但会让测试被迫拼装一堆无关参数。
    "create_archive_point",          # 生产用 _with_policy
    "execute_llm_node",              # 生产用 _with_search_tools / _with_project_search
    "initialize_project",            # 生产用 publish_initialized_project（原子发布）
    "insert_lines",                  # 生产用 insert_lines_to_patch
    "mark_change_realized",          # 生产用 _on_state（在已持锁的事务里批量应用）
    "register_executor_adapters_for_project",  # 生产用 _with_search
    "replace_chapter_summary_entities",        # 同上，_on_state
    "replace_lines",                 # 生产用 replace_lines_to_patch
    "resume",                        # 生产用 resume_from_node / resume_workflow
    "save_chapter_knowledge_with_operation",   # 生产用 _locked
    "save_knowledge",                # 生产用 _with_operation（带幂等回执）
    "search_project_documents",      # 生产用 _with_cancellation
    "should_skip_human_confirmation",  # 生产直接调 ApprovalPolicy::should_auto_approve
    # 三、builder 方法：`X::new().with_y()` 形式，链式配置本身就不产生独立调用点。
    "with_checkpoint",
    "with_global_root",
    "with_input_source",
    "with_master_password",
    "with_patch_commit",
    "with_prompt_template",
    "with_variables",
    "with_worker_executable",
}


def collect_declarations():
    """返回 {name: [(相对路径, 行号, 种类)]}。"""
    found = defaultdict(list)
    for path in sorted(SRC.rglob("*.rs")):
        prod, _ = split_test_blocks(path.read_text(encoding="utf-8"))
        rel = path.relative_to(CORE)
        for match in DECL.finditer(prod):
            kind, name = match.group(1), match.group(2)
            line = prod[: match.start()].count("\n") + 1
            found[name].append((str(rel), line, kind))
    return found


def build_corpora():
    """生产语料（src 去测试块/注释/再导出）与测试语料（src 测试块 + tests/）。

    生产侧必须同时剔除注释与 `use` 行，否则「只在文档注释里被提到」和
    「只被 mod.rs 再导出」都会被误算成有调用者——那正是这类扫描最容易自我掩盖的地方。
    """
    prod_parts, test_parts = [], []
    for path in sorted(SRC.rglob("*.rs")):
        prod, test = split_test_blocks(path.read_text(encoding="utf-8"))
        prod_parts.append((path, strip_reexports(strip_noise(prod))))
        test_parts.append(strip_noise(test))
    for path in sorted(TESTS.rglob("*.rs")):
        test_parts.append(strip_noise(path.read_text(encoding="utf-8")))
    return prod_parts, "\n".join(test_parts)


def main():
    declarations = collect_declarations()
    prod_parts, test_text = build_corpora()

    # 逐个名字统计引用。声明处自身不算引用，故按文件比对行号排除。
    prod_counts = defaultdict(int)
    for name in declarations:
        word = re.compile(r"\b" + re.escape(name) + r"\b")
        decl_sites = {(p, l) for p, l, _ in declarations[name]}
        for path, text in prod_parts:
            rel = str(path.relative_to(CORE))
            for match in word.finditer(text):
                line = text[: match.start()].count("\n") + 1
                if (rel, line) in decl_sites:
                    continue  # 声明自身
                prod_counts[name] += 1

    test_counts = {
        name: len(re.findall(r"\b" + re.escape(name) + r"\b", test_text))
        for name in declarations
    }

    dead, test_only, expected = [], [], []
    for name, sites in sorted(declarations.items()):
        if prod_counts[name] > 0:
            continue
        if name in EXPECTED_TEST_ONLY:
            expected.append((name, sites, test_counts[name]))
            continue
        bucket = test_only if test_counts[name] > 0 else dead
        bucket.append((name, sites, test_counts[name]))

    total = len(declarations)
    print(f"公开项总数：{total}")
    print(f"生产零引用：{len(dead) + len(test_only)}")
    print(f"  ├─ 纯死代码（测试也不用）：{len(dead)}")
    print(f"  └─ 仅测试引用（危险类）：{len(test_only)}")
    print(f"（另有 {len(expected)} 项属已判定的测试钩子/简化重载/builder，见 EXPECTED_TEST_ONLY，不再列出）")

    for title, rows in (("纯死代码", dead), ("仅测试引用", test_only)):
        print(f"\n=== {title} ===")
        by_module = defaultdict(list)
        for name, sites, tc in rows:
            module = sites[0][0].split("/")[1] if "/" in sites[0][0] else sites[0][0]
            by_module[module].append((name, sites, tc))
        for module in sorted(by_module, key=lambda m: -len(by_module[m])):
            entries = by_module[module]
            print(f"\n[{module}] {len(entries)} 项")
            for name, sites, tc in entries[:40]:
                where = f"{sites[0][0]}:{sites[0][1]}"
                kind = sites[0][2]
                suffix = f"  (测试引用 {tc})" if tc else ""
                print(f"  {kind:7} {name:45} {where}{suffix}")
            if len(entries) > 40:
                print(f"  … 另有 {len(entries) - 40} 项")
    return 0

if __name__ == "__main__":
    sys.exit(main())
