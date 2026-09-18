//! Asset index resolution and download.
//!
//! Assets are content-addressed under `assets/objects/<aa>/<sha1>`; the index
//! maps logical names to those hashes. Downloads are performed with bounded
//! concurrency and are resumable because existing, correctly-hashed objects are
//! skipped.

use std::collections::HashMap;

use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::util::{
    download_file, ensure_dir, fetch_json, sha1_file, Paths, Progress, ProgressCallback,
};
use crate::version::{AssetIndexRef, RuleContext, VersionDetails};

const RESOURCES_BASE: &str = "https://resources.download.minecraft.net";

/// Parsed asset index document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetIndex {
    #[serde(default)]
    pub objects: HashMap<String, AssetObject>,
    #[serde(default)]
    pub map_to_resources: bool,
    #[serde(default)]
    pub virtual_: bool,
}

/// One entry in the asset index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

/// Outcome of ensuring assets are present.
#[derive(Debug, Clone)]
pub struct AssetReport {
    pub index_id: String,
    pub total: usize,
    pub downloaded: usize,
    pub skipped: usize,
}

/// Ensure the asset index and all referenced objects exist locally.
pub async fn ensure_assets(
    client: &reqwest::Client,
    paths: &Paths,
    index: &AssetIndexRef,
    progress: Option<ProgressCallback>,
) -> Result<AssetReport> {
    let index_path = paths.asset_indexes_dir().join(format!("{}.json", index.id));
    if !index_path.exists() || !index.sha1.is_empty() {
        let sha1 = (!index.sha1.is_empty()).then_some(index.sha1.as_str());
        download_file(client, &index.url, &index_path, sha1, None).await?;
    }

    let parsed: AssetIndex = fetch_json(client, &index.url).await?;
    let total = parsed.objects.len();
    let objects_dir = paths.asset_objects_dir();
    ensure_dir(&objects_dir).await?;

    let entries: Vec<(String, AssetObject)> = parsed
        .objects
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let skipped = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let results: Vec<Result<()>> = stream::iter(entries)
        .map(|(_, object)| {
            let objects_dir = objects_dir.clone();
            let client = client.clone();
            let progress = progress.clone();
            let counter = counter.clone();
            let skipped = skipped.clone();
            async move {
                let prefix = &object.hash[0..2];
                let dest = objects_dir.join(prefix).join(&object.hash);
                if dest.exists() {
                    if let Ok(actual) = sha1_file(&dest).await {
                        if actual == object.hash {
                            skipped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            return Ok(());
                        }
                    }
                }
                let url = format!("{RESOURCES_BASE}/{prefix}/{}", object.hash);
                download_file(&client, &url, &dest, Some(&object.hash), None).await?;
                let done = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if let Some(cb) = &progress {
                    cb(Progress {
                        current: done as u64,
                        total: Some(total as u64),
                        message: format!("assets {done}/{total}"),
                    });
                }
                Ok(())
            }
        })
        .buffer_unordered(32)
        .collect()
        .await;

    let mut downloaded: usize = 0;
    for result in results {
        result?;
        downloaded += 1;
    }

    let skipped = skipped.load(std::sync::atomic::Ordering::Relaxed);
    Ok(AssetReport {
        index_id: index.id.clone(),
        total,
        downloaded: downloaded.saturating_sub(skipped),
        skipped,
    })
}

/// Convenience: ensure assets for a version document.
pub async fn ensure_version_assets(
    client: &reqwest::Client,
    paths: &Paths,
    details: &VersionDetails,
    progress: Option<ProgressCallback>,
) -> Result<Option<AssetReport>> {
    match &details.asset_index {
        Some(index) => Ok(Some(ensure_assets(client, paths, index, progress).await?)),
        None => Ok(None),
    }
}

/// Extract native libraries for the current platform into `natives_dir`.
///
/// `libraries` should be the already-resolved list from the version document.
pub fn extract_natives(
    paths: &Paths,
    libraries: &[crate::version::Library],
    rule_ctx: &RuleContext,
    natives_dir: &std::path::Path,
) -> Result<usize> {
    std::fs::create_dir_all(natives_dir)?;
    let mut extracted = 0;

    for library in libraries {
        if !library.applies(rule_ctx) {
            continue;
        }
        let Some(classifier) = library.native_classifier(rule_ctx) else {
            continue;
        };
        let Some(downloads) = &library.downloads.classifiers else {
            continue;
        };
        let Some(artifact) = downloads.get(&classifier) else {
            continue;
        };
        let rel = artifact
            .path
            .clone()
            .unwrap_or_else(|| library.relative_path().unwrap_or_default());
        if rel.is_empty() {
            continue;
        }
        let jar_path = paths.libraries_dir().join(&rel);
        if !jar_path.exists() {
            continue;
        }

        let exclude = library
            .extract
            .as_ref()
            .map(|e| e.exclude.clone())
            .unwrap_or_default();
        extracted += extract_native_jar(&jar_path, natives_dir, &exclude)?;
    }
    Ok(extracted)
}

fn extract_native_jar(
    jar_path: &std::path::Path,
    natives_dir: &std::path::Path,
    exclude: &[String],
) -> Result<usize> {
    let file = std::fs::File::open(jar_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| CoreError::Archive(e.to_string()))?;
    let mut count = 0;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| CoreError::Archive(e.to_string()))?;
        let name = entry.name().to_string();

        if name.ends_with('/') || exclude.iter().any(|e| name.starts_with(e)) {
            continue;
        }
        // Skip signatures and metadata common to native jars.
        if name.starts_with("META-INF/") {
            continue;
        }

        let safe = match crate::util::sanitize_archive_path(std::path::Path::new(&name)) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let out_path = natives_dir.join(safe);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out)?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_asset_index() {
        let raw = r#"{"objects":{"minecraft/lang/en_us.json":{"hash":"abcdef","size":42}}}"#;
        let index: AssetIndex = serde_json::from_str(raw).unwrap();
        assert_eq!(index.objects.len(), 1);
        assert_eq!(index.objects["minecraft/lang/en_us.json"].size, 42);
    }
}
