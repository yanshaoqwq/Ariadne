//! 发布性能矩阵：只在显式 `--ignored` 验收任务中运行。
//!
//! 该测试使用正式项目文档服务、持久化 outbox 和 `ProjectRetrievalRuntime`，
//! 不使用内存全文后端或测试专用索引，因此结果可作为发布证据而不是单元测试替身。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ariadne::config::MemorySecretStore;
use ariadne::contracts::{
    content_version_for_bytes, CoreResult, Edge, EdgeId, NodeId, NodeInstance, PortEndpoint, RunId,
    RunStatus, WorkflowDefinition, WorkflowEdgeKind, WorkflowId, EXECUTION_INPUT_PORT,
    EXECUTION_OUTPUT_PORT,
};
use ariadne::documents::{DocumentRepository, DocumentWriteRequest};
use ariadne::frontend::initialize_project;
use ariadne::providers::ProviderCallContext;
use ariadne::retrieval::{ensure_managed_qdrant_binary, ProjectRetrievalRuntime};
use ariadne::workflow::{
    WorkflowNodeExecutionOutput, WorkflowNodeExecutionRequest, WorkflowNodeExecutor,
    WorkflowRuntime,
};
use serde::Serialize;
use serde_json::Value;

const RELEASE_CHARACTER_COUNT: usize = 1_000_000;

#[derive(Debug, Serialize)]
struct RetrievalEvidence {
    schema_version: u32,
    probe: &'static str,
    build_profile: &'static str,
    character_count: usize,
    byte_count: usize,
    chunk_size_chars: usize,
    import_ms: u128,
    initial_index_ms: u128,
    initial_search_ms: u128,
    incremental_update_ms: u128,
    incremental_search_ms: u128,
    rebuild_ms: u128,
    rebuild_search_ms: u128,
    peak_rss_bytes: u64,
    index_bytes: u64,
    initial_hits: usize,
    incremental_hits: usize,
    rebuild_hits: usize,
}

#[derive(Debug, Serialize)]
struct SchedulerSample {
    node_count: usize,
    runnable_width: usize,
    sample_count: usize,
    median_ms: f64,
    p95_ms: f64,
    median_nodes_per_second: f64,
}

#[derive(Debug, Serialize)]
struct SchedulerEvidence {
    schema_version: u32,
    probe: &'static str,
    build_profile: &'static str,
    samples: Vec<SchedulerSample>,
    growth_ratio_500_to_1000: f64,
    soak_duration_seconds: u64,
    soak_iterations: usize,
    soak_completed_nodes: usize,
    soak_failures: usize,
    peak_rss_bytes: u64,
}

#[derive(Debug, Serialize)]
struct QdrantProvisioningEvidence {
    schema_version: u32,
    probe: &'static str,
    build_profile: &'static str,
    rid: String,
    qdrant_version: String,
    archive_sha256: String,
    binary_sha256: String,
    first_use_installed: bool,
    cache_hit_without_source_archive: bool,
}

#[test]
#[ignore = "发布验收：显式运行 cargo test --release --test release_acceptance -- --ignored"]
fn million_character_retrieval_release_matrix() {
    assert_release_profile();
    let temp = tempfile::tempdir().expect("temporary project");
    initialize_project(temp.path()).expect("initialize project");

    let document = temp.path().join("documents").join("release-matrix.md");
    let initial = make_fixture("唯一初始检索标记");
    let initial_bytes = initial.len();
    let service = ariadne::documents::FileDocumentService::new(
        ariadne::frontend::project_document_permission(temp.path()),
        temp.path().join(".runtime").join("artifacts"),
    );

    let import_started = Instant::now();
    let first = service
        .save_document(DocumentWriteRequest {
            path: document.clone(),
            content: initial.clone(),
            format: None,
            base_version: None,
        })
        .expect("import million-character document");
    let import_ms = import_started.elapsed().as_millis();

    let runtime = ProjectRetrievalRuntime::open(temp.path(), &MemorySecretStore::default())
        .expect("open formal retrieval runtime");

    let index_started = Instant::now();
    let processed = runtime.process_outbox().expect("index initial document");
    assert!(processed > 0, "the formal document outbox must be consumed");
    let initial_index_ms = index_started.elapsed().as_millis();

    let initial_search_started = Instant::now();
    let initial_hits = runtime
        .search(
            "唯一初始检索标记".to_owned(),
            10,
            ProviderCallContext::new("release-matrix"),
        )
        .expect("search indexed initial marker");
    let initial_search_ms = initial_search_started.elapsed().as_millis();
    assert!(
        !initial_hits.is_empty(),
        "initial marker must be searchable"
    );

    let updated = initial.replace("唯一初始检索标记", "唯一增量检索标记");
    let incremental_started = Instant::now();
    let second = service
        .save_document(DocumentWriteRequest {
            path: document.clone(),
            content: updated,
            format: None,
            base_version: Some(first.metadata.version.clone()),
        })
        .expect("save incremental update");
    runtime.process_outbox().expect("index incremental update");
    let incremental_update_ms = incremental_started.elapsed().as_millis();

    let incremental_search_started = Instant::now();
    let incremental_hits = runtime
        .search(
            "唯一增量检索标记".to_owned(),
            10,
            ProviderCallContext::new("release-matrix"),
        )
        .expect("search incrementally updated marker");
    let incremental_search_ms = incremental_search_started.elapsed().as_millis();
    assert!(
        !incremental_hits.is_empty(),
        "incremental marker must be searchable"
    );

    let rebuild_started = Instant::now();
    runtime
        .enqueue_configuration_rebuild()
        .expect("enqueue full rebuild");
    runtime.process_outbox().expect("rebuild formal indexes");
    let rebuild_ms = rebuild_started.elapsed().as_millis();

    let rebuild_search_started = Instant::now();
    let rebuild_hits = runtime
        .search(
            "唯一增量检索标记".to_owned(),
            10,
            ProviderCallContext::new("release-matrix"),
        )
        .expect("search after full rebuild");
    let rebuild_search_ms = rebuild_search_started.elapsed().as_millis();
    assert!(
        !rebuild_hits.is_empty(),
        "rebuild marker must be searchable"
    );

    let evidence = RetrievalEvidence {
        schema_version: 1,
        probe: "million_character_retrieval",
        build_profile: release_build_profile(),
        character_count: initial.chars().count(),
        byte_count: initial_bytes,
        chunk_size_chars: runtime.config().rag.chunk_size_chars as usize,
        import_ms,
        initial_index_ms,
        initial_search_ms,
        incremental_update_ms,
        incremental_search_ms,
        rebuild_ms,
        rebuild_search_ms,
        peak_rss_bytes: peak_rss_bytes(),
        index_bytes: directory_size(&temp.path().join(".indexes")),
        initial_hits: initial_hits.len(),
        incremental_hits: incremental_hits.len(),
        rebuild_hits: rebuild_hits.len(),
    };
    write_evidence("million-character-retrieval.json", &evidence);

    assert_eq!(evidence.character_count, RELEASE_CHARACTER_COUNT);
    assert_eq!(
        second.metadata.version,
        content_version_for_bytes(
            fs::read(&document)
                .expect("read updated document")
                .as_slice(),
        )
    );
}

#[test]
#[ignore = "发布验收：显式运行 cargo test --release --test release_acceptance -- --ignored"]
fn scheduler_high_runnable_throughput_and_soak() {
    assert_release_profile();
    const SAMPLE_COUNT: usize = 5;
    let mut samples = Vec::new();
    let mut medians = Vec::new();
    for node_count in [500, 1000] {
        let workflow = wide_workflow(node_count);
        let mut durations_ms = Vec::with_capacity(SAMPLE_COUNT);
        for sample_index in 0..SAMPLE_COUNT {
            let started = Instant::now();
            let completed = execute_workflow(
                &workflow,
                &format!("release-sample-{node_count}-{sample_index}"),
            );
            let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(completed, node_count);
            durations_ms.push(elapsed_ms);
        }
        durations_ms.sort_by(f64::total_cmp);
        let median_ms = percentile(&durations_ms, 0.5);
        let p95_ms = percentile(&durations_ms, 0.95);
        medians.push(median_ms);
        samples.push(SchedulerSample {
            node_count,
            runnable_width: node_count - 1,
            sample_count: SAMPLE_COUNT,
            median_ms,
            p95_ms,
            median_nodes_per_second: node_count as f64 / (median_ms / 1000.0).max(0.000_001),
        });
    }

    let requested_soak_seconds = std::env::var("ARIADNE_SCHEDULER_SOAK_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(60)
        .max(1);
    let soak_started = Instant::now();
    let soak_workflow = wide_workflow(1000);
    let mut soak_iterations = 0usize;
    let mut soak_completed_nodes = 0usize;
    let mut soak_failures = 0usize;
    while soak_started.elapsed().as_secs() < requested_soak_seconds {
        let run_id = format!("release-soak-{soak_iterations}");
        let completed = execute_workflow(&soak_workflow, &run_id);
        if completed == 1000 {
            soak_completed_nodes += completed;
        } else {
            soak_failures += 1;
        }
        soak_iterations += 1;
    }

    let evidence = SchedulerEvidence {
        schema_version: 1,
        probe: "scheduler_throughput",
        build_profile: release_build_profile(),
        samples,
        growth_ratio_500_to_1000: medians[1] / medians[0].max(0.000_001),
        soak_duration_seconds: soak_started.elapsed().as_secs(),
        soak_iterations,
        soak_completed_nodes,
        soak_failures,
        peak_rss_bytes: peak_rss_bytes(),
    };
    assert_eq!(evidence.soak_failures, 0);
    assert!(evidence.soak_iterations > 0);
    write_evidence("scheduler-throughput.json", &evidence);
}

#[test]
#[ignore = "发布验收：设置 ARIADNE_QDRANT_ARCHIVE 后显式运行 cargo test --release --test release_acceptance -- --ignored"]
fn qdrant_runtime_provisioning_installs_then_uses_cache() {
    assert_release_profile();
    let temporary_cache = std::env::var_os("ARIADNE_QDRANT_CACHE_DIR")
        .is_none()
        .then(|| tempfile::tempdir().expect("temporary Qdrant provisioning root"));
    if let Some(temp) = &temporary_cache {
        std::env::set_var("ARIADNE_QDRANT_CACHE_DIR", temp.path().join("cache"));
    }
    let source_archive = std::env::var_os("ARIADNE_QDRANT_ARCHIVE").map(PathBuf::from);
    if let Some(source_archive) = &source_archive {
        assert!(source_archive.is_file(), "Qdrant source archive must exist");
    }

    let first = ensure_managed_qdrant_binary().expect("provision Qdrant from verified archive");
    assert!(first.is_file());
    let metadata_path = first
        .parent()
        .expect("managed Qdrant directory")
        .join("qdrant-sidecar.json");
    let metadata: Value =
        serde_json::from_slice(&fs::read(&metadata_path).expect("read managed Qdrant metadata"))
            .expect("parse managed Qdrant metadata");

    let missing_source = first
        .parent()
        .expect("managed Qdrant directory")
        .join("source-archive-was-removed");
    std::env::set_var("ARIADNE_QDRANT_ARCHIVE", &missing_source);
    let second = ensure_managed_qdrant_binary().expect("reuse Qdrant without source archive");
    assert_eq!(first, second);

    let evidence = QdrantProvisioningEvidence {
        schema_version: 1,
        probe: "qdrant_runtime_provisioning",
        build_profile: release_build_profile(),
        rid: metadata["rid"].as_str().expect("metadata RID").to_owned(),
        qdrant_version: metadata["version"]
            .as_str()
            .expect("metadata version")
            .to_owned(),
        archive_sha256: metadata["archive_sha256"]
            .as_str()
            .expect("metadata archive digest")
            .to_owned(),
        binary_sha256: metadata["binary_sha256"]
            .as_str()
            .expect("metadata binary digest")
            .to_owned(),
        first_use_installed: true,
        cache_hit_without_source_archive: true,
    };
    write_evidence("qdrant-runtime-provisioning.json", &evidence);
    std::env::remove_var("ARIADNE_QDRANT_ARCHIVE");
    if temporary_cache.is_some() {
        std::env::remove_var("ARIADNE_QDRANT_CACHE_DIR");
    }
}

#[derive(Default)]
struct ReleaseExecutor {
    calls: usize,
}

impl WorkflowNodeExecutor for ReleaseExecutor {
    fn execute(
        &mut self,
        _request: WorkflowNodeExecutionRequest,
    ) -> CoreResult<WorkflowNodeExecutionOutput> {
        self.calls += 1;
        Ok(WorkflowNodeExecutionOutput::default())
    }
}

fn execute_workflow(workflow: &WorkflowDefinition, run_id: &str) -> usize {
    let mut executor = ReleaseExecutor::default();
    let mut runtime = WorkflowRuntime::new(workflow, RunId::from(run_id)).expect("create runtime");
    let status = runtime.run(workflow, &mut executor).expect("run workflow");
    assert_eq!(status, RunStatus::Succeeded);
    executor.calls
}

fn wide_workflow(node_count: usize) -> WorkflowDefinition {
    assert!(node_count >= 2);
    let nodes = (0..node_count)
        .map(|index| NodeInstance {
            id: NodeId::from(format!("node-{index}")),
            type_name: "release_probe".to_owned(),
            label: None,
            config: Value::Null,
            position: None,
        })
        .collect();
    let edges = (1..node_count)
        .map(|index| Edge {
            id: EdgeId::from(format!("edge-{index}")),
            kind: WorkflowEdgeKind::Control,
            from: PortEndpoint {
                node_id: NodeId::from("node-0"),
                port_name: EXECUTION_OUTPUT_PORT.to_owned(),
            },
            to: PortEndpoint {
                node_id: NodeId::from(format!("node-{index}")),
                port_name: EXECUTION_INPUT_PORT.to_owned(),
            },
            alias: None,
            communication: None,
        })
        .collect();
    WorkflowDefinition {
        id: WorkflowId::from(format!("release-wide-{node_count}")),
        name: format!("Release wide {node_count}"),
        nodes,
        edges,
        metadata: Value::Null,
    }
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    assert!(!values.is_empty());
    let index = ((values.len() - 1) as f64 * percentile).round() as usize;
    values[index.min(values.len() - 1)]
}

fn release_build_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn assert_release_profile() {
    assert_eq!(
        release_build_profile(),
        "release",
        "release evidence must be generated with cargo test --release"
    );
}

fn make_fixture(marker: &str) -> String {
    let unit = "第一幕，人物沿着旧城河岸寻找被隐藏的线索；这段正文用于发布前真实检索矩阵。\n";
    let mut content = String::with_capacity(RELEASE_CHARACTER_COUNT * 3);
    content.push_str(marker);
    let unit_chars = unit.chars().count();
    let mut char_count = marker.chars().count();
    while char_count + unit_chars <= RELEASE_CHARACTER_COUNT {
        content.push_str(unit);
        char_count += unit_chars;
    }
    if char_count < RELEASE_CHARACTER_COUNT {
        content.extend(unit.chars().take(RELEASE_CHARACTER_COUNT - char_count));
    }
    debug_assert_eq!(content.chars().count(), RELEASE_CHARACTER_COUNT);
    content
}

fn write_evidence<T: Serialize>(name: &str, value: &T) {
    let requested = std::env::var_os("ARIADNE_RELEASE_EVIDENCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("artifacts/release-evidence"));
    let directory = if requested.is_absolute() {
        requested
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join(requested)
    };
    fs::create_dir_all(&directory).expect("create release evidence directory");
    let path = directory.join(name);
    let bytes = serde_json::to_vec_pretty(value).expect("serialize release evidence");
    fs::write(path, bytes).expect("write release evidence");
}

fn directory_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    walk_files(path)
        .into_iter()
        .filter_map(|file| fs::metadata(file).ok())
        .map(|metadata| metadata.len())
        .sum()
}

fn walk_files(path: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(path) else {
        return files;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        if child.is_dir() {
            files.extend(walk_files(&child));
        } else {
            files.push(child);
        }
    }
    files
}

#[cfg(target_os = "linux")]
fn peak_rss_bytes() -> u64 {
    let Ok(status) = fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|kilobytes| kilobytes.saturating_mul(1024))
        .unwrap_or(0)
}

#[cfg(not(target_os = "linux"))]
fn peak_rss_bytes() -> u64 {
    0
}
