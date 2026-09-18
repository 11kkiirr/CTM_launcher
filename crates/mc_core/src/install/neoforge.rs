//! NeoForge installation.
//!
//! NeoForge reuses the Forge installer pipeline. We resolve versions from the
//! NeoForged Maven metadata API and delegate execution to the shared sandbox
//! installer runner.

use serde::Deserialize;

use crate::error::{CoreError, Result};
use crate::install::common;
use crate::install::forge;
use crate::install::vanilla;
use crate::install::{InstalledVersion, Installer};
use crate::util::fetch_json;

const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases/net/neoforged/neoforge";
const LEGACY_MAVEN: &str = "https://maven.neoforged.net/releases/net/neoforged/forge";
const NEOFORGE_VERSIONS: &str =
    "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge";
const LEGACY_VERSIONS: &str =
    "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/forge";

#[derive(Debug, Deserialize)]
struct MavenVersions {
    #[serde(default)]
    versions: Vec<String>,
}

/// Map a NeoForge version (`20.4.237`) to its Minecraft version (`1.20.4`).
pub fn neoforge_to_mc(version: &str) -> Option<String> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let minor = parts[0];
    let patch = parts[1];
    if patch == "0" {
        Some(format!("1.{minor}"))
    } else {
        Some(format!("1.{minor}.{patch}"))
    }
}

/// List NeoForge loader versions available for a game version.
pub async fn available_loaders(installer: &Installer, game_version: &str) -> Result<Vec<String>> {
    if game_version == "1.20.1" {
        let data: MavenVersions = fetch_json(&installer.client, LEGACY_VERSIONS).await?;
        let mut versions: Vec<String> = data
            .versions
            .into_iter()
            .filter_map(|v| v.strip_prefix("1.20.1-").map(str::to_string))
            .collect();
        versions.reverse();
        return Ok(versions);
    }

    let data: MavenVersions = fetch_json(&installer.client, NEOFORGE_VERSIONS).await?;
    let mut versions: Vec<String> = data
        .versions
        .into_iter()
        .filter(|v| neoforge_to_mc(v).as_deref() == Some(game_version))
        .collect();
    versions.reverse();
    Ok(versions)
}

async fn newest_loader(installer: &Installer, game_version: &str) -> Result<String> {
    available_loaders(installer, game_version)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| CoreError::Install(format!("no NeoForge build for {game_version}")))
}

/// Install NeoForge for `game_version`.
pub async fn install(
    installer: &Installer,
    game_version: &str,
    loader_version: Option<&str>,
) -> Result<InstalledVersion> {
    let loader = match loader_version {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => newest_loader(installer, game_version).await?,
    };

    let (url, sandbox_key) = if game_version == "1.20.1" {
        // Legacy coordinates embed the game version in the artifact.
        let artifact = format!("1.20.1-{loader}");
        (
            format!("{LEGACY_MAVEN}/{artifact}/forge-{artifact}-installer.jar"),
            format!("neoforge-legacy-{artifact}"),
        )
    } else {
        (
            format!("{NEOFORGE_MAVEN}/{loader}/neoforge-{loader}-installer.jar"),
            format!("neoforge-{loader}"),
        )
    };

    let version_id = forge::run_installer(installer, game_version, &url, &sandbox_key).await?;
    vanilla::install(installer, game_version).await?;
    let resolved = common::resolve_version(&installer.paths, &version_id).await?;
    let json_path = installer
        .paths
        .versions_dir()
        .join(&version_id)
        .join(format!("{version_id}.json"));
    Ok(common::installed(version_id, resolved, json_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_versions() {
        assert_eq!(neoforge_to_mc("20.4.237").as_deref(), Some("1.20.4"));
        assert_eq!(neoforge_to_mc("21.0.167").as_deref(), Some("1.21"));
        assert_eq!(neoforge_to_mc("21.1.77").as_deref(), Some("1.21.1"));
    }
}
