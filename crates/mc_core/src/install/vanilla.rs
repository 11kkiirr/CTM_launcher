//! Vanilla Minecraft installation from Mojang's metadata.

use crate::error::{CoreError, Result};
use crate::install::common;
use crate::install::{InstalledVersion, Installer};
use crate::version::RuleContext;

/// Install the vanilla client and its libraries.
pub async fn install(installer: &Installer, game_version: &str) -> Result<InstalledVersion> {
    common::tick(
        &Some(installer.progress.clone()),
        format!("Resolving {game_version}"),
    );

    let manifest = common::fetch_manifest(&installer.client).await?;
    let summary = manifest
        .versions
        .iter()
        .find(|v| v.id == game_version)
        .ok_or_else(|| CoreError::NotFound(format!("Minecraft version {game_version}")))?;

    let sha1 = (!summary.sha1.is_empty()).then_some(summary.sha1.as_str());
    let details = common::download_version_json(
        &installer.client,
        &installer.paths,
        &summary.id,
        &summary.url,
        sha1,
    )
    .await?;

    common::tick(
        &Some(installer.progress.clone()),
        format!("Downloading client jar for {}", summary.id),
    );
    let client = details.client_download()?.clone();
    common::download_client(
        &installer.client,
        &installer.paths,
        &summary.id,
        &client,
        Some(installer.progress.clone()),
    )
    .await?;

    common::tick(
        &Some(installer.progress.clone()),
        format!("Downloading {} libraries", details.libraries.len()),
    );
    let ctx = RuleContext::current();
    common::download_libraries(
        &installer.client,
        &installer.paths,
        &details.libraries,
        &ctx,
        Some(installer.progress.clone()),
    )
    .await?;

    common::download_logging(
        &installer.client,
        &installer.paths,
        details.logging.as_ref(),
    )
    .await?;

    let json_path = installer
        .paths
        .versions_dir()
        .join(&details.id)
        .join(format!("{}.json", details.id));
    Ok(common::installed(details.id.clone(), details, json_path))
}

/// Resolve the list of available vanilla versions, newest first.
pub async fn available(installer: &Installer) -> Result<Vec<crate::version::VersionSummary>> {
    let manifest = common::fetch_manifest(&installer.client).await?;
    Ok(manifest.versions)
}

/// Find a manifest entry by id.
pub async fn find(
    installer: &Installer,
    game_version: &str,
) -> Result<crate::version::VersionSummary> {
    let manifest = common::fetch_manifest(&installer.client).await?;
    manifest
        .versions
        .into_iter()
        .find(|v| v.id == game_version)
        .ok_or_else(|| CoreError::NotFound(format!("Minecraft version {game_version}")))
}
