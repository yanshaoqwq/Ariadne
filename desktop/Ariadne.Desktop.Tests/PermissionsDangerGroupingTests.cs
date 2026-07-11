using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// Exercises shipped danger-tool classification used by the permissions settings UI.
/// </summary>
public sealed class PermissionsDangerGroupingTests
{
    [Theory]
    [InlineData("writer-rewrite-file", true)]
    [InlineData("outliner-insert-lines", true)]
    [InlineData("planner-replace-lines", true)]
    [InlineData("agent-delete-file", true)]
    [InlineData("scope-secret-read", true)]
    [InlineData("writer-find", false)]
    [InlineData("writer-search", false)]
    [InlineData("project-ai-workflow-tools", false)]
    [InlineData("register", false)]
    [InlineData("", false)]
    public void IsDangerToolId_ClassifiesWriteAndSensitiveTools(string toolId, bool expected)
    {
        Assert.Equal(expected, ToolControlItemViewModel.IsDangerToolId(toolId));
    }

    [Fact]
    public void ToolControlGroup_RefreshPartitions_SplitsSafeAndDanger()
    {
        var group = new ToolControlGroupViewModel("writer", "Writer");
        group.Controls.Add(new ToolControlItemViewModel(
            "writer-find", "查找", true, ToolControlItemViewModel.IsDangerToolId("writer-find"), () => { }));
        group.Controls.Add(new ToolControlItemViewModel(
            "writer-rewrite-file", "重写", false, ToolControlItemViewModel.IsDangerToolId("writer-rewrite-file"), () => { }));
        group.Controls.Add(new ToolControlItemViewModel(
            "writer-insert-lines", "插入", true, ToolControlItemViewModel.IsDangerToolId("writer-insert-lines"), () => { }));

        group.RefreshPartitions();

        Assert.True(group.HasSafeControls);
        Assert.True(group.HasDangerControls);
        Assert.Single(group.SafeControls);
        Assert.Equal(2, group.DangerControls.Count);
        Assert.All(group.DangerControls, item => Assert.True(item.IsDangerous));
        Assert.All(group.SafeControls, item => Assert.False(item.IsDangerous));
    }
}
