//! Game execution: asset preparation, JVM argument construction and process
//! management.

pub mod arguments;
pub mod assets;
pub mod java;
pub mod process;

use std::path::PathBuf;

use crate::auth::Account;
use crate::error::{CoreError, Result};
use crate::install::common as install_common;
use crate::install::InstalledVersion;
use crate::instance::Instance;
use crate::util::{ensure_dir, Paths, ProgressCallback};
use crate::version::RuleContext;

pub use java::{discover as discover_java, find_for_major, JavaInstallation};
pub use process::{LogLine, LogReceiver, ProcessHandle, StreamKind};

/// Default launcher identity passed to the game.
pub const LAUNCHER_NAME: &str = "ctmlauncher";
pub const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// A fully prepared, ready-to-spawn launch command.
#[derive(Debug, Clone)]
pub struct LaunchPlan {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub natives_dir: PathBuf,
    pub version_id: String,
}

impl LaunchPlan {
    /// Render the command as a single shell-ish string for display.
    pub fn display(&self) -> String {
        self.command
            .iter()
            .map(|part| {
                if part.contains(' ') {
                    format!("\"{part}\"")
                } else {
                    part.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Orchestrates asset preparation and command construction.
#[derive(Clone)]
pub struct Launcher {
    pub client: reqwest::Client,
    pub paths: Paths,
    pub progress: ProgressCallback,
}

impl Launcher {
    pub fn new(client: reqwest::Client, paths: Paths, progress: ProgressCallback) -> Self {
        Self {
            client,
            paths,
            progress,
        }
    }

    /// Resolve a version id from disk and prepare it for launching.
    pub async fn prepare(
        &self,
        instance: &Instance,
        account: &Account,
        java: &JavaInstallation,
        version_id: &str,
        client_id: &str,
    ) -> Result<LaunchPlan> {
        let version = crate::install::common::resolve_version(&self.paths, version_id).await?;

        // Assets are downloaded lazily at launch time.
        assets::ensure_version_assets(
            &self.client,
            &self.paths,
            &version,
            Some(self.progress.clone()),
        )
        .await?;

        // Fresh natives directory per launch.
        let natives_dir = instance.root.join("natives");
        if natives_dir.exists() {
            let _ = tokio::fs::remove_dir_all(&natives_dir).await;
        }
        ensure_dir(&natives_dir).await?;

        let rule_ctx = RuleContext::current();
        assets::extract_natives(&self.paths, &version.libraries, &rule_ctx, &natives_dir)?;

        // Ensure the log4j configuration is present if the version declares one.
        let logging_file =
            install_common::download_logging(&self.client, &self.paths, version.logging.as_ref())
                .await?;

        let classpath = arguments::classpath(&self.paths, &version, &rule_ctx);
        let game_dir = instance.game_dir();
        ensure_dir(&game_dir).await?;

        let asset_index_name = version
            .asset_index
            .as_ref()
            .map(|a| a.id.clone())
            .unwrap_or_else(|| version.assets.clone());

        let user_type = match account.kind {
            crate::auth::AccountKind::Microsoft => "msa",
            crate::auth::AccountKind::Offline => "legacy",
        };

        let ctx = arguments::ArgContext {
            details: &version,
            rule_ctx: &rule_ctx,
            jvm: &instance.metadata.jvm,
            java: &java.path,
            paths: &self.paths,
            classpath: &classpath,
            natives_dir: &natives_dir,
            game_dir: &game_dir,
            assets_dir: &self.paths.assets_dir(),
            asset_index_name: &asset_index_name,
            username: &account.username,
            uuid: &account.id,
            access_token: &account.launch_token(),
            user_type,
            version_name: &version.id,
            launcher_name: LAUNCHER_NAME,
            launcher_version: LAUNCHER_VERSION,
            client_id,
            xuid: account.xuid.as_deref(),
            logging_file: logging_file.as_deref(),
        };

        let command = arguments::build(&ctx);
        Ok(LaunchPlan {
            command,
            cwd: game_dir,
            natives_dir,
            version_id: version_id.to_string(),
        })
    }

    /// Spawn a prepared plan.
    pub async fn start(&self, plan: &LaunchPlan) -> Result<(ProcessHandle, LogReceiver)> {
        process::spawn(&plan.command, &plan.cwd).await
    }

    /// Convenience: prepare and immediately start.
    pub async fn launch(
        &self,
        instance: &Instance,
        account: &Account,
        java: &JavaInstallation,
        version_id: &str,
        client_id: &str,
    ) -> Result<(LaunchPlan, ProcessHandle, LogReceiver)> {
        let plan = self
            .prepare(instance, account, java, version_id, client_id)
            .await?;
        let (handle, logs) = self.start(&plan).await?;
        Ok((plan, handle, logs))
    }
}

/// Select a Java runtime for a version: explicit path, then best match, then
/// any discovered runtime.
pub async fn select_java(instance: &Instance, required_major: u32) -> Result<JavaInstallation> {
    if let Some(path) = &instance.metadata.jvm.java_path {
        return java::probe(path).await;
    }
    if let Some(found) = java::find_for_major(required_major).await {
        return Ok(found);
    }
    java::discover()
        .await
        .into_iter()
        .next()
        .ok_or_else(|| CoreError::Launch("no Java runtime found; set one in Settings".into()))
}

/// Resolve a version id for an instance based on its loader metadata.
pub fn version_id_for(instance: &Instance) -> String {
    match (&instance.metadata.loader, &instance.metadata.loader_version) {
        (crate::instance::LoaderType::Vanilla, _) | (crate::instance::LoaderType::Paper, _) => {
            instance.metadata.game_version.clone()
        }
        (crate::instance::LoaderType::Fabric, Some(v)) => {
            format!("fabric-loader-{v}-{}", instance.metadata.game_version)
        }
        (crate::instance::LoaderType::Quilt, Some(v)) => {
            format!("quilt-loader-{v}-{}", instance.metadata.game_version)
        }
        (crate::instance::LoaderType::Forge, Some(v)) => {
            format!("{}-forge-{v}", instance.metadata.game_version)
        }
        (crate::instance::LoaderType::NeoForge, Some(v)) => {
            if instance.metadata.game_version == "1.20.1" {
                format!("1.20.1-forge-{v}")
            } else {
                format!("{}-neoforge-{v}", instance.metadata.game_version)
            }
        }
        _ => instance.metadata.game_version.clone(),
    }
}

/// Resolve a previously installed version for an instance.
pub async fn resolve_instance_version(
    paths: &Paths,
    instance: &Instance,
) -> Result<InstalledVersion> {
    let id = version_id_for(instance);
    let details = crate::install::common::resolve_version(paths, &id).await?;
    let json_path = paths.versions_dir().join(&id).join(format!("{id}.json"));
    Ok(InstalledVersion {
        id,
        details,
        json_path,
    })
}
