//! Modrinth API integration: browsing, installing, dependency resolution and
//! update checking.

pub mod client;
pub mod models;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};
use crate::util::{download_file, sha1_file, ProgressCallback};

pub use client::ModrinthClient;
pub use models::{
    Dependency, DependencyType, InstalledMod, Project, SearchHit, SearchResults, Version,
    VersionFile,
};

/// Suffix appended to disabled mod files.
pub const DISABLED_SUFFIX: &str = ".disabled";

/// `true` when a filename represents a disabled mod.
pub fn is_disabled(file_name: &str) -> bool {
    file_name.ends_with(DISABLED_SUFFIX)
}

/// The logical name of a mod file with any `.disabled` suffix removed.
pub fn logical_name(file_name: &str) -> &str {
    file_name.strip_suffix(DISABLED_SUFFIX).unwrap_or(file_name)
}

/// Toggle a mod file's enabled state by adding/removing `.disabled`.
///
/// Returns the new path.
pub async fn toggle_mod(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| CoreError::Other("invalid mod path".into()))?;

    let new_path = if is_disabled(name) {
        let enabled = path.with_file_name(logical_name(name));
        enabled
    } else {
        let mut disabled_name = name.to_string();
        disabled_name.push_str(DISABLED_SUFFIX);
        path.with_file_name(disabled_name)
    };

    if new_path.exists() {
        return Err(CoreError::Other(format!(
            "cannot toggle mod, target already exists: {}",
            new_path.display()
        )));
    }
    tokio::fs::rename(path, &new_path).await?;
    Ok(new_path)
}

/// Delete a mod file.
pub async fn remove_mod(path: impl AsRef<Path>) -> Result<()> {
    tokio::fs::remove_file(path.as_ref()).await?;
    Ok(())
}

/// Scan an instance's `mods/` directory, hashing every `.jar`.
pub async fn scan_installed_mods(mods_dir: impl AsRef<Path>) -> Result<Vec<InstalledMod>> {
    let mods_dir = mods_dir.as_ref();
    if !mods_dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut entries = tokio::fs::read_dir(mods_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let file_type = entry.file_type().await?;
        if !file_type.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.ends_with(".jar") && !file_name.ends_with(".jar.disabled") {
            continue;
        }
        let path = entry.path();
        let metadata = entry.metadata().await?;
        let sha1 = sha1_file(&path).await.unwrap_or_default();
        out.push(InstalledMod {
            path,
            file_name: file_name.clone(),
            enabled: !is_disabled(&file_name),
            sha1,
            size: metadata.len(),
        });
    }
    out.sort_by(|a, b| {
        logical_name(&a.file_name)
            .to_lowercase()
            .cmp(&logical_name(&b.file_name).to_lowercase())
    });
    Ok(out)
}

/// Download a Modrinth version file into `mods_dir`, verifying its SHA-1.
pub async fn install_version_file(
    client: &reqwest::Client,
    file: &VersionFile,
    mods_dir: impl AsRef<Path>,
    progress: Option<ProgressCallback>,
) -> Result<PathBuf> {
    let dest = mods_dir.as_ref().join(&file.filename);
    download_file(client, &file.url, &dest, file.sha1(), progress).await
}

/// Resolve the full dependency closure for `version`.
///
/// Performs a breadth-first walk of required dependencies, deduplicating by
/// project id and respecting `max_depth` to avoid pathological graphs.
pub async fn resolve_dependencies(
    client: &ModrinthClient,
    version: &Version,
    game_version: Option<&str>,
    loader: Option<&str>,
    max_depth: usize,
) -> Result<Vec<Version>> {
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(version.project_id.clone());
    let mut resolved: Vec<Version> = Vec::new();
    let mut queue: Vec<(String, usize)> = version
        .required_dependencies()
        .into_iter()
        .filter_map(|d| d.project_id.clone().map(|p| (p, 1usize)))
        .collect();

    while let Some((project_id, depth)) = queue.pop() {
        if depth > max_depth || !seen.insert(project_id.clone()) {
            continue;
        }
        let Some(dep_version) = client
            .latest_version(&project_id, game_version, loader)
            .await?
        else {
            continue;
        };
        for child in dep_version.required_dependencies() {
            if let Some(pid) = &child.project_id {
                queue.push((pid.clone(), depth + 1));
            }
        }
        resolved.push(dep_version);
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_suffix_helpers() {
        assert!(is_disabled("sodium.jar.disabled"));
        assert!(!is_disabled("sodium.jar"));
        assert_eq!(logical_name("sodium.jar.disabled"), "sodium.jar");
        assert_eq!(logical_name("sodium.jar"), "sodium.jar");
    }

    #[tokio::test]
    async fn toggle_round_trip() {
        let dir = std::env::temp_dir().join(format!("ctm-mod-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let file = dir.join("test.jar");
        tokio::fs::write(&file, b"data").await.unwrap();

        let disabled = toggle_mod(&file).await.unwrap();
        assert!(disabled.to_string_lossy().ends_with("test.jar.disabled"));
        assert!(!file.exists());

        let enabled = toggle_mod(&disabled).await.unwrap();
        assert_eq!(enabled, file);
        assert!(file.exists());

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn scan_finds_jars_only() {
        let dir = std::env::temp_dir().join(format!("ctm-scan-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("a.jar"), b"a").await.unwrap();
        tokio::fs::write(dir.join("b.jar.disabled"), b"b")
            .await
            .unwrap();
        tokio::fs::write(dir.join("readme.txt"), b"c")
            .await
            .unwrap();

        let mods = scan_installed_mods(&dir).await.unwrap();
        assert_eq!(mods.len(), 2);
        assert!(mods.iter().any(|m| !m.enabled));
        assert!(mods.iter().any(|m| m.enabled));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
