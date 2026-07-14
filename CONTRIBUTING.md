# 参与 Ariadne 开发

提交改动前请先创建 Issue 说明问题与预期行为，并保持一次 Pull Request 只处理一个可独立审查的逻辑变更。

所有贡献必须：

- 遵循 [AGENTS.md](AGENTS.md) 中的目录、测试、本地化和安全规则；
- 不包含密钥、个人数据、生成缓存、构建产物或无权再许可的第三方材料；
- 为新增或改变的公共行为提供对应测试；
- 对所有用户可见文本使用 `core/resources/display_name.json` 键；
- 在 Pull Request 中列明第三方代码、图片、字体、模板、模型输出或数据的来源和许可。

## 贡献者许可协议

项目采用非商业 source-available 许可，并保留单独商业授权能力。外部贡献在合并前必须阅读并接受 [Ariadne Contributor License Agreement](CLA.md)。未记录接受声明的贡献不得合并。

Pull Request 中应保留以下完整声明：

> I have read and agree to the Ariadne Contributor License Agreement, and I have the right to submit this contribution.

## 验证

至少运行与改动范围对应的 Rust 或 .NET 测试。发布、依赖、资源或跨层契约改动还必须通过仓库发布门禁；具体命令见 README 和 `.github/workflows`。
