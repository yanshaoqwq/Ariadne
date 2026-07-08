# Ariadne Desktop

正式桌面前端使用 Avalonia UI + FluentTheme + .NET 10。

## 分工

- `Ariadne.Desktop/Views/**`：窗口和页面 XAML。
- `Ariadne.Desktop/Controls/**`：可复用桌面控件。
- `Ariadne.Desktop/Resources/**`：主题、样式和图标资源。
- `Ariadne.Desktop/ViewModels/**`：UI 状态和命令绑定。
- `Ariadne.Desktop/Backend/**`：后端 IPC 客户端边界。
- `Ariadne.Desktop/Localization/**`：`display_name.json` 绑定服务。

所有显示文本必须来自 `core/resources/display_name.json`。

## 验证

```bash
dotnet restore desktop/Ariadne.slnx
dotnet build desktop/Ariadne.slnx
dotnet run --project desktop/Ariadne.Desktop
```

源码仍按 Avalonia 12 + net10.0 维护。
