use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use flate2::read::GzDecoder;
use fs4::FileExt;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tar::Archive;
use zip::ZipArchive;

use crate::contracts::{CoreError, CoreResult};

const MANAGED_BINARY_SENTINEL: &str = "qdrant";
const MANIFEST_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../packaging/qdrant-sidecars.json"
));
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct QdrantReleaseManifest {
    schema_version: u32,
    version: String,
    release_api: String,
    targets: Vec<QdrantReleaseTarget>,
}

#[derive(Debug, Clone, Deserialize)]
struct QdrantReleaseTarget {
    rid: String,
    asset: String,
    binary: String,
    archive_sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    #[serde(default)]
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubReleaseAsset {
    name: String,
    digest: Option<String>,
    browser_download_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ManagedQdrantMetadata {
    schema_version: u32,
    product: String,
    version: String,
    rid: String,
    asset: String,
    source_url: String,
    archive_sha256: String,
    binary_sha256: String,
}

/// 将默认的 `qdrant` 哨兵解析为 Ariadne 管理的固定版本二进制。
/// 显式配置其它路径时不联网、不改写用户选择。
pub fn resolve_qdrant_binary_path(configured: &Path) -> CoreResult<PathBuf> {
    if configured != Path::new(MANAGED_BINARY_SENTINEL) {
        return Ok(configured.to_path_buf());
    }

    ensure_managed_qdrant_binary()
}

/// 下载、校验并缓存当前平台的 Qdrant。缓存完整时不会发出网络请求。
pub fn ensure_managed_qdrant_binary() -> CoreResult<PathBuf> {
    let manifest = release_manifest()?;
    let rid = current_release_rid()?;
    let target = manifest
        .targets
        .iter()
        .find(|target| target.rid == rid)
        .cloned()
        .ok_or_else(|| provisioning_error(format!("release manifest does not support {rid}")))?;
    validate_target(&manifest, &target)?;

    let qdrant_cache_root = managed_cache_root()?.join("qdrant");
    let current_version_directory = format!("v{}", manifest.version);
    let cache_directory = qdrant_cache_root
        .join(&current_version_directory)
        .join(&target.rid);
    fs::create_dir_all(&cache_directory)?;
    let binary_path = cache_directory.join(&target.binary);
    let metadata_path = cache_directory.join("qdrant-sidecar.json");
    if cached_binary_is_valid(&binary_path, &metadata_path, &manifest, &target)? {
        cleanup_old_qdrant_versions(&qdrant_cache_root, &current_version_directory);
        return Ok(binary_path);
    }

    let lock_path = cache_directory.join("provision.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    FileExt::lock_exclusive(&lock)?;

    let mut temporary_paths = Vec::new();
    let result = (|| {
        if cached_binary_is_valid(&binary_path, &metadata_path, &manifest, &target)? {
            return Ok(binary_path.clone());
        }

        let client = Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(600))
            .user_agent(format!(
                "Ariadne/{}/QdrantProvisioner",
                crate::PRODUCT_VERSION
            ))
            .build()
            .map_err(|error| provisioning_error(format!("cannot create HTTP client: {error}")))?;
        let (expected_archive_sha256, source_url) =
            resolve_download_source(&client, &manifest, &target)?;
        let local_archive = std::env::var_os("ARIADNE_QDRANT_ARCHIVE").map(PathBuf::from);

        let archive_path = if let Some(path) = local_archive {
            if !path.is_file() {
                return Err(provisioning_error(format!(
                    "ARIADNE_QDRANT_ARCHIVE does not name a file: {}",
                    path.display()
                )));
            }
            path
        } else {
            let (path, mut file) = create_temporary_file(&cache_directory, "archive")?;
            temporary_paths.push(path.clone());
            download_archive(&client, &source_url, &mut file)?;
            file.sync_all()?;
            path
        };

        let actual_archive_sha256 = sha256_file(&archive_path)?;
        if actual_archive_sha256 != expected_archive_sha256 {
            return Err(provisioning_error(format!(
                "Qdrant archive SHA-256 mismatch: expected {expected_archive_sha256}, got {actual_archive_sha256}"
            )));
        }

        let (temporary_binary_path, mut temporary_binary) =
            create_temporary_file(&cache_directory, "binary")?;
        temporary_paths.push(temporary_binary_path.clone());
        extract_binary(
            &archive_path,
            &target.asset,
            &target.binary,
            &mut temporary_binary,
        )?;
        temporary_binary.flush()?;
        temporary_binary.sync_all()?;
        drop(temporary_binary);
        make_executable(&temporary_binary_path)?;
        verify_qdrant_version(&temporary_binary_path, &manifest.version)?;
        let binary_sha256 = sha256_file(&temporary_binary_path)?;

        if binary_path.exists() {
            fs::remove_file(&binary_path)?;
        }
        fs::rename(&temporary_binary_path, &binary_path)?;

        let metadata = ManagedQdrantMetadata {
            schema_version: 1,
            product: "qdrant".to_owned(),
            version: manifest.version.clone(),
            rid: target.rid.clone(),
            asset: target.asset.clone(),
            source_url,
            archive_sha256: actual_archive_sha256,
            binary_sha256,
        };
        write_metadata_atomically(&metadata_path, &metadata)?;
        Ok(binary_path.clone())
    })();

    let _ = FileExt::unlock(&lock);
    for path in temporary_paths {
        let _ = fs::remove_file(path);
    }
    if result.is_ok() {
        cleanup_old_qdrant_versions(&qdrant_cache_root, &current_version_directory);
    }
    result
}

fn release_manifest() -> CoreResult<QdrantReleaseManifest> {
    let manifest: QdrantReleaseManifest = serde_json::from_str(MANIFEST_JSON)?;
    if manifest.schema_version != 1 {
        return Err(provisioning_error(
            "unsupported Qdrant release manifest schema",
        ));
    }
    Ok(manifest)
}

fn validate_target(
    manifest: &QdrantReleaseManifest,
    target: &QdrantReleaseTarget,
) -> CoreResult<()> {
    if !is_release_version(&manifest.version) {
        return Err(provisioning_error("Qdrant release version is invalid"));
    }
    if target.asset.contains('/') || target.asset.contains('\\') || target.asset.trim().is_empty() {
        return Err(provisioning_error("Qdrant release asset name is invalid"));
    }
    if Path::new(&target.binary).components().count() != 1 || target.binary.trim().is_empty() {
        return Err(provisioning_error("Qdrant binary name is invalid"));
    }
    if let Some(digest) = &target.archive_sha256 {
        if !is_sha256(digest) {
            return Err(provisioning_error(
                "pinned Qdrant archive digest is invalid",
            ));
        }
    }
    Ok(())
}

fn resolve_download_source(
    client: &Client,
    manifest: &QdrantReleaseManifest,
    target: &QdrantReleaseTarget,
) -> CoreResult<(String, String)> {
    if let Some(digest) = &target.archive_sha256 {
        let url = format!(
            "https://github.com/qdrant/qdrant/releases/download/v{}/{}",
            manifest.version, target.asset
        );
        return Ok((digest.to_ascii_lowercase(), url));
    }

    if !manifest
        .release_api
        .starts_with("https://api.github.com/repos/qdrant/qdrant/releases/")
    {
        return Err(provisioning_error("Qdrant release API URL is invalid"));
    }
    let mut request = client
        .get(&manifest.release_api)
        .header(ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(token) = std::env::var_os("GITHUB_TOKEN").filter(|value| !value.is_empty()) {
        let value = format!("Bearer {}", token.to_string_lossy());
        request = request.header(AUTHORIZATION, value);
    }
    let release = request
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| {
            provisioning_error(format!("cannot read Qdrant release metadata: {error}"))
        })?
        .json::<GithubRelease>()
        .map_err(|error| provisioning_error(format!("invalid Qdrant release metadata: {error}")))?;
    let mut matches = release
        .assets
        .into_iter()
        .filter(|asset| asset.name == target.asset);
    let asset = matches
        .next()
        .ok_or_else(|| provisioning_error("Qdrant release asset is missing"))?;
    if matches.next().is_some() {
        return Err(provisioning_error("Qdrant release asset is duplicated"));
    }
    if !asset
        .browser_download_url
        .starts_with("https://github.com/qdrant/qdrant/releases/download/")
    {
        return Err(provisioning_error("Qdrant release asset URL is invalid"));
    }
    let digest = asset
        .digest
        .and_then(|value| value.strip_prefix("sha256:").map(str::to_owned))
        .filter(|value| is_sha256(value))
        .ok_or_else(|| provisioning_error("Qdrant release asset has no valid SHA-256 digest"))?;
    Ok((digest.to_ascii_lowercase(), asset.browser_download_url))
}

fn download_archive(client: &Client, url: &str, output: &mut File) -> CoreResult<()> {
    let response = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| provisioning_error(format!("cannot download Qdrant: {error}")))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARCHIVE_BYTES)
    {
        return Err(provisioning_error(
            "Qdrant archive exceeds the download limit",
        ));
    }
    let mut bounded = response.take(MAX_ARCHIVE_BYTES + 1);
    let copied = io::copy(&mut bounded, output)?;
    if copied > MAX_ARCHIVE_BYTES {
        return Err(provisioning_error(
            "Qdrant archive exceeds the download limit",
        ));
    }
    Ok(())
}

fn extract_binary(
    archive_path: &Path,
    archive_name: &str,
    binary_name: &str,
    output: &mut File,
) -> CoreResult<()> {
    if archive_name.to_ascii_lowercase().ends_with(".zip") {
        return extract_zip_binary(archive_path, binary_name, output);
    }
    extract_tar_binary(archive_path, binary_name, output)
}

fn extract_zip_binary(archive_path: &Path, binary_name: &str, output: &mut File) -> CoreResult<()> {
    let file = File::open(archive_path)?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| provisioning_error(format!("invalid Qdrant ZIP archive: {error}")))?;
    let mut found = false;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| provisioning_error(format!("invalid Qdrant ZIP entry: {error}")))?;
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| provisioning_error("Qdrant ZIP contains an unsafe path"))?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(provisioning_error("Qdrant ZIP contains a symbolic link"));
        }
        if !entry.is_dir() && enclosed.file_name() == Some(OsStr::new(binary_name)) {
            if found {
                return Err(provisioning_error("Qdrant ZIP contains duplicate binaries"));
            }
            io::copy(&mut entry, output)?;
            found = true;
        }
    }
    if !found {
        return Err(provisioning_error(
            "Qdrant ZIP does not contain the expected binary",
        ));
    }
    Ok(())
}

fn extract_tar_binary(archive_path: &Path, binary_name: &str, output: &mut File) -> CoreResult<()> {
    let file = File::open(archive_path)?;
    let mut archive = Archive::new(GzDecoder::new(file));
    let mut found = false;
    for entry in archive
        .entries()
        .map_err(|error| provisioning_error(format!("invalid Qdrant TAR archive: {error}")))?
    {
        let mut entry = entry
            .map_err(|error| provisioning_error(format!("invalid Qdrant TAR entry: {error}")))?;
        let path = entry
            .path()
            .map_err(|error| provisioning_error(format!("invalid Qdrant TAR path: {error}")))?;
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(provisioning_error("Qdrant TAR contains an unsafe path"));
        }
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            return Err(provisioning_error("Qdrant TAR contains a link"));
        }
        if entry_type.is_file() && path.file_name() == Some(OsStr::new(binary_name)) {
            if found {
                return Err(provisioning_error("Qdrant TAR contains duplicate binaries"));
            }
            io::copy(&mut entry, output)?;
            found = true;
        }
    }
    if !found {
        return Err(provisioning_error(
            "Qdrant TAR does not contain the expected binary",
        ));
    }
    Ok(())
}

fn cached_binary_is_valid(
    binary_path: &Path,
    metadata_path: &Path,
    manifest: &QdrantReleaseManifest,
    target: &QdrantReleaseTarget,
) -> CoreResult<bool> {
    if !binary_path.is_file() || !metadata_path.is_file() {
        return Ok(false);
    }
    let metadata = match fs::read_to_string(metadata_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<ManagedQdrantMetadata>(&raw).ok())
    {
        Some(metadata) => metadata,
        None => return Ok(false),
    };
    if metadata.schema_version != 1
        || metadata.product != "qdrant"
        || metadata.version != manifest.version
        || metadata.rid != target.rid
        || metadata.asset != target.asset
        || !is_sha256(&metadata.archive_sha256)
        || !is_sha256(&metadata.binary_sha256)
        || target
            .archive_sha256
            .as_ref()
            .is_some_and(|digest| !digest.eq_ignore_ascii_case(&metadata.archive_sha256))
    {
        return Ok(false);
    }
    if sha256_file(binary_path)? != metadata.binary_sha256 {
        return Ok(false);
    }
    Ok(verify_qdrant_version(binary_path, &manifest.version).is_ok())
}

fn verify_qdrant_version(binary_path: &Path, expected_version: &str) -> CoreResult<()> {
    let output = Command::new(binary_path)
        .arg("--version")
        .output()
        .map_err(|error| provisioning_error(format!("cannot execute cached Qdrant: {error}")))?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() || !text.split_whitespace().any(|part| part == expected_version) {
        return Err(provisioning_error(format!(
            "Qdrant binary version does not match {expected_version}"
        )));
    }
    Ok(())
}

fn managed_cache_root() -> CoreResult<PathBuf> {
    if let Some(path) = non_empty_env_path("ARIADNE_QDRANT_CACHE_DIR") {
        return Ok(path);
    }

    #[cfg(target_os = "windows")]
    if let Some(path) = non_empty_env_path("LOCALAPPDATA") {
        return Ok(path.join("Ariadne").join("Cache"));
    }
    #[cfg(target_os = "macos")]
    if let Some(path) = non_empty_env_path("HOME") {
        return Ok(path.join("Library").join("Caches").join("Ariadne"));
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Some(path) = non_empty_env_path("XDG_CACHE_HOME") {
            return Ok(path.join("ariadne"));
        }
        if let Some(path) = non_empty_env_path("HOME") {
            return Ok(path.join(".cache").join("ariadne"));
        }
    }

    Err(provisioning_error(
        "cannot determine a user cache directory for Qdrant",
    ))
}

fn non_empty_env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn current_release_rid() -> CoreResult<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("linux-x64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("windows", "x86_64") => Ok("win-x64"),
        ("macos", "x86_64") => Ok("osx-x64"),
        ("macos", "aarch64") => Ok("osx-arm64"),
        (os, arch) => Err(provisioning_error(format!(
            "Qdrant automatic provisioning is unsupported on {os}/{arch}"
        ))),
    }
}

fn create_temporary_file(directory: &Path, label: &str) -> CoreResult<(PathBuf, File)> {
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for attempt in 0..32_u32 {
        let path = directory.join(format!(
            ".qdrant-{label}-{}-{epoch}-{attempt}.tmp",
            std::process::id()
        ));
        match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(provisioning_error(
        "cannot allocate a Qdrant temporary file",
    ))
}

fn write_metadata_atomically(path: &Path, metadata: &ManagedQdrantMetadata) -> CoreResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| provisioning_error("Qdrant metadata path has no parent"))?;
    let (temporary_path, mut temporary) = create_temporary_file(parent, "metadata")?;
    let result = (|| {
        serde_json::to_writer_pretty(&mut temporary, metadata)?;
        temporary.write_all(b"\n")?;
        temporary.sync_all()?;
        drop(temporary);
        if path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(&temporary_path, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary_path);
    }
    result
}

fn cleanup_old_qdrant_versions(qdrant_cache_root: &Path, current_version: &str) {
    let Ok(entries) = fs::read_dir(qdrant_cache_root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if name == current_version
            || !name.starts_with('v')
            || !is_release_version(&name[1..])
            || !file_type.is_dir()
            || file_type.is_symlink()
        {
            continue;
        }
        let _ = fs::remove_dir_all(entry.path());
    }
}

fn sha256_file(path: &Path) -> CoreResult<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let digest = digest.finalize();
    Ok(format!("{digest:x}"))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> CoreResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> CoreResult<()> {
    Ok(())
}

fn is_release_version(value: &str) -> bool {
    !value.is_empty()
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn provisioning_error(message: impl Into<String>) -> CoreError {
    CoreError::External {
        service: "qdrant_provisioning".to_owned(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_manifest_covers_every_release_rid_once() {
        let manifest = release_manifest().unwrap();
        let mut rids = manifest
            .targets
            .iter()
            .map(|target| target.rid.as_str())
            .collect::<Vec<_>>();
        rids.sort_unstable();
        rids.dedup();

        assert_eq!(manifest.targets.len(), 5);
        assert_eq!(
            rids,
            vec![
                "linux-arm64",
                "linux-x64",
                "osx-arm64",
                "osx-x64",
                "win-x64"
            ]
        );
        for target in &manifest.targets {
            validate_target(&manifest, target).unwrap();
        }
    }

    #[test]
    fn explicit_qdrant_path_bypasses_managed_provisioning() {
        let configured = Path::new("/opt/ariadne/qdrant-custom");

        assert_eq!(resolve_qdrant_binary_path(configured).unwrap(), configured);
    }

    #[test]
    fn current_platform_maps_to_release_matrix() {
        let rid = current_release_rid().unwrap();
        assert!([
            "linux-arm64",
            "linux-x64",
            "osx-arm64",
            "osx-x64",
            "win-x64"
        ]
        .contains(&rid));
    }

    #[test]
    fn old_managed_versions_are_cleaned_without_touching_unowned_entries() {
        let directory = tempfile::tempdir().unwrap();
        let qdrant_root = directory.path().join("qdrant");
        let current = qdrant_root.join("v1.18.2");
        let old = qdrant_root.join("v1.17.0");
        let unrelated = qdrant_root.join("manual");
        fs::create_dir_all(&current).unwrap();
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&unrelated).unwrap();

        cleanup_old_qdrant_versions(&qdrant_root, "v1.18.2");

        assert!(current.is_dir());
        assert!(!old.exists());
        assert!(unrelated.is_dir());
    }
}
