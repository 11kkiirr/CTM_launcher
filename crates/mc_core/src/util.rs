//! Shared filesystem, hashing, download and archive utilities.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use directories::ProjectDirs;
use futures::StreamExt;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha1::{Digest as Sha1Digest, Sha1};
use sha2::Sha512;
use tokio::io::AsyncReadExt;

use crate::error::{CoreError, Result};

/// A single progress tick emitted by long-running engine operations.
#[derive(Debug, Clone, Default)]
pub struct Progress {
    /// Bytes completed (or items completed for non-byte operations).
    pub current: u64,
    /// Total expected bytes/items, when known.
    pub total: Option<u64>,
    /// Human readable status message.
    pub message: String,
}

impl Progress {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            current: 0,
            total: None,
            message: message.into(),
        }
    }

    /// Fraction in `0.0..=1.0` if a total is known.
    pub fn fraction(&self) -> Option<f64> {
        match self.total {
            Some(t) if t > 0 => Some((self.current as f64 / t as f64).clamp(0.0, 1.0)),
            _ => None,
        }
    }
}

/// Thread-safe progress callback shared across async tasks.
pub type ProgressCallback = Arc<dyn Fn(Progress) + Send + Sync>;

/// A callback that discards all progress events.
pub fn noop_progress() -> ProgressCallback {
    Arc::new(|_| {})
}

/// Resolve XDG-compliant project directories.
pub fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("dev", "ctmlauncher", "CTMLauncher")
        .ok_or_else(|| CoreError::other("could not resolve project directories"))
}

/// Canonical filesystem layout used by the launcher.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl Paths {
    /// Build the default layout from the platform's XDG directories.
    pub fn discover() -> Result<Self> {
        let dirs = project_dirs()?;
        Ok(Self {
            config_dir: dirs.config_dir().to_path_buf(),
            data_dir: dirs.data_dir().to_path_buf(),
            cache_dir: dirs.cache_dir().to_path_buf(),
        })
    }

    /// Build a layout rooted at an explicit data directory (useful for tests).
    pub fn rooted_at(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            config_dir: root.join("config"),
            data_dir: root.clone(),
            cache_dir: root.join("cache"),
        }
    }

    pub fn instances_dir(&self) -> PathBuf {
        self.data_dir.join("instances")
    }

    pub fn instance_dir(&self, id: &str) -> PathBuf {
        self.instances_dir().join(id)
    }

    pub fn accounts_file(&self) -> PathBuf {
        self.config_dir.join("accounts.json")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    /// ASCII-art background for the builds grid.
    pub fn ascii_bg_file(&self) -> PathBuf {
        self.config_dir.join("ascii_bg.txt")
    }

    pub fn shared_dir(&self) -> PathBuf {
        self.data_dir.join("shared")
    }

    /// Shared vanilla version metadata + client jars.
    pub fn versions_dir(&self) -> PathBuf {
        self.shared_dir().join("versions")
    }

    pub fn libraries_dir(&self) -> PathBuf {
        self.shared_dir().join("libraries")
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.shared_dir().join("assets")
    }

    pub fn asset_indexes_dir(&self) -> PathBuf {
        self.assets_dir().join("indexes")
    }

    pub fn asset_objects_dir(&self) -> PathBuf {
        self.assets_dir().join("objects")
    }

    pub fn java_dir(&self) -> PathBuf {
        self.data_dir.join("java")
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.cache_dir.join("downloads")
    }

    pub fn modrinth_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("modrinth")
    }

    /// Create every directory required by the launcher.
    pub fn ensure_layout(&self) -> Result<()> {
        for dir in [
            self.config_dir.clone(),
            self.data_dir.clone(),
            self.cache_dir.clone(),
            self.instances_dir(),
            self.shared_dir(),
            self.versions_dir(),
            self.libraries_dir(),
            self.assets_dir(),
            self.asset_indexes_dir(),
            self.asset_objects_dir(),
            self.java_dir(),
            self.downloads_dir(),
            self.modrinth_cache_dir(),
        ] {
            std::fs::create_dir_all(&dir)?;
        }
        Ok(())
    }
}

/// Create a directory and all of its parents.
pub async fn ensure_dir(path: impl AsRef<Path>) -> Result<()> {
    tokio::fs::create_dir_all(path.as_ref()).await?;
    Ok(())
}

/// Hash a file's contents with SHA-1, streaming to avoid loading it in memory.
pub async fn sha1_file(path: impl AsRef<Path>) -> Result<String> {
    let mut file = tokio::fs::File::open(path.as_ref()).await?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Hash a file's contents with SHA-512.
pub async fn sha512_file(path: impl AsRef<Path>) -> Result<String> {
    let mut file = tokio::fs::File::open(path.as_ref()).await?;
    let mut hasher = Sha512::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// SHA-1 of an in-memory buffer.
pub fn sha1_bytes(data: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// SHA-512 of an in-memory buffer.
pub fn sha512_bytes(data: &[u8]) -> String {
    let mut hasher = Sha512::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Download `url` to `dest`, creating parent directories.
///
/// When `expected_sha1` is provided the download is verified and a mismatch is
/// reported as [`CoreError::HashMismatch`]. Existing files with a matching hash
/// are reused without a network round-trip.
pub async fn download_file(
    client: &reqwest::Client,
    url: &str,
    dest: impl AsRef<Path>,
    expected_sha1: Option<&str>,
    progress: Option<ProgressCallback>,
) -> Result<PathBuf> {
    let dest = dest.as_ref().to_path_buf();

    if dest.exists() {
        if let Some(expected) = expected_sha1 {
            if sha1_file(&dest).await? == expected.to_ascii_lowercase() {
                return Ok(dest);
            }
        } else {
            return Ok(dest);
        }
    }

    if let Some(parent) = dest.parent() {
        ensure_dir(parent).await?;
    }

    let mut last_err: Option<CoreError> = None;
    for attempt in 0..3u32 {
        match download_once(client, url, &dest, progress.clone()).await {
            Ok(()) => {
                if let Some(expected) = expected_sha1 {
                    let actual = sha1_file(&dest).await?;
                    if actual != expected.to_ascii_lowercase() {
                        let _ = tokio::fs::remove_file(&dest).await;
                        last_err = Some(CoreError::HashMismatch {
                            path: dest.display().to_string(),
                            expected: expected.to_string(),
                            actual,
                        });
                        continue;
                    }
                }
                return Ok(dest);
            }
            Err(err) => {
                last_err = Some(err);
                let backoff = 200u64 * (attempt as u64 + 1);
                tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
            }
        }
    }
    Err(last_err.unwrap_or_else(|| CoreError::other("download failed")))
}

async fn download_once(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    progress: Option<ProgressCallback>,
) -> Result<()> {
    let response = client.get(url).send().await?.error_for_status()?;
    let total = response.content_length();

    let tmp = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;

    if let Some(cb) = &progress {
        cb(Progress {
            current: 0,
            total,
            message: format!("downloading {url}"),
        });
    }

    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if let Some(cb) = &progress {
            cb(Progress {
                current: downloaded,
                total,
                message: String::new(),
            });
        }
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&tmp, dest).await?;
    Ok(())
}

/// Fetch and deserialize JSON from a URL.
pub async fn fetch_json<T: DeserializeOwned>(client: &reqwest::Client, url: &str) -> Result<T> {
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json::<T>().await?)
}

/// Fetch raw bytes from a URL.
pub async fn fetch_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let response = client.get(url).send().await?.error_for_status()?;
    Ok(response.bytes().await?.to_vec())
}

/// POST JSON and deserialize the JSON response.
pub async fn post_json<B: Serialize, T: DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    body: &B,
) -> Result<T> {
    let response = client
        .post(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .form(body)
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json::<T>().await?)
}

/// Read and deserialize a JSON file, returning `default` when it is missing.
pub async fn read_json_or_default<T: DeserializeOwned + Default>(
    path: impl AsRef<Path>,
) -> Result<T> {
    match tokio::fs::read(path.as_ref()).await {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(err.into()),
    }
}

/// Read and deserialize a JSON file.
pub async fn read_json<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T> {
    let bytes = tokio::fs::read(path.as_ref()).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Serialize `value` as pretty JSON and write it atomically.
pub async fn write_json<T: Serialize>(path: impl AsRef<Path>, value: &T) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        ensure_dir(parent).await?;
    }
    let data = serde_json::to_vec_pretty(value)?;
    atomic_write(path, &data).await
}

/// Write bytes to a temporary sibling then rename into place.
pub async fn atomic_write(path: impl AsRef<Path>, data: &[u8]) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        ensure_dir(parent).await?;
    }
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, data).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

/// Reject archive entries containing `..`, absolute or prefix components.
pub fn sanitize_archive_path(raw: &Path) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for component in raw.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => {
                return Err(CoreError::Archive(format!(
                    "unsafe archive path: {}",
                    raw.display()
                )))
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(CoreError::Archive("empty archive path".into()));
    }
    Ok(out)
}

/// Extract a zip archive into `dest`, returning the number of files written.
pub fn extract_zip(zip_path: impl AsRef<Path>, dest: impl AsRef<Path>) -> Result<usize> {
    let file = std::fs::File::open(zip_path.as_ref())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| CoreError::Archive(e.to_string()))?;
    let dest = dest.as_ref();
    let mut count = 0;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| CoreError::Archive(e.to_string()))?;
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let safe = sanitize_archive_path(&rel)?;
        let out_path = dest.join(safe);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out)?;

        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode));
        }
        count += 1;
    }
    Ok(count)
}

/// Extract a gzip-compressed tarball into `dest`.
pub fn extract_tar_gz(tar_path: impl AsRef<Path>, dest: impl AsRef<Path>) -> Result<usize> {
    let file = std::fs::File::open(tar_path.as_ref())?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let dest = dest.as_ref();
    std::fs::create_dir_all(dest)?;
    let mut count = 0;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let rel = entry.path()?.into_owned();
        let safe = sanitize_archive_path(&rel)?;
        let out_path = dest.join(safe);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        entry.unpack(&out_path)?;
        count += 1;
    }
    Ok(count)
}

/// Copy a directory tree recursively.
pub fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<()> {
    let src = src.as_ref();
    let dst = dst.as_ref();
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rejects_traversal() {
        assert!(sanitize_archive_path(Path::new("../evil")).is_err());
        assert!(sanitize_archive_path(Path::new("/abs/evil")).is_err());
        assert!(sanitize_archive_path(Path::new("a/b/c")).is_ok());
    }

    #[test]
    fn hashes_are_stable() {
        // SHA-1 of "abc"
        assert_eq!(
            sha1_bytes(b"abc"),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    #[test]
    fn progress_fraction_is_clamped() {
        let p = Progress {
            current: 5,
            total: Some(10),
            message: String::new(),
        };
        assert_eq!(p.fraction(), Some(0.5));
        let unknown = Progress::new("x");
        assert_eq!(unknown.fraction(), None);
    }
}
