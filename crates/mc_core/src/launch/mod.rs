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

/// Select a Java runtime for a version.
///
/// Order of preference:
/// 1. The instance's explicit Java path.
/// 2. An installed runtime that satisfies the required major version.
/// 3. The Mojang-provided runtime for the version's component (downloaded on
///    demand into the launcher's `java/` directory).
/// 4. Any discovered runtime, as a last resort.
pub async fn select_java(
    client: &reqwest::Client,
    paths: &Paths,
    instance: &Instance,
    component: Option<&str>,
    required_major: u32,
    progress: Option<ProgressCallback>,
) -> Result<JavaInstallation> {
    if let Some(path) = &instance.metadata.jvm.java_path {
        return java::probe(path).await;
    }
    if let Some(found) = java::find_for_major(required_major).await {
        return Ok(found);
    }
    if let Ok(found) =
        java::ensure_runtime(client, paths, component, required_major, progress).await
    {
        return Ok(found);
    }
    java::discover()
        .await
        .into_iter()
        .next()
        .ok_or_else(|| CoreError::Launch("no Java runtime found; set one in Settings".into()))
}

/// Candidate version ids for an instance, most specific first.
///
/// Loader installers do not agree on id conventions: Forge uses
/// `{game}-forge-{loader}`, Fabric/Quilt use `{loader}-loader-{version}-{game}`,
/// while NeoForge (1.20.2+) uses `neoforge-{loader}`. We therefore try several
/// candidates and fall back to scanning the installed versions.
pub fn candidate_version_ids(instance: &Instance) -> Vec<String> {
    use crate::instance::LoaderType::*;
    let game = &instance.metadata.game_version;
    let version = instance.metadata.loader_version.as_deref();

    match (&instance.metadata.loader, version) {
        (Vanilla | Paper, _) => vec![game.clone()],
        (Fabric, Some(v)) => vec![format!("fabric-loader-{v}-{game}")],
        (Quilt, Some(v)) => vec![format!("quilt-loader-{v}-{game}")],
        (Forge, Some(v)) => vec![format!("{game}-forge-{v}")],
        (NeoForge, Some(v)) if game == "1.20.1" => vec![format!("{game}-forge-{v}")],
        (NeoForge, Some(v)) => vec![format!("neoforge-{v}"), format!("{game}-neoforge-{v}")],
        (_, None) => vec![game.clone()],
    }
}

/// The canonical version id for an instance (first candidate).
pub fn version_id_for(instance: &Instance) -> String {
    candidate_version_ids(instance)
        .into_iter()
        .next()
        .unwrap_or_else(|| instance.metadata.game_version.clone())
}

/// Path to a version's JSON document.
pub fn version_json_path(paths: &Paths, id: &str) -> PathBuf {
    paths.versions_dir().join(id).join(format!("{id}.json"))
}

/// Locate an installed version for `instance`, or `None` when not installed.
pub async fn installed_version_id(paths: &Paths, instance: &Instance) -> Option<String> {
    for id in candidate_version_ids(instance) {
        if version_json_path(paths, &id).exists() {
            return Some(id);
        }
    }
    scan_installed_version(paths, instance).await
}

/// Scan the versions directory for a document that inherits the instance's game
/// version and belongs to the same loader. Handles id-convention drift.
async fn scan_installed_version(paths: &Paths, instance: &Instance) -> Option<String> {
    let root = paths.versions_dir();
    let entries = std::fs::read_dir(&root).ok()?;
    let game = &instance.metadata.game_version;
    let loader_key = instance.metadata.loader.as_str();

    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let dir = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let json = dir.join(format!("{name}.json"));
        let Ok(bytes) = std::fs::read(&json) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let inherits = value
            .get("inheritsFrom")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let id = value.get("id").and_then(|v| v.as_str()).unwrap_or(&name);
        if inherits == game && id.to_ascii_lowercase().contains(loader_key) {
            return Some(id.to_string());
        }
    }
    None
}

/// Resolve a previously installed version for an instance.
pub async fn resolve_instance_version(
    paths: &Paths,
    instance: &Instance,
) -> Result<InstalledVersion> {
    let id = installed_version_id(paths, instance).await.ok_or_else(|| {
        CoreError::NotFound(format!(
            "installed version for '{}' ({})",
            instance.name(),
            instance.metadata.descriptor()
        ))
    })?;
    let details = crate::install::common::resolve_version(paths, &id).await?;
    let json_path = version_json_path(paths, &id);
    Ok(InstalledVersion {
        id,
        details,
        json_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::{Instance, InstanceMetadata, LoaderType};

    fn instance(loader: LoaderType, version: Option<&str>) -> Instance {
        let metadata = InstanceMetadata::new(
            "test",
            "Test",
            "1.21.1",
            loader,
            version.map(str::to_string),
        );
        Instance::new(PathBuf::from("/tmp/test"), metadata)
    }

    #[test]
    fn neoforge_prefers_bare_id() {
        let candidates = candidate_version_ids(&instance(LoaderType::NeoForge, Some("21.1.248")));
        assert_eq!(candidates[0], "neoforge-21.1.248");
        assert_eq!(candidates[1], "1.21.1-neoforge-21.1.248");
    }

    #[test]
    fn forge_and_fabric_ids() {
        assert_eq!(
            version_id_for(&instance(LoaderType::Forge, Some("47.2.0"))),
            "1.21.1-forge-47.2.0"
        );
        assert_eq!(
            version_id_for(&instance(LoaderType::Fabric, Some("0.15.7"))),
            "fabric-loader-0.15.7-1.21.1"
        );
    }

    #[test]
    fn vanilla_id_is_game_version() {
        assert_eq!(
            version_id_for(&instance(LoaderType::Vanilla, None)),
            "1.21.1"
        );
    }

    #[tokio::test]
    async fn finds_installed_neoforge_bare_id() {
        // Regression: NeoForge installs `neoforge-<version>`, not
        // `<game>-neoforge-<version>`.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ctm-ver-{nanos}"));
        let paths = Paths::rooted_at(&dir);
        let vdir = paths.versions_dir().join("neoforge-21.1.248");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(
            vdir.join("neoforge-21.1.248.json"),
            br#"{"id":"neoforge-21.1.248","inheritsFrom":"1.21.1"}"#,
        )
        .unwrap();

        let inst = instance(LoaderType::NeoForge, Some("21.1.248"));
        assert_eq!(
            installed_version_id(&paths, &inst).await.as_deref(),
            Some("neoforge-21.1.248")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn scans_when_canonical_id_differs() {
        // A directory whose name does not match the JSON id is still found.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ctm-ver2-{nanos}"));
        let paths = Paths::rooted_at(&dir);
        let vdir = paths.versions_dir().join("weird-dir");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(
            vdir.join("weird-dir.json"),
            br#"{"id":"neoforge-21.1.248","inheritsFrom":"1.21.1"}"#,
        )
        .unwrap();

        let inst = instance(LoaderType::NeoForge, Some("21.1.248"));
        assert_eq!(
            installed_version_id(&paths, &inst).await.as_deref(),
            Some("neoforge-21.1.248")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
