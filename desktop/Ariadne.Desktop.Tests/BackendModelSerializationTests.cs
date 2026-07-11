using System.Text.Json;
using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class BackendModelSerializationTests
{
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
}
