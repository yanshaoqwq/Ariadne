using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class WorkflowNodeCatalogTests
{
    private static WorkflowNodeViewModel NewNode(string type) => new(
        id: "node",
        nodeType: type,
        label: type,
        defaultWorkDir: string.Empty,
        x: 0,
        y: 0,
        runRequested: _ => { },
        clearSelection: () => { },
        markDirty: () => { });

    [Fact]
    public void ShippedCatalog_DefinesEveryPaletteGroupOnce()
    {
        Assert.Equal(17, WorkflowNodeCatalog.All.Count);
        Assert.Single(WorkflowNodeCatalog.ForGroup("entry"));
        Assert.Equal(9, WorkflowNodeCatalog.ForGroup("writing").Count());
        Assert.Equal(7, WorkflowNodeCatalog.ForGroup("utility").Count());
        Assert.Equal(
            WorkflowNodeCatalog.All.Count,
            WorkflowNodeCatalog.All.Select(entry => entry.NodeType).Distinct().Count());
    }

    [Fact]
    public void ModelNodes_DeclareProjectAndWebSearchTools()
    {
        var modelNodes = WorkflowNodeCatalog.All.Where(entry => entry.HasModelExecution).ToArray();

        Assert.Equal(10, modelNodes.Length);
        Assert.All(modelNodes, entry =>
        {
            Assert.False(string.IsNullOrWhiteSpace(entry.ProjectSearchTool));
            Assert.False(string.IsNullOrWhiteSpace(entry.WebSearchTool));
        });
    }

    [Theory]
    [InlineData("document", "document_read")]
    [InlineData("project_search", "search")]
    [InlineData("eval", "condition")]
    public void LegacyAliases_UseCanonicalConfiguration(string alias, string canonicalType)
    {
        var descriptor = WorkflowNodeCatalog.FindKnown(alias);

        Assert.NotNull(descriptor);
        Assert.Equal(canonicalType, descriptor.NodeType);
        Assert.Equal(NewNode(canonicalType).IsUtilityNode, NewNode(alias).IsUtilityNode);
        Assert.Equal(NewNode(canonicalType).ShowPromptEditor, NewNode(alias).ShowPromptEditor);
    }

    [Fact]
    public void ExtensionNode_RemainsLoadableWithoutPretendingToBeBuiltIn()
    {
        var descriptor = WorkflowNodeCatalog.Resolve("executor_adapter:custom");
        var node = NewNode("executor_adapter:custom");

        Assert.Equal("extension", descriptor.LibraryGroup);
        Assert.False(node.IsUtilityNode);
        Assert.False(node.IsAgentNode);
        Assert.True(node.ShowDataInPinEditor);
    }
}
