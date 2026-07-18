# Ariadne 发布打包

发布矩阵由 `release-matrix.json` 固定：Linux x64/arm64、Windows x64、macOS x64/arm64。每个目标必须在对应原生 runner 上构建和执行烟雾测试，不接受在单一开发机上只复制 Debug 输出。

`python3 scripts/verify-release-engineering.py` 校验固定工具链、质量门禁、tag 发布拒绝、原生 RID 矩阵、签名输入、法律清单和 `workspace.package.version` 的跨 Rust/.NET/安装器传播。`release-legal.json` 固定许可证表达式、Required Notice、许可文本哈希、CLA 接受语句和法律复核状态；CI、tag gate 与打包 job 均先运行该合同。`scripts/check-release-readiness.py --static-only` 只消费必须人工维护的外部批准清单，当前仅有 `LEGAL_REVIEW`；最终 evidence gate 再独立校验性能、WCAG 和五 RID Qdrant 运行时证据，任一缺失都会拒绝发布。

## CI 执行模式

- Pull Request 与 `main` push 只运行 60 分钟上限的 `quality`：发布工程合同、Rust fmt/Clippy/全量测试、cargo-deny、Desktop Release 测试和第三方声明一致性。相同分支的新提交会取消旧运行，避免排队任务持续占用 runner。
- 五 RID 原生安装、Qdrant E2E、300 秒调度 soak、UI/WCAG 证据只在手动触发 `CI` 且设置 `full_release_matrix=true` 时运行。原生包、性能和最终证据 job 都有显式超时；普通 PR 不重复执行整套发布验收。
- `v*` tag 使用独立 `Release` workflow：先校验唯一静态 blocker 与 tag，再运行质量、强制签名/公证的五 RID 原生打包、动态证据门禁，最后只创建 draft release。活动 tag 发布不会被后续运行自动取消。

## 统一发布目录

```bash
scripts/build-release.sh linux-arm64
```

该命令会执行 Release Rust 构建、.NET 自包含 publish、组装和 `verify-package`。输出包含：

- `Ariadne.Desktop` 自包含桌面程序及运行时；
- `Backend/ariadne-ipc` sidecar；
- `Tools/ariadne` CLI；正式产物不包含远程/REST server；
- `Resources/`、许可文件、第三方声明和平台图标；
- `release-manifest.json` 文件大小与 SHA-256 清单。

`verify-package` 会拒绝 PDB、target/obj/bin、密钥/数据库、`ariadne-server`、构建仓库绝对路径、缺失资源和 hash 漂移，并从包内桌面入口启动包内 sidecar。

## 本机发布证据

性能证据必须来自 Release 构建；探针会写入 `build_profile=release`，`verify-release-evidence.py` 会拒绝 Debug 证据。Linux 本机可执行：

```bash
ARIADNE_RELEASE_EVIDENCE_DIR="$PWD/artifacts/release-evidence" \
ARIADNE_SCHEDULER_SOAK_SECONDS=300 \
cargo test --release --test release_acceptance -- --ignored --test-threads=1

dotnet build desktop/Ariadne.Desktop/Ariadne.Desktop.csproj --configuration Release --no-restore
dotnet desktop/Ariadne.Desktop/bin/Release/net10.0/Ariadne.Desktop.dll \
  --release-wcag-probe artifacts/release-evidence/wcag-contrast.json
xvfb-run -a -s "-screen 0 1600x900x24" \
  dotnet desktop/Ariadne.Desktop/bin/Release/net10.0/Ariadne.Desktop.dll \
  --release-ui-probe artifacts/release-evidence/desktop-ui-performance.json
```

随后运行 `python3 scripts/verify-release-evidence.py --rid linux-arm64`。Qdrant 不进入 Ariadne 安装包；检索首次使用时才从 `qdrant-sidecars.json` 的固定 HTTPS 地址下载对应 RID 的官方归档，逐项校验版本、归档 SHA-256 和二进制 SHA-256 后写入用户缓存，后续直接复用。当前 Qdrant 1.18.2 Linux arm64 归档约 28 MiB、解压二进制约 71 MiB，向量索引数据另计；升级成功后会清理旧的受管版本缓存。

最终校验要求五个原生 RID 各自产出 runtime provisioning、cache reuse、index-upsert-search 与 clean shutdown 证据。单一 Linux arm64 开发机只可验收本 RID，不得伪造其它平台结果。

## 安装产物

- Linux：`packaging/linux/build-deb.sh <staging>`，随后运行 `smoke-deb.sh`；正式发布的 `.asc` 在生成后和 smoke 时都必须由 GPG 重新验证。
- Windows：安装 Inno Setup 后运行 `build-installer.ps1`，随后运行 `smoke-installer.ps1`。正式发布先在组装 manifest 前签名并验证 Ariadne Desktop/CLI/IPC 第一方二进制，再验证带时间戳的安装器和卸载器 Authenticode 签名。
- macOS：`packaging/macos/build-installer.sh <staging>` 生成 `.app`、`.pkg` 和 `.dmg`；smoke 会只读挂载 DMG 并从镜像内启动应用，再执行 pkg 首次安装、覆盖升级和数据保留。正式发布还要求 codesign/pkg 签名、stapler ticket 与 Gatekeeper assessment 全部通过。

手动 `full_release_matrix` 可以使用无证书或 macOS ad-hoc 签名完成原生结构烟测；tag 发布设置 `ARIADNE_REQUIRE_SIGNED_RELEASE=1` 后必须提供正式签名输入，否则立即失败：Linux `ARIADNE_LINUX_SIGNING_KEY`、Windows `ARIADNE_WINDOWS_SIGNTOOL`、macOS `ARIADNE_MACOS_SIGNING_IDENTITY` / `ARIADNE_MACOS_INSTALLER_IDENTITY`。对应私钥必须由 CI secret 导入临时 runner，仓库不保存证书、私钥或口令。

macOS 正式发布还必须完成公证。可提供预配置的 `ARIADNE_MACOS_NOTARY_PROFILE`，或同时提供 `ARIADNE_MACOS_NOTARY_APPLE_ID`、`ARIADNE_MACOS_NOTARY_TEAM_ID`、`ARIADNE_MACOS_NOTARY_PASSWORD`；脚本会对 `.pkg` 与 `.dmg` 执行 `notarytool --wait` 并 staple。Release workflow 使用 `ARIADNE_MACOS_CERTIFICATE_P12` / `ARIADNE_MACOS_CERTIFICATE_PASSWORD` 临时导入 Developer ID 证书。

`ARIADNE_WINDOWS_SIGNTOOL` 是 Inno Setup SignTool 命令模板，必须包含 `$f` 文件占位符并配置 RFC 3161 时间戳。相同模板会在组装前签名四个 Ariadne 第一方 PE 文件，也会由 Inno Setup 签名安装器/卸载器；任何签名状态无效或正式发布缺少时间戳都会立即失败。

安装器只拥有应用目录和系统快捷方式。用户配置、最近项目和创作项目位于用户数据目录或用户选择的项目目录；升级与卸载烟雾测试必须证明这些数据不会被删除。
