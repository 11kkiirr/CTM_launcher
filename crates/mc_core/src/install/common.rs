//! Helpers shared by every installer: library/asset/logging downloads and
//! inherited-version resolution.

use std::path::{Path, PathBuf};

use futures::future::join_all;

use crate::error::{CoreError, Result};
use crate::install::InstalledVersion;
use crate::util::{download_file, ensure_dir, fetch_json, Paths, Progress, ProgressCallback};
use crate::version::{
    DownloadArtifact, Library, Logging, RuleContext, VersionDetails, VersionManifest,
    VERSION_MANIFEST_URL,
};

/// Fetch the global Mojang version manifest.
pub async fn fetch_manifest(client: &reqwest::Client) -> Result<VersionManifest> {
    fetch_json(client, VERSION_MANIFEST_URL).await
}

/// Download a version's JSON document into the shared versions directory and
/// return its parsed contents.
pub async fn download_version_json(
    client: &reqwest::Client,
    paths: &Paths,
    id: &str,
    url: &str,
    sha1: Option<&str>,
) -> Result<VersionDetails> {
    let dir = paths.versions_dir().join(id);
    ensure_dir(&dir).await?;
    let json_path = dir.join(format!("{id}.json"));
    download_file(client, url, &json_path, sha1, None).await?;
    let bytes = tokio::fs::read(&json_path).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Download the client jar for a vanilla version.
pub async fn download_client(
    client: &reqwest::Client,
    paths: &Paths,
    id: &str,
    artifact: &DownloadArtifact,
    progress: Option<ProgressCallback>,
) -> Result<PathBuf> {
    let dest = paths.versions_dir().join(id).join(format!("{id}.jar"));
    let sha1 = (!artifact.sha1.is_empty()).then_some(artifact.sha1.as_str());
    download_file(client, &artifact.url, &dest, sha1, progress).await
}

/// Download all applicable libraries for `libraries` into the shared
/// `libraries/` directory. Native classifier jars are downloaded as well and
/// later extracted at launch time.
pub async fn download_libraries(
    client: &reqwest::Client,
    paths: &Paths,
    libraries: &[Library],
    rule_ctx: &RuleContext,
    progress: Option<ProgressCallback>,
) -> Result<Vec<PathBuf>> {
    let lib_root = paths.libraries_dir();
    let mut futures = Vec::new();

    for library in libraries {
        if !library.applies(rule_ctx) {
            continue;
        }

        if let Some(artifact) = &library.downloads.artifact {
            let rel = artifact
                .path
                .clone()
                .unwrap_or_else(|| library.relative_path().unwrap_or_default());
            if rel.is_empty() {
                continue;
            }
            futures.push(download_one(
                client,
                lib_root.join(&rel),
                artifact.clone(),
                progress.clone(),
            ));
        } else if let Some(base) = &library.url {
            let rel = library.relative_path()?;
            let artifact = DownloadArtifact {
                sha1: String::new(),
                size: 0,
                url: format!("{}/{rel}", base.trim_end_matches('/')),
                path: Some(rel.clone()),
            };
            futures.push(download_one(
                client,
                lib_root.join(&rel),
                artifact,
                progress.clone(),
            ));
        }

        if let Some(classifier) = library.native_classifier(rule_ctx) {
            if let Some(downloads) = &library.downloads.classifiers {
                if let Some(artifact) = downloads.get(&classifier) {
                    let rel = artifact
                        .path
                        .clone()
                        .unwrap_or_else(|| library.relative_path().unwrap_or_default());
                    futures.push(download_one(
                        client,
                        lib_root.join(&rel),
                        artifact.clone(),
                        progress.clone(),
                    ));
                }
            }
        }
    }

    let results = join_all(futures).await;
    let mut out = Vec::with_capacity(results.len());
    for result in results {
        out.push(result?);
    }
    Ok(out)
}

async fn download_one(
    client: &reqwest::Client,
    dest: PathBuf,
    artifact: DownloadArtifact,
    progress: Option<ProgressCallback>,
) -> Result<PathBuf> {
    let sha1 = (!artifact.sha1.is_empty()).then_some(artifact.sha1.as_str());
    download_file(client, &artifact.url, &dest, sha1, progress).await
}

/// Download the optional client logging configuration referenced by a version.
pub async fn download_logging(
    client: &reqwest::Client,
    paths: &Paths,
    logging: Option<&Logging>,
) -> Result<Option<PathBuf>> {
    let Some(file) = logging
        .and_then(|l| l.client.as_ref())
        .and_then(|c| c.file.as_ref())
    else {
        return Ok(None);
    };
    let name = file
        .path
        .clone()
        .unwrap_or_else(|| "log4j2.xml".to_string());
    let dest = paths.shared_dir().join("logging").join(name);
    let sha1 = (!file.sha1.is_empty()).then_some(file.sha1.as_str());
    download_file(client, &file.url, &dest, sha1, None).await?;
    Ok(Some(dest))
}

/// Load a version JSON from the shared versions directory.
pub async fn load_version_json(paths: &Paths, id: &str) -> Result<VersionDetails> {
    let path = paths.versions_dir().join(id).join(format!("{id}.json"));
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| CoreError::NotFound(format!("version metadata '{id}'")))?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Resolve a version, recursively merging its `inheritsFrom` parent.
///
/// Child values win; library and argument lists are concatenated with the
/// parent's entries first, matching the official launcher's behaviour.
pub async fn resolve_version(paths: &Paths, id: &str) -> Result<VersionDetails> {
    let mut chain = Vec::new();
    let mut current = id.to_string();
    loop {
        let details = load_version_json(paths, &current).await?;
        let parent = details.inherits_from.clone();
        chain.push(details);
        match parent {
            Some(p) => current = p,
            None => break,
        }
        if chain.len() > 16 {
            return Err(CoreError::Install(
                "version inheritance cycle detected".into(),
            ));
        }
    }

    // Fold from the root parent down to the requested child.
    chain.reverse();
    let mut merged = chain.remove(0);
    for child in chain {
        merge_versions(&mut merged, child);
    }
    Ok(merged)
}

/// Merge `child` on top of `parent` (parent already in `base`).
fn merge_versions(base: &mut VersionDetails, child: VersionDetails) {
    if !child.main_class.is_empty() {
        base.main_class = child.main_class;
    }
    if !child.assets.is_empty() {
        base.assets = child.assets;
    }
    if child.asset_index.is_some() {
        base.asset_index = child.asset_index;
    }
    if child.downloads.client.is_some() {
        base.downloads.client = child.downloads.client;
    }
    if child.java_version.is_some() {
        base.java_version = child.java_version;
    }
    if child.logging.is_some() {
        base.logging = child.logging;
    }
    base.id = child.id;
    base.inherits_from = None;

    // Libraries from the parent first, then the child.
    base.libraries.extend(child.libraries);

    match (base.arguments.as_mut(), child.arguments) {
        (Some(base_args), Some(child_args)) => {
            base_args.game.extend(child_args.game);
            base_args.jvm.extend(child_args.jvm);
        }
        (None, Some(child_args)) => base.arguments = Some(child_args),
        _ => {}
    }
    if let Some(extra) = child.minecraft_arguments {
        base.minecraft_arguments = Some(extra);
    }
}

/// Persist a resolved (loader) version document so launching can reload it.
pub async fn write_version_json(
    paths: &Paths,
    id: &str,
    details: &VersionDetails,
) -> Result<PathBuf> {
    let dir = paths.versions_dir().join(id);
    ensure_dir(&dir).await?;
    let path = dir.join(format!("{id}.json"));
    crate::util::write_json(&path, details).await?;
    Ok(path)
}

/// Convenience: wrap a resolved version into [`InstalledVersion`].
pub fn installed(
    id: impl Into<String>,
    details: VersionDetails,
    json_path: PathBuf,
) -> InstalledVersion {
    InstalledVersion {
        id: id.into(),
        details,
        json_path,
    }
}

/// Emit a progress tick with a message only (no byte totals).
pub fn tick(progress: &Option<ProgressCallback>, message: impl Into<String>) {
    if let Some(cb) = progress {
        cb(Progress::new(message));
    }
}

/// Check whether a path exists, returning a domain error otherwise.
pub fn require_exists(path: &Path, what: &str) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        Err(CoreError::NotFound(format!("{what} at {}", path.display())))
    }
}
