use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::contracts::artifacts::{ArtifactDescriptor, DocumentPatch};
use crate::contracts::permissions::{PermissionPolicy, PermissionRequest};
use crate::contracts::ports::PortValue;
use crate::contracts::{CancellationToken, CoreError, CoreResult};
use crate::documents::models::{
    ArtifactWriteReport, ArtifactWriteRequest, DocumentContent, DocumentFormat, DocumentId,
    DocumentMetadata, DocumentReadRequest, DocumentRepository, DocumentWriteReport,
    DocumentWriteRequest, IndexInvalidation, PatchApplyReport, PatchCheckpointRequest,
    PatchPreview,
};
use crate::documents::IndexInvalidationOutbox;
use crate::git::GitService;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ArtifactOperationReceipt {
    operation_id: String,
    artifact_id: String,
    kind: crate::contracts::ArtifactKind,
    media_type: String,
    checksum: String,
    metadata_checksum: String,
    size_bytes: u64,
}

/// 文档和 Artifact 的文件系统服务。
#[derive(Debug, Clone)]
pub struct FileDocumentService {
    permissions: PermissionPolicy,
    artifact_root: PathBuf,
    export_root: Option<PathBuf>,
    invalidation_outbox: IndexInvalidationOutbox,
}

impl FileDocumentService {
    /// 创建文件系统文档服务。
    pub fn new(permissions: PermissionPolicy, artifact_root: impl Into<PathBuf>) -> Self {
        let artifact_root = artifact_root.into();
        let runtime_root = artifact_root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| artifact_root.clone());
        Self {
            permissions,
            artifact_root,
            export_root: None,
            invalidation_outbox: IndexInvalidationOutbox::new(
                runtime_root.join("index_invalidation.db"),
            ),
        }
    }

    /// 将 `exports/...` artifact id 映射到项目配置的导出目录。
    pub fn with_export_root(mut self, export_root: impl Into<PathBuf>) -> Self {
        self.export_root = Some(export_root.into());
        self
    }

    pub fn invalidation_outbox(&self) -> &IndexInvalidationOutbox {
        &self.invalidation_outbox
    }

    /// 生成 document_ref 端口值，运行状态只保存引用，不保存正文。
    pub fn document_ref_for_path(&self, path: &Path) -> CoreResult<PortValue> {
        let metadata = self.metadata_for_path(path, None)?;
        Ok(PortValue::document_ref(metadata.document_id, None))
    }

    /// 生成 chunk_ref 端口值。
    pub fn chunk_ref(chunk_id: impl Into<String>) -> PortValue {
        PortValue::chunk_ref(chunk_id)
    }

    /// 生成 artifact_ref 端口值。
    pub fn artifact_ref(artifact_id: impl Into<String>) -> PortValue {
        PortValue::artifact_ref(artifact_id)
    }

    /// 应用 patch，并可在写回后创建 Git checkpoint。
    pub fn apply_patch(
        &self,
        patch: &DocumentPatch,
        git: Option<&GitService>,
        checkpoint_request: Option<&PatchCheckpointRequest>,
    ) -> CoreResult<PatchApplyReport> {
        self.apply_patch_with_cancellation(
            patch,
            git,
            checkpoint_request,
            &CancellationToken::new(),
        )
    }

    /// 应用 patch，并在正文替换、Git checkpoint 与 outbox 提交边界响应取消。
    pub fn apply_patch_with_cancellation(
        &self,
        patch: &DocumentPatch,
        git: Option<&GitService>,
        checkpoint_request: Option<&PatchCheckpointRequest>,
        cancellation: &CancellationToken,
    ) -> CoreResult<PatchApplyReport> {
        cancellation.check()?;
        let path = PathBuf::from(&patch.document_id);
        let _project_mutation = self
            .invalidation_outbox
            .acquire_project_mutation("document_patch")?;
        // D1-a：校验与替换在独占写锁下完成，避免 TOCTOU 静默覆盖并发新写。
        let _write_lock = crate::config::store::PathWriteLock::acquire(&path)?;
        let original = self.open_document(DocumentReadRequest {
            path: path.clone(),
            format: None,
        })?;
        validate_base_version(patch, &original.metadata.version)?;

        let patched = apply_patch_to_string(&original.content, patch)?;
        cancellation.check()?;
        let preview = preview_from_contents(
            &original.metadata.document_id,
            &original.metadata.version,
            patch.hunks.len(),
            &original.content,
            &patched,
        );

        self.ensure_write(&path)?;
        validate_content_format(&patched, original.metadata.format)?;
        // Re-validate under lock after compute (second reader may have raced before lock).
        let still = self.open_document(DocumentReadRequest {
            path: path.clone(),
            format: None,
        })?;
        validate_base_version(patch, &still.metadata.version)?;
        let result_version = content_version(patched.as_bytes());
        let outbox_event = self.invalidation_outbox.prepare(
            &original.metadata.document_id,
            "patch_applied",
            &result_version,
            false,
        )?;
        if let Err(error) = cancellation.check() {
            let _ = self.invalidation_outbox.cancel(&outbox_event);
            return Err(error);
        }
        if let Err(error) = crate::config::store::atomic_write(&path, patched.as_bytes()) {
            let _ = self.invalidation_outbox.cancel(&outbox_event);
            return Err(error);
        }

        if let Err(error) = cancellation.check() {
            // D1-a：仅当磁盘仍是本 operation 写出的 result_version 才回滚。
            restore_previous_file_if_result(
                &path,
                Some(original.content.as_bytes()),
                &result_version,
            )?;
            let _ = self.invalidation_outbox.cancel(&outbox_event);
            return Err(error);
        }

        let checkpoint = match (git, checkpoint_request) {
            (Some(git), Some(request)) => {
                match git.create_checkpoint(&request.node_id, request.message.as_deref()) {
                    Ok(checkpoint) => Some(checkpoint),
                    Err(checkpoint_error) => {
                        if let Err(rollback_error) = restore_previous_file_if_result(
                            &path,
                            Some(original.content.as_bytes()),
                            &result_version,
                        ) {
                            return Err(CoreError::External {
                            service: "document_patch_transaction".to_owned(),
                            message: format!(
                                "checkpoint failed after document write; rollback also failed: checkpoint={checkpoint_error}; rollback={rollback_error}"
                            ),
                        });
                        }
                        let _ = self.invalidation_outbox.cancel(&outbox_event);
                        return Err(CoreError::External {
                            service: "git_checkpoint".to_owned(),
                            message: format!(
                            "patch checkpoint failed; document was rolled back: {checkpoint_error}"
                        ),
                        });
                    }
                }
            }
            _ => None,
        };

        let metadata = self.metadata_for_path(&path, Some(original.metadata.format))?;
        self.invalidation_outbox.activate(&outbox_event)?;
        let index_invalidation = index_invalidation(&metadata.document_id, "patch_applied");

        Ok(PatchApplyReport {
            preview,
            metadata,
            index_invalidation,
            checkpoint,
        })
    }

    /// 写入 Artifact 文件并返回可传递的 artifact 描述。
    pub fn write_artifact(&self, request: ArtifactWriteRequest) -> CoreResult<ArtifactWriteReport> {
        self.write_artifact_with_cancellation(request, &CancellationToken::new())
    }

    /// 写入 artifact；operation receipt 使文件已替换后的取消仍可确定性恢复。
    pub fn write_artifact_with_cancellation(
        &self,
        request: ArtifactWriteRequest,
        cancellation: &CancellationToken,
    ) -> CoreResult<ArtifactWriteReport> {
        cancellation.check()?;
        let _project_mutation = self
            .invalidation_outbox
            .acquire_project_mutation("artifact_write")?;
        let ArtifactWriteRequest {
            artifact_id,
            kind,
            media_type,
            bytes,
            operation_id,
            metadata,
        } = request;
        validate_artifact_id(&artifact_id)?;
        let path = self.artifact_path(&artifact_id);
        self.ensure_write(&path)?;
        let checksum = content_version(&bytes);
        let metadata_checksum = content_version(&serde_json::to_vec(&metadata)?);
        let had_operation_id = operation_id.is_some();
        let normalized_operation_id = operation_id
            .map(|operation_id| operation_id.trim().to_owned())
            .filter(|operation_id| !operation_id.is_empty());
        if had_operation_id && normalized_operation_id.is_none() {
            return Err(CoreError::validation(
                "artifact operation_id cannot be blank",
            ));
        }
        let receipt =
            normalized_operation_id
                .as_ref()
                .map(|operation_id| ArtifactOperationReceipt {
                    operation_id: operation_id.clone(),
                    artifact_id: artifact_id.clone(),
                    kind: kind.clone(),
                    media_type: media_type.clone(),
                    checksum: checksum.clone(),
                    metadata_checksum: metadata_checksum.clone(),
                    size_bytes: bytes.len() as u64,
                });
        let receipt_path = receipt.as_ref().map(|receipt| {
            self.artifact_root.join(".operations").join(format!(
                "{}.json",
                content_version(receipt.operation_id.as_bytes())
            ))
        });

        if let (Some(expected), Some(receipt_path)) = (&receipt, &receipt_path) {
            self.ensure_write(receipt_path)?;
            if receipt_path.exists() {
                self.ensure_read(receipt_path)?;
                let persisted =
                    serde_json::from_slice::<ArtifactOperationReceipt>(&fs::read(receipt_path)?)?;
                if persisted != *expected {
                    return Err(CoreError::validation(format!(
                        "artifact operation_id reused with a different payload: {}",
                        expected.operation_id
                    )));
                }
                if path.exists() {
                    self.ensure_read(&path)?;
                    if content_version(&fs::read(&path)?) != checksum {
                        return Err(CoreError::validation(format!(
                            "artifact content conflicts with committed operation: {}",
                            expected.operation_id
                        )));
                    }
                } else {
                    cancellation.check()?;
                    crate::config::store::atomic_write(&path, &bytes)?;
                }
                return Ok(ArtifactWriteReport {
                    descriptor: ArtifactDescriptor {
                        artifact_id,
                        kind,
                        media_type,
                        storage_uri: format!("file://{}", path.display()),
                        size_bytes: Some(bytes.len() as u64),
                        checksum: Some(checksum),
                        created_by_run: None,
                        created_by_node: None,
                        sources: Vec::new(),
                        metadata,
                    },
                });
            }
        }

        cancellation.check()?;
        crate::config::store::atomic_write(&path, &bytes)?;
        if let (Some(receipt), Some(receipt_path)) = (&receipt, &receipt_path) {
            cancellation.check()?;
            crate::config::store::atomic_write(receipt_path, &serde_json::to_vec_pretty(receipt)?)?;
        }

        let descriptor = ArtifactDescriptor {
            artifact_id,
            kind,
            media_type,
            storage_uri: format!("file://{}", path.display()),
            size_bytes: Some(bytes.len() as u64),
            checksum: Some(checksum),
            created_by_run: None,
            created_by_node: None,
            sources: Vec::new(),
            metadata,
        };

        Ok(ArtifactWriteReport { descriptor })
    }

    fn artifact_path(&self, artifact_id: &str) -> PathBuf {
        if let Some(relative) = artifact_id.strip_prefix("exports/") {
            if let Some(export_root) = &self.export_root {
                return export_root.join(relative);
            }
        }
        self.artifact_root.join(artifact_id)
    }

    /// 读取文件并在实际打开前执行路径权限检查。
    fn read_text(&self, path: &Path) -> CoreResult<String> {
        self.ensure_read(path)?;
        Ok(fs::read_to_string(path)?)
    }

    /// 获取文档元数据；此函数也会触发读权限检查。
    fn metadata_for_path(
        &self,
        path: &Path,
        explicit_format: Option<DocumentFormat>,
    ) -> CoreResult<DocumentMetadata> {
        self.ensure_read(path)?;
        let metadata = fs::metadata(path)?;
        let format = resolve_format(path, explicit_format)?;
        Ok(DocumentMetadata {
            document_id: document_id_for_path(path)?,
            path: path.to_path_buf(),
            format,
            media_type: format.media_type().to_owned(),
            size_bytes: metadata.len(),
            version: file_version(path)?,
        })
    }

    /// 执行读权限检查，复用 Module 0 的路径沙箱。
    fn ensure_read(&self, path: &Path) -> CoreResult<()> {
        self.permissions.ensure(&PermissionRequest::FileRead {
            path: path.to_path_buf(),
        })
    }

    /// 执行写权限检查，复用 Module 0 的路径沙箱。
    fn ensure_write(&self, path: &Path) -> CoreResult<()> {
        self.permissions.ensure(&PermissionRequest::FileWrite {
            path: path.to_path_buf(),
        })
    }
}

impl DocumentRepository for FileDocumentService {
    /// 读取 Markdown、txt 或 JSON 文档。
    fn open_document(&self, request: DocumentReadRequest) -> CoreResult<DocumentContent> {
        let content = self.read_text(&request.path)?;
        let metadata = self.metadata_for_path(&request.path, request.format)?;
        validate_content_format(&content, metadata.format)?;
        Ok(DocumentContent { metadata, content })
    }

    /// 保存完整文档内容，并返回索引失效通知。
    fn save_document(&self, request: DocumentWriteRequest) -> CoreResult<DocumentWriteReport> {
        self.save_document_with_cancellation(request, &CancellationToken::new())
    }

    /// 保存文档，并在正文替换与 outbox 激活边界响应取消。
    fn preview_patch(&self, patch: &DocumentPatch) -> CoreResult<PatchPreview> {
        self.preview_patch_impl(patch)
    }
}

impl FileDocumentService {
    pub fn save_document_with_cancellation(
        &self,
        request: DocumentWriteRequest,
        cancellation: &CancellationToken,
    ) -> CoreResult<DocumentWriteReport> {
        self.save_document_with_policy(request, cancellation, false)
    }

    /// 仅在目标不存在时创建文档。存在判定与原子替换共享同一独占路径锁，
    /// 供导入等必须显式确认覆盖的入口使用。
    pub fn create_document(
        &self,
        request: DocumentWriteRequest,
    ) -> CoreResult<DocumentWriteReport> {
        self.save_document_with_policy(request, &CancellationToken::new(), true)
    }

    fn save_document_with_policy(
        &self,
        request: DocumentWriteRequest,
        cancellation: &CancellationToken,
        create_only: bool,
    ) -> CoreResult<DocumentWriteReport> {
        cancellation.check()?;
        self.ensure_write(&request.path)?;
        let _project_mutation = self
            .invalidation_outbox
            .acquire_project_mutation("document_save")?;
        // D1-a：base_version 校验与替换在独占锁下，防止并发保存静默丢写。
        let _write_lock = crate::config::store::PathWriteLock::acquire(&request.path)?;
        if create_only && request.path.try_exists()? {
            return Err(CoreError::DocumentAlreadyExists {
                path: request.path.clone(),
            });
        }
        let format = resolve_format(&request.path, request.format)?;
        validate_content_format(&request.content, format)?;
        validate_write_base_version(
            self,
            &request.path,
            Some(format),
            request.base_version.as_deref(),
        )?;
        cancellation.check()?;
        if let Some(parent) = request.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let previous = fs::read(&request.path).ok();
        // Re-check version under lock immediately before replace.
        validate_write_base_version(
            self,
            &request.path,
            Some(format),
            request.base_version.as_deref(),
        )?;
        let result_version = content_version(request.content.as_bytes());
        crate::config::store::atomic_write(&request.path, request.content.as_bytes())?;

        if let Err(error) = cancellation.check() {
            restore_previous_file_if_result(&request.path, previous.as_deref(), &result_version)?;
            return Err(error);
        }

        let metadata = self.metadata_for_path(&request.path, Some(format))?;
        let outbox_event = match self.invalidation_outbox.prepare(
            &metadata.document_id,
            "document_saved",
            &metadata.version,
            false,
        ) {
            Ok(event) => event,
            Err(error) => {
                restore_previous_file_if_result(
                    &request.path,
                    previous.as_deref(),
                    &result_version,
                )?;
                return Err(error);
            }
        };
        if let Err(error) = cancellation.check() {
            restore_previous_file_if_result(&request.path, previous.as_deref(), &result_version)?;
            let _ = self.invalidation_outbox.cancel(&outbox_event);
            return Err(error);
        }
        if let Err(error) = self.invalidation_outbox.activate(&outbox_event) {
            restore_previous_file_if_result(&request.path, previous.as_deref(), &result_version)?;
            let _ = self.invalidation_outbox.cancel(&outbox_event);
            return Err(error);
        }
        Ok(DocumentWriteReport {
            index_invalidation: index_invalidation(&metadata.document_id, "document_saved"),
            metadata,
        })
    }

    /// 预览 patch 结果，不写入文件。
    fn preview_patch_impl(&self, patch: &DocumentPatch) -> CoreResult<PatchPreview> {
        let path = PathBuf::from(&patch.document_id);
        let original = self.open_document(DocumentReadRequest { path, format: None })?;
        validate_base_version(patch, &original.metadata.version)?;
        let patched = apply_patch_to_string(&original.content, patch)?;
        validate_content_format(&patched, original.metadata.format)?;
        Ok(preview_from_contents(
            &original.metadata.document_id,
            &original.metadata.version,
            patch.hunks.len(),
            &original.content,
            &patched,
        ))
    }
}

fn restore_previous_file(path: &Path, previous: Option<&[u8]>) -> CoreResult<()> {
    match previous {
        Some(bytes) => crate::config::store::atomic_write(path, bytes)?,
        None if path.exists() => fs::remove_file(path)?,
        None => {}
    }
    Ok(())
}

/// D1-a：回滚仅在当前正文仍是本 operation 的 result_version 时执行，
/// 避免 B 成功写新正文后 A 失败回滚覆盖新写。
fn restore_previous_file_if_result(
    path: &Path,
    previous: Option<&[u8]>,
    result_version: &str,
) -> CoreResult<()> {
    let Ok(current) = fs::read(path) else {
        return restore_previous_file(path, previous);
    };
    if content_version(&current) != result_version {
        // Concurrent writer advanced past this operation — do not clobber.
        return Ok(());
    }
    restore_previous_file(path, previous)
}

/// 解析文档格式，拒绝不支持的文件扩展名。
fn resolve_format(
    path: &Path,
    explicit_format: Option<DocumentFormat>,
) -> CoreResult<DocumentFormat> {
    explicit_format
        .or_else(|| DocumentFormat::from_path(path))
        .ok_or_else(|| CoreError::validation("unsupported document format"))
}

/// 针对 JSON 文档做结构校验，避免写入无法再解析的数据。
fn validate_content_format(content: &str, format: DocumentFormat) -> CoreResult<()> {
    if format == DocumentFormat::Json {
        serde_json::from_str::<serde_json::Value>(content)?;
    }

    Ok(())
}

/// 文档 id 以 canonicalize 后的真实路径为准，保证同一文件不会产生多个引用。
fn document_id_for_path(path: &Path) -> CoreResult<DocumentId> {
    Ok(path.canonicalize()?.to_string_lossy().into_owned())
}

/// 文件版本使用流式内容 hash，避免同长度快速改写或时间戳精度导致漏检。
fn file_version(path: &Path) -> CoreResult<String> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hash = 0xcbf29ce484222325u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    Ok(format!("{hash:016x}"))
}

/// patch 指定 base_version 时必须与当前文件版本一致。
fn validate_base_version(patch: &DocumentPatch, current_version: &str) -> CoreResult<()> {
    if let Some(base_version) = &patch.base_version {
        if base_version != current_version {
            return Err(CoreError::validation(
                "patch base_version does not match current document",
            ));
        }
    }

    Ok(())
}

fn validate_write_base_version(
    documents: &FileDocumentService,
    path: &Path,
    format: Option<DocumentFormat>,
    base_version: Option<&str>,
) -> CoreResult<()> {
    let Some(base_version) = base_version else {
        return Ok(());
    };
    let current = documents.metadata_for_path(path, format)?;
    if current.version != base_version {
        return Err(CoreError::validation(
            "document base_version does not match current document",
        ));
    }

    Ok(())
}

/// 按 UTF-8 字节范围应用 patch，并拒绝交叠、倒序和非字符边界范围。
fn apply_patch_to_string(original: &str, patch: &DocumentPatch) -> CoreResult<String> {
    let mut hunks = patch.hunks.clone();
    hunks.sort_by_key(|hunk| (hunk.range.start, hunk.range.end));

    let mut result = String::with_capacity(original.len());
    let mut cursor = 0usize;
    for hunk in hunks {
        let start = usize::try_from(hunk.range.start)
            .map_err(|_| CoreError::validation("patch range start is too large"))?;
        let end = usize::try_from(hunk.range.end)
            .map_err(|_| CoreError::validation("patch range end is too large"))?;

        if start > end || start < cursor || end > original.len() {
            return Err(CoreError::validation(
                "patch range is invalid or overlapping",
            ));
        }
        if !original.is_char_boundary(start) || !original.is_char_boundary(end) {
            return Err(CoreError::validation(
                "patch range must align to utf-8 boundaries",
            ));
        }

        // 逐段复制未修改区域，再拼接替换文本，避免对大文本做多次整体替换。
        result.push_str(&original[cursor..start]);
        result.push_str(&hunk.replacement);
        cursor = end;
    }
    result.push_str(&original[cursor..]);

    Ok(result)
}

/// 生成轻量 diff 预览，避免把完整大文本放进运行状态。
fn preview_from_contents(
    document_id: &str,
    base_version: &str,
    hunk_count: usize,
    original: &str,
    patched: &str,
) -> PatchPreview {
    PatchPreview {
        document_id: document_id.to_owned(),
        base_version: base_version.to_owned(),
        result_version: content_version(patched.as_bytes()),
        hunk_count,
        changed: original != patched,
        diff: compact_diff(original, patched),
    }
}

/// 生成索引失效通知，后续后台索引器据此做增量更新。
fn index_invalidation(document_id: &str, reason: &str) -> IndexInvalidation {
    IndexInvalidation {
        document_id: document_id.to_owned(),
        reason: reason.to_owned(),
        full_rebuild_required: false,
    }
}

/// 轻量内容版本，使用固定 FNV-1a 哈希，避免新增依赖和随机种子差异。
fn content_version(bytes: &[u8]) -> String {
    crate::contracts::content_version_for_bytes(bytes)
}

/// 为 UI 预览生成首个变更窗口，而不是返回完整 diff。
fn compact_diff(original: &str, patched: &str) -> String {
    if original == patched {
        return "no changes".to_owned();
    }

    let prefix = common_prefix_boundary(original, patched);
    let suffix = common_suffix_boundary(&original[prefix..], &patched[prefix..]);
    let original_changed = &original[prefix..original.len() - suffix];
    let patched_changed = &patched[prefix..patched.len() - suffix];

    format!(
        "-{}\n+{}",
        clip_preview(original_changed),
        clip_preview(patched_changed)
    )
}

/// 计算公共前缀，并回退到合法 UTF-8 边界。
fn common_prefix_boundary(left: &str, right: &str) -> usize {
    let mut prefix = 0usize;
    for ((left_index, left_char), (right_index, right_char)) in
        left.char_indices().zip(right.char_indices())
    {
        if left_char != right_char {
            break;
        }
        prefix = left_index + left_char.len_utf8();
        if left_index != right_index {
            break;
        }
    }
    prefix
}

/// 计算公共后缀，并确保不会越过已经剥离的前缀。
fn common_suffix_boundary(left: &str, right: &str) -> usize {
    let mut suffix = 0usize;
    for (left_char, right_char) in left.chars().rev().zip(right.chars().rev()) {
        if left_char != right_char {
            break;
        }
        suffix += left_char.len_utf8();
        if suffix >= left.len() || suffix >= right.len() {
            break;
        }
    }
    suffix
}

/// 截断预览文本，保证 diff 预览不会携带大段正文。
fn clip_preview(value: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 240;
    let mut clipped = value.chars().take(MAX_PREVIEW_CHARS).collect::<String>();
    if value.chars().count() > MAX_PREVIEW_CHARS {
        clipped.push_str("...");
    }
    clipped
}

/// Artifact id 只允许相对安全路径片段，避免写出 artifact 根目录。
fn validate_artifact_id(artifact_id: &str) -> CoreResult<()> {
    if artifact_id.trim().is_empty()
        || artifact_id.starts_with('/')
        || artifact_id.contains("..")
        || artifact_id.contains('\\')
    {
        return Err(CoreError::validation("invalid artifact_id"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::artifacts::PatchHunk;
    use crate::contracts::ports::TextRange;

    #[test]
    fn patch_rejects_utf8_boundary_split() {
        let patch = DocumentPatch {
            document_id: "/tmp/example.md".to_owned(),
            base_version: None,
            hunks: vec![PatchHunk {
                range: TextRange { start: 1, end: 2 },
                replacement: "x".to_owned(),
            }],
        };

        let error = apply_patch_to_string("中", &patch).unwrap_err();

        assert!(error.to_string().contains("utf-8"));
    }

    #[test]
    fn compact_diff_is_capped() {
        let original = "a".repeat(400);
        let patched = format!("{}b", "a".repeat(200));

        let diff = compact_diff(&original, &patched);

        assert!(diff.len() < 520);
    }
}
