//! Game and modloader installation.
//!
//! The [`Installer`] facade resolves a Mojang base version and then layers a
//! modloader profile on top of it. Every loader ultimately produces an
//! [`InstalledVersion`] pointing at a resolved JSON document that the launcher
//! can execute.

pub mod common;
pub mod fabric;
pub mod forge;
pub mod neoforge;
pub mod quilt;
pub mod vanilla;

use std::path::PathBuf;

use crate::error::Result;
use crate::instance::LoaderType;
use crate::util::{Paths, ProgressCallback};
use crate::version::{VersionDetails, VersionManifest};

/// A resolved, launch-ready version.
#[derive(Debug, Clone)]
pub struct InstalledVersion {
    /// Version id used to locate the JSON document.
    pub id: String,
    /// Fully merged version details (inheritance resolved).
    pub details: VersionDetails,
    /// Path to the resolved JSON document on disk.
    pub json_path: PathBuf,
}

/// Facade that ties together HTTP, paths and progress reporting for installs.
#[derive(Clone)]
pub struct Installer {
    pub client: reqwest::Client,
    pub paths: Paths,
    pub progress: ProgressCallback,
}

impl Installer {
    pub fn new(client: reqwest::Client, paths: Paths, progress: ProgressCallback) -> Self {
        Self {
            client,
            paths,
            progress,
        }
    }

    /// Fetch the Mojang version manifest.
    pub async fn manifest(&self) -> Result<VersionManifest> {
        common::fetch_manifest(&self.client).await
    }

    /// Install only the vanilla base version (client jar + libraries).
    pub async fn install_vanilla(&self, game_version: &str) -> Result<InstalledVersion> {
        vanilla::install(self, game_version).await
    }

    /// Install a base version plus the requested loader.
    ///
    /// When `loader_version` is `None` the loader's "recommended"/latest stable
    /// version is selected automatically.
    pub async fn install_loader(
        &self,
        game_version: &str,
        loader: LoaderType,
        loader_version: Option<&str>,
    ) -> Result<InstalledVersion> {
        match loader {
            LoaderType::Vanilla | LoaderType::Paper => self.install_vanilla(game_version).await,
            LoaderType::Fabric => fabric::install(self, game_version, loader_version).await,
            LoaderType::Quilt => quilt::install(self, game_version, loader_version).await,
            LoaderType::Forge => forge::install(self, game_version, loader_version).await,
            LoaderType::NeoForge => neoforge::install(self, game_version, loader_version).await,
        }
    }

    /// Resolve a previously installed version from disk.
    pub async fn resolve(&self, id: &str) -> Result<InstalledVersion> {
        let details = common::resolve_version(&self.paths, id).await?;
        let json_path = self
            .paths
            .versions_dir()
            .join(id)
            .join(format!("{id}.json"));
        Ok(InstalledVersion {
            id: id.to_string(),
            details,
            json_path,
        })
    }
}
