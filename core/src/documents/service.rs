use std::fs;
use std::path::{Path, PathBuf};

use crate::contracts::artifacts::{ArtifactDescriptor, DocumentPatch};
use crate::contracts::permissions::{PermissionPolicy, PermissionRequest};
use crate::contracts::ports::PortValue;
use crate::contracts::{CoreError, CoreResult};
use crate::documents::models::{
    ArtifactWriteReport, ArtifactWriteRequest, DocumentContent, DocumentFormat, DocumentId,
    DocumentMetadata, DocumentReadRequest, DocumentRepository, DocumentWriteReport,
    DocumentWriteRequest, IndexInvalidation, PatchApplyReport, PatchCheckpointRequest,
    PatchPreview,
};
use crate::git::GitService;

/// 文档和 Artifact 的文件系统服务。
#[derive(Debug, Clone)]
pub struct FileDocumentService {
    permissions: PermissionPolicy,
    artifact_root: PathBuf,
}

impl FileDocumentService {
    /// 创建文件系统文档服务。
    pub fn new(permissions: PermissionPolicy, artifact_root: impl Into<PathBuf>) -> Self {
        Self {
            permissions,
            artifact_root: artifact_root.into(),
        }
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
        let path = PathBuf::from(&patch.document_id);
        let original = self.open_document(DocumentReadRequest {
            path: path.clone(),
            format: None,
        })?;
        validate_base_version(patch, &original.metadata.version)?;

        let patched = apply_patch_to_string(&original.content, patch)?;
        let preview = preview_from_contents(
            &original.metadata.document_id,
            &original.metadata.version,
            patch.hunks.len(),
            &original.content,
            &patched,
        );

        self.ensure_write(&path)?;
        validate_content_format(&patched, original.metadata.format)?;
        fs::write(&path, patched.as_bytes())?;

        let metadata = self.metadata_for_path(&path, Some(original.metadata.format))?;
        let index_invalidation = index_invalidation(&metadata.document_id, "patch_applied");
        let checkpoint = match (git, checkpoint_request) {
            (Some(git), Some(request)) => {
                Some(git.create_checkpoint(&request.node_id, request.message.as_deref())?)
            }
            _ => None,
        };

        Ok(PatchApplyReport {
            preview,
            metadata,
            index_invalidation,
            checkpoint,
        })
    }

    /// 写入 Artifact 文件并返回可传递的 artifact 描述。
    pub fn write_artifact(&self, request: ArtifactWriteRequest) -> CoreResult<ArtifactWriteReport> {
        validate_artifact_id(&request.artifact_id)?;
        let path = self.artifact_root.join(&request.artifact_id);
        self.ensure_write(&path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &request.bytes)?;

        let descriptor = ArtifactDescriptor {
            artifact_id: request.artifact_id,
            kind: request.kind,
            media_type: request.media_type,
            storage_uri: format!("file://{}", path.display()),
            size_bytes: Some(request.bytes.len() as u64),
            checksum: Some(content_version(&request.bytes)),
            created_by_run: None,
            created_by_node: None,
            sources: Vec::new(),
            metadata: request.metadata,
        };

        Ok(ArtifactWriteReport { descriptor })
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
            version: file_version(path, metadata.len())?,
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
        self.ensure_write(&request.path)?;
        let format = resolve_format(&request.path, request.format)?;
        validate_content_format(&request.content, format)?;
        if let Some(parent) = request.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&request.path, request.content.as_bytes())?;

        let metadata = self.metadata_for_path(&request.path, Some(format))?;
        Ok(DocumentWriteReport {
            index_invalidation: index_invalidation(&metadata.document_id, "document_saved"),
            metadata,
        })
    }

    /// 预览 patch 结果，不写入文件。
    fn preview_patch(&self, patch: &DocumentPatch) -> CoreResult<PatchPreview> {
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

/// 文件版本号由修改时间和长度组成，足够支撑乐观并发检查。
fn file_version(path: &Path, size_bytes: u64) -> CoreResult<String> {
    let modified = fs::metadata(path)?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            CoreError::validation(format!("file modified time is invalid: {error}"))
        })?;
    Ok(format!(
        "{}.{:09}-{}",
        modified.as_secs(),
        modified.subsec_nanos(),
        size_bytes
    ))
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
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
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
