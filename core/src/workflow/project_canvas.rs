use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::contracts::{CanvasPosition, EdgeId, NodeId, WorkflowDefinition, WorkflowId};
pub const PROJECT_CANVAS_WORKFLOW_ID: &str = "default";
pub const PROJECT_CANVAS_NAME: &str = "Project Canvas";
const PROJECT_CANVAS_METADATA_KEY: &str = "project_canvas";
const IMPORT_GAP_X: f64 = 280.0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectCanvasImportReceipt {
    pub workflow_id: String,
    pub revision: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ProjectCanvasMetadata {
    #[serde(default)]
    imported_workflows: Vec<ProjectCanvasImportReceipt>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ProjectCanvasAnnotation {
    annotation_id: String,
    title: String,
    #[serde(default)]
    node_ids: Vec<NodeId>,
    #[serde(default)]
    metadata: Value,
}

/// 将一个旧工作流或模板图并入项目规范画布。节点、边、通信发起端和批注使用同一份
/// ID 映射，避免分别补丁造成悬空引用；导入回执存入画布 metadata，重开时不会重复叠加。
pub fn merge_workflow_into_project_canvas(
    canvas: &mut WorkflowDefinition,
    source: WorkflowDefinition,
    source_revision: impl Into<String>,
) -> bool {
    let source_id = source.id.as_str().to_owned();
    if source_id == PROJECT_CANVAS_WORKFLOW_ID || project_canvas_contains_source(canvas, &source_id)
    {
        return false;
    }

    let namespace = normalized_namespace(&source_id);
    let mut occupied_node_ids = canvas
        .nodes
        .iter()
        .map(|node| node.id.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let mut occupied_edge_ids = canvas
        .edges
        .iter()
        .map(|edge| edge.id.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let mut node_ids = BTreeMap::new();

    for node in &source.nodes {
        let mapped = unique_id(&mut occupied_node_ids, &namespace, node.id.as_str());
        node_ids.insert(node.id.as_str().to_owned(), mapped);
    }

    let offset_x = project_canvas_import_offset_x(canvas, &source);
    let mut imported_nodes = source.nodes;
    for node in &mut imported_nodes {
        let original = node.id.as_str().to_owned();
        node.id = NodeId::from(
            node_ids
                .get(&original)
                .expect("all imported node ids are mapped")
                .clone(),
        );
        let position = node.position.unwrap_or(CanvasPosition { x: 0.0, y: 0.0 });
        node.position = Some(CanvasPosition {
            x: position.x + offset_x,
            y: position.y,
        });
    }

    let mut imported_edges = source.edges;
    for edge in &mut imported_edges {
        let original_edge_id = edge.id.as_str().to_owned();
        edge.id = EdgeId::from(unique_id(
            &mut occupied_edge_ids,
            &namespace,
            &original_edge_id,
        ));
        edge.from.node_id = remap_node_id(&node_ids, &edge.from.node_id);
        edge.to.node_id = remap_node_id(&node_ids, &edge.to.node_id);
        if let Some(communication) = edge.communication.as_mut() {
            if let Some(initiator) = communication.initiator_node_id.as_mut() {
                *initiator = remap_node_id(&node_ids, initiator);
            }
        }
    }

    merge_canvas_annotations(
        &mut canvas.metadata,
        &source.metadata,
        &namespace,
        &node_ids,
    );
    canvas.nodes.extend(imported_nodes);
    canvas.edges.extend(imported_edges);
    record_project_canvas_import(
        canvas,
        ProjectCanvasImportReceipt {
            workflow_id: source_id,
            revision: source_revision.into(),
        },
    );
    true
}

pub fn normalize_project_canvas_identity(canvas: &mut WorkflowDefinition) {
    canvas.id = WorkflowId::from(PROJECT_CANVAS_WORKFLOW_ID);
    if canvas.name.trim().is_empty() || canvas.name == "Default Workflow" {
        canvas.name = PROJECT_CANVAS_NAME.to_owned();
    }
}

pub fn project_canvas_imports(canvas: &WorkflowDefinition) -> Vec<ProjectCanvasImportReceipt> {
    project_canvas_metadata(&canvas.metadata).imported_workflows
}

fn project_canvas_contains_source(canvas: &WorkflowDefinition, workflow_id: &str) -> bool {
    project_canvas_imports(canvas)
        .iter()
        .any(|receipt| receipt.workflow_id == workflow_id)
}

fn project_canvas_import_offset_x(canvas: &WorkflowDefinition, source: &WorkflowDefinition) -> f64 {
    if canvas.nodes.is_empty() {
        return 0.0;
    }
    let canvas_right = canvas
        .nodes
        .iter()
        .filter_map(|node| node.position.map(|position| position.x))
        .fold(0.0, f64::max);
    let source_left = source
        .nodes
        .iter()
        .filter_map(|node| node.position.map(|position| position.x))
        .reduce(f64::min)
        .unwrap_or(0.0);
    canvas_right + IMPORT_GAP_X - source_left
}

fn normalized_namespace(workflow_id: &str) -> String {
    let normalized = workflow_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let normalized = normalized.trim_matches('-');
    if normalized.is_empty() {
        "workflow".to_owned()
    } else {
        normalized.to_owned()
    }
}

fn unique_id(occupied: &mut BTreeSet<String>, namespace: &str, original: &str) -> String {
    let base = format!("{namespace}--{original}");
    if occupied.insert(base.clone()) {
        return base;
    }
    for suffix in 2_u64.. {
        let candidate = format!("{base}--{suffix}");
        if occupied.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("an unbounded numeric suffix always yields a unique canvas id")
}

fn remap_node_id(node_ids: &BTreeMap<String, String>, node_id: &NodeId) -> NodeId {
    NodeId::from(
        node_ids
            .get(node_id.as_str())
            .expect("validated source edges only reference mapped nodes")
            .clone(),
    )
}

fn merge_canvas_annotations(
    canvas_metadata: &mut Value,
    source_metadata: &Value,
    namespace: &str,
    node_ids: &BTreeMap<String, String>,
) {
    let Some(source_annotations) = source_metadata.get("canvas_annotations").and_then(|value| {
        serde_json::from_value::<Vec<ProjectCanvasAnnotation>>(value.clone()).ok()
    }) else {
        return;
    };
    let metadata = ensure_object(canvas_metadata);
    let mut annotations = metadata
        .remove("canvas_annotations")
        .and_then(|value| serde_json::from_value::<Vec<ProjectCanvasAnnotation>>(value).ok())
        .unwrap_or_default();
    let mut occupied = annotations
        .iter()
        .map(|annotation| annotation.annotation_id.clone())
        .collect::<BTreeSet<_>>();
    for mut annotation in source_annotations {
        annotation.annotation_id =
            unique_id(&mut occupied, namespace, annotation.annotation_id.as_str());
        annotation.node_ids = annotation
            .node_ids
            .iter()
            .map(|node_id| remap_node_id(node_ids, node_id))
            .collect();
        annotations.push(annotation);
    }
    metadata.insert(
        "canvas_annotations".to_owned(),
        serde_json::to_value(annotations).expect("canvas annotations are serializable"),
    );
}

fn record_project_canvas_import(
    canvas: &mut WorkflowDefinition,
    receipt: ProjectCanvasImportReceipt,
) {
    let metadata = ensure_object(&mut canvas.metadata);
    let mut project_canvas = metadata
        .remove(PROJECT_CANVAS_METADATA_KEY)
        .and_then(|value| serde_json::from_value::<ProjectCanvasMetadata>(value).ok())
        .unwrap_or_default();
    project_canvas.imported_workflows.push(receipt);
    metadata.insert(
        PROJECT_CANVAS_METADATA_KEY.to_owned(),
        serde_json::to_value(project_canvas).expect("project canvas metadata is serializable"),
    );
}

fn project_canvas_metadata(metadata: &Value) -> ProjectCanvasMetadata {
    metadata
        .get(PROJECT_CANVAS_METADATA_KEY)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

fn ensure_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value
        .as_object_mut()
        .expect("value was normalized to object")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::contracts::{Edge, NodeInstance, PortEndpoint, WorkflowEdgeKind};

    use super::*;

    fn workflow(id: &str, node_id: &str, x: f64) -> WorkflowDefinition {
        WorkflowDefinition {
            id: WorkflowId::from(id),
            name: id.to_owned(),
            nodes: vec![NodeInstance {
                id: NodeId::from(node_id),
                type_name: "start".to_owned(),
                label: None,
                config: Value::Null,
                position: Some(CanvasPosition { x, y: 20.0 }),
            }],
            edges: Vec::new(),
            metadata: Value::Null,
        }
    }

    #[test]
    fn merge_namespaces_conflicting_ids_and_is_idempotent_by_source() {
        let mut canvas = workflow(PROJECT_CANVAS_WORKFLOW_ID, "start", 0.0);
        let mut source = workflow("review/flow", "start", 0.0);
        source.nodes.push(NodeInstance {
            id: NodeId::from("writer"),
            type_name: "writer".to_owned(),
            label: None,
            config: Value::Null,
            position: Some(CanvasPosition { x: 100.0, y: 20.0 }),
        });
        source.edges.push(Edge {
            id: EdgeId::from("next"),
            kind: WorkflowEdgeKind::Control,
            from: PortEndpoint {
                node_id: NodeId::from("start"),
                port_name: "exec_out".to_owned(),
            },
            to: PortEndpoint {
                node_id: NodeId::from("writer"),
                port_name: "exec_in".to_owned(),
            },
            alias: None,
            communication: None,
        });

        assert!(merge_workflow_into_project_canvas(
            &mut canvas,
            source.clone(),
            "revision-a"
        ));
        assert!(!merge_workflow_into_project_canvas(
            &mut canvas,
            source,
            "revision-a"
        ));
        assert_eq!(canvas.nodes.len(), 3);
        assert_eq!(canvas.edges.len(), 1);
        assert_eq!(canvas.edges[0].from.node_id.as_str(), "review-flow--start");
        assert_eq!(canvas.edges[0].to.node_id.as_str(), "review-flow--writer");
        assert_eq!(project_canvas_imports(&canvas).len(), 1);
    }

    #[test]
    fn merge_rewrites_annotation_node_references() {
        let mut canvas = workflow(PROJECT_CANVAS_WORKFLOW_ID, "root", 0.0);
        let mut source = workflow("template", "start", 0.0);
        source.metadata = json!({
            "canvas_annotations": [{
                "annotation_id": "group",
                "title": "Imported",
                "node_ids": ["start"],
                "metadata": null
            }]
        });

        merge_workflow_into_project_canvas(&mut canvas, source, "revision");

        let annotations: Vec<ProjectCanvasAnnotation> =
            serde_json::from_value(canvas.metadata["canvas_annotations"].clone()).unwrap();
        assert_eq!(annotations[0].node_ids[0].as_str(), "template--start");
    }
}
