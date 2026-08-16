# Repository Guidelines

Ariadne（Literature Agent）是面向百万字长篇小说写作的桌面 AI agent 编排工具：
多 agent 编排、RAG 上下文检索、Git 版本管理、审批工作流、成本追踪。

本文件是对所有贡献者的仓库规范。流程与许可要求见 [CONTRIBUTING.md](CONTRIBUTING.md)，
构建与发布细节见 [README.md](README.md) 与 `.github/workflows/`。

## Project Structure & Module Organization

Rust + Avalonia 双端工作区，通过本地 JSON-line stdio IPC 通信：
桌面端调 `core/src/commands.rs` 暴露的 command service。

- **`core/`** — Rust 后端 crate。`core/src/` 按域分模块：
  `contracts`（核心契约层：ports / workflow / errors / registry / permissions）、
  `config`、`costs`、`documents`、`git`、`knowledge`、`llm`、`providers`、`rag`、
  `retrieval`、`skills`、`workflow`、`frontend`、`commands`、`ipc`。
  契约与集成测试在 `core/tests/`，运行期资源（文案、提示词）在 `core/resources/`，
  辅助脚本在 `core/scripts/`。
- **`desktop/`** — Avalonia UI + FluentTheme + XAML/C# + MVVM（net10.0）。
  `Ariadne.Desktop/Views/**` 窗口与页面、`ViewModels/**` 状态与命令、
  `Resources/Styles/AriadneTheme.axaml` 视觉地基（主题令牌 + 控件样式 + 矢量图标）、
  `Backend/**` IPC 客户端边界、`Localization/` 文案服务；
  测试在 `Ariadne.Desktop.Tests/`。
- **`packaging/`**、**`scripts/`**、**`tools/`** — 打包、发布门禁校验、许可清单工具。

模块依赖**自下而上**：契约层稳定，上层模块添加实现而不破坏既有契约。

## Build, Test, and Development Commands

Rust 版本由 `rust-toolchain.toml` 固定（rustup 会自动切换）；.NET SDK 版本见 `global.json`。

```bash
cargo build --workspace --bins
cargo test --workspace --all-targets --all-features --locked
cargo test --test rag_contracts          # 单个测试文件
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

dotnet restore desktop/Ariadne.slnx
dotnet build desktop/Ariadne.slnx
dotnet test desktop/Ariadne.slnx
dotnet run --project desktop/Ariadne.Desktop
desktop/run-ui.sh [run|shot|build]       # 启动器：开窗 / 截图 / 构建
```

CI 门禁（`.github/workflows/ci.yml`）跑的就是上面这套加 `cargo deny`
与 `scripts/verify-release-engineering.py`。**clippy 是 `-D warnings`**，
新警告会直接挂 CI。


## Coding Style & Naming Conventions

**Rust**：2021 idioms + `rustfmt` 默认配置。模块名小写下划线，公开类型 `PascalCase`，
函数/字段/模块 `snake_case`。遵循既有域边界，不要过早添加跨域 helper。

**C#**：MVVM。`RelayCommand` / `ViewModelBase` / `SetProperty`。
ViewModel 不引用 View 类型；View 需要回调时由 View 注入 `Action`/`Func` 到 VM
（既有模式见 `WorksPageViewModel` 的 `RequestEditorCopy` 等属性）。

**注释用中文，且要解释「为什么」**——尤其是那些「看起来可以更简单」的地方，
也注意不要顺手改回缺陷版本：

```rust
/// 生成稳定的注册项 id，便于测试和后续审计。
fn next_change_id(state: &mut WritingKnowledgeState, function: RegisterFunction) -> String {
    loop {
        // 用独立自增序列而非 changes.len()：len() 在并发登记下会给出重复 id。
        state.next_change_sequence = state.next_change_sequence.saturating_add(1);
        let candidate = format!("register-{prefix}-{}", state.next_change_sequence);
        // 序列可能与历史落库 id 撞上（例如加载旧快照后），碰撞则继续取下一个。
        if !state.changes.contains_key(&candidate) {
            return candidate;
        }
    }
}
```

## 产品硬约束

### 所有用户可见文案走 key
先在 `core/resources/display_name.json`（zh）建 key 再引用。
缺 key 时 `DisplayNameService.Text` 返回 `[key]` 便于自查；
en/ja 缺失会回落到 zh，不阻断功能但用户会看到中文。

> `WritingNodeDefinition::validate`（`core/src/rag/models.rs`）对同一工具名用**两套 key 规则**：
> display_names 查 `tool.{name}`（连字符），prompts 查下划线版。加新工具时两边都要补，
> **漏 display_name 会让节点定义校验失败**而非只是少一行文案。

### 颜色只能来自主题
**不要用魔法数字写颜色。** 全部走 `{DynamicResource Ariadne.*}` 或主题令牌加值。
硬编码颜色在多套主题下必然有若干套是错的——实践中还会「抄错」
（把暗色值抄进亮色路径，任何单一主题下都不完全正确）。

排版尺度同理：版心宽度、页面边距用 `Ariadne.Reading.*` / `Ariadne.Page.*`，
不要各页写死。魔数会让「改字号时版心不跟随」这类漂移无法察觉。

### 图标用矢量 Geometry
`ViewModels/IconGeometries.cs` + 主题 `Ariadne.Icon.*`。
**禁用图标字体**——Segoe Fluent Icons 在 Linux 上缺失会渲染成豆腐块。
中文字体挂 fallback（Inter 不含中文 → WenQuanYi Zen Hei），正文阅读可用衬线。

### 不可编辑的内容不用 TextBox 承载
`TextBox IsReadOnly="True"` 与可编辑输入框像素级同款、照样亮边抢焦点、照样占 Tab 位——
是最坏的一类假控件。用 `SelectableTextBlock` 或 `ItemsControl`。
衍生语法规则：**可编辑→有槽，只读→无槽**，让「能不能改」看一眼即知。

### 不依赖 WebView
桌面原生。无 WebView / Vite / CSS / React / localhost。

### 错误必须配文字
不能只把按钮灰掉。禁用的运行按钮旁必须写明原因。

### 用户已知的值不要让用户手打
后端多处是精确等值匹配，手敲差一个字符就是空结果且无提示。
产品持有候选值时（章节 id、节点别名、引脚名）应给可搜索下拉或点击回填。

## 关键陷阱

### Rust
**UTF-8 安全**：正文是中文，切分文本一律用字符感知迭代
（`split_inclusive('\n')` / `str::lines()`），**不要按字节找 `\n`**——
切在多字节字符中间会让 `TextRange` 落在非字符边界，后续 patch 直接 panic 或写出乱码。

**并发安全**：所有共享状态修改必须在**单个**锁作用域内完成；
**ID 生成不得依赖 `len()`**（并发下会给出重复 id）。

**引用式数据流**：大文本**永不**内联传递，用
`DocumentRef` / `ChunkRef` / `ArtifactRef` / `SourceSpan`——
百万字小说不能在工作流边上拷贝。

**错误处理**：统一 `CoreResult<T>` = `Result<T, CoreError>`；
**生产路径禁止 `unwrap()` / `expect()`**，用 `?` 或显式 `match` / `if let`。

**`#[serde(default)]` 会把「漏抄字段」变成静默数据丢失**：读回时缺键不报错、
静默变成空默认值——写进去了、读出来没了，无任何错误可查。
任何手抄的字段镜像（持久化投影结构、DTO、测试里复制的哈希公式）都是这类缺陷的温床，
优先收敛成单一来源。

### Avalonia
**样式选择器必须带元素类型前缀**：裸 `.my-class` 会在编译期报 `AVLN2200`
（`Setter` 要能确定目标类型）。两处宿主类型不同就分开写两组。

**同优先级按文档顺序、后者胜**，没有 CSS 那种选择器特异性权重。
`TextBox.search-input:focus` 声明在通用 `TextBox:focus` **之前**就会被完全盖掉——
`.search-input` 比 `TextBox` 具体得多，但这不给它任何优先权。
⇒ 专用样式必须声明在通用样式**之后**。

**内联属性压不掉模板层样式**：`BorderThickness="0"` 设在 `TextBox` 自身，
而主题焦点样式设在 `/template/ Border#PART_BorderElement`——
两者作用在不同对象上，压根不冲突，所以谁也没盖掉谁。

**`MaxHeight` 对 `TextBlock` 是「裁掉多余」，对 `ScrollViewer` 才是「限高可滚」**——
写法一样、行为相反。且裁切发生在测量阶段，`TextBlock` 上报的 `DesiredSize` 已被钉死，
外层再包滚动也救不回来。

**`AvaloniaEdit` 不吃 `TextBlock.LineHeight`**——它自己排版，
行高来自字体度量 × `TextEditorOptions.LineHeightFactor`（默认 1.16）。

**裸 `ItemsControl` 不自带滚动**（`ItemsPresenter` 不是 `IScrollable`，
内部 `VirtualizingStackPanel` 也不实现 `ILogicalScrollable`）⇒ 长内容必须外包 `ScrollViewer`。

**虚拟化会回收远处的容器**：Ctrl+A 之类的批量操作只能刷到已实体化的项，
滚回来的是新实例 ⇒ 需要 `ContainerPrepared` 补状态。

**视图切换要保位置**：捕获必须在 `IsVisible` 翻转**之前**（翻转后旧视图不可测，
未测量的控件返回 0 = 静默丢位置）；恢复必须等一轮布局（`DispatcherPriority.Loaded`）。

**`ItemTemplate` 内用 `Classes.xxx` 绑定优于 Style Selector 走 DataContext。**

## Testing Guidelines

Rust 测试在 `core/tests/`，命名 `*_contracts.rs`，用 `tempfile` 建测试库，
默认 mock 外部服务（Qdrant、LLM API）。
C# 测试在 `desktop/Ariadne.Desktop.Tests/`，用 `DispatchProxy` mock `IAriadneBackendClient`。

新增或改变公共行为必须配对应测试（`CONTRIBUTING.md` 的要求）。
**不要在文档或注释里写死测试条数**——它每天都在变，写死只会变成又一处过期信息。

### C# 测试的三个坑

**`DispatchProxy` 的宿主类不能 `sealed`**——它要在运行时派生该类型，
`sealed` 会得到 `ArgumentException: The base type cannot be sealed`。
用 `private class` 而不是 `private sealed class`。

**`Window` 不能塞进另一个 `Window` 的 `Content`**（`already has a visual parent`）。
测顶层窗口时直接 `new MainWindow { ... }.Show()`。

**源码文本断言要查「调用」而非「字符串出现」**：
`Assert.DoesNotContain("OldMethod", src)` 会被注释里提到旧名的历史记录绊倒，
应写 `Assert.DoesNotContain("Helpers.OldMethod", src)`。

### 「测试全绿」≠「功能可用」

这个仓库出现过多次「实现完整 + 有测试覆盖 + 生产零调用者」。
**只有断言真实出站请求 / 真实落盘产物 / 真实运行态的用例才拦得住这种。**

| ❌ 弱判据 | ✅ 强判据 |
|---|---|
| 「命令能否执行」 | 「点击后**发出的请求**真的带上那个 id」 |
| 「能否构造出一个确认项」 | 「真实入口调用**之后**确认项是否入库且为 Pending」 |
| 「列表非空」 | 「点击后**绑定属性**等于那个值」 |
| 「常量等于多少」 | 「**推导出的字数**在合理区间」（改字号没跟着改版心时同样会红） |
| 「事件表里有记录」 | 「消息**进入对话载荷**」（AI 不读事件表，它读对话） |

**判据选错一层，测试就成了装饰。** 一个真实例子：持久化投影结构是手抄的字段镜像、
漏抄了四个字段；而原有的 round-trip 测试走 `serde_json::to_string(&state)`——
测的是数据结构**自己的** `Serialize`，**根本碰不到投影结构**，
所以全绿而真实持久化是断的。
⇒ **涉及持久化的测试必须经过真实 store 存取一轮**。

### 变异测试

**写完修复把修复摘掉，确认新用例真的失败再放行。**

这条抓出过多个空测：
- 内外层循环共用同一信号，摘掉修复仍全绿
- 只比较不同格式（其扩展名本来就不同），漏掉同格式撞名——
  顺带暴露了实现里秒级时间戳会撞名的真实漏洞
- 只钉「A 在 B 之前」的相对顺序，而真正的性质是「A 在状态翻转之前」

**变异点要落在决定用户可见结果的那一环**，不是随便改个常量。
若变异后测试仍绿，先怀疑三件事：(a) 构建缓存；(b) 断言选错了对象；
(c) 那段代码本来就没有行为效果（此时该删，并在注释里记录）。

### mock 的两面

**mock 会掩盖一整类缺陷**：IPC 的 BOM 毒死连接那条只在真实进程管道上才复现——
**任何跨进程边界至少要有一个真实进程的测试**。

**mock 也要守生产契约**：让本该非 null 的后端方法返回 `null` 会导致 NRE。
生产代码假设 IPC 要么给值要么抛是合理的，是 mock 违约了。

### 测试隔离

起真实后端 sidecar 的桌面测试**必须注入独立的 `ARIADNE_APP_STATE_ROOT`**。
否则会写进用户真实应用状态目录，测试残留会把用户自己的项目挤掉。`ARIADNE_APP_STATE_REQUIRE_ISOLATION`
哨兵会在违反时 panic。

## 死代码判定

扫描器在 `core/scripts/dead_api_scan.py`。

⚠️ **它会误报**（已自伤两次，同一类正则错误：不剔除 `r#"..."#`；
把「以 r 结尾的普通字符串」误当原始字符串开头）。两次都把**有真实调用者**的函数
报成死代码。**误报比漏报危险得多**——漏报只是少清一点，误报会让人删掉在用的东西。

⇒ **删前先 `grep -rw` 复核，删后让 `cargo check --all-targets` 当裁判。**

**判定标准**：所有模块落地之后仍没有消费者的契约，就是没用的契约，不是「待实现」。

**四类要分清**：
1. **真缺陷** = 接线缺失 ⇒ **优先接线**
2. **被取代的设计** ⇒ 删
3. **从未落地的设计** ⇒ 删
4. **无害的对称 API** ⇒ 保留 + 注释说明为何不接线

**有些死代码是陷阱，接上去就是错的**——例如「检查端口可用」的 bind-then-drop 竞态、
绕过并发防护的下载路径。删掉并注明原因。

**不要造测试专用的公开 API**——那正是扫描器会标记的东西。
用现成的构造参数注入。

## 提示词是唯一的质量杠杆

本产品全部输出由 LLM 产生。**代码缺陷让功能不可用，提示词缺陷让功能可用但产出平庸**——
后者无报错、无测试失败，只是小说不好看。

改 `core/resources/prompt_list.json` 时：
- **角色要给水准锚点与可推出行为的身份**：「你是一位极高水平的小说家」
  而非「你是 Writer 节点」（「节点」是画布上的方块、是实现概念，既不给水准也不给身份）
- **判据不是「是不是中文」，而是「模型能否从这个词直接推出行为方式」**——
  自造岗位名（「审慎者」「意见者」）同样不可用，因为现实创作行业里不存在
- **不要把内部 API 名嵌进句子**——`必须显式 register` 这类**整句删掉**，
  不要改写成中文留着。工具能力与权限边界**已经在给 LLM 的工具定义里体现**
  （模型看得到自己有哪些工具、没有哪些），提示词里再讲一遍是重复，
  等于拿提示词预算换零收益
- **占位符改名必须同步渲染器**（`core/src/rag/prompt_template.rs` 的 `resolve_variable`），
  且**旧名要保留为兼容别名**——存量工作流的节点配置存的是渲染前的模板字符串，
  只认新名会让所有已保存的工作流在下次运行时 fail-loud

⚠️ `display_name.json` 的 UI 标签**不在范围内**——那些是画布节点名、
给用户看的短标签，自造岗位名在那里合理。两者不要混改。

## Commit & Pull Request Guidelines

短祈使句主题，`feat:` / `fix:` / `refactor:` / `i18n:` 前缀。一个 PR 一件可独立审查的逻辑变更。

**提交正文要写清「原缺陷是什么」「为什么这样改」「变异测试摘掉哪处后哪条用例转红」**——
本仓库的提交历史是设计决策的主要留存处。

PR 说明行为变更、列出跑过的测试、提到影响的模块。许可与 CLA 要求见
[CONTRIBUTING.md](CONTRIBUTING.md)。

⚠️ **`cargo fmt` 可能重排与你改动无关的历史文件**。提交前用 `git diff --name-only`
确认，只提交自己碰过的文件。

⚠️ **不要用 `git checkout` 清理未提交的工作**。
`git status --short | awk '{print $2}' | xargs git checkout --` 这类命令在
路径基准与过滤条件不匹配时会**丢掉全部未提交工作**。
做变异测试时用 `cp <file> <file>.bak` 备份、用 `cp` 还原；
需要精确差集时每改完一个文件就 `git add`，用 `git diff --cached --name-only` 取集合。

多人同时改动时，`core/resources/display_name.json` 会被多方加 key——
**只增不删、只碰自己要用的 key，不要重写整份文件**。

## Security & Configuration Tips

不要提交密钥、本地凭据、个人数据、生成缓存、构建产物，或无权再许可的第三方材料。
第三方代码/图片/字体/模板/模型输出/数据的来源与许可要在 PR 中列明。

预算门禁分布在多层（`core/src/costs/budget.rs`、`core/src/llm/service.rs`、
`core/src/commands.rs`）。注意工作流节点走 `core/src/workflow/integration.rs` 的
`executor.complete_llm`，**绕过 `LlmService`**，由 `commands.rs` 的
`ensure_workflow_daily_budget_allows_call` 兜住。

写工具的作用域校验（大纲工具只能改 `planning/global.md` 等）一律先过
`resolve_line_patch_target`。**新增任何写工具都必须走这个收口**，
否则等于绕过全部越权防护。

### 几个刻意的不一致，不要「统一」

- 日预算 `budget_usd` 的 `0` = 不设上限；Auto Mode `preauthorized_budget_usd` 的
  `Some(0.0)` = **显式零额度**，「不限制」只由 `None` 表达。
  把它规整成 `None` 会静默解除用户刻意设的零额度，属安全性倒退。
- 知识库冲突判定**只比 `fact.value`，刻意不含 `source_version`**——
  把来源版本纳入条件会让每次正常复述都变成冲突、瞬间堵死审批队列。
  但覆盖前**必须** `merge_sources_from` 保留全部出处。
- `runtime_autosave_ms` **刻意未接线**：状态跃迁现在每次同步落盘，
  改成按毫秒节流会在崩溃时丢窗口内的跃迁——拿可恢复性换用户没要求的性能优化。
  它要么从设置页移除、要么标注「暂未启用」，属产品决策，别当成漏接的线顺手补上。
