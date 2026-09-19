//! Forge installation.
//!
//! Modern Forge ships an installer that must execute binary patch processors.
//! Rather than reimplementing that pipeline, we run the official installer in
//! an isolated sandbox (`-Duser.home` + `APPDATA` redirection), then harvest
//! the produced `versions/` and `libraries/` trees into the shared store. This
//! is the same approach used by several third-party launchers and keeps the
//! result byte-for-byte identical to the official installer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{CoreError, Result};
use crate::install::common;
use crate::install::vanilla;
use crate::install::{InstalledVersion, Installer};
use crate::util::{copy_dir_all, download_file, ensure_dir, fetch_json, write_json};

const MAVEN: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
const PROMOTIONS: &str =
    "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";

#[derive(Debug, Deserialize)]
struct Promotions {
    #[serde(default)]
    promos: BTreeMap<String, String>,
}

/// List known Forge loader versions for a game version (newest first).
pub async fn available_loaders(installer: &Installer, game_version: &str) -> Result<Vec<String>> {
    let promotions: Promotions = fetch_json(&installer.client, PROMOTIONS).await?;
    let prefix = format!("{game_version}-");
    let mut versions: Vec<String> = promotions
        .promos
        .iter()
        .filter(|(k, _)| k.starts_with(&prefix))
        .map(|(_, v)| v.clone())
        .collect();
    versions.sort();
    versions.dedup();
    versions.reverse();
    Ok(versions)
}

/// Resolve the recommended (or latest) Forge version for a game version.
async fn recommended_loader(installer: &Installer, game_version: &str) -> Result<String> {
    let promotions: Promotions = fetch_json(&installer.client, PROMOTIONS).await?;
    for suffix in ["recommended", "latest"] {
        if let Some(v) = promotions.promos.get(&format!("{game_version}-{suffix}")) {
            return Ok(v.clone());
        }
    }
    // Fall back to the newest listed build for this game version.
    let prefix = format!("{game_version}-");
    promotions
        .promos
        .iter()
        .filter(|(k, _)| k.starts_with(&prefix))
        .map(|(_, v)| v.clone())
        .max()
        .ok_or_else(|| CoreError::Install(format!("no Forge build for {game_version}")))
}

/// Install Forge for `game_version`.
pub async fn install(
    installer: &Installer,
    game_version: &str,
    loader_version: Option<&str>,
) -> Result<InstalledVersion> {
    let loader = match loader_version {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => recommended_loader(installer, game_version).await?,
    };

    let url =
        format!("{MAVEN}/{game_version}-{loader}/forge-{game_version}-{loader}-installer.jar");
    let sandbox_key = format!("forge-{game_version}-{loader}");
    let version_id = run_installer(installer, game_version, &url, &sandbox_key).await?;

    vanilla::install(installer, game_version).await?;
    let resolved = common::resolve_version(&installer.paths, &version_id).await?;
    let json_path = installer
        .paths
        .versions_dir()
        .join(&version_id)
        .join(format!("{version_id}.json"));
    Ok(common::installed(version_id, resolved, json_path))
}

/// Download and execute an installer jar in a sandbox, then harvest artifacts.
///
/// Returns the id of the version document the installer produced.
pub(crate) async fn run_installer(
    installer: &Installer,
    game_version: &str,
    installer_url: &str,
    sandbox_key: &str,
) -> Result<String> {
    common::tick(
        &Some(installer.progress.clone()),
        format!("Downloading installer for {sandbox_key}"),
    );
    let jar = installer
        .paths
        .downloads_dir()
        .join(format!("{sandbox_key}-installer.jar"));
    download_file(&installer.client, installer_url, &jar, None, None).await?;

    let sandbox = installer
        .paths
        .cache_dir
        .join("installer-sandbox")
        .join(sandbox_key);
    if sandbox.exists() {
        let _ = tokio::fs::remove_dir_all(&sandbox).await;
    }
    ensure_dir(&sandbox).await?;
    write_launcher_profiles(&sandbox).await?;

    let java = crate::launch::java::discover()
        .await
        .into_iter()
        .next()
        .map(|j| j.path)
        .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "java.exe" } else { "java" }));

    common::tick(
        &Some(installer.progress.clone()),
        format!("Running installer for {sandbox_key} (this may take a while)"),
    );

    let output = tokio::process::Command::new(&java)
        .arg(format!("-Duser.home={}", sandbox.display()))
        .arg("-jar")
        .arg(&jar)
        .arg("--installClient")
        .current_dir(&sandbox)
        .env("HOME", &sandbox)
        .env("USERPROFILE", &sandbox)
        .env("APPDATA", sandbox.join("AppData").join("Roaming"))
        .output()
        .await
        .map_err(|e| CoreError::Install(format!("failed to launch Forge installer: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(CoreError::Install(format!(
            "installer exited with {}:\n{stdout}\n{stderr}",
            output.status
        )));
    }

    let mc_dir = locate_minecraft_dir(&sandbox)?;
    let versions_dir = mc_dir.join("versions");

    // Collect the profile the installer produced (anything that is not the
    // vanilla base), and read its authoritative `id` from the JSON document.
    let mut produced: Option<(String, String)> = None;
    if versions_dir.exists() {
        for entry in std::fs::read_dir(&versions_dir)? {
            let entry = entry?;
            let dir_name = entry.file_name().to_string_lossy().to_string();
            if dir_name == game_version {
                continue;
            }
            let json = entry.path().join(format!("{dir_name}.json"));
            let id = std::fs::read(&json)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .and_then(|value| value.get("id").and_then(|v| v.as_str()).map(str::to_string))
                .unwrap_or_else(|| dir_name.clone());
            produced = Some((dir_name, id));
        }
    }
    let (dir_name, version_id) = produced
        .ok_or_else(|| CoreError::Install("installer produced no version profile".into()))?;

    common::tick(
        &Some(installer.progress.clone()),
        format!("Harvesting {version_id}"),
    );

    let dest_dir = installer.paths.versions_dir().join(&version_id);
    copy_dir_all(versions_dir.join(&dir_name), &dest_dir)?;
    // Normalize file names to the JSON id so lookups are deterministic.
    if dir_name != version_id {
        let old_json = dest_dir.join(format!("{dir_name}.json"));
        let new_json = dest_dir.join(format!("{version_id}.json"));
        if old_json.exists() && !new_json.exists() {
            let _ = std::fs::rename(&old_json, &new_json);
        }
    }
    let sandbox_libs = mc_dir.join("libraries");
    if sandbox_libs.exists() {
        copy_dir_all(&sandbox_libs, installer.paths.libraries_dir())?;
    }

    Ok(version_id)
}

async fn write_launcher_profiles(sandbox: &Path) -> Result<()> {
    let mc = sandbox.join(".minecraft");
    ensure_dir(&mc).await?;
    let profiles = serde_json::json!({
        "profiles": {},
        "selectedProfile": "",
        "clientToken": "ctmlauncher",
        "authenticationDatabase": {},
        "launcherVersion": { "name": "ctmlauncher", "format": 21 }
    });
    write_json(mc.join("launcher_profiles.json"), &profiles).await?;
    // Windows-style layout, in case the installer resolves APPDATA.
    let appdata_mc = sandbox.join("AppData").join("Roaming").join(".minecraft");
    ensure_dir(&appdata_mc).await?;
    write_json(appdata_mc.join("launcher_profiles.json"), &profiles).await?;
    Ok(())
}

/// Find whichever `.minecraft` layout the installer actually used.
fn locate_minecraft_dir(sandbox: &Path) -> Result<PathBuf> {
    let candidates = [
        sandbox.join(".minecraft"),
        sandbox.join("AppData").join("Roaming").join(".minecraft"),
        sandbox
            .join("Library")
            .join("Application Support")
            .join("minecraft"),
    ];
    for candidate in candidates {
        if candidate.join("versions").exists() {
            return Ok(candidate);
        }
    }
    Err(CoreError::Install(
        "could not locate installer output directory".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_promotions() {
        let raw = r#"{"homepage":"x","promos":{"1.20.1-recommended":"47.2.0","1.20.1-latest":"47.3.0","1.19.2-latest":"43.2.0"}}"#;
        let p: Promotions = serde_json::from_str(raw).unwrap();
        assert_eq!(p.promos.get("1.20.1-recommended").unwrap(), "47.2.0");
    }
}
