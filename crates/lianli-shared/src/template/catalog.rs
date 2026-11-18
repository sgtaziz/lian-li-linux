//! Catalog downloads verify SHA-256 hashes before writing each staged file.

pub use super::catalog_review::{
    remove_catalog_directory, review_catalog_directory, CatalogDirectoryReview, CatalogReviewedFile,
};
pub use super::catalog_storage::{inspect_storage, CatalogStorageEntry};
use crate::sensors::SensorInfo;
use crate::template::{resolve_sensor_categories, LcdTemplate, SensorBindings};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

const CATALOG_BASE_URL: &str =
    "https://raw.githubusercontent.com/sgtaziz/lian-li-linux/main/templates";
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_METADATA_BYTES: usize = 1024 * 1024;
const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;
const MAX_INSTALL_BYTES: usize = 256 * 1024 * 1024;
const MAX_FILES: usize = 128;
const MAX_VARIANT_LABEL_CHARS: usize = 64;
const INSTALL_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);

fn fetch_bytes(url: &str, limit: usize) -> Result<Vec<u8>> {
    download(&client()?, url, limit, FETCH_TIMEOUT, &|| false)
}

fn download(
    client: &reqwest::blocking::Client,
    url: &str,
    limit: usize,
    timeout: Duration,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<u8>> {
    check_cancelled(cancelled)?;
    let resp = client
        .get(url)
        .timeout(timeout)
        .send()
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("status for {url}"))?;
    if resp
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        bail!("download exceeds the {limit}-byte limit");
    }
    read_bounded_cancellable(resp, limit, cancelled).with_context(|| format!("body of {url}"))
}

#[cfg(test)]
fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>> {
    read_bounded_cancellable(reader, limit, &|| false)
}

fn check_cancelled(cancelled: &impl Fn() -> bool) -> Result<()> {
    if cancelled() {
        bail!("catalog installation cancelled during daemon shutdown");
    }
    Ok(())
}

fn read_bounded_cancellable(
    mut reader: impl Read,
    limit: usize,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        check_cancelled(cancelled)?;
        let count = reader.read(&mut buffer)?;
        check_cancelled(cancelled)?;
        if count == 0 {
            break;
        }
        if count > limit - bytes.len() {
            bail!("download exceeds the {limit}-byte limit");
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(bytes)
}

pub(super) fn validate_relative_path(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 512
        || value.split('/').count() > 32
        || value.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || !part.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b' ')
                })
        })
    {
        bail!("catalog paths must be relative, with plain file names and no traversal");
    }
    Ok(())
}

fn validate_catalog_template(template: &CatalogTemplate) -> Result<()> {
    validate_relative_path(&template.id).context("invalid template ID")?;
    if template.id.contains('/') || template.id.starts_with('.') || template.id.len() > 128 {
        bail!("template ID must be a single visible directory name of at most 128 bytes");
    }
    for label in [&template.group, &template.variant].into_iter().flatten() {
        if label.trim().is_empty() || label.chars().count() > MAX_VARIANT_LABEL_CHARS {
            bail!("catalog group and variant labels must have 1 to {MAX_VARIANT_LABEL_CHARS} characters");
        }
    }
    validate_relative_path(&template.folder).context("invalid catalog folder")?;
    validate_relative_path(&template.template_file).context("invalid template file")?;
    validate_relative_path(&template.preview).context("invalid preview file")?;
    if template.files.len() > MAX_FILES {
        bail!("catalog template exceeds the {MAX_FILES}-file limit");
    }
    let mut paths = HashSet::from([template.template_file.as_str()]);
    for file in &template.files {
        validate_relative_path(&file.path).context("invalid asset file")?;
        if !paths.insert(&file.path) {
            bail!("catalog contains duplicate file paths");
        }
    }
    for path in &paths {
        if path.split('/').next() == Some(super::catalog_storage::RECEIPT_NAME) {
            bail!("catalog file conflicts with the reserved ownership receipt");
        }
        let mut ancestor = Path::new(path).parent();
        while let Some(parent) = ancestor {
            if paths.contains(parent.to_str().unwrap_or_default()) {
                bail!("catalog file path conflicts with a parent directory");
            }
            ancestor = parent.parent();
        }
    }
    for hash in std::iter::once(&template.template_sha256)
        .chain(std::iter::once(&template.preview_sha256))
        .chain(template.files.iter().map(|file| &file.sha256))
    {
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("catalog SHA-256 must contain exactly 64 hexadecimal digits");
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CatalogManifest {
    pub schema_version: u32,
    pub templates: Vec<CatalogTemplate>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CatalogTemplate {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    pub min_daemon_version: String,
    pub folder: String,
    pub template_file: String,
    pub template_sha256: String,
    pub preview: String,
    pub preview_sha256: String,
    #[serde(default)]
    pub base_width: u32,
    #[serde(default)]
    pub base_height: u32,
    #[serde(default)]
    pub rotated: bool,
    #[serde(default)]
    pub files: Vec<CatalogFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CatalogFile {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogInstallStatus {
    pub operation_id: String,
    pub template_id: String,
    pub finished: bool,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_ui: Option<crate::ui_message::UiMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogCleanupReview {
    pub contents: CatalogDirectoryReview,
    pub references: CatalogStorageEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogReviewStatus {
    pub operation_id: String,
    pub finished: bool,
    pub review: Option<CatalogCleanupReview>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_ui: Option<crate::ui_message::UiMessage>,
    #[serde(default)]
    pub removed: bool,
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent(concat!("lianli-gui/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("building reqwest client")
}

fn asset_url(folder: &str, path: &str) -> String {
    format!("{CATALOG_BASE_URL}/assets/{folder}/{path}")
}

pub fn fetch_manifest() -> Result<CatalogManifest> {
    let url = format!("{CATALOG_BASE_URL}/default_templates.json");
    debug!("fetching template catalog manifest from {url}");
    let bytes = fetch_bytes(&url, MAX_METADATA_BYTES)?;
    let manifest: CatalogManifest =
        serde_json::from_slice(&bytes).context("parsing manifest JSON")?;
    if manifest.schema_version != 1 {
        bail!(
            "Unsupported catalog schema version {}. Update Lian Li Linux.",
            manifest.schema_version
        );
    }
    if manifest.templates.len() > 128 {
        bail!("catalog exceeds the 128-template limit");
    }
    let mut ids = HashSet::new();
    for template in &manifest.templates {
        validate_catalog_template(template)?;
        if !ids.insert(&template.id) {
            bail!("catalog contains duplicate template IDs");
        }
    }
    info!(
        "fetched catalog with {} template(s)",
        manifest.templates.len()
    );
    Ok(manifest)
}

pub fn fetch_preview(template: &CatalogTemplate) -> Result<Vec<u8>> {
    validate_catalog_template(template)?;
    let url = asset_url(&template.folder, &template.preview);
    let bytes = fetch_bytes(&url, MAX_METADATA_BYTES)?;
    verify_sha256(&bytes, &template.preview_sha256).context("preview sha256 mismatch")?;
    Ok(bytes)
}

pub fn is_supported(template: &CatalogTemplate, daemon_version: &str) -> bool {
    version_ge(daemon_version, &template.min_daemon_version)
}

pub struct PreparedTemplate {
    template: LcdTemplate,
    directory: AssetDirectory,
}

struct AssetDirectory {
    staging: tempfile::TempDir,
    _root: std::fs::File,
    target: PathBuf,
}

impl PreparedTemplate {
    pub fn template(&self) -> &LcdTemplate {
        &self.template
    }

    /// Preserve verified assets after persistence or when publication is uncertain.
    pub fn commit(self) -> LcdTemplate {
        let _ = self.directory.staging.keep();
        self.template
    }
}

#[cfg(test)]
fn prepare_directory(config_dir: &Path, id: &str) -> Result<AssetDirectory> {
    prepare_directory_checked(config_dir, id, &|| false)
}

fn prepare_directory_checked(
    config_dir: &Path,
    id: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<AssetDirectory> {
    let root_path = templates_install_dir(config_dir)?;
    let root = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&root_path)
        .context("opening catalog installation directory")?;
    super::catalog_storage::check_capacity(&root, MAX_INSTALL_BYTES as u64, cancelled)?;
    let pinned = PathBuf::from(format!("/proc/self/fd/{}", root.as_raw_fd()));
    let staging = tempfile::Builder::new()
        .prefix(&format!("catalog-{id}-"))
        .tempdir_in(&pinned)
        .context("creating private catalog asset directory")?;
    let target = root_path.join(
        staging
            .path()
            .file_name()
            .context("missing asset directory name")?,
    );
    Ok(AssetDirectory {
        staging,
        _root: root,
        target,
    })
}

fn remaining_download_time(deadline: Instant, now: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(now)
        .filter(|remaining| !remaining.is_zero())
        .map(|remaining| remaining.min(FETCH_TIMEOUT))
        .context("catalog installation exceeded its two-minute download deadline")
}

pub fn install_template(
    template: &CatalogTemplate,
    bindings: &SensorBindings,
    sensors: &[SensorInfo],
    config_dir: &Path,
    cancelled: impl Fn() -> bool,
) -> Result<PreparedTemplate> {
    check_cancelled(&cancelled)?;
    validate_catalog_template(template)?;
    if !is_supported(template, env!("CARGO_PKG_VERSION")) {
        bail!("catalog template requires a newer daemon version");
    }
    let deadline = Instant::now() + INSTALL_DOWNLOAD_TIMEOUT;
    let client = client()?;
    let directory = prepare_directory_checked(config_dir, &template.id, &cancelled)?;
    let staging_dir = directory.staging.path();
    let receipt_bytes = super::catalog_storage::write_receipt(staging_dir, template)?;
    directory
        ._root
        .sync_all()
        .context("syncing catalog preparation ownership")?;

    let tpl_url = asset_url(&template.folder, &template.template_file);
    let tpl_bytes = download(
        &client,
        &tpl_url,
        MAX_METADATA_BYTES,
        remaining_download_time(deadline, Instant::now())?,
        &cancelled,
    )?;
    verify_sha256(&tpl_bytes, &template.template_sha256)
        .context("template.json sha256 mismatch")?;
    let mut lcd_template: LcdTemplate = serde_json::from_slice(&tpl_bytes)
        .context("parsing downloaded template.json after sha256 verify")?;
    validate_template_assets(&lcd_template, template)?;
    lcd_template.validate().map_err(|e| anyhow!("{e}"))?;
    let template_path = staging_dir.join(&template.template_file);
    if let Some(parent) = template_path.parent() {
        std::fs::create_dir_all(parent).context("creating template directory")?;
    }
    write_staged_file(&template_path, &tpl_bytes).context("writing staged template.json")?;

    let mut remaining = MAX_INSTALL_BYTES - tpl_bytes.len() - receipt_bytes;
    for file in &template.files {
        let url = asset_url(&template.folder, &file.path);
        let bytes = download(
            &client,
            &url,
            MAX_ASSET_BYTES.min(remaining),
            remaining_download_time(deadline, Instant::now())?,
            &cancelled,
        )?;
        remaining -= bytes.len();
        verify_sha256(&bytes, &file.sha256)
            .with_context(|| format!("{} sha256 mismatch", file.path))?;
        let dest = staging_dir.join(&file.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).context("creating asset directory")?;
        }
        write_staged_file(&dest, &bytes).with_context(|| format!("writing {}", dest.display()))?;
    }

    rewrite_asset_paths(&mut lcd_template, &directory.target);
    resolve_sensor_categories(&mut lcd_template, bindings, sensors);
    lcd_template.validate().map_err(|e| anyhow!("{e}"))?;

    check_cancelled(&cancelled)?;
    sync_directory(staging_dir)?;
    check_cancelled(&cancelled)?;
    directory
        ._root
        .sync_all()
        .context("syncing catalog installation directory")?;
    Ok(PreparedTemplate {
        template: lcd_template,
        directory,
    })
}

fn write_staged_file(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    file.write_all(bytes)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    for entry in std::fs::read_dir(path).context("reading staged catalog directory")? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sync_directory(&entry.path())?;
        } else {
            std::fs::File::open(entry.path())?
                .sync_all()
                .context("syncing catalog asset")?;
        }
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    std::fs::File::open(path)?
        .sync_all()
        .context("syncing catalog asset directory")
}

fn validate_template_assets(template: &LcdTemplate, catalog: &CatalogTemplate) -> Result<()> {
    if template.id != catalog.id {
        bail!("downloaded template ID does not match the catalog");
    }
    let files: HashSet<&str> = catalog
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    for dependency in crate::media_dependencies::template_dependencies(template) {
        let path = dependency
            .path
            .to_str()
            .context("asset path is not UTF-8")?;
        validate_relative_path(path)?;
        if !files.contains(path) {
            bail!("template references an asset missing from its catalog file list");
        }
    }
    Ok(())
}

fn rewrite_asset_paths(template: &mut LcdTemplate, base: &std::path::Path) {
    crate::media_dependencies::map_template_paths(template, |path| {
        if path.is_relative() {
            *path = base.join(&*path);
        }
    });
}

fn verify_sha256(bytes: &[u8], expected_hex: &str) -> Result<()> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected_hex) {
        bail!("sha256 mismatch: expected {expected_hex}, got {actual}");
    }
    Ok(())
}

fn templates_install_dir(config_dir: &Path) -> Result<PathBuf> {
    let config_dir = config_dir
        .canonicalize()
        .context("resolving daemon configuration directory")?;
    let dir = config_dir.join("templates");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

fn version_ge(have: &str, need: &str) -> bool {
    let parse = |s: &str| -> (u32, u32, u32) {
        let mut it = s.trim_start_matches('v').split('.').map(|p| {
            p.split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|d| d.parse::<u32>().ok())
                .unwrap_or(0)
        });
        (
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
        )
    };
    let h = parse(have);
    let n = parse(need);
    match h.0.cmp(&n.0) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => match h.1.cmp(&n.1) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => h.2 >= n.2,
        },
    }
}

pub fn filter_supported(
    templates: Vec<CatalogTemplate>,
    daemon_version: &str,
) -> Vec<CatalogTemplate> {
    let (supported, unsupported): (Vec<_>, Vec<_>) = templates
        .into_iter()
        .partition(|t| is_supported(t, daemon_version));
    if !unsupported.is_empty() {
        warn!(
            "skipping {} template(s) that require a newer daemon version",
            unsupported.len()
        );
    }
    supported
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_publication_normalizes_permissions_without_overwriting_files() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o777)).unwrap();
        let path = nested.join("asset");
        write_staged_file(&path, b"first").unwrap();
        assert!(write_staged_file(&path, b"second").is_err());
        sync_directory(root.path()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert_eq!(
            std::fs::metadata(nested).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn cancellation_stops_before_reading_and_discards_a_chunk_cancelled_during_read() {
        use std::cell::Cell;
        struct CancellingReader<'a>(&'a Cell<bool>, &'a Cell<usize>);
        impl Read for CancellingReader<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                self.1.set(self.1.get() + 1);
                buffer[0] = b'x';
                self.0.set(true);
                Ok(1)
            }
        }
        let cancelled = Cell::new(true);
        let reads = Cell::new(0);
        assert!(
            read_bounded_cancellable(CancellingReader(&cancelled, &reads), 8, &|| cancelled.get())
                .is_err()
        );
        assert_eq!(reads.get(), 0);
        cancelled.set(false);
        assert!(
            read_bounded_cancellable(CancellingReader(&cancelled, &reads), 8, &|| cancelled.get())
                .is_err()
        );
        assert_eq!(reads.get(), 1);
    }

    fn catalog() -> CatalogManifest {
        serde_json::from_str(include_str!("../../../../templates/default_templates.json")).unwrap()
    }

    #[test]
    fn bounded_download_accepts_exact_limit_and_rejects_overflow() {
        assert_eq!(read_bounded(&b"1234"[..], 4).unwrap(), b"1234");
        assert!(read_bounded(&b"12345"[..], 4).is_err());
        assert!(read_bounded(&b"1"[..], 0).is_err());
        assert!(read_bounded(&b""[..], 0).unwrap().is_empty());
    }

    #[test]
    fn download_deadline_caps_each_request_and_rejects_expiry() {
        let now = Instant::now();
        assert_eq!(
            remaining_download_time(now + Duration::from_secs(30), now).unwrap(),
            FETCH_TIMEOUT
        );
        assert_eq!(
            remaining_download_time(now + Duration::from_secs(2), now).unwrap(),
            Duration::from_secs(2)
        );
        assert!(remaining_download_time(now, now).is_err());
        assert!(remaining_download_time(now, now + Duration::from_secs(1)).is_err());
    }

    #[test]
    fn private_staging_cleanup_preserves_previous_assets_and_other_installs() {
        let config = tempfile::tempdir().unwrap();
        let first = prepare_directory(config.path(), "cooler").unwrap();
        let first_path = first.target.clone();
        std::fs::write(first.staging.path().join("asset"), b"old").unwrap();
        sync_directory(first.staging.path()).unwrap();
        let _ = first.staging.keep();
        drop(first._root);
        let failed = prepare_directory(config.path(), "cooler").unwrap();
        let failed_path = failed.target.clone();
        std::fs::write(failed.staging.path().join("partial"), b"partial").unwrap();
        let concurrent = prepare_directory(config.path(), "cooler").unwrap();
        let concurrent_path = concurrent.target.clone();
        assert_ne!(failed_path, concurrent_path);
        drop(failed);
        assert!(!failed_path.exists());
        assert!(concurrent_path.exists());
        assert_eq!(std::fs::read(first_path.join("asset")).unwrap(), b"old");
        drop(concurrent);
        assert!(!concurrent_path.exists());
    }

    #[test]
    fn staging_rejects_symlink_root_and_cleans_through_pinned_directory() {
        use std::os::unix::fs::symlink;
        let config = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = config.path().join("templates");
        symlink(outside.path(), &root).unwrap();
        assert!(prepare_directory(config.path(), "cooler").is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
        std::fs::remove_file(&root).unwrap();
        let pending = prepare_directory(config.path(), "cooler").unwrap();
        let name = pending.target.file_name().unwrap().to_owned();
        let moved = config.path().join("moved");
        std::fs::rename(&root, &moved).unwrap();
        symlink(outside.path(), &root).unwrap();
        drop(pending);
        assert!(!moved.join(name).exists());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[test]
    fn catalog_rejects_unsafe_and_conflicting_paths() {
        let original = catalog().templates.remove(0);
        for path in [
            "../outside",
            "/absolute",
            "a/../b",
            "a//b",
            "./b",
            "a?b",
            "a%2fb",
            "a\\b",
        ] {
            let mut template = original.clone();
            template.files[0].path = path.into();
            assert!(validate_catalog_template(&template).is_err(), "{path}");
        }
        let mut template = original.clone();
        template.files[0].path = template.template_file.clone();
        assert!(validate_catalog_template(&template).is_err());
        template.files[0].path = super::super::catalog_storage::RECEIPT_NAME.into();
        assert!(validate_catalog_template(&template).is_err());
        template.files[0].path = format!("{}/child", template.template_file);
        assert!(validate_catalog_template(&template).is_err());
        template = original.clone();
        template.id = "nested/id".into();
        assert!(validate_catalog_template(&template).is_err());
        template = original.clone();
        template.files = vec![original.files[0].clone(); MAX_FILES + 1];
        assert!(validate_catalog_template(&template).is_err());
        template = original;
        template.template_sha256 = "bad".into();
        assert!(validate_catalog_template(&template).is_err());
    }

    #[test]
    fn catalog_variants_need_short_visible_labels() {
        let mut template = catalog().templates.remove(0);
        template.group = Some("graphite-vertical".into());
        template.variant = Some("Cyan".into());
        assert!(validate_catalog_template(&template).is_ok());
        template.variant = Some("  ".into());
        assert!(validate_catalog_template(&template).is_err());
        template.variant = Some("x".repeat(MAX_VARIANT_LABEL_CHARS + 1));
        assert!(validate_catalog_template(&template).is_err());
    }

    #[test]
    fn bundled_catalog_assets_are_confined_and_complete() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/assets");
        for entry in catalog().templates {
            validate_catalog_template(&entry).unwrap();
            let bytes = std::fs::read(root.join(&entry.folder).join(&entry.template_file)).unwrap();
            let mut template: LcdTemplate = serde_json::from_slice(&bytes).unwrap();
            validate_template_assets(&template, &entry).unwrap();
            template.id.push_str("-unexpected");
            assert!(validate_template_assets(&template, &entry).is_err());
        }
        let entry = catalog().templates.remove(0);
        let bytes = std::fs::read(root.join(&entry.folder).join(&entry.template_file)).unwrap();
        let mut template: LcdTemplate = serde_json::from_slice(&bytes).unwrap();
        for path in ["/etc/passwd", "../outside", "unlisted.png"] {
            template.background = crate::template::TemplateBackground::Image { path: path.into() };
            assert!(validate_template_assets(&template, &entry).is_err());
        }
    }
}
