using Avalonia.Layout;
using Avalonia.Media;

namespace Ariadne.Desktop.ViewModels;

/// <summary>Project AI 对话气泡；对齐 ui设计方案 侧栏对话阅读节奏。</summary>
public sealed class ChatBubbleViewModel : ViewModelBase
{
    private static readonly IBrush UserBrush = new SolidColorBrush(Color.Parse("#2E2E726B"));
    private static readonly IBrush AssistantBrush = new SolidColorBrush(Color.Parse("#14000000"));

    public ChatBubbleViewModel(string role, string content)
    {
        Role = (role ?? string.Empty).Trim().ToLowerInvariant();
        Content = content ?? string.Empty;
        IsUser = Role is "user";
        IsAssistant = !IsUser;
        HorizontalAlignment = IsUser ? HorizontalAlignment.Right : HorizontalAlignment.Left;
        BubbleBackground = IsUser ? UserBrush : AssistantBrush;
    }

    public string Role { get; }
    public string Content { get; }
    public bool IsUser { get; }
    public bool IsAssistant { get; }
    public HorizontalAlignment HorizontalAlignment { get; }
    public IBrush BubbleBackground { get; }
    public string RoleLabel => IsUser ? "你" : "Ariadne";
}
