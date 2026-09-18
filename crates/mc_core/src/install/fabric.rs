//! Fabric installation via the Fabric Meta API.

use serde::Deserialize;

use crate::error::{CoreError, Result};
use crate::install::common;
use crate::install::vanilla;
use crate::install::{InstalledVersion, Installer};
use crate::util::fetch_json;
use crate::version::{RuleContext, VersionDetails};

const META: &str = "https://meta.fabricmc.net/v2";

#[derive(Debug, Deserialize)]
struct LoaderEntry {
    loader: LoaderInfo,
}

#[derive(Debug, Deserialize)]
struct LoaderInfo {
    version: String,
    #[serde(default)]
    stable: bool,
}

/// List Fabric loader versions available for a game version (newest first).
pub async fn available_loaders(installer: &Installer, game_version: &str) -> Result<Vec<String>> {
    let url = format!("{META}/versions/loader/{game_version}");
    let entries: Vec<LoaderEntry> = fetch_json(&installer.client, &url).await?;
    Ok(entries.into_iter().map(|e| e.loader.version).collect())
}

/// Pick the newest stable loader, falling back to the newest overall.
async fn latest_stable_loader(installer: &Installer, game_version: &str) -> Result<String> {
    let url = format!("{META}/versions/loader/{game_version}");
    let entries: Vec<LoaderEntry> = fetch_json(&installer.client, &url).await?;
    entries
        .iter()
        .find(|e| e.loader.stable)
        .or_else(|| entries.first())
        .map(|e| e.loader.version.clone())
        .ok_or_else(|| CoreError::Install(format!("no Fabric loader available for {game_version}")))
}

/// Install Fabric for `game_version`.
pub async fn install(
    installer: &Installer,
    game_version: &str,
    loader_version: Option<&str>,
) -> Result<InstalledVersion> {
    let loader = match loader_version {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => latest_stable_loader(installer, game_version).await?,
    };

    common::tick(
        &Some(installer.progress.clone()),
        format!("Fetching Fabric {loader} profile"),
    );
    let profile_url = format!("{META}/versions/loader/{game_version}/{loader}/profile/json");
    let profile: VersionDetails = fetch_json(&installer.client, &profile_url).await?;

    // Fabric profiles inherit the vanilla version; make sure it is present.
    vanilla::install(installer, game_version).await?;

    common::tick(
        &Some(installer.progress.clone()),
        format!("Downloading Fabric {loader} libraries"),
    );
    let ctx = RuleContext::current();
    common::download_libraries(
        &installer.client,
        &installer.paths,
        &profile.libraries,
        &ctx,
        Some(installer.progress.clone()),
    )
    .await?;

    let json_path = common::write_version_json(&installer.paths, &profile.id, &profile).await?;
    let resolved = common::resolve_version(&installer.paths, &profile.id).await?;
    Ok(common::installed(profile.id, resolved, json_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_loader_list() {
        let raw = r#"[
            {"loader":{"separator":".","build":1,"maven":"x","version":"0.15.7","stable":true}},
            {"loader":{"separator":".","build":2,"maven":"y","version":"0.16.0-beta","stable":false}}
        ]"#;
        let entries: Vec<LoaderEntry> = serde_json::from_str(raw).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].loader.stable);
        assert_eq!(entries[0].loader.version, "0.15.7");
    }
}
