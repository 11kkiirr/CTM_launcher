//! Modpack import pipeline for Modrinth `.mrpack` archives.

pub mod mrpack;

use std::path::Path;

use futures::stream::{self, StreamExt};

use crate::error::{CoreError, Result};
use crate::install::Installer;
use crate::instance::{Instance, InstanceManager, ModpackOrigin};
use crate::util::{download_file, Paths, Progress, ProgressCallback};

pub use mrpack::{
    extract_overrides, file_destination, parse_mrpack, MrpackEnv, MrpackFile, MrpackIndex,
};

/// Orchestrates creation of an instance from a `.mrpack` archive.
#[derive(Clone)]
pub struct ModpackInstaller {
    pub installer: Installer,
    pub instances: InstanceManager,
    pub paths: Paths,
    pub progress: ProgressCallback,
}

impl ModpackInstaller {
    pub fn new(
        installer: Installer,
        instances: InstanceManager,
        paths: Paths,
        progress: ProgressCallback,
    ) -> Self {
        Self {
            installer,
            instances,
            paths,
            progress,
        }
    }

    /// Import a `.mrpack` file, returning the created instance.
    pub async fn import(
        &self,
        archive: impl AsRef<Path>,
        name_override: Option<&str>,
    ) -> Result<Instance> {
        let archive = archive.as_ref();
        let index = parse_mrpack(archive)?;

        let game_version = index.game_version().ok_or_else(|| {
            CoreError::Modpack("modpack does not declare a Minecraft version".into())
        })?;
        let (loader, loader_version) = index.loader();

        let fallback_name = archive
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported Pack")
            .to_string();
        let name = name_override
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
            .or_else(|| (!index.name.is_empty()).then(|| index.name.clone()))
            .unwrap_or(fallback_name);

        (self.progress)(Progress::new(format!("Creating instance '{name}'")));
        let mut instance = self
            .instances
            .create(&name, &game_version, loader, loader_version.clone())
            .await?;

        // Install the base game + loader.
        self.installer
            .install_loader(&game_version, loader, loader_version.as_deref())
            .await?;

        // Download all client-side files.
        self.download_files(&instance, &index).await?;

        // Apply overrides.
        (self.progress)(Progress::new("Extracting overrides"));
        extract_overrides(archive, instance.game_dir())?;

        // Record provenance.
        instance.metadata.modpack = Some(ModpackOrigin {
            name: index.name.clone(),
            version: Some(index.version_id.clone()),
            source: Some("Modrinth".to_string()),
        });
        instance.save().await?;

        Ok(instance)
    }

    async fn download_files(&self, instance: &Instance, index: &MrpackIndex) -> Result<()> {
        let game_dir = instance.game_dir();
        let client = self.installer.client.clone();
        let total = index.files.iter().filter(|f| f.applies_to_client()).count();
        let progress = self.progress.clone();

        (progress)(Progress {
            current: 0,
            total: Some(total as u64),
            message: format!("Downloading {total} modpack files"),
        });

        let files: Vec<MrpackFile> = index
            .files
            .iter()
            .filter(|f| f.applies_to_client())
            .cloned()
            .collect();

        let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let results: Vec<Result<()>> = stream::iter(files)
            .map(|file| {
                let game_dir = game_dir.clone();
                let client = client.clone();
                let progress = progress.clone();
                let done = done.clone();
                async move {
                    let url = file
                        .downloads
                        .first()
                        .ok_or_else(|| {
                            CoreError::Modpack(format!("no download URL for {}", file.path))
                        })?
                        .clone();
                    let dest = file_destination(&game_dir, &file)?;
                    download_file(&client, &url, &dest, file.sha1(), None).await?;
                    let count = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    (progress)(Progress {
                        current: count as u64,
                        total: Some(total as u64),
                        message: format!("{count}/{total} files"),
                    });
                    Ok(())
                }
            })
            .buffer_unordered(16)
            .collect()
            .await;

        for result in results {
            result?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::LoaderType;

    #[test]
    fn loader_selection_from_index() {
        let index: MrpackIndex = serde_json::from_str(
            r#"{"dependencies":{"minecraft":"1.20.1","quilt-loader":"0.23.1"}}"#,
        )
        .unwrap();
        let (loader, version) = index.loader();
        assert_eq!(loader, LoaderType::Quilt);
        assert_eq!(version.as_deref(), Some("0.23.1"));
    }
}
