//! JVM and game argument construction.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::instance::JvmConfig;
use crate::util::Paths;
use crate::version::{Argument, RuleContext, VersionDetails};

/// Everything needed to assemble a launch command line.
pub struct ArgContext<'a> {
    pub details: &'a VersionDetails,
    pub rule_ctx: &'a RuleContext,
    pub jvm: &'a JvmConfig,
    pub java: &'a Path,
    pub paths: &'a Paths,
    pub classpath: &'a [PathBuf],
    pub natives_dir: &'a Path,
    pub game_dir: &'a Path,
    pub assets_dir: &'a Path,
    pub asset_index_name: &'a str,
    pub username: &'a str,
    pub uuid: &'a str,
    pub access_token: &'a str,
    pub user_type: &'a str,
    pub version_name: &'a str,
    pub launcher_name: &'a str,
    pub launcher_version: &'a str,
    pub client_id: &'a str,
    pub xuid: Option<&'a str>,
    pub logging_file: Option<&'a Path>,
}

/// Platform-specific classpath separator.
pub fn classpath_separator() -> char {
    if cfg!(windows) {
        ';'
    } else {
        ':'
    }
}

/// Build the full argument vector (including the Java executable).
pub fn build(ctx: &ArgContext<'_>) -> Vec<String> {
    let mut argv = Vec::new();
    argv.push(ctx.java.to_string_lossy().to_string());

    let substitutions = substitutions(ctx);

    // --- Memory ---------------------------------------------------------
    argv.push(format!("-Xms{}M", ctx.jvm.min_memory_mb));
    argv.push(format!("-Xmx{}M", ctx.jvm.max_memory_mb));

    // --- Garbage collector ---------------------------------------------
    for flag in ctx.jvm.gc.flags() {
        argv.push((*flag).to_string());
    }

    // --- JVM-provided version arguments ---------------------------------
    if let Some(arguments) = &ctx.details.arguments {
        for argument in &arguments.jvm {
            append_argument(&mut argv, argument, ctx.rule_ctx, &substitutions);
        }
    }

    // --- Native / LWJGL paths -------------------------------------------
    let natives = ctx.natives_dir.to_string_lossy().to_string();
    argv.push(format!("-Djava.library.path={natives}"));
    argv.push(format!("-Djna.tmpdir={natives}"));
    argv.push(format!(
        "-Dorg.lwjgl.system.SharedLibraryExtractPath={natives}"
    ));
    argv.push(format!("-Dio.netty.native.workdir={natives}"));
    argv.push("-Dminecraft.launcher.brand=ctmlauncher".to_string());
    argv.push(format!(
        "-Dminecraft.launcher.version={}",
        ctx.launcher_version
    ));

    if let Some(logging) = ctx.logging_file {
        argv.push(format!(
            "-Dlog4j.configurationFile={}",
            logging.to_string_lossy()
        ));
    }

    // --- Classpath -------------------------------------------------------
    let separator = classpath_separator();
    let classpath = ctx
        .classpath
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(&separator.to_string());
    argv.push("-cp".to_string());
    argv.push(classpath);

    // --- User JVM flags --------------------------------------------------
    for flag in &ctx.jvm.custom_jvm_args {
        argv.push(flag.clone());
    }

    // --- Main class ------------------------------------------------------
    argv.push(ctx.details.main_class.clone());

    // --- Game arguments --------------------------------------------------
    if let Some(arguments) = &ctx.details.arguments {
        for argument in &arguments.game {
            append_argument(&mut argv, argument, ctx.rule_ctx, &substitutions);
        }
    } else if let Some(legacy) = &ctx.details.minecraft_arguments {
        for token in legacy.split_whitespace() {
            argv.push(substitute(token, &substitutions));
        }
    }

    // --- Extra user game arguments --------------------------------------
    for arg in &ctx.jvm.extra_game_args {
        argv.push(arg.clone());
    }

    argv
}

fn append_argument(
    argv: &mut Vec<String>,
    argument: &Argument,
    rule_ctx: &RuleContext,
    substitutions: &HashMap<&'static str, String>,
) {
    match argument {
        Argument::Plain(value) => argv.push(substitute(value, substitutions)),
        Argument::Conditional { rules, value } => {
            if rule_ctx.allows(rules) {
                for item in value.as_slice() {
                    argv.push(substitute(item, substitutions));
                }
            }
        }
    }
}

/// Build the `${token}` substitution table.
fn substitutions(ctx: &ArgContext<'_>) -> HashMap<&'static str, String> {
    let mut map = HashMap::new();
    let cp_sep = classpath_separator().to_string();
    let classpath = ctx
        .classpath
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(&cp_sep);

    map.insert("auth_player_name", ctx.username.to_string());
    map.insert("version_name", ctx.version_name.to_string());
    map.insert("game_directory", ctx.game_dir.to_string_lossy().to_string());
    map.insert("assets_root", ctx.assets_dir.to_string_lossy().to_string());
    map.insert("assets_index_name", ctx.asset_index_name.to_string());
    map.insert("auth_uuid", ctx.uuid.to_string());
    map.insert("auth_access_token", ctx.access_token.to_string());
    map.insert("auth_session", format!("token:{}", ctx.access_token));
    map.insert("user_type", ctx.user_type.to_string());
    map.insert("version_type", ctx.details.kind.clone());
    map.insert("user_properties", "{}".to_string());
    map.insert("clientid", ctx.client_id.to_string());
    map.insert("auth_xuid", ctx.xuid.unwrap_or_default().to_string());
    map.insert(
        "natives_directory",
        ctx.natives_dir.to_string_lossy().to_string(),
    );
    map.insert("launcher_name", ctx.launcher_name.to_string());
    map.insert("launcher_version", ctx.launcher_version.to_string());
    map.insert("classpath", classpath);
    map.insert("classpath_separator", cp_sep);
    map.insert(
        "library_directory",
        ctx.paths.libraries_dir().to_string_lossy().to_string(),
    );
    map.insert("resolution_width", "854".to_string());
    map.insert("resolution_height", "480".to_string());
    map
}

/// Replace every `${key}` occurrence using `map`.
pub fn substitute(input: &str, map: &HashMap<&'static str, String>) -> String {
    let mut output = input.to_string();
    for (key, value) in map {
        let token = format!("${{{key}}}");
        if output.contains(&token) {
            output = output.replace(&token, value);
        }
    }
    output
}

/// Assemble the classpath for a resolved version.
///
/// Includes the client jar plus every applicable library artifact. Native
/// classifier jars are extracted separately and are not placed on the
/// classpath, matching the official launcher.
pub fn classpath(paths: &Paths, details: &VersionDetails, rule_ctx: &RuleContext) -> Vec<PathBuf> {
    let mut entries = Vec::new();

    if let Some(path) = client_jar_path(paths, details) {
        entries.push(path);
    }

    for library in &details.libraries {
        if !library.applies(rule_ctx) {
            continue;
        }
        let Some(artifact) = &library.downloads.artifact else {
            // Legacy libraries without explicit downloads still have a Maven path.
            if library.url.is_some() {
                if let Ok(rel) = library.relative_path() {
                    entries.push(paths.libraries_dir().join(rel));
                }
            }
            continue;
        };
        let rel = artifact
            .path
            .clone()
            .unwrap_or_else(|| library.relative_path().unwrap_or_default());
        if !rel.is_empty() {
            entries.push(paths.libraries_dir().join(rel));
        }
    }

    entries
}

/// Resolve the vanilla client jar location from the merged version document.
pub fn client_jar_path(paths: &Paths, details: &VersionDetails) -> Option<PathBuf> {
    if let Some(client) = &details.downloads.client {
        if let Some(path) = &client.path {
            let candidate = paths.versions_dir().join(path);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    // Fallback: `<versions>/<id>/<id>.jar`.
    let candidate = paths
        .versions_dir()
        .join(&details.id)
        .join(format!("{}.jar", details.id));
    candidate.exists().then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitution_replaces_tokens() {
        let mut map = HashMap::new();
        map.insert("auth_player_name", "Steve".to_string());
        assert_eq!(
            substitute("--username ${auth_player_name}", &map),
            "--username Steve"
        );
    }

    #[test]
    fn classpath_separator_is_platform_correct() {
        if cfg!(windows) {
            assert_eq!(classpath_separator(), ';');
        } else {
            assert_eq!(classpath_separator(), ':');
        }
    }
}
