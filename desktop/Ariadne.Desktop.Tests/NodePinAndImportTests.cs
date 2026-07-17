using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 驱动 shipped WorkflowNodeViewModel / NodeConfigData：数据入增删与 import_path 持久化。
/// </summary>
public sealed class NodePinAndImportTests
{
    private static WorkflowNodeViewModel NewNode(
        string type = "document_read",
        Action<WorkflowNodeViewModel>? runRequested = null)
    {
        return new WorkflowNodeViewModel(
            id: "n1",
            nodeType: type,
            label: "test",
            defaultWorkDir: string.Empty,
            x: 0,
            y: 0,
            runRequested: runRequested ?? (_ => { }),
            clearSelection: () => { },
            markDirty: () => { });
    }

    [Fact]
    public void RunCommand_DelegatesToHostRunCoordinator()
    {
        WorkflowNodeViewModel? requestedNode = null;
        var node = NewNode("start", requested => requestedNode = requested);

        node.RunCommand.Execute(null);

        Assert.Same(node, requestedNode);
    }

    [Fact]
    public void AddDataInPin_IncreasesHandles_WithDistinctIds()
    {
        var node = NewNode("writer");
        Assert.Single(node.DataInPins);
        Assert.Equal("input", node.DataInPins[0].Handle);

        node.AddDataInPin();
        node.AddDataInPin();

        Assert.Equal(3, node.DataInPins.Count);
        var handles = node.DataInPins.Select(p => p.Handle).ToArray();
        Assert.Equal("input", handles[0]);
        Assert.Contains("data-in-1", handles);
        Assert.Equal(handles.Length, handles.Distinct(StringComparer.OrdinalIgnoreCase).Count());
    }

    [Fact]
    public void RemoveDataInPin_KeepsAtLeastOne_AndDropsHandle()
    {
        var node = NewNode("writer");
        node.AddDataInPin();
        Assert.Equal(2, node.DataInPins.Count);
        var second = node.DataInPins[1].Handle;

        node.RemoveDataInPin(second);
        Assert.Single(node.DataInPins);
        Assert.Equal("input", node.DataInPins[0].Handle);

        // 不能删光
        node.RemoveDataInPin("input");
        Assert.Single(node.DataInPins);
    }

    [Fact]
    public void RemoveDataInPin_InvokesHostCallback_ForEdgeDrop()
    {
        var node = NewNode("writer");
        node.AddDataInPin();
        string? removed = null;
        node.DataInPinRemoved = h => removed = h;
        var handle = node.DataInPins[1].Handle;
        node.RemoveDataInPin(handle);
        Assert.Equal(handle, removed);
    }

    [Fact]
    public void ToData_PersistsImportPath_AndDataInHandles()
    {
        var node = NewNode("document_read");
        Assert.True(node.IsImportNode);
        Assert.False(node.ShowPromptEditor);

        node.ImportPath = "/tmp/story.md";
        node.AddDataInPin();
        var data = node.ToData();

        Assert.Equal("/tmp/story.md", data["import_path"]?.ToString());
        Assert.True(data.ContainsKey("data_in_handles"));
        var handles = (data["data_in_handles"] as IEnumerable<object?>)
                      ?.Select(o => o?.ToString() ?? string.Empty)
                      .Where(s => s.Length > 0)
                      .ToArray()
                      ?? Array.Empty<string>();
        Assert.Contains("input", handles);
        Assert.True(handles.Length >= 2);
    }

    [Fact]
    public void NodeConfigData_MergeUiFields_WritesImportAndPins()
    {
        var data = NodeConfigData.MergeUiFields(
            extra: null,
            name: "doc",
            workDir: string.Empty,
            userNote: string.Empty,
            isStartNode: false,
            exposedAsTool: false,
            promptTemplate: string.Empty,
            modelId: string.Empty,
            budgetUsd: string.Empty,
            timeoutMs: string.Empty,
            breakpointEnabled: false,
            importPath: "/proj/a.txt",
            dataInHandles: new[] { "input", "data-in-1" });

        Assert.Equal("/proj/a.txt", data["import_path"]?.ToString());
        var list = (data["data_in_handles"] as IEnumerable<object?>)
                   ?.Select(o => o?.ToString())
                   .ToArray();
        Assert.NotNull(list);
        Assert.Equal(new[] { "input", "data-in-1" }, list);
    }

    [Fact]
    public void LocalCenter_MatchesTopAlignedLayoutRules()
    {
        // 与 XAML：内容 pad-top=8、pin=14、Spacing=8、VerticalAlignment=Top 一致
        var first = NodePortSpec.LocalCenterForHandle("input");
        var second = NodePortSpec.LocalCenterForHandle("data-in-1");
        Assert.Equal(NodePortSpec.DataPortY, first.Y, 6);
        Assert.Equal(second.Y, first.Y + NodePortSpec.DataPortSpacing, 6);
        Assert.Equal(NodePortSpec.DataPortSpacing, NodePortSpec.DataPinBox + NodePortSpec.DataPortGap, 6);
        // 首 pin 中心 = CardTop + Title + pad + half pin
        var expected = NodePortSpec.CardTopOffset + NodePortSpec.TitleBarHeight
                       + NodePortSpec.ContentBarPaddingY + NodePortSpec.DataPinBox / 2.0;
        Assert.Equal(NodePortSpec.DataPortY, expected, 6);
    }

    [Fact]
    public void DocumentNode_ToData_WritesBackendPathAndIncludeContent()
    {
        var node = NewNode("document_read");
        Assert.True(node.IsDocumentNode);
        Assert.False(node.IsExportNode);
        node.ImportPath = "/docs/ch1.md";
        node.IncludeContent = true;
        var data = node.ToData();
        Assert.Equal("/docs/ch1.md", data["path"]?.ToString());
        Assert.Equal(true, data["include_content"]);
    }

    [Fact]
    public void SearchConditionLoopApprovalExport_ToData_MatchBackendKeys()
    {
        var search = NewNode("search");
        search.QueryAlias = "q";
        search.SearchLimit = "5";
        var s = search.ToData();
        Assert.Equal("q", s["query_alias"]?.ToString());
        Assert.Equal(5, Convert.ToInt32(s["limit"]));

        var cond = NewNode("condition");
        cond.ConditionInputAlias = "score";
        cond.ConditionOperator = "equals";
        cond.ConditionExpected = "1";
        var c = cond.ToData();
        Assert.Equal("score", c["input_alias"]?.ToString());
        Assert.Equal("equals", c["operator"]?.ToString());

        var loop = NewNode("loop");
        loop.MaxIterations = "3";
        loop.StopInputAlias = "done";
        loop.StopExpected = "true";
        var l = loop.ToData();
        Assert.Equal(3, Convert.ToInt32(l["max_iterations"]));
        Assert.True(l.ContainsKey("stop_condition"));
        var stop = Assert.IsType<Dictionary<string, object?>>(l["stop_condition"]);
        Assert.Equal("done", stop["input_alias"]?.ToString());
        Assert.Equal(true, stop["equals"]);
        Assert.Equal(300000L, Convert.ToInt64(l["timeout_ms"]));

        var ap = NewNode("approval");
        ap.ApprovalId = "ap-1";
        ap.AutoApprove = true;
        var a = ap.ToData();
        Assert.Equal("ap-1", a["approval_id"]?.ToString());
        Assert.Equal(true, a["auto_approve"]);

        var ex = NewNode("export");
        Assert.True(ex.IsExportNode);
        Assert.False(ex.IsDocumentNode);
        ex.ExportArtifactId = "art-1";
        ex.ExportFormat = "epub";
        ex.ExportTitle = "Book";
        var e = ex.ToData();
        Assert.Equal("art-1", e["artifact_id"]?.ToString());
        Assert.Equal("epub", e["format"]?.ToString());
        Assert.Equal("Book", e["title"]?.ToString());
    }

    [Fact]
    public void Summarizer_ToData_UsesDedicatedBusinessConfigAndCanClearStaleProvider()
    {
        var node = NewNode("summarizer");
        Assert.True(node.IsSummarizerNode);
        node.RetainOpaqueData(new Dictionary<string, object?>
        {
            ["provider_id"] = "old-provider",
            ["chapter_id"] = "old-chapter",
            ["chapter_document_id"] = "documents/old.md",
            ["chapter_text_alias"] = "old_text",
            ["auto_mode"] = false,
            ["temperature"] = 0.2,
        });
        node.SummarizerProviderId = "provider-main";
        node.ModelId = "model-main";
        node.SummarizerChapterId = "chapter-1";
        node.SummarizerChapterDocumentId = "documents/chapter-1.md";
        node.SummarizerChapterTextAlias = "chapter_body";
        node.SummarizerAutoMode = true;

        var data = node.ToData();

        Assert.Equal("provider-main", data["provider_id"]);
        Assert.Equal("model-main", data["model_id"]);
        Assert.Equal("chapter-1", data["chapter_id"]);
        Assert.Equal("documents/chapter-1.md", data["chapter_document_id"]);
        Assert.Equal("chapter_body", data["chapter_text_alias"]);
        Assert.Equal(true, data["auto_mode"]);
        Assert.Equal(0.2, data["temperature"]);

        node.SummarizerProviderId = "  ";
        Assert.False(node.ToData().ContainsKey("provider_id"));
    }

    /// <summary>未实现后端桩：节点构造不调用后端即可测 UI 字段。DispatchProxy 要求非 sealed。</summary>
    private class UnimplementedBackendProxy : DispatchProxy
    {
        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }

            if (targetMethod.ReturnType == typeof(bool) && targetMethod.Name == "get_HasProjectRoot")
            {
                return false;
            }

            if (targetMethod.ReturnType == typeof(void) || targetMethod.ReturnType == typeof(Task))
            {
                return targetMethod.ReturnType == typeof(Task) ? Task.CompletedTask : null;
            }

            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var t = targetMethod.ReturnType.GetGenericArguments()[0];
                var completed = typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(t)
                    .Invoke(null, new object?[] { t.IsValueType ? Activator.CreateInstance(t) : null });
                return completed;
            }

            return targetMethod.ReturnType.IsValueType
                ? Activator.CreateInstance(targetMethod.ReturnType)
                : null;
        }
    }
}
