using System.Text.Json;
using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class BackendModelSerializationTests
{
    [Fact]
    public void ProviderStatus_PreservesConfiguredBoundary()
    {
        const string json = """
        {
          "has_openai_key": false,
          "has_anthropic_key": false,
          "has_gemini_key": false,
          "providers": [{
            "provider": "openai",
            "display_name": "OpenAI",
            "provider_type": "open_ai",
            "configured": false,
            "enabled": false,
            "models": [],
            "has_key": false
          }]
        }
        """;

        var status = JsonSerializer.Deserialize<ProviderConfigStatus>(
            json,
            new JsonSerializerOptions(JsonSerializerDefaults.Web));

        Assert.NotNull(status);
        Assert.False(Assert.Single(status.Providers).Configured);
    }

    [Fact]
    public void ProviderRemovalPreview_PreservesRevisionAndReferenceLocations()
    {
        const string json = """
        {
          "provider_id": "openai",
          "display_name": "OpenAI",
          "revision": "abc123",
          "has_key": true,
          "default_roles": ["llm"],
          "blocking_references": [{
            "reference_type": "workflow",
            "owner_id": "draft-flow",
            "node_id": "writer",
            "model_id": "gpt-test"
          }]
        }
        """;

        var preview = JsonSerializer.Deserialize<ProviderRemovalPreview>(
            json,
            new JsonSerializerOptions(JsonSerializerDefaults.Web));

        Assert.NotNull(preview);
        Assert.Equal("abc123", preview.Revision);
        Assert.True(preview.HasKey);
        Assert.Equal("llm", Assert.Single(preview.DefaultRoles));
        var reference = Assert.Single(preview.BlockingReferences);
        Assert.Equal("draft-flow", reference.OwnerId);
        Assert.Equal("writer", reference.NodeId);
    }

    [Fact]
    public void WorkflowRunState_DeserializesStructuredRunFailure()
    {
        const string json = """
        {
          "workflow_id": "wf",
          "run_id": "run-1",
          "status": "failed",
          "pause_reason": null,
          "stop_reason": null,
          "failure": {
            "code": "workflow_worker_failed",
            "stage": "executor_init",
            "message": "provider missing",
            "recovery_suggestion": "error.workflow.worker_failed.recovery"
          },
          "events": []
        }
        """;

        var state = JsonSerializer.Deserialize<WorkflowRunState>(json, new JsonSerializerOptions(JsonSerializerDefaults.Web));

        Assert.NotNull(state);
        Assert.Equal("failed", state.Status);
        Assert.NotNull(state.Failure);
        Assert.Equal("workflow_worker_failed", state.Failure.Code);
        Assert.Equal("error.workflow.worker_failed.recovery", state.Failure.RecoverySuggestion);
    }

    [Fact]
    public void WorkflowPackReport_DeserializesRealIpcShapeAndExposesPackedGraph()
    {
        const string json = """
        {
          "workflow": {
            "workflow_id": "main-flow",
            "name": "Main Flow",
            "nodes": [{
              "id": "sub-review",
              "type": "subworkflow",
              "label": "Review",
              "data": {},
              "position": { "x": 120.0, "y": 40.0 }
            }],
            "edges": [],
            "metadata": {}
          },
          "subworkflow_node_id": "sub-review",
          "embedded_workflow": {
            "workflow_id": "main-flow::sub-review",
            "name": "Review",
            "nodes": [],
            "edges": [],
            "metadata": {}
          },
          "boundary_inputs": [{ "node_id": "writer", "port_name": "draft" }],
          "boundary_outputs": [{ "node_id": "reviewer", "port_name": "review" }]
        }
        """;

        var report = JsonSerializer.Deserialize<WorkflowPackReport>(json, new JsonSerializerOptions(JsonSerializerDefaults.Web));

        Assert.NotNull(report);
        Assert.Equal("main-flow", report.Workflow.WorkflowId);
        Assert.Single(report.Workflow.Nodes);
        Assert.Equal("sub-review", report.Workflow.Nodes[0].Id);
        Assert.Equal("main-flow::sub-review", report.EmbeddedWorkflow.WorkflowId);
        Assert.Equal("writer", Assert.Single(report.BoundaryInputs).NodeId);
        Assert.Equal("review", Assert.Single(report.BoundaryOutputs).PortName);
    }

    [Fact]
    public void WorkflowPackReport_DeserializedAsTopLevelGraphSilentlyProducesInvalidNullMembers()
    {
        const string json = """
        {
          "workflow": { "workflow_id": "main", "name": "Main", "nodes": [], "edges": [], "metadata": {} },
          "subworkflow_node_id": "sub",
          "embedded_workflow": { "workflow_id": "embedded", "name": "Embedded", "nodes": [], "edges": [], "metadata": {} },
          "boundary_inputs": [],
          "boundary_outputs": []
        }
        """;

        var invalid = JsonSerializer.Deserialize<WorkflowGraphData>(json, new JsonSerializerOptions(JsonSerializerDefaults.Web));

        Assert.NotNull(invalid);
        Assert.Null(invalid.WorkflowId);
        Assert.Null(invalid.Nodes);
    }

    [Fact]
    public void InDoubtOperationAndResolution_DeserializeFromIpcShapes()
    {
        const string operationJson = """
        {
          "operation_id": "op-1",
          "workflow_id": "wf",
          "run_id": "run-1",
          "node_id": "writer",
          "attempt": 1,
          "kind": "llm",
          "provider": "openai",
          "request_hash": "hash",
          "lease_generation": 2,
          "status": "in_doubt",
          "created_at_ms": 1,
          "updated_at_ms": 2,
          "in_doubt_at_ms": 2
        }
        """;
        const string resolutionJson = """
        {
          "operation_id": "op-1",
          "decision": "use_response",
          "workflow": { "workflow_id": "wf", "run_id": "run-1", "status": "queued" }
        }
        """;
        var options = new JsonSerializerOptions(JsonSerializerDefaults.Web);

        var operation = JsonSerializer.Deserialize<WorkflowOperation>(operationJson, options);
        var resolution = JsonSerializer.Deserialize<ResolveInDoubtOperationResult>(resolutionJson, options);

        Assert.NotNull(operation);
        Assert.Equal("in_doubt", operation.Status);
        Assert.Equal("writer", operation.NodeId);
        Assert.NotNull(resolution);
        Assert.Equal("use_response", resolution.Decision);
        Assert.Equal("queued", resolution.Workflow.Status);
    }

    [Fact]
    public void ChapterSummaryView_DeserializesFormalProjectionAndTypedRegisterContent()
    {
        const string json = """
        {
          "chapter_id": "chapter-1",
          "chapter_summary": "章节正式总结",
          "stage": {
            "stage_id": "stage-main",
            "summary": "阶段正式总结",
            "chapter_ids": ["chapter-1"]
          },
          "segments": [{
            "segment_id": "chapter-1::seg-1",
            "number": "1",
            "chapter_id": "chapter-1",
            "summary": "故事段概括",
            "source": {
              "document_id": "documents/chapter-1.md",
              "range": { "start": 3, "end": 7 },
              "version": "v1"
            },
            "metadata": {}
          }],
          "events": [{
            "event_id": "event-1",
            "summary": "事件概括",
            "status": "ongoing",
            "segment_ids": ["chapter-1::seg-1"],
            "chapter_ids": ["chapter-1"],
            "metadata": {}
          }],
          "realized_changes": [{
            "change_id": "change-1",
            "function": "character_trait",
            "status": "realized",
            "content": {
              "kind": "character_trait",
              "content": {
                "character": "阿青",
                "trait_name": "勇气",
                "from_value": "犹疑",
                "to_value": "坚定",
                "reason": "作出选择"
              }
            },
            "linked_segment_ids": ["chapter-1::seg-1"],
            "metadata": {}
          }],
          "foreshadowing": [{
            "foreshadowing_id": "f-1",
            "title": "旧钥匙",
            "description": "钥匙再次出现",
            "status": "recovered",
            "planted_segment_ids": ["chapter-1::seg-1"],
            "recovered_segment_ids": ["chapter-1::seg-1"],
            "metadata": {}
          }],
          "confirmations": [{
            "confirmation_id": "confirm-1",
            "kind": "chapter_summary",
            "state": "approved",
            "revision_id": "rev-1"
          }]
        }
        """;

        var view = JsonSerializer.Deserialize<ChapterSummaryView>(
            json,
            new JsonSerializerOptions(JsonSerializerDefaults.Web));

        Assert.NotNull(view);
        Assert.Equal("stage-main", view.Stage?.StageId);
        Assert.Equal(3, Assert.Single(view.Segments).Source.Range.Start);
        var change = Assert.Single(view.RealizedChanges);
        Assert.Equal("character_trait", change.Function);
        Assert.Equal("阿青", change.Content.GetProperty("content").GetProperty("character").GetString());
        Assert.Equal("approved", Assert.Single(view.Confirmations).State);
    }
}
