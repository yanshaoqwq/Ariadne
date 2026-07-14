# Ariadne 发布打包

发布矩阵由 `release-matrix.json` 固定：Linux x64/arm64、Windows x64、macOS x64/arm64。每个目标必须在对应原生 runner 上构建和执行烟雾测试，不接受在单一开发机上只复制 Debug 输出。

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

## 安装产物

- Linux：`packaging/linux/build-deb.sh <staging>`，随后运行 `smoke-deb.sh`。
- Windows：安装 Inno Setup 后运行 `build-installer.ps1`，随后运行 `smoke-installer.ps1`。
- macOS：`packaging/macos/build-installer.sh <staging>` 生成 `.app`、`.pkg` 和 `.dmg`。

普通 PR/CI 可以使用无证书或 macOS ad-hoc 签名完成结构烟测；tag 发布设置 `ARIADNE_REQUIRE_SIGNED_RELEASE=1` 后必须提供正式签名输入，否则立即失败：Linux `ARIADNE_LINUX_SIGNING_KEY`、Windows `ARIADNE_WINDOWS_SIGNTOOL`、macOS `ARIADNE_MACOS_SIGNING_IDENTITY` / `ARIADNE_MACOS_INSTALLER_IDENTITY`。对应私钥必须由 CI secret 导入临时 runner，仓库不保存证书、私钥或口令。

macOS 正式发布还必须完成公证。可提供预配置的 `ARIADNE_MACOS_NOTARY_PROFILE`，或同时提供 `ARIADNE_MACOS_NOTARY_APPLE_ID`、`ARIADNE_MACOS_NOTARY_TEAM_ID`、`ARIADNE_MACOS_NOTARY_PASSWORD`；脚本会对 `.pkg` 与 `.dmg` 执行 `notarytool --wait` 并 staple。Release workflow 使用 `ARIADNE_MACOS_CERTIFICATE_P12` / `ARIADNE_MACOS_CERTIFICATE_PASSWORD` 临时导入 Developer ID 证书。

安装器只拥有应用目录和系统快捷方式。用户配置、最近项目和创作项目位于用户数据目录或用户选择的项目目录；升级与卸载烟雾测试必须证明这些数据不会被删除。
