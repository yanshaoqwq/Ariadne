use std::fs;
use std::path::Path;

use ariadne::contracts::artifacts::{ArtifactKind, DocumentPatch, PatchHunk};
use ariadne::contracts::{PermissionPolicy, PortValue, SourceSpan, TextRange};
use ariadne::documents::{
    ArtifactWriteRequest, ChapterDocumentEntry, ChapterDocumentIndex, ChapterDocumentKind,
    DocumentReadRequest, DocumentRepository, DocumentWriteRequest, FileDocumentService,
    PatchCheckpointRequest,
};
use ariadne::git::GitService;
use serde_json::json;

/// 创建允许在临时目录内读写的文档服务。
fn test_service(root: &Path) -> FileDocumentService {
    let artifact_root = root.join(".runtime").join("artifacts");
    let policy = PermissionPolicy {
        readable_file_roots: vec![root.to_path_buf()],
        writable_file_roots: vec![root.to_path_buf()],
        ..PermissionPolicy::default()
    };
    FileDocumentService::new(policy, artifact_root)
}

/// 初始化测试 Git 仓库，并写入本地提交身份。
fn init_test_repo(root: &Path) -> GitService {
    let service = GitService::new(root);
    service.init_repository().unwrap();
    run_git(root, ["config", "user.name", "Ariadne Test"]);
    run_git(root, ["config", "user.email", "ariadne@example.test"]);
    service
}

/// 执行测试用 Git 命令，失败时输出 stderr。
fn run_git<const N: usize>(repo: &Path, args: [&str; N]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 验证文档服务能读写受支持文档，并只在端口中传递 document_ref。
#[test]
fn document_service_reads_and_writes_supported_documents() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path());
    let path = temp_dir.path().join("chapter.md");

    let report = service
        .save_document(DocumentWriteRequest {
            path: path.clone(),
            content: "# 第一章\n正文".to_owned(),
            format: None,
            base_version: None,
        })
        .unwrap();
    let content = service
        .open_document(DocumentReadRequest {
            path: path.clone(),
            format: None,
        })
        .unwrap();
    let document_ref = service.document_ref_for_path(&path).unwrap();

    assert_eq!(report.index_invalidation.reason, "document_saved");
    let pending = service.invalidation_outbox().pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].document_id, report.metadata.document_id);
    assert_eq!(pending[0].source_version, report.metadata.version);
    assert_eq!(content.content, "# 第一章\n正文");
    assert_eq!(
        document_ref,
        PortValue::document_ref(content.metadata.document_id, None)
    );
}

/// 验证 JSON 文档写入前会做结构校验。
#[test]
fn document_service_validates_json_documents() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path());
    let path = temp_dir.path().join("data.json");

    let error = service
        .save_document(DocumentWriteRequest {
            path,
            content: "{not-json}".to_owned(),
            format: None,
            base_version: None,
        })
        .unwrap_err();

    assert!(error.to_string().contains("json"));
}

/// 保存完整文档时，调用方携带旧版本必须被拒绝，避免覆盖外部更新。
#[test]
fn document_service_rejects_stale_full_document_save() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path());
    let path = temp_dir.path().join("chapter.md");

    let original = service
        .save_document(DocumentWriteRequest {
            path: path.clone(),
            content: "第一版".to_owned(),
            format: None,
            base_version: None,
        })
        .unwrap();
    let old_version = original.metadata.version;

    service
        .save_document(DocumentWriteRequest {
            path: path.clone(),
            content: "第二版更长".to_owned(),
            format: None,
            base_version: Some(old_version.clone()),
        })
        .unwrap();

    let error = service
        .save_document(DocumentWriteRequest {
            path,
            content: "第三版".to_owned(),
            format: None,
            base_version: Some(old_version),
        })
        .unwrap_err();

    assert!(error
        .to_string()
        .contains("base_version does not match current document"));
}

/// 验证 patch 可以先预览、再写回，并联动 Git checkpoint。
#[test]
fn document_service_previews_and_applies_patch_with_checkpoint() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path());
    let git = init_test_repo(temp_dir.path());
    let path = temp_dir.path().join("chapter.txt");
    fs::write(&path, "alpha beta gamma").unwrap();
    git.create_archive_point("base", None).unwrap();

    let document = service
        .open_document(DocumentReadRequest {
            path: path.clone(),
            format: None,
        })
        .unwrap();
    let beta_start = document.content.find("beta").unwrap() as u64;
    let patch = DocumentPatch {
        document_id: document.metadata.document_id.clone(),
        base_version: Some(document.metadata.version.clone()),
        hunks: vec![PatchHunk {
            range: TextRange {
                start: beta_start,
                end: beta_start + 4,
            },
            replacement: "delta".to_owned(),
        }],
    };

    let preview = service.preview_patch(&patch).unwrap();
    let report = service
        .apply_patch(
            &patch,
            Some(&git),
            Some(&PatchCheckpointRequest {
                node_id: "node-6".to_owned(),
                message: None,
            }),
        )
        .unwrap();

    assert!(preview.changed);
    assert_eq!(fs::read_to_string(&path).unwrap(), "alpha delta gamma");
    assert_eq!(report.index_invalidation.reason, "patch_applied");
    assert_eq!(report.checkpoint.unwrap().node_id, "node-6");
    let pending = service.invalidation_outbox().pending().unwrap();
    assert!(pending.iter().any(|event| {
        event.document_id == report.metadata.document_id
            && event.source_version == report.metadata.version
            && event.reason == "patch_applied"
    }));
}

/// Git checkpoint 失败时正文必须恢复，调用方不会看到普通失败但文件已改变。
#[test]
fn document_service_rolls_back_patch_when_checkpoint_fails() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path());
    let path = temp_dir.path().join("chapter.txt");
    fs::write(&path, "alpha beta gamma").unwrap();
    let document = service
        .open_document(DocumentReadRequest {
            path: path.clone(),
            format: None,
        })
        .unwrap();
    let beta_start = document.content.find("beta").unwrap() as u64;
    let patch = DocumentPatch {
        document_id: document.metadata.document_id,
        base_version: Some(document.metadata.version),
        hunks: vec![PatchHunk {
            range: TextRange {
                start: beta_start,
                end: beta_start + 4,
            },
            replacement: "delta".to_owned(),
        }],
    };
    let non_repository = tempfile::tempdir().unwrap();
    let git = GitService::new(non_repository.path());

    let error = service
        .apply_patch(
            &patch,
            Some(&git),
            Some(&PatchCheckpointRequest {
                node_id: "node-failing-checkpoint".to_owned(),
                message: None,
            }),
        )
        .unwrap_err();

    assert!(error.to_string().contains("document was rolled back"));
    assert_eq!(fs::read_to_string(path).unwrap(), "alpha beta gamma");
    assert!(service.invalidation_outbox().pending().unwrap().is_empty());
}

#[test]
fn index_invalidation_outbox_claims_retries_and_completes_events() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path());
    let path = temp_dir.path().join("chapter.md");
    let report = service
        .save_document(DocumentWriteRequest {
            path,
            content: "第一版".to_owned(),
            format: None,
            base_version: None,
        })
        .unwrap();

    let claimed = service.invalidation_outbox().claim_next().unwrap().unwrap();
    assert_eq!(claimed.document_id, report.metadata.document_id);
    assert_eq!(claimed.status, "processing");
    assert!(service
        .invalidation_outbox()
        .claim_next()
        .unwrap()
        .is_none());

    service
        .invalidation_outbox()
        .retry(&claimed.event_id)
        .unwrap();
    let retried = service.invalidation_outbox().claim_next().unwrap().unwrap();
    assert_eq!(retried.attempt_count, 1);
    service
        .invalidation_outbox()
        .complete(&retried.event_id)
        .unwrap();
    assert!(service.invalidation_outbox().pending().unwrap().is_empty());
}

#[test]
fn index_invalidation_outbox_recovers_events_interrupted_by_process_exit() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path());
    let path = temp_dir.path().join("chapter.md");
    service
        .save_document(DocumentWriteRequest {
            path,
            content: "待恢复索引".to_owned(),
            format: None,
            base_version: None,
        })
        .unwrap();

    let claimed = service.invalidation_outbox().claim_next().unwrap().unwrap();
    assert_eq!(claimed.status, "processing");
    assert_eq!(
        service.invalidation_outbox().requeue_interrupted().unwrap(),
        1
    );

    let recovered = service.invalidation_outbox().claim_next().unwrap().unwrap();
    assert_eq!(recovered.event_id, claimed.event_id);
    assert_eq!(recovered.attempt_count, 1);
}

#[test]
fn index_invalidation_outbox_dead_letters_poison_event_and_unblocks_queue() {
    let temp = tempfile::tempdir().unwrap();
    let service = test_service(temp.path());
    let poison_id = service
        .invalidation_outbox()
        .prepare("poison.md", "save", "v1", false)
        .unwrap();
    service.invalidation_outbox().activate(&poison_id).unwrap();
    let healthy_id = service
        .invalidation_outbox()
        .prepare("healthy.md", "save", "v1", false)
        .unwrap();
    service.invalidation_outbox().activate(&healthy_id).unwrap();

    for attempt in 1..=5 {
        let claimed = service.invalidation_outbox().claim_next().unwrap().unwrap();
        assert_eq!(claimed.event_id, poison_id);
        let dead_lettered = service
            .invalidation_outbox()
            .retry(&claimed.event_id)
            .unwrap();
        assert_eq!(dead_lettered, attempt == 5);
    }

    let next = service.invalidation_outbox().claim_next().unwrap().unwrap();
    assert_eq!(next.event_id, healthy_id);
}

#[test]
fn project_maintenance_blocks_writes_until_completed_and_preserves_failure() {
    let temp = tempfile::tempdir().unwrap();
    let service = test_service(temp.path());
    let outbox = service.invalidation_outbox();

    outbox
        .begin_maintenance("git_restore", "stopping_runtime")
        .unwrap();
    assert!(outbox
        .ensure_available()
        .unwrap_err()
        .to_string()
        .contains("git_restore"));
    outbox
        .update_maintenance_phase("rebuilding_full_text_indexes")
        .unwrap();
    outbox.complete_maintenance("completed").unwrap();
    outbox.ensure_available().unwrap();

    outbox
        .begin_maintenance("git_restore", "checking_out_branch")
        .unwrap();
    outbox
        .fail_maintenance("restore_incomplete", "checkout failed")
        .unwrap();
    let state = outbox.maintenance_state().unwrap().unwrap();
    assert_eq!(state.status, "failed");
    assert_eq!(state.error.as_deref(), Some("checkout failed"));
    assert!(outbox.ensure_available().is_err());

    outbox.begin_maintenance("git_restore", "retrying").unwrap();
    assert_eq!(
        outbox.maintenance_state().unwrap().unwrap().status,
        "active"
    );
}

/// 验证父目录穿越会被路径沙箱拒绝。
#[test]
fn document_service_rejects_parent_escape() {
    let temp_dir = tempfile::tempdir().unwrap();
    let allowed = temp_dir.path().join("allowed");
    fs::create_dir_all(&allowed).unwrap();
    let service = test_service(&allowed);

    let error = service
        .save_document(DocumentWriteRequest {
            path: allowed.join("../outside.md"),
            content: "escape".to_owned(),
            format: None,
            base_version: None,
        })
        .unwrap_err();

    assert!(error.to_string().contains("permission denied"));
}

/// 验证符号链接逃逸会被路径沙箱拒绝。
#[test]
#[cfg(unix)]
fn document_service_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let temp_dir = tempfile::tempdir().unwrap();
    let allowed = temp_dir.path().join("allowed");
    let outside = temp_dir.path().join("outside");
    fs::create_dir_all(&allowed).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, allowed.join("link")).unwrap();
    let service = test_service(&allowed);

    let error = service
        .save_document(DocumentWriteRequest {
            path: allowed.join("link").join("secret.txt"),
            content: "escape".to_owned(),
            format: None,
            base_version: None,
        })
        .unwrap_err();

    assert!(error.to_string().contains("permission denied"));
}

/// 验证 Artifact 写入固定根目录，并返回可传递的描述信息。
#[test]
fn document_service_writes_artifacts_under_artifact_root() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path());

    let report = service
        .write_artifact(ArtifactWriteRequest {
            artifact_id: "outputs/result.txt".to_owned(),
            kind: ArtifactKind::ModelOutput,
            media_type: "text/plain; charset=utf-8".to_owned(),
            bytes: b"model output".to_vec(),
            metadata: json!({ "node": "n1" }),
        })
        .unwrap();

    assert_eq!(report.descriptor.artifact_id, "outputs/result.txt");
    assert_eq!(report.descriptor.size_bytes, Some(12));
    assert!(report.descriptor.storage_uri.contains(".runtime/artifacts"));
}

/// 验证章节索引只把 chapter_body 纳入作品页/合并导出，并按 order 排序。
#[test]
fn chapter_document_index_orders_and_filters_exportable_bodies() {
    let index = ChapterDocumentIndex::new(
        "v1",
        vec![
            chapter_entry("chapter-2", "doc-2", 20, ChapterDocumentKind::ChapterBody),
            chapter_entry(
                "chapter-1-outline",
                "outline-1",
                5,
                ChapterDocumentKind::Outline,
            ),
            chapter_entry("chapter-1", "doc-1", 10, ChapterDocumentKind::ChapterBody),
        ],
    )
    .unwrap();

    let bodies = index.chapter_bodies();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0].chapter_id, "chapter-1");
    assert_eq!(bodies[1].chapter_id, "chapter-2");
    assert_eq!(
        index.export_document_ids(&[]).unwrap(),
        vec!["doc-1".to_owned(), "doc-2".to_owned()]
    );
    assert_eq!(
        index
            .export_document_ids(&["chapter-2".to_owned()])
            .unwrap(),
        vec!["doc-2".to_owned()]
    );
}

/// 验证章节索引对缺失正文和重复正文引用给出可诊断错误。
#[test]
fn chapter_document_index_reports_missing_or_duplicate_body_entries() {
    let index = ChapterDocumentIndex::new(
        "v1",
        vec![chapter_entry(
            "chapter-1-outline",
            "outline-1",
            5,
            ChapterDocumentKind::Outline,
        )],
    )
    .unwrap();
    let missing = index
        .export_document_ids(&["chapter-1".to_owned()])
        .unwrap_err();
    assert!(missing.to_string().contains("chapter_body"));

    let duplicate = ChapterDocumentIndex::new(
        "v1",
        vec![
            chapter_entry("chapter-1", "doc-1", 10, ChapterDocumentKind::ChapterBody),
            chapter_entry(
                "chapter-1-copy",
                "doc-1",
                11,
                ChapterDocumentKind::ChapterBody,
            ),
        ],
    )
    .unwrap_err();
    assert!(duplicate.to_string().contains("duplicate"));
}

/// 构造章节索引测试条目。
fn chapter_entry(
    chapter_id: &str,
    document_id: &str,
    order: u64,
    kind: ChapterDocumentKind,
) -> ChapterDocumentEntry {
    ChapterDocumentEntry {
        chapter_id: chapter_id.to_owned(),
        document_id: document_id.to_owned(),
        path: Path::new(document_id).with_extension("md"),
        title: chapter_id.to_owned(),
        order,
        kind,
        version: "v1".to_owned(),
        word_count: Some(1200),
        outline_ref: Some(SourceSpan {
            document_id: "outline-doc".to_owned(),
            range: TextRange { start: 0, end: 5 },
            version: Some("outline-v1".to_owned()),
        }),
    }
}
