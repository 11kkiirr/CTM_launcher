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
    Dependency, DependencyType, GalleryImage, InstalledMod, Project, SearchHit, SearchResults,
    Version, VersionFile,
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
///
/// The SHA-1 is intentionally *not* computed here: hashing large jars makes a
/// scan slow. It is filled in lazily by [`update_check_sha1`] when the user
/// asks to check for updates.
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

        // Parse jar manifest for mod name and version; also read mod id
        // from fabric.mod.json / quilt.mod.json when present.
        let (mod_name, version, mod_id) = parse_jar_manifest(&path).await;

        // Format file modification time as install date.
        let install_date = metadata
            .modified()
            .ok()
            .map(|t| {
                let dt: chrono::DateTime<chrono::Local> = t.into();
                dt.format("%Y-%m-%d %H:%M").to_string()
            })
            .unwrap_or_default();

        out.push(InstalledMod {
            path,
            file_name: file_name.clone(),
            enabled: !is_disabled(&file_name),
            sha1: String::new(),
            size: metadata.len(),
            mod_name,
            version,
            mod_id,
            install_date,
        });
    }
    out.sort_by(|a, b| {
        logical_name(&a.file_name)
            .to_lowercase()
            .cmp(&logical_name(&b.file_name).to_lowercase())
    });
    Ok(out)
}

/// Extract mod name, version and id from a jar.
///
/// Name/version come from `META-INF/MANIFEST.MF` (`Fabric-Mod` /
/// `Implementation-Title` headers), falling back to `fabric.mod.json`.
/// The id is read from `fabric.mod.json` (`id`) or `quilt.mod.json`
/// (`quilt_loader.id`) so Browse can match installed files to Modrinth slugs.
async fn parse_jar_manifest(path: &std::path::Path) -> (String, String, String) {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || parse_jar_manifest_sync(&path))
        .await
        .unwrap_or_else(|_| (String::new(), String::new(), String::new()))
}

fn parse_jar_manifest_sync(path: &std::path::Path) -> (String, String, String) {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return (String::new(), String::new(), String::new()),
    };
    let reader = std::io::BufReader::new(file);
    let mut archive = match zip::ZipArchive::new(reader) {
        Ok(z) => z,
        Err(_) => return (String::new(), String::new(), String::new()),
    };

    let mut name = String::new();
    let mut version = String::new();
    let mut mod_id = String::new();

    if let Ok(mut manifest) = archive.by_name("META-INF/MANIFEST.MF") {
        let mut contents = String::new();
        if std::io::Read::read_to_string(&mut manifest, &mut contents).is_ok() {
            for line in contents.lines() {
                let trimmed = line.trim();
                if let Some(val) = trimmed.strip_prefix("Fabric-Mod:") {
                    let val = val.trim();
                    if !val.is_empty() && name.is_empty() {
                        name = val.to_string();
                    }
                } else if let Some(val) = trimmed.strip_prefix("Implementation-Title:") {
                    let val = val.trim();
                    if !val.is_empty() && name.is_empty() {
                        name = val.to_string();
                    }
                }
                if let Some(val) = trimmed.strip_prefix("Fabric-Version:") {
                    let val = val.trim();
                    if !val.is_empty() && version.is_empty() {
                        version = val.to_string();
                    }
                } else if let Some(val) = trimmed.strip_prefix("Implementation-Version:") {
                    let val = val.trim();
                    if !val.is_empty() && version.is_empty() {
                        version = val.to_string();
                    }
                }
            }
        }
    }

    if let Ok(mut f) = archive.by_name("fabric.mod.json") {
        let mut contents = String::new();
        if std::io::Read::read_to_string(&mut f, &mut contents).is_ok() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&contents) {
                if let Some(id) = v.get("id").and_then(|x| x.as_str()) {
                    if !id.is_empty() {
                        mod_id = id.to_string();
                    }
                }
                if name.is_empty() {
                    if let Some(n) = v.get("name").and_then(|x| x.as_str()) {
                        name = n.to_string();
                    }
                }
                if version.is_empty() {
                    if let Some(ver) = v.get("version").and_then(|x| x.as_str()) {
                        version = ver.to_string();
                    }
                }
            }
        }
    }

    if mod_id.is_empty() {
        if let Ok(mut f) = archive.by_name("quilt.mod.json") {
            let mut contents = String::new();
            if std::io::Read::read_to_string(&mut f, &mut contents).is_ok() {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&contents) {
                    if let Some(id) = v
                        .get("quilt_loader")
                        .and_then(|q| q.get("id"))
                        .and_then(|x| x.as_str())
                    {
                        if !id.is_empty() {
                            mod_id = id.to_string();
                        }
                    }
                    if name.is_empty() {
                        if let Some(n) = v
                            .get("quilt_loader")
                            .and_then(|q| q.get("metadata"))
                            .and_then(|m| m.get("name"))
                            .and_then(|x| x.as_str())
                        {
                            name = n.to_string();
                        }
                    }
                }
            }
        }
    }

    (name, version, mod_id)
}

/// Ensure a mod has its SHA-1 computed, hashing lazily when missing.
pub async fn update_check_sha1(module: &InstalledMod) -> Result<String> {
    if !module.sha1.is_empty() {
        return Ok(module.sha1.clone());
    }
    sha1_file(&module.path).await
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

    #[tokio::test]
    async fn scan_reads_fabric_mod_id() {
        use std::io::Write as _;

        let dir = std::env::temp_dir().join(format!("ctm-scan-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let jar_path = dir.join("sodium.jar");
        {
            let file = std::fs::File::create(&jar_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("fabric.mod.json", options).unwrap();
            zip.write_all(br#"{"id":"sodium","name":"Sodium","version":"0.6.0"}"#)
                .unwrap();
            zip.finish().unwrap();
        }

        let mods = scan_installed_mods(&dir).await.unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].mod_id, "sodium");
        assert_eq!(mods[0].mod_name, "Sodium");
        assert_eq!(mods[0].version, "0.6.0");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
