//! The dedicated `CODEX_HOME` (design §2.3, D9): Codex's config and sign-in live here, apart from
//! the student's own Codex (`~/.codex`, whose AGENTS.md, skills and MCP servers — including
//! PageLamp's own v0.1 entry — must not leak into PageLamp's runs).
//!
//! - PageLamp writes only `config.toml` (before every Codex command, under the lock) and
//!   `pagelamp.lock`. It never reads anything else here: not `auth.json`, not the "Codex Auth"
//!   keychain item. The folder never goes into logs, reports or exports.
//! - The lock is held around each config rewrite + run, and around login and logout: the desktop
//!   app, the CLI and the Swift shell share this folder, and how Codex rotates tokens under two
//!   concurrent runs is unknown. A second process gets `Busy`, never a parallel run.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = "config.toml";
pub const LOCK_FILE: &str = "pagelamp.lock";
/// The model catalog PageLamp points Codex at (`model_catalog_json`).
pub const CATALOG_FILE: &str = "pagelamp-models.json";

/// The pinned models' entries, copied unchanged from 0.158.0's `models.json` (Apache-2.0; see
/// `data/codex-models.NOTICE`).
const VENDORED_MODELS: &str = include_str!("../../data/codex-models.json");

/// The catalog Codex gets: the vendored entries with exactly two fields changed.
/// 0.158.0 takes a model's tool mode from its catalog entry before any feature switch
/// (`requested_tool_mode`), and every pinned model says `"code_mode_only"`, which would give the
/// model a JavaScript `exec` tool whatever `features.code_mode` says. `model_catalog_json` is a
/// documented Codex setting that replaces the bundled catalog; Codex itself stays unmodified.
/// - `tool_mode: "direct"`: no code mode;
/// - `shell_type: "disabled"`: no shell tool (a second switch next to `features.shell_tool`).
pub fn model_catalog() -> String {
    let mut catalog: serde_json::Value =
        serde_json::from_str(VENDORED_MODELS).expect("codex-models.json is valid");
    for model in catalog["models"].as_array_mut().expect("a models array") {
        model["tool_mode"] = "direct".into();
        model["shell_type"] = "disabled".into();
    }
    serde_json::to_string_pretty(&catalog).expect("the catalog serialises")
}

/// The config PageLamp writes (design §2.3; D11: analytics off). Every key was checked against
/// `codex-rs/core/config.schema.json` of the pinned 0.158.0, where unknown keys fail
/// `--strict-config` (the root, `[features]`, `[tools]` and the other tables allow no extra
/// keys). Differences from the design's list: `view_image` is a feature (there is no
/// `tools.view_image`), `code_mode` takes a plain boolean, and 0.158.0 has more tools to turn off
/// (browser and computer use, image generation, plan updates, user input requests). Left out on
/// purpose, because 0.158.0 ignores them: `unified_exec` (forced on unless managed requirements
/// pin it; it only picks the shell backend, and `shell_tool = false` removes every shell tool),
/// `js_repl` and `plugin_hooks` (skipped), and the legacy aliases `collab`, `codex_hooks` and
/// `[features] web_search` (the top-level `web_search` is the real switch). The same tool
/// switches are passed again as `-c` overrides on every run, and the JSONL tripwire kills a run
/// that uses a tool anyway.
pub const CONFIG_TOML: &str = r#"# Written by PageLamp before every Codex command it starts; changes here are overwritten.
# This folder is PageLamp's own CODEX_HOME: your own Codex (~/.codex) is never used or changed.
cli_auth_credentials_store = "auto"
check_for_update_on_startup = false
project_root_markers = []
project_doc_max_bytes = 0
web_search = "disabled"

[analytics]
enabled = false

[feedback]
enabled = false

[history]
persistence = "none"

[tools.update_plan]
enabled = false

[tools.experimental_request_user_input]
enabled = false

[features]
shell_tool = false
apps = false
plugins = false
multi_agent = false
code_mode = false
hooks = false
view_image = false
image_generation = false
standalone_web_search = false
browser_use = false
computer_use = false
memories = false

[windows]
sandbox = "unelevated"
"#;

/// Passed again as `-c` overrides on every run (design §2.3), so a system or managed config layer
/// can't turn a tool back on. Each is also set in `CONFIG_TOML` (a test checks).
pub const RUN_OVERRIDES: &[&str] = &[
    "features.shell_tool=false",
    "features.apps=false",
    "features.plugins=false",
    "features.multi_agent=false",
    "features.code_mode=false",
    "features.hooks=false",
    "features.view_image=false",
    "features.image_generation=false",
    "features.standalone_web_search=false",
    "features.browser_use=false",
    "features.computer_use=false",
    "features.memories=false",
    "tools.update_plan.enabled=false",
    "tools.experimental_request_user_input.enabled=false",
    "web_search=\"disabled\"",
    "history.persistence=\"none\"",
    "project_doc_max_bytes=0",
];

/// A path as a TOML string (quoted and escaped, so Windows paths survive).
pub(crate) fn toml_string(path: &Path) -> String {
    toml::Value::String(path.to_string_lossy().into_owned()).to_string()
}

#[derive(Debug, thiserror::Error)]
pub enum HomeError {
    /// Another PageLamp process (or window) is running Codex, signing in or out.
    #[error("Codex is in use by another PageLamp window")]
    Busy,
    #[error("could not prepare Codex's folder: {0}")]
    Io(String),
}

impl From<std::io::Error> for HomeError {
    fn from(err: std::io::Error) -> Self {
        HomeError::Io(err.to_string())
    }
}

/// `<local>/codex-home`.
#[derive(Clone, Debug)]
pub struct CodexHome {
    dir: PathBuf,
}

/// Held while PageLamp rewrites the config and runs a Codex command; released on drop (and by
/// the OS if the process dies).
#[derive(Debug)]
pub struct HomeLock {
    _file: File,
}

impl CodexHome {
    pub fn new(dir: impl Into<PathBuf>) -> CodexHome {
        CodexHome { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Take the lock without waiting (`Busy` when another process or window holds it).
    pub fn lock(&self) -> Result<HomeLock, HomeError> {
        pagelamp_core::paths::create_private_dir_all(&self.dir)?;
        let file = self.open_lock_file()?;
        match super::process::try_lock_briefly(&file) {
            Ok(()) => Ok(HomeLock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(HomeError::Busy),
            Err(TryLockError::Error(err)) => Err(err.into()),
        }
    }

    /// Whether someone holds the lock right now (probe: take and release it).
    pub fn is_locked(&self) -> bool {
        let Ok(file) = self.open_lock_file() else {
            return false;
        };
        matches!(file.try_lock(), Err(TryLockError::WouldBlock))
    }

    /// Rewrite `config.toml` and the model catalog it names (each atomically: a temporary file,
    /// then a rename). Needs the lock.
    pub fn write_config(&self, _lock: &HomeLock) -> Result<(), HomeError> {
        self.write_atomically(CATALOG_FILE, model_catalog().as_bytes())?;
        // A top-level key: it goes before the first table, or TOML would put it in that table.
        let first_table = CONFIG_TOML
            .find("\n[")
            .map_or(CONFIG_TOML.len(), |at| at + 1);
        let config = format!(
            "{}# PageLamp's model catalog (see pagelamp-models.json).\nmodel_catalog_json = {}\n\n{}",
            &CONFIG_TOML[..first_table],
            toml_string(&self.catalog_path()?),
            &CONFIG_TOML[first_table..]
        );
        self.write_atomically(CONFIG_FILE, config.as_bytes())
    }

    /// The absolute path of the model catalog (Codex requires an absolute path).
    pub fn catalog_path(&self) -> Result<PathBuf, HomeError> {
        Ok(std::path::absolute(self.dir.join(CATALOG_FILE))?)
    }

    fn write_atomically(&self, name: &str, bytes: &[u8]) -> Result<(), HomeError> {
        let target = self.dir.join(name);
        let temp = self.dir.join(format!("{name}.{}.tmp", std::process::id()));
        {
            let mut file = File::create(&temp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        std::fs::rename(&temp, &target).inspect_err(|_| {
            let _ = std::fs::remove_file(&temp);
        })?;
        Ok(())
    }

    fn open_lock_file(&self) -> std::io::Result<File> {
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join(LOCK_FILE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_is_valid_toml_with_every_tool_off() {
        let config: toml::Table = toml::from_str(CONFIG_TOML).unwrap();
        assert_eq!(config["cli_auth_credentials_store"].as_str(), Some("auto"));
        assert_eq!(config["web_search"].as_str(), Some("disabled"));
        assert_eq!(config["analytics"]["enabled"].as_bool(), Some(false), "D11");
        assert_eq!(config["history"]["persistence"].as_str(), Some("none"));
        assert_eq!(
            config["project_root_markers"].as_array().map(Vec::len),
            Some(0)
        );
        assert_eq!(config["windows"]["sandbox"].as_str(), Some("unelevated"));
        let features = config["features"].as_table().unwrap();
        assert!(
            features.values().all(|v| v.as_bool() == Some(false)),
            "{features:?}"
        );
        for tool in [
            "shell_tool",
            "apps",
            "multi_agent",
            "code_mode",
            "view_image",
        ] {
            assert!(features.contains_key(tool), "{tool}");
        }
        // Keys 0.158.0 ignores stay out (see CONFIG_TOML).
        for ignored in [
            "unified_exec",
            "js_repl",
            "plugin_hooks",
            "collab",
            "codex_hooks",
            "web_search",
        ] {
            assert!(!features.contains_key(ignored), "{ignored}");
        }
        // No MCP server is ever configured here.
        assert!(!config.contains_key("mcp_servers"));
        // Every run override says the same as the config.
        for over in RUN_OVERRIDES {
            let (path, value) = over.split_once('=').unwrap();
            let expected: toml::Value = toml::from_str::<toml::Table>(&format!("v = {value}"))
                .unwrap()
                .remove("v")
                .unwrap();
            let actual = path
                .split('.')
                .try_fold(&toml::Value::Table(config.clone()), |node, key| {
                    node.get(key)
                })
                .cloned();
            assert_eq!(actual, Some(expected), "{over}");
        }
    }

    /// For the CI contract test (`codex --strict-config` against the real binary):
    /// `CODEX_CONFIG_OUT=<dir> cargo test -p pagelamp-llm --lib -- --ignored contract_config`
    /// writes `<dir>/config.toml` exactly as PageLamp does.
    #[test]
    #[ignore = "writes the config for the CI contract test"]
    fn contract_config() {
        let dir = std::env::var_os("CODEX_CONFIG_OUT").expect("CODEX_CONFIG_OUT");
        let home = CodexHome::new(dir);
        let lock = home.lock().unwrap();
        home.write_config(&lock).unwrap();
    }

    #[test]
    fn the_catalog_differs_from_codexs_own_entries_in_two_fields_only() {
        let original: serde_json::Value = serde_json::from_str(VENDORED_MODELS).unwrap();
        let ours: serde_json::Value = serde_json::from_str(&model_catalog()).unwrap();
        let (original, ours) = (
            original["models"].as_array().unwrap(),
            ours["models"].as_array().unwrap(),
        );
        assert_eq!(original.len(), ours.len());
        for (before, after) in original.iter().zip(ours) {
            let (before, after) = (before.as_object().unwrap(), after.as_object().unwrap());
            assert_eq!(
                before.keys().collect::<Vec<_>>(),
                after.keys().collect::<Vec<_>>()
            );
            let changed: Vec<&String> = before
                .keys()
                .filter(|key| before[*key] != after[*key])
                .collect();
            assert!(
                changed
                    .iter()
                    .all(|key| *key == "tool_mode" || *key == "shell_type"),
                "{}: {changed:?}",
                before["slug"]
            );
            assert_eq!(after["tool_mode"], "direct");
            assert_eq!(after["shell_type"], "disabled");
        }
        // Every model the pin allows has an entry.
        let slugs: Vec<&str> = ours.iter().filter_map(|m| m["slug"].as_str()).collect();
        for model in &crate::codex::pin().models.supported {
            assert!(slugs.contains(&model.as_str()), "{model}");
        }
    }

    #[test]
    fn one_holder_at_a_time_and_the_config_is_rewritten() {
        let temp = tempfile::tempdir().unwrap();
        let home = CodexHome::new(temp.path().join("codex-home"));
        assert!(!home.is_locked());
        let lock = home.lock().unwrap();
        assert!(home.is_locked());
        assert!(matches!(home.lock(), Err(HomeError::Busy)));
        std::fs::write(home.dir().join(CONFIG_FILE), "model = \"someone-elses\"\n").unwrap();
        home.write_config(&lock).unwrap();
        let written = std::fs::read_to_string(home.dir().join(CONFIG_FILE)).unwrap();
        let config: toml::Table = toml::from_str(&written).unwrap();
        assert!(
            !config["windows"]
                .as_table()
                .unwrap()
                .contains_key("model_catalog_json"),
            "a top-level key"
        );
        assert_eq!(
            written.replace(
                &format!(
                    "# PageLamp's model catalog (see pagelamp-models.json).\nmodel_catalog_json = {}\n\n",
                    toml_string(&home.catalog_path().unwrap())
                ),
                ""
            ),
            CONFIG_TOML
        );
        let catalog = config["model_catalog_json"].as_str().unwrap();
        assert!(Path::new(catalog).is_absolute());
        assert_eq!(
            std::fs::read_to_string(catalog).unwrap(),
            model_catalog(),
            "the catalog next to the config"
        );
        drop(lock);
        assert!(!home.is_locked());
        home.lock().unwrap();
        // Only the config, the catalog and the lock file: nothing temporary is left.
        let mut names: Vec<String> = std::fs::read_dir(home.dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, [CONFIG_FILE, CATALOG_FILE, LOCK_FILE]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(home.dir()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "it may hold a credential file");
        }
    }
}
