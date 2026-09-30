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
    // The base version document declares the Java this Minecraft version
    // needs; the installer is run with a runtime in that range.
    let base = vanilla::install(installer, game_version).await?;
    let required_major = base.details.required_java_major();
    let version_id =
        run_installer(installer, game_version, &url, &sandbox_key, required_major).await?;

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
    required_java_major: u32,
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

    let java = crate::launch::java::find_for_installer(
        &installer.client,
        &installer.paths,
        required_java_major,
        Some(installer.progress.clone()),
    )
    .await?
    .path;
    common::tick(
        &Some(installer.progress.clone()),
        format!("Running installer for {sandbox_key} (this may take a while)"),
    );

    // The installer resolves its target directory as `File(".")` when the
    // option carries no argument, and then insists on finding
    // `launcher_profiles.json` *inside that same directory*. Our sandbox
    // profile lives in `.minecraft`, so pass the directory explicitly.
    let target = sandbox.join(".minecraft");

    let output = tokio::process::Command::new(&java)
        .arg(format!("-Duser.home={}", sandbox.display()))
        .arg("-jar")
        .arg(&jar)
        .arg(format!("--installClient={}", target.display()))
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
    use crate::instance::LoaderType;

    #[test]
    fn parses_promotions() {
        let raw = r#"{"homepage":"x","promos":{"1.20.1-recommended":"47.2.0","1.20.1-latest":"47.3.0","1.19.2-latest":"43.2.0"}}"#;
        let p: Promotions = serde_json::from_str(raw).unwrap();
        assert_eq!(p.promos.get("1.20.1-recommended").unwrap(), "47.2.0");
    }

    /// The installer refuses to run unless `launcher_profiles.json` sits
    /// *inside* the target directory passed as `--installClient=<dir>`, and we
    /// pass `<sandbox>/.minecraft`. Keep those two in agreement.
    #[tokio::test]
    async fn sandbox_profile_lands_in_the_install_target() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ctm-forge-sandbox-{nanos}"));
        ensure_dir(&root).await.unwrap();
        write_launcher_profiles(&root).await.unwrap();

        let target = root.join(".minecraft");
        assert!(
            target.join("launcher_profiles.json").exists(),
            "installer profile missing from the target directory"
        );
        assert!(
            root.join("AppData")
                .join("Roaming")
                .join(".minecraft")
                .join("launcher_profiles.json")
                .exists(),
            "windows-style profile missing"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    fn e2e_root(tag: &str) -> PathBuf {
        match std::env::var("CTM_E2E_ROOT") {
            Ok(dir) => PathBuf::from(dir),
            Err(_) => {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                std::env::temp_dir().join(format!("ctm-{tag}-e2e-{nanos}"))
            }
        }
    }

    fn e2e_cleanup(root: &Path) {
        if std::env::var("CTM_E2E_ROOT").is_err() {
            let _ = std::fs::remove_dir_all(root);
        }
    }

    /// Spawn the prepared game, collect its output and report whether it was
    /// still running when the timeout hit (a good sign: the game reaches the
    /// main menu and idles there).
    async fn watch_launch(plan: &crate::launch::LaunchPlan, seconds: u64) -> Vec<String> {
        let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let mut cmd = tokio::process::Command::new(&plan.command[0]);
        cmd.args(&plan.command[1..])
            .current_dir(&plan.cwd)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        for (key, value) in std::env::vars() {
            if key == "DISPLAY" || key == "WAYLAND_DISPLAY" || key == "XDG_RUNTIME_DIR" {
                cmd.env(key, value);
            }
        }
        let mut child = cmd.spawn().expect("spawn game");
        fn pump<R>(pipe: R, sink: std::sync::Arc<std::sync::Mutex<Vec<String>>>)
        where
            R: tokio::io::AsyncRead + Unpin + Send + 'static,
        {
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let mut reader = tokio::io::BufReader::new(pipe);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => sink.lock().unwrap().push(line.trim_end().to_string()),
                    }
                }
            });
        }
        if let Some(pipe) = child.stdout.take() {
            pump(pipe, lines.clone());
        }
        if let Some(pipe) = child.stderr.take() {
            pump(pipe, lines.clone());
        }

        let wait = tokio::time::timeout(std::time::Duration::from_secs(seconds), child.wait()).await;
        match wait {
            Ok(status) => println!("game exited on its own: {status:?}"),
            Err(_) => println!("game still running after {seconds}s"),
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let output = lines.lock().unwrap().clone();
        output
    }

    /// Install `loader` for 1.20.1, build a launch plan and run the game for a
    /// while, printing how far it gets.
    async fn e2e_install_and_launch(loader: LoaderType, tag: &str) {
        use crate::auth::Account;
        use crate::util::Paths;

        let root = e2e_root(tag);
        let paths = Paths::rooted_at(&root);
        let progress: crate::util::ProgressCallback =
            std::sync::Arc::new(|p| println!("[progress] {}", p.message));
        let client = reqwest::Client::new();
        let installer = Installer::new(client.clone(), paths.clone(), progress.clone());

        let (game, loader_version, instance_id, instance_name) = match loader {
            LoaderType::Forge => (
                "1.20.1",
                Some("47.2.0".to_string()),
                "forge-e2e",
                "Forge E2E",
            ),
            LoaderType::Fabric => (
                "1.20.1",
                Some("0.15.11".to_string()),
                "fabric-e2e",
                "Fabric E2E",
            ),
            _ => unreachable!("e2e covers forge and fabric"),
        };
        let version_id = match loader {
            LoaderType::Forge => "1.20.1-forge-47.2.0".to_string(),
            _ => format!("fabric-loader-0.15.11-{game}"),
        };

        let json_path = paths
            .versions_dir()
            .join(&version_id)
            .join(format!("{version_id}.json"));
        if json_path.exists() {
            println!("reusing existing install at {}", json_path.display());
        } else {
            let installed = installer
                .install_loader(game, loader, loader_version.as_deref())
                .await
                .expect("loader install failed");
            println!("installed version id: {}", installed.id);
            assert_eq!(installed.id, version_id);
        }
        assert!(json_path.exists(), "version json missing after install");

        let instance = {
            let instance = crate::instance::Instance::new(
                paths.instance_dir(instance_id),
                crate::instance::InstanceMetadata::new(
                    instance_id,
                    instance_name,
                    game,
                    loader,
                    loader_version,
                ),
            );
            instance.scaffold().await.expect("scaffold instance");
            instance
        };

        let resolved = common::resolve_version(&paths, &version_id).await.expect("resolve");
        let required = resolved.required_java_major();
        let component = resolved.java_component().map(str::to_string);
        println!("required java major: {required}");

        let java = crate::launch::select_java(
            &client,
            &paths,
            &instance,
            component.as_deref(),
            required,
            Some(progress.clone()),
        )
        .await
        .expect("select java");
        println!("java: {} (major {})", java.path.display(), java.major);

        let launcher = crate::launch::Launcher::new(client.clone(), paths.clone(), progress);
        let plan = launcher
            .prepare(
                &instance,
                &Account::offline("E2ETester"),
                &java,
                &version_id,
                "ctm-e2e",
            )
            .await
            .expect("prepare launch");
        println!("command: {}", plan.display());

        let output = watch_launch(&plan, 45).await;
        println!("--- game output ({} lines) ---", output.len());
        for line in output.iter().take(80) {
            println!("  {line}");
        }
        assert!(
            output
                .iter()
                .any(|l| l.contains("Backend library") || l.contains("LWJGL") || l.contains("Sound engine") || l.contains("Reloading ResourceManager")),
            "game produced no recognizable startup output"
        );

        e2e_cleanup(&root);
    }

    /// Full network-backed Forge install + launch smoke test.
    ///
    /// Run manually: `cargo test -p mc_core forge_e2e -- --ignored --nocapture`
    /// Set `CTM_E2E_ROOT` to reuse a directory (and skip the reinstall).
    #[tokio::test]
    #[ignore]
    async fn forge_e2e_install_and_launch() {
        e2e_install_and_launch(LoaderType::Forge, "forge").await;
    }

    /// Same smoke test for Fabric, guarding the shared installer/launch paths.
    #[tokio::test]
    #[ignore]
    async fn fabric_e2e_install_and_launch() {
        e2e_install_and_launch(LoaderType::Fabric, "fabric").await;
    }
}
