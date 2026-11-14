//! User settings (`config.toml`). No environment variables are required for anything.

use crate::versioned::{Step, upgrade};
use craft_anything_evidence::{ErrorCode, ToolError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Beginner,
    Experienced,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GhidraSettings {
    /// A Ghidra install to use instead of the managed one.
    pub install_dir: Option<PathBuf>,
    /// A Java runtime to use instead of the managed one.
    pub java_home: Option<PathBuf>,
    /// Seconds to wait for Ghidra to import and analyse a target.
    pub startup_timeout_secs: Option<u64>,
    /// A Ghidra language id to force, e.g. `RISCV:LE:64:RV64GC`, `PowerPC:BE:32:default`,
    /// `6502:LE:16:default`, `MIPS:BE:32:default`, `ARM:LE:32:v7`. Lets Deep mode target
    /// processors our own engine doesn't cover, with no per-architecture code. Only used in Deep
    /// mode with Ghidra installed.
    pub language_id: Option<String>,
    /// A Ghidra compiler-spec id to pair with `language_id` (e.g. `gcc`, `default`, `windows`).
    pub compiler_spec: Option<String>,
    /// Memory for Ghidra in GB (4 to 31). More makes its first look at a big program faster.
    /// `None`: half this computer's memory.
    pub memory_gb: Option<u64>,
}

/// One list for the per-engine path overrides: it declares the `ToolPaths` fields, `override_for`,
/// and the list the Engines settings page shows. The ids and their order are exactly the engines in
/// `craft_anything_engines::ENGINES` that have a path setting (a test checks it).
macro_rules! tool_paths {
    ($($id:ident),* $(,)?) => {
        /// Paths to external tools when the user prefers their own install over the managed one.
        #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct ToolPaths {
            $(pub $id: Option<PathBuf>,)*
        }

        impl ToolPaths {
            /// Every override as `(engine id, path)`, in settings-page order.
            pub fn entries_mut(&mut self) -> Vec<(&'static str, &mut Option<PathBuf>)> {
                vec![$((stringify!($id), &mut self.$id)),*]
            }
        }

        impl craft_anything_engines::ToolPathLookup for ToolPaths {
            fn override_for(&self, id: &str) -> Option<PathBuf> {
                match id {
                    $(stringify!($id) => self.$id.clone(),)*
                    _ => None,
                }
            }
        }
    };
}

// Ghidra is not here: its folder is `GhidraSettings::install_dir`.
tool_paths!(adb, jadx, apktool, chromium, frida, gdb, lldb, mitmdump, binwalk, unblob, ilspy, java, tshark, objdump, rizin, rtl433, vineflower);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineSettings {
    pub ghidra: GhidraSettings,
    pub tools: ToolPaths,
    /// Folder for managed engine installs; defaults to `<data dir>/engines`.
    pub store_dir: Option<PathBuf>,
    /// URL of a user-run JEB MCP server, e.g. `http://127.0.0.1:8425/mcp` (loopback, plain HTTP only).
    pub jeb_url: Option<String>,
    /// Find engines only where these settings and the managed store say, never on PATH or in the
    /// usual install folders. Never saved: test sessions set it so they don't find what this
    /// computer has installed ([`crate::testing::session`]).
    #[serde(skip)]
    pub only_configured: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The config.toml format version ([`SETTINGS_VERSION`]). Missing means 0: written before versions.
    pub version: u32,
    pub mode: Mode,
    /// Deep mode: tools may look inside the original (its code, files and page source), with our
    /// own engines and with Ghidra when it's installed. Off is Clean room: nothing inside the
    /// original is read, only what can be seen from outside ([`crate::clean_room`]).
    pub deep_mode: bool,
    /// `studio` (dark) or `studio-light`.
    pub theme: String,
    /// UI language (BCP 47), empty = system.
    pub language: String,
    pub first_run_done: bool,
    pub engines: EngineSettings,
    /// AI backends and roles. Owned and parsed by the `ai` crate; kept opaque here so the engine
    /// doesn't depend on it.
    pub ai: toml::Table,
    /// Ask before running tools that execute code or change devices (always true unless a project allows them).
    pub ask_before_running: bool,
    /// Auto mode: a build keeps going without stopping to ask the person. The builder's actions are
    /// allowed except a few that are never done unasked (deleting outside the project, system
    /// changes, publishing), and, unless `questions` says otherwise, its questions get "decide
    /// yourself". On by default.
    pub auto_mode: bool,
    /// What the builder's questions get in projects that don't choose (`Manifest::questions`):
    /// none follows Auto mode (decide when it's on, ask when it's off).
    pub questions: Option<crate::workspace::Questions>,
    /// Study an app or game target by running it once and picturing its window
    /// (`native_window_capture` into capture/window.png). Off by default: it runs the person's
    /// program.
    pub runtime_study: bool,
    /// Recently opened projects and targets.
    pub recent: Vec<PathBuf>,
    /// Where new projects are created; `None` = `Documents/Craft Anything`.
    pub projects_dir: Option<PathBuf>,
    /// More places projects are listed from: folders of projects, or projects started in a folder
    /// the person picked (added when they start one there).
    pub project_folders: Vec<PathBuf>,
    /// What the files already in a folder picked for a new project are: `None` asks each time;
    /// the person's "don't ask again" answer otherwise.
    pub existing_files: Option<crate::workspace::ExistingFiles>,
    /// Generated game assets: the budget a new project starts with and the picture model.
    pub assets: AssetSettings,
    /// External MCP servers whose tools are offered as Craft Anything tools (`[[plugins.mcp]]`).
    pub plugins: crate::plugins::PluginSettings,
    /// How builds are checked and when refining stops (`[build]`).
    pub build: BuildSettings,
    /// Why config.toml, though there, wasn't loaded (in plain words): it couldn't be read, or
    /// couldn't be used and couldn't be set aside. These settings are then defaults standing in
    /// for the person's, and [`Settings::save_to`] refuses to write them over that file. Never
    /// saved, so code that rebuilds settings from their saved form carries it over.
    #[serde(skip)]
    pub not_loaded: Option<String>,
}

/// How builds are checked and when refining stops.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildSettings {
    /// Likeness (percent) at which a compared build is good enough and refining stops.
    pub good_enough: f64,
    /// Refinement rounds after the first build in the app (the CLI's `--rounds` sets its own).
    pub max_rounds: u32,
    /// Widths (CSS px) a website and its rebuild are compared at.
    pub web_widths: Vec<u32>,
    /// Other pages of a website studied and checked besides the start page.
    pub max_pages: usize,
    /// When set, an app or game must look at least this alike (percent) to the person's pictures
    /// (or the original running) to be good enough; unset, that likeness is only reported.
    pub picture_good_enough: Option<f64>,
    /// Projects that may build at the same time; more wait their turn (at least 1).
    pub parallel: usize,
}

impl Default for BuildSettings {
    fn default() -> Self {
        BuildSettings { good_enough: 90.0, max_rounds: 6, web_widths: vec![1440, 768, 390], max_pages: 4, picture_good_enough: None, parallel: 2 }
    }
}

impl BuildSettings {
    /// The widths to compare at: the configured ones a capture can take (320-3840 px, at most 4),
    /// else the defaults (desktop, tablet and phone).
    pub fn widths(&self) -> Vec<u32> {
        let w: Vec<u32> = self.web_widths.iter().copied().filter(|w| (320..=3840).contains(w)).take(4).collect();
        if w.is_empty() { BuildSettings::default().web_widths } else { w }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssetSettings {
    /// What a new project may spend on generated pictures and 3D models before the person is
    /// asked (US dollars). Each project keeps its own budget once created.
    pub budget_usd: f64,
    /// The picture model, by its id in the asset table (`nano-banana-pro`, `flux-dev`…); `None` = the
    /// table's default.
    pub image_model: Option<String>,
}

impl Default for AssetSettings {
    fn default() -> Self {
        AssetSettings { budget_usd: crate::workspace::ASSET_BUDGET_USD, image_model: None }
    }
}

impl AssetSettings {
    /// The budget for a new project; a negative or non-number value gives the built-in default.
    pub fn budget(&self) -> f64 {
        if self.budget_usd.is_finite() && self.budget_usd >= 0.0 { self.budget_usd } else { crate::workspace::ASSET_BUDGET_USD }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: SETTINGS_VERSION,
            mode: Mode::Beginner,
            deep_mode: false,
            theme: "studio".into(),
            language: String::new(),
            first_run_done: false,
            engines: EngineSettings::default(),
            ai: toml::Table::new(),
            ask_before_running: true,
            auto_mode: true,
            questions: None,
            runtime_study: false,
            recent: Vec::new(),
            projects_dir: None,
            project_folders: Vec::new(),
            existing_files: None,
            assets: AssetSettings::default(),
            plugins: crate::plugins::PluginSettings::default(),
            build: BuildSettings::default(),
            not_loaded: None,
        }
    }
}

/// How many config files that couldn't be used are kept beside config.toml
/// (`config.toml.broken`, then `.broken.1`…): an earlier copy is never replaced by a later one.
const BROKEN_KEPT: u32 = 16;

/// Move a settings file that can't be used aside, under the first backup name that is free. On an
/// error (the system refuses, or every name is taken) the file is still where it was.
fn set_aside(path: &Path) -> std::io::Result<PathBuf> {
    for n in 0..BROKEN_KEPT {
        let to = path.with_extension(if n == 0 { "toml.broken".to_string() } else { format!("toml.broken.{n}") });
        match std::fs::symlink_metadata(&to) {
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        std::fs::rename(path, &to)?;
        return Ok(to);
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("{BROKEN_KEPT} earlier copies are already kept beside it")))
}

/// The current `config.toml` format version. Bumping it means appending a step to [`SETTINGS_STEPS`].
pub const SETTINGS_VERSION: u32 = 3;

/// The steps that bring an older `config.toml` forward; step `i` goes from version `i` to `i + 1`.
const SETTINGS_STEPS: &[Step<toml::Table>] = &[settings_v0_to_v1, settings_v1_to_v2, settings_v2_to_v3];
const _: () = assert!(SETTINGS_STEPS.len() == SETTINGS_VERSION as usize, "one step per config.toml version");

/// Version 0 (no `version` key) has the same shape as version 1; only the number is new.
fn settings_v0_to_v1(_: &mut toml::Table) -> Result<(), String> {
    Ok(())
}

/// Version 2: Deep mode is its own switch. `[engines.ghidra] enabled` (the old switch, which
/// already meant Deep mode with or without Ghidra installed) becomes the top-level `deep_mode`.
fn settings_v1_to_v2(table: &mut toml::Table) -> Result<(), String> {
    let old = table
        .get_mut("engines")
        .and_then(toml::Value::as_table_mut)
        .and_then(|e| e.get_mut("ghidra"))
        .and_then(toml::Value::as_table_mut)
        .and_then(|g| g.remove("enabled"));
    if let Some(v) = old {
        let on = v.as_bool().ok_or_else(|| format!("engines.ghidra.enabled ({v}) isn't true or false"))?;
        table.entry("deep_mode").or_insert(toml::Value::Boolean(on));
    }
    Ok(())
}

/// Version 3 adds more project folders and the answer about a picked folder's files; both start
/// empty. The number is new so an older build doesn't save over the list (and lose it).
fn settings_v2_to_v3(_: &mut toml::Table) -> Result<(), String> {
    Ok(())
}

/// Read a `config.toml` of any version: older ones are brought forward step by step; a newer one is
/// read as far as its keys are known and keeps its version, so [`Settings::save_to`] won't write it.
pub fn read_settings(text: &str) -> Result<Settings, String> {
    let mut table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
    let found = match table.get("version") {
        None => 0,
        Some(v) => v.as_integer().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| format!("its version ({v}) isn't a whole number"))?,
    };
    if found <= SETTINGS_VERSION {
        let reached = upgrade(&mut table, found, 0, SETTINGS_STEPS)?;
        table.insert("version".into(), toml::Value::Integer(reached.into()));
    }
    table.try_into::<Settings>().map_err(|e| e.to_string())
}

/// Where settings, logs and the engine store live: `CRAFT_ANYTHING_CONFIG_DIR`, then a portable
/// `portable.txt` marker next to the executable, then the platform config dir.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CRAFT_ANYTHING_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf))
        && exe_dir.join("portable.txt").exists()
    {
        return exe_dir.join("CraftAnythingData");
    }
    dirs::config_dir().map(|d| d.join("craft-anything")).unwrap_or_else(|| PathBuf::from(".craft-anything"))
}

impl Settings {
    /// The folder new projects go in.
    pub fn projects_dir(&self) -> PathBuf {
        self.projects_dir.clone().unwrap_or_else(crate::workspace::default_projects_dir)
    }

    /// What the builder's questions get in a project that doesn't choose.
    pub fn questions(&self) -> crate::workspace::Questions {
        use crate::workspace::Questions;
        self.questions.unwrap_or(if self.auto_mode { Questions::Decide } else { Questions::Ask })
    }

    /// What the builder's questions get in `project` (its own choice, else the app's default).
    pub fn questions_in(&self, project: Option<&crate::workspace::Manifest>) -> crate::workspace::Questions {
        project.and_then(|m| m.questions).unwrap_or_else(|| self.questions())
    }

    /// Everywhere projects are listed from: the folder new projects go in, then the others, each once.
    pub fn project_locations(&self) -> Vec<PathBuf> {
        let mut v = vec![self.projects_dir()];
        for f in &self.project_folders {
            if !v.contains(f) {
                v.push(f.clone());
            }
        }
        v
    }

    /// Keep a project started in a picked folder in the list. Returns whether the list changed:
    /// false when one of the locations already shows it (it is one, or sits directly in one).
    pub fn remember_project(&mut self, root: &Path) -> bool {
        let shown = self.project_locations().iter().any(|l| l == root || root.parent() == Some(l.as_path()));
        if !shown {
            self.project_folders.push(root.to_path_buf());
        }
        !shown
    }

    pub fn path() -> PathBuf {
        config_dir().join("config.toml")
    }

    /// Defaults standing in for a config.toml that is there but wasn't loaded; never saved over it.
    fn standing_in(why: String) -> Settings {
        Settings { not_loaded: Some(why), ..Settings::default() }
    }

    /// Load settings; a missing file gives defaults and an older one is brought up to date. A file
    /// that is there but can't be used is never a crash and never lost: one that can't be
    /// understood (not TOML, not UTF-8 text) is set aside under a new name and replaced with
    /// defaults; one that can't be read at all, or can't be set aside, stays where it is, and the
    /// defaults returned refuse to be saved over it ([`Settings::not_loaded`]).
    pub fn load_from(path: &Path) -> Settings {
        Settings::from_read(path, std::fs::read(path))
    }

    /// [`Settings::load_from`] given what reading `path` gave.
    fn from_read(path: &Path, read: std::io::Result<Vec<u8>>) -> Settings {
        let bytes = match read {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Settings::default(),
            Err(e) => {
                // It may hold the person's settings (a lock, a permission, a failing disk): the
                // defaults used meanwhile must never be written over it.
                log::warn!("{} couldn't be read ({e}); using defaults, and changes won't be saved over it", path.display());
                return Settings::standing_in(format!("{} couldn't be read ({e})", path.display()));
            }
        };
        let read = match std::str::from_utf8(&bytes) {
            Ok(text) => read_settings(text),
            Err(e) => Err(format!("it isn't UTF-8 text: {e}")),
        };
        let e = match read {
            Ok(s) => {
                if s.version > SETTINGS_VERSION {
                    log::warn!("{} is from a newer Craft Anything (version {}); changes won't be saved over it", path.display(), s.version);
                }
                return s;
            }
            Err(e) => e,
        };
        // A newer file this build can't read is left in place, and never saved over.
        let newer = toml::from_str::<toml::Table>(&String::from_utf8_lossy(&bytes))
            .ok()
            .and_then(|t| t.get("version").and_then(toml::Value::as_integer))
            .and_then(|n| u32::try_from(n).ok());
        if let Some(version) = newer.filter(|v| *v > SETTINGS_VERSION) {
            log::warn!("{} is from a newer Craft Anything (version {version}) and unreadable here ({e}); using defaults without saving", path.display());
            return Settings { version, ..Settings::default() };
        }
        match set_aside(path) {
            Ok(kept) => {
                log::warn!("{} is unreadable ({e}); using defaults and keeping it as {}", path.display(), kept.display());
                Settings::default()
            }
            Err(io) => {
                log::warn!("{} is unreadable ({e}) and couldn't be set aside ({io}); using defaults, and changes won't be saved over it", path.display());
                Settings::standing_in(format!("{} can't be used ({e}) and couldn't be set aside ({io})", path.display()))
            }
        }
    }

    /// Load startup settings, first recovering app edits deferred by a busy shutdown.
    pub fn load() -> Settings {
        Settings::load_waiting(&Settings::path(), &STARTUP_READ_WAITS, &mut |p| std::fs::read(p))
    }

    /// [`Settings::load`] for `path`. A virus scan or another program may hold config.toml for a
    /// moment, and the whole session would otherwise run on defaults that refuse every save, so
    /// it is read again after each of `waits`.
    fn load_waiting(path: &Path, waits: &[Duration], read_file: &mut dyn FnMut(&Path) -> std::io::Result<Vec<u8>>) -> Settings {
        let read = read_waiting(path, waits, &mut *read_file);
        // Still unreadable: deferred edits stay beside it for the next start, as the defaults
        // standing in for it can't be saved. Merged into a file read now, they'd be lost when the
        // session's own save at exit discards them.
        if matches!(&read, Err(e) if e.kind() != std::io::ErrorKind::NotFound) {
            return Settings::from_read(path, read);
        }
        if let Err(e) = Self::recover_pending(path) {
            log::warn!("Couldn't recover pending settings: {}", e.message);
        }
        Settings::from_read(path, read_waiting(path, waits, read_file))
    }

    /// Keep pending non-AI edits without touching config.toml or waiting for the engine session.
    /// The sidecar uses the settings schema/version and migrations; startup merges it with the
    /// latest committed AI table, so a writer still finishing at shutdown cannot be overwritten.
    pub fn defer_save_to(&self, path: &Path) -> Result<(), ToolError> {
        let mut pending = self.clone();
        pending.ai.clear();
        pending.save_to(&path.with_extension("pending.toml"))
    }

    /// Remove deferred edits superseded by a successful full save. A missing sidecar is fine.
    pub fn discard_pending(path: &Path) -> Result<(), ToolError> {
        let pending_path = path.with_extension("pending.toml");
        match std::fs::remove_file(&pending_path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(ToolError::io(pending_path.display(), e)),
        }
    }

    /// Recover a deferred snapshot only when both files are readable and supported. Leave the
    /// sidecar intact on failure, and remove it only after the merged settings are saved.
    fn recover_pending(path: &Path) -> Result<(), ToolError> {
        let pending_path = path.with_extension("pending.toml");
        let text = match std::fs::read_to_string(&pending_path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(ToolError::io(pending_path.display(), e)),
        };
        let bad_settings = |e: String| ToolError::new(ErrorCode::BadSettings, e);
        let mut pending = read_settings(&text).map_err(bad_settings)?;
        let current = match std::fs::read_to_string(path) {
            Ok(text) => read_settings(&text).map_err(bad_settings)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => return Err(ToolError::io(path.display(), e)),
        };
        if current.version > SETTINGS_VERSION || pending.version > SETTINGS_VERSION {
            return Err(ToolError::new(ErrorCode::BadSettings, "Update Craft Anything to recover settings saved by a newer version."));
        }
        pending.ai = current.ai;
        pending.save_to(path)?;
        Self::discard_pending(path)
    }

    /// Save settings privately. A non-empty parent is treated as a dedicated config directory:
    /// create it if needed and restrict it to 0700 on Unix before replacing the 0600 file.
    /// This includes an existing directory selected by `CRAFT_ANYTHING_CONFIG_DIR`; callers must
    /// not select an unrelated shared directory. Ancestors are not chmodded. A bare filename does
    /// not change the working directory's permissions. Directory preparation failure leaves any
    /// existing config file unchanged; directory creation/permission changes are not rolled back.
    pub fn save_to(&self, path: &Path) -> Result<(), ToolError> {
        self.save_to_with_dir_permissions(path, craft_anything_platform::fs::set_private_dir)
    }

    fn save_to_with_dir_permissions(&self, path: &Path, set_private_dir: impl FnOnce(&Path) -> Result<(), ToolError>) -> Result<(), ToolError> {
        if let Some(why) = &self.not_loaded {
            return Err(ToolError::new(ErrorCode::BadSettings, format!("Settings weren't saved: {why}, so these are defaults standing in for it."))
                .with_remediation("Nothing was written. Make that file readable or move it aside, then start Craft Anything again."));
        }
        if self.version > SETTINGS_VERSION {
            return Err(ToolError::new(
                ErrorCode::BadSettings,
                format!("{} was written by a newer Craft Anything (version {}), so this one doesn't change it.", path.display(), self.version),
            )
            .with_remediation("Update Craft Anything to change settings, or move config.toml aside to start fresh. Nothing was written."));
        }
        let text = toml::to_string_pretty(self).map_err(|e| ToolError::new(ErrorCode::BadSettings, e.to_string()))?;
        // `config.toml` can hold plugin headers and environment (API keys), so it is owner-only,
        // as is the folder that holds it (audit: craft-anything #2).
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| ToolError::io(dir.display(), e))?;
            set_private_dir(dir)?;
        }
        craft_anything_platform::write_atomic_private(path, text.as_bytes())
    }

    pub fn save(&self) -> Result<(), ToolError> {
        self.save_to(&Settings::path())
    }

    pub fn engine_store_dir(&self) -> PathBuf {
        self.engines.store_dir.clone().unwrap_or_else(|| config_dir().join("engines"))
    }

    /// Ghidra's headless launcher, when Deep mode can really use Ghidra (`None`: not installed).
    pub fn ghidra_launcher(&self) -> Option<PathBuf> {
        let (dir, store) = (self.engines.ghidra.install_dir.as_deref(), self.engine_store_dir());
        if self.engines.only_configured {
            craft_anything_engines::ghidra::get::headless_configured(dir, &store)
        } else {
            craft_anything_engines::ghidra::get::headless(dir, &store)
        }
    }

    /// The Java runtime Ghidra runs on (`None`: let Ghidra find its own).
    pub fn ghidra_java(&self) -> Option<PathBuf> {
        craft_anything_engines::ghidra::get::java_home(self.engines.ghidra.java_home.as_deref(), &self.engine_store_dir())
    }

    /// Where an external engine is on this computer, honouring every setting that can name one:
    /// the path overrides (`engines.tools`) and, for Ghidra, its own folder setting
    /// (`engines.ghidra.install_dir`). The one lookup behind every listing (`engines_list`,
    /// `doctor`, `capabilities`) and behind `Requires::Engine`, so they all agree with what Deep
    /// mode actually runs.
    pub fn engine_path(&self, id: &str) -> Option<PathBuf> {
        if id == craft_anything_engines::ghidra::get::ID {
            return self.ghidra_launcher();
        }
        if self.engines.only_configured {
            craft_anything_engines::locate_configured(id, &self.engines.tools, &self.engine_store_dir())
        } else {
            craft_anything_engines::locate(id, &self.engines.tools, &self.engine_store_dir())
        }
    }

    /// Why an engine wasn't found and what to do about it, as (reason, remediation).
    pub fn engine_not_found(&self, id: &str) -> (String, String) {
        use craft_anything_engines::ToolPathLookup;
        craft_anything_engines::not_found_help(id, self.engines.tools.override_for(id).as_deref(), craft_anything_platform::in_flatpak())
    }
}

/// How long startup waits before each new try at a config.toml that couldn't be read: 2 s in all.
const STARTUP_READ_WAITS: [Duration; 6] = [
    Duration::from_millis(100),
    Duration::from_millis(200),
    Duration::from_millis(300),
    Duration::from_millis(400),
    Duration::from_millis(500),
    Duration::from_millis(500),
];

/// Read `path`, trying again after each wait while it fails in a way that may pass (a lock, a
/// sharing violation, a denied access), not when it is missing or isn't a file at all.
fn read_waiting(path: &Path, waits: &[Duration], mut read: impl FnMut(&Path) -> std::io::Result<Vec<u8>>) -> std::io::Result<Vec<u8>> {
    use std::io::ErrorKind::{InvalidFilename, InvalidInput, IsADirectory, NotADirectory, NotFound};
    let mut waits = waits.iter();
    loop {
        match read(path) {
            Err(e) if !matches!(e.kind(), NotFound | IsADirectory | NotADirectory | InvalidFilename | InvalidInput) => match waits.next() {
                Some(wait) => {
                    log::info!("{} couldn't be read ({e}); trying again in {} ms", path.display(), wait.as_millis());
                    std::thread::sleep(*wait);
                }
                None => return Err(e),
            },
            done => return done,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn build_settings_have_safe_widths_and_old_configs_get_defaults() {
        let b = BuildSettings { web_widths: vec![10, 1280, 99_999, 375, 200], ..BuildSettings::default() };
        assert_eq!(b.widths(), [1280, 375], "widths a capture can't take are dropped");
        assert_eq!(BuildSettings { web_widths: vec![], ..BuildSettings::default() }.widths(), [1440, 768, 390]);
        let old: Settings = toml::from_str(
            "version = 1
auto_mode = true
",
        )
        .unwrap();
        assert_eq!(old.build, BuildSettings::default());
        assert_eq!(old.build.picture_good_enough, None, "picture likeness is only reported unless set");
    }

    use super::*;

    #[test]
    fn only_configured_finds_no_engine_from_this_computer_but_keeps_the_ones_set() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        s.engines.store_dir = Some(dir.path().join("engines"));
        s.engines.only_configured = true;
        assert_eq!(s.ghidra_launcher(), None, "PATH and the usual install folders aren't searched");
        assert_eq!(s.engine_path("java"), None);
        // The folder the person picked still counts, and so does the managed store.
        let launcher_name = if cfg!(windows) { "analyzeHeadless.bat" } else { "analyzeHeadless" };
        let picked = dir.path().join("picked").join("ghidra_12_PUBLIC");
        std::fs::create_dir_all(picked.join("support")).unwrap();
        std::fs::write(picked.join("support").join(launcher_name), "").unwrap();
        s.engines.ghidra.install_dir = Some(dir.path().join("picked"));
        assert_eq!(s.ghidra_launcher(), Some(picked.join("support").join(launcher_name)));
        assert_eq!(s.engine_path("ghidra"), s.ghidra_launcher());
        s.engines.ghidra.install_dir = None;
        let managed = dir.path().join("engines").join("ghidra").join("ghidra_12_PUBLIC").join("support");
        std::fs::create_dir_all(&managed).unwrap();
        std::fs::write(managed.join(launcher_name), "").unwrap();
        assert_eq!(s.ghidra_launcher(), Some(managed.join(launcher_name)));
        // It is never written to config.toml, and a loaded config looks around the computer.
        let text = toml::to_string_pretty(&s).unwrap();
        assert!(!text.contains("only_configured"), "{text}");
        let loaded: Settings = toml::from_str(&text).unwrap();
        assert!(!loaded.engines.only_configured);
    }

    #[test]
    fn the_test_session_looks_only_where_its_settings_say() {
        assert!(crate::testing::session().settings.engines.only_configured);
    }

    #[test]
    fn path_overrides_match_the_engine_table_exactly() {
        use craft_anything_engines::ToolPathLookup;
        let mut paths = ToolPaths::default();
        let ids: Vec<&str> = paths.entries_mut().into_iter().map(|(id, _)| id).collect();
        let table: Vec<&str> = craft_anything_engines::path_setting_ids().collect();
        assert_eq!(ids, table, "tool_paths! must list the engine table's path-setting ids, in table order");
        for (id, v) in paths.entries_mut() {
            *v = Some(PathBuf::from(format!("/opt/{id}")));
        }
        for id in table {
            assert_eq!(paths.override_for(id), Some(PathBuf::from(format!("/opt/{id}"))), "no override for {id}");
        }
        // An older config.toml without the newer fields still loads.
        let old: ToolPaths = toml::from_str("adb = \"/x/adb\"\n").unwrap();
        assert_eq!(old.adb, Some(PathBuf::from("/x/adb")));
        assert_eq!(old.tshark, None);
    }

    /// A config.toml as written before versions existed.
    const CONFIG_V0: &str = r#"mode = "experienced"
theme = "studio-light"
language = ""
first_run_done = true
ask_before_running = true
auto_mode = false
recent = ["/home/me/app.exe"]

[engines]
store_dir = "/data/engines"

[engines.ghidra]
enabled = true

[engines.tools]
adb = "/x/adb"

[ai]
default = "Claude Code"
"#;

    #[test]
    fn configs_from_before_versions_load_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, CONFIG_V0).unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s.version, SETTINGS_VERSION);
        assert_eq!((s.mode, s.theme.as_str(), s.first_run_done, s.auto_mode), (Mode::Experienced, "studio-light", true, false));
        assert!(s.deep_mode, "the old Ghidra switch becomes Deep mode");
        assert_eq!(s.engines.tools.adb, Some(PathBuf::from("/x/adb")));
        assert_eq!(s.ai.get("default").and_then(|v| v.as_str()), Some("Claude Code"));
        assert!(!path.with_extension("toml.broken").exists());
        s.save_to(&path).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.starts_with(&format!("version = {SETTINGS_VERSION}\n")) && saved.contains("deep_mode = true") && !saved.contains("enabled"), "{saved}");
        assert_eq!(Settings::load_from(&path), s);
    }

    #[test]
    fn newer_configs_load_but_are_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let newer = format!("version = 9\nsomething_new = 3\n{CONFIG_V0}");
        std::fs::write(&path, &newer).unwrap();
        let s = Settings::load_from(&path);
        assert_eq!((s.version, s.mode), (9, Mode::Experienced));
        let err = s.save_to(&path).unwrap_err();
        assert!(err.message.contains("newer") && err.remediation.is_some());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), newer);
        // Newer and unreadable here: defaults, but the file stays where it is and isn't saved over.
        let unreadable = "version = 9\nmode = { kind = \"pro\" }\n";
        std::fs::write(&path, unreadable).unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s, Settings { version: 9, ..Settings::default() });
        assert!(s.save_to(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), unreadable);
        assert!(!path.with_extension("toml.broken").exists());
        // A version that isn't a number is a broken file: set aside, defaults used.
        std::fs::write(&path, "version = \"two\"\n").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        assert!(path.with_extension("toml.broken").exists());
        assert!(read_settings("version = -1").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn config_is_saved_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Settings::default().save_to(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let nested = dir.path().join("cfg").join("config.toml");
        Settings::default().save_to(&nested).unwrap();
        assert_eq!(std::fs::metadata(nested.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn directory_permission_failure_leaves_config_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let old = b"previous settings";
        std::fs::write(&path, old).unwrap();
        let changed = Settings { language: "de".into(), ..Settings::default() };
        let error = changed
            .save_to_with_dir_permissions(&path, |parent| {
                assert_eq!(parent, dir.path());
                assert_eq!(std::fs::read(&path).unwrap(), old);
                Err(ToolError::io(parent.display(), std::io::Error::new(std::io::ErrorKind::PermissionDenied, "injected chmod refusal")))
            })
            .unwrap_err();
        assert!(error.message.contains("injected chmod refusal"));
        assert_eq!(std::fs::read(&path).unwrap(), old);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);

        let new_path = dir.path().join("new-config").join("config.toml");
        assert!(
            changed
                .save_to_with_dir_permissions(&new_path, |parent| {
                    assert!(parent.is_dir());
                    assert!(!new_path.exists());
                    Err(ToolError::io(parent.display(), std::io::Error::new(std::io::ErrorKind::PermissionDenied, "injected chmod refusal")))
                })
                .is_err()
        );
        assert!(!new_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn only_the_selected_existing_config_directory_is_made_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("selected-config");
        let unrelated = root.path().join("unrelated");
        std::fs::create_dir(&parent).unwrap();
        std::fs::create_dir(&unrelated).unwrap();
        for path in [root.path(), parent.as_path(), unrelated.as_path()] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = parent.join("config.toml");
        Settings::default()
            .save_to_with_dir_permissions(&path, |dir| {
                assert!(!path.exists(), "the file must not be written before securing its directory");
                craft_anything_platform::fs::set_private_dir(dir)
            })
            .unwrap();
        for (path, mode) in [(root.path(), 0o755), (parent.as_path(), 0o700), (unrelated.as_path(), 0o755), (path.as_path(), 0o600)] {
            assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, mode);
        }
    }

    #[test]
    fn round_trip_and_corrupt_file_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut s = Settings { mode: Mode::Experienced, ..Default::default() };
        s.deep_mode = true;
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
        std::fs::write(&path, "mode = [[[").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        assert!(path.with_extension("toml.broken").exists());
    }

    /// A config.toml that isn't UTF-8 used to read as "no file": defaults, then the next save over it.
    #[test]
    fn a_config_that_isnt_utf8_is_set_aside_whole_before_anything_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let first: &[u8] = b"mode = \"experienced\"\ntheme = \"caf\xe9\"\n";
        std::fs::write(&path, first).unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s, Settings::default(), "defaults that may be saved: the file is kept under another name");
        assert!(!path.exists());
        assert_eq!(std::fs::read(path.with_extension("toml.broken")).unwrap(), first, "every byte of it");
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
        // A later file that can't be used gets its own name; the earlier copy isn't replaced.
        let second: &[u8] = b"\xff\xfe not text at all";
        std::fs::write(&path, second).unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        assert_eq!(std::fs::read(path.with_extension("toml.broken")).unwrap(), first);
        assert_eq!(std::fs::read(path.with_extension("toml.broken.1")).unwrap(), second);
        // Not UTF-8, but plainly from a newer version: left where it is, like any newer file.
        let newer: &[u8] = b"version = 9\nmode = { kind = \"caf\xe9\" }\n";
        std::fs::write(&path, newer).unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s, Settings { version: 9, ..Settings::default() });
        assert!(s.save_to(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), newer);
        assert!(!path.with_extension("toml.broken.2").exists());
    }

    /// Any read failure but "not found" used to read as "no file" too. The defaults it gives now
    /// refuse to be saved (or deferred) over a file nobody could read or copy.
    #[test]
    fn a_config_that_cant_be_read_is_never_saved_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // A folder in its place can't be read as a file on any system, and can't be backed up as one.
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("mine.txt"), "mine").unwrap();
        let s = Settings::load_from(&path);
        assert!(s.not_loaded.as_deref().is_some_and(|why| why.contains("couldn't be read")), "{:?}", s.not_loaded);
        assert_eq!(Settings { not_loaded: None, ..s.clone() }, Settings::default());
        let mut edited = s.clone();
        edited.language = "de".into();
        for settings in [&s, &edited] {
            let err = settings.save_to(&path).unwrap_err();
            assert_eq!(err.code, ErrorCode::BadSettings);
            assert!(err.message.contains("couldn't be read") && err.remediation.is_some(), "{err:?}");
            assert!(settings.defer_save_to(&path).is_err(), "a deferred save would be merged over it at the next start");
        }
        assert!(!path.with_extension("pending.toml").exists());
        assert!(!path.with_extension("toml.broken").exists());
        assert_eq!(std::fs::read_to_string(path.join("mine.txt")).unwrap(), "mine");
        // The reason is not part of the file, and a missing file is still plain defaults.
        assert!(!toml::to_string_pretty(&s).unwrap().contains("not_loaded"));
        assert_eq!(Settings::load_from(&dir.path().join("none.toml")), Settings::default());
        assert_eq!(Settings::load_from(&dir.path().join("no-folder").join("config.toml")).not_loaded, None);
    }

    /// The file a person has, unreadable only for a moment (another program holds it): the
    /// defaults loaded meanwhile must not replace it once it can be written again.
    #[cfg(windows)]
    #[test]
    fn a_config_another_program_holds_is_not_replaced_by_defaults() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Settings { language: "de".into(), first_run_done: true, ..Settings::default() }.save_to(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        // Held with no sharing: reading fails with a sharing violation, not "not found".
        let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        let s = Settings::load_from(&path);
        assert!(s.not_loaded.is_some(), "a held file isn't a missing file");
        drop(held);
        assert!(s.save_to(&path).is_err(), "readable again, but what's in memory is still only defaults");
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(Settings::load_from(&path).language, "de", "the next start reads it as usual");
        // Held so it can be read but not renamed: a file that can't be used can't be set aside
        // either, so it stays, and isn't saved over.
        let garbled: &[u8] = b"mode = [[[ \xff";
        std::fs::write(&path, garbled).unwrap();
        let held = std::fs::OpenOptions::new().read(true).share_mode(1).open(&path).unwrap();
        let s = Settings::load_from(&path);
        assert!(s.not_loaded.as_deref().is_some_and(|why| why.contains("couldn't be set aside")), "{:?}", s.not_loaded);
        drop(held);
        assert!(s.save_to(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), garbled);
        assert!(!path.with_extension("toml.broken").exists());
    }

    /// The same on Unix, where it is a permission that stops the read.
    #[cfg(unix)]
    #[test]
    fn a_config_without_read_permission_is_not_replaced_by_defaults() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Settings { language: "de".into(), first_run_done: true, ..Settings::default() }.save_to(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&path).is_ok() {
            return; // Run as root: nothing is unreadable.
        }
        let s = Settings::load_from(&path);
        let refused = s.save_to(&path);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(s.not_loaded.is_some() && refused.is_err());
        assert!(s.save_to(&path).is_err(), "readable again, but what's in memory is still only defaults");
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(!path.with_extension("toml.broken").exists());
        assert_eq!(Settings::load_from(&path).language, "de");
    }

    /// Startup tries a file it couldn't read again, after each wait, and stops at "not found".
    #[test]
    fn a_config_that_cant_be_read_is_tried_again() {
        let path = Path::new("config.toml");
        let held = || std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let waits = [Duration::ZERO; 3];
        let mut tries = 0;
        let read = read_waiting(path, &waits, |_| {
            tries += 1;
            if tries < 3 { Err(held()) } else { Ok(b"language = \"de\"".to_vec()) }
        });
        assert_eq!((read.unwrap(), tries), (b"language = \"de\"".to_vec(), 3));
        tries = 0;
        let read = read_waiting(path, &waits, |_| {
            tries += 1;
            Err(held())
        });
        assert_eq!((read.unwrap_err().kind(), tries), (std::io::ErrorKind::PermissionDenied, 4));
        tries = 0;
        let read = read_waiting(path, &waits, |_| {
            tries += 1;
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        });
        assert_eq!((read.unwrap_err().kind(), tries), (std::io::ErrorKind::NotFound, 1));
        // Nor is a folder where the file should be.
        let dir = tempfile::tempdir().unwrap();
        tries = 0;
        let read = read_waiting(dir.path(), &waits, |p| {
            tries += 1;
            std::fs::read(p)
        });
        assert!(read.is_err());
        if cfg!(unix) {
            assert_eq!(tries, 1);
        }
        assert_eq!(STARTUP_READ_WAITS.iter().sum::<Duration>(), Duration::from_secs(2));
    }

    /// A file unreadable for a moment at startup is read once it can be, instead of the session
    /// running on defaults that refuse every save.
    #[cfg(unix)]
    #[test]
    fn a_config_readable_again_within_the_waits_is_loaded() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Settings { language: "de".into(), first_run_done: true, ..Settings::default() }.save_to(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&path).is_ok() {
            return; // Run as root: nothing is unreadable.
        }
        // Without waits, as before: defaults standing in for it.
        assert!(Settings::from_read(&path, read_waiting(&path, &[], |p| std::fs::read(p))).not_loaded.is_some());
        let mut tries = 0;
        let s = Settings::from_read(
            &path,
            read_waiting(&path, &[Duration::ZERO; 3], |p| {
                tries += 1;
                if tries == 3 {
                    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600)).unwrap();
                }
                std::fs::read(p)
            }),
        );
        assert_eq!((s.not_loaded, s.language.as_str(), tries), (None, "de", 3));
    }

    /// Edits deferred at the last exit are merged only into a file startup could read: while it
    /// can't, they stay beside it (the session's save at exit would otherwise discard them).
    #[cfg(unix)]
    #[test]
    fn deferred_edits_wait_for_a_config_startup_can_read() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Settings { language: "de".into(), first_run_done: true, ..Settings::default() }.save_to(&path).unwrap();
        Settings { language: "fr".into(), first_run_done: true, ..Settings::default() }.defer_save_to(&path).unwrap();
        let pending = path.with_extension("pending.toml");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&path).is_ok() {
            return; // Run as root: nothing is unreadable.
        }
        let s = Settings::load_waiting(&path, &[Duration::ZERO; 2], &mut |p| std::fs::read(p));
        assert!(s.not_loaded.is_some() && s.save_to(&path).is_err());
        assert!(pending.exists(), "kept for the next start");
        // Readable on the second try: the edits are merged, not left for an exit save to discard.
        let mut tries = 0;
        let s = Settings::load_waiting(&path, &[Duration::ZERO; 2], &mut |p| {
            tries += 1;
            if tries == 2 {
                std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
            std::fs::read(p)
        });
        assert_eq!((s.not_loaded, s.language.as_str(), pending.exists()), (None, "fr", false));
    }

    /// A file that can't be used and can't be set aside (here: every backup name is taken) stays
    /// where it is, and the defaults used instead are never saved over it.
    #[test]
    fn a_config_that_cant_be_set_aside_is_never_saved_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let kept = |n: u32| path.with_extension(if n == 0 { "toml.broken".to_string() } else { format!("toml.broken.{n}") });
        for n in 0..BROKEN_KEPT {
            std::fs::write(kept(n), format!("earlier {n}")).unwrap();
        }
        let garbled = "mode = [[[";
        std::fs::write(&path, garbled).unwrap();
        let s = Settings::load_from(&path);
        assert!(s.not_loaded.as_deref().is_some_and(|why| why.contains("couldn't be set aside")), "{:?}", s.not_loaded);
        assert_eq!(s.save_to(&path).unwrap_err().code, ErrorCode::BadSettings);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), garbled);
        for n in 0..BROKEN_KEPT {
            assert_eq!(std::fs::read_to_string(kept(n)).unwrap(), format!("earlier {n}"), "no earlier copy is replaced");
        }
        // With a name free again it is set aside there, and settings can be saved.
        std::fs::remove_file(kept(7)).unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s, Settings::default());
        assert_eq!(std::fs::read_to_string(kept(7)).unwrap(), garbled);
        s.save_to(&path).unwrap();
    }

    /// Deferred saves cannot replace damaged/newer data, and failed recovery keeps both originals.
    #[test]
    fn deferred_settings_keep_ai_and_refuse_unsupported_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let pending_path = path.with_extension("pending.toml");
        let mut pending = Settings { language: "de".into(), ..Settings::default() };
        pending.ai.insert("model".into(), "stale".into());
        pending.defer_save_to(&path).unwrap();
        let pending_text = std::fs::read_to_string(&pending_path).unwrap();
        assert!(read_settings(&pending_text).unwrap().ai.is_empty());
        assert!(!path.exists(), "deferring never writes the shared config");
        for invalid in ["mode = [[[", "version = 999\n", "version = 999\nmode = []"] {
            std::fs::write(&path, invalid).unwrap();
            assert!(Settings::recover_pending(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
            assert_eq!(std::fs::read_to_string(&pending_path).unwrap(), pending_text);
        }
        let mut current = Settings::default();
        current.ai.insert("model".into(), "latest".into());
        current.save_to(&path).unwrap();
        let original = std::fs::read_to_string(&path).unwrap();
        for invalid in ["mode = [[[", "version = 999\n"] {
            std::fs::write(&pending_path, invalid).unwrap();
            assert!(Settings::recover_pending(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
            assert_eq!(std::fs::read_to_string(&pending_path).unwrap(), invalid);
        }
        std::fs::write(&pending_path, &pending_text).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(Settings::recover_pending(&path).is_err(), "I/O failure must not consume pending edits");
        assert_eq!(std::fs::read_to_string(&pending_path).unwrap(), pending_text);
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, original).unwrap();
        // An old sidecar migrates through the existing settings schema before recovery.
        std::fs::write(&pending_path, "language = \"de\"\n[ai]\nmodel = \"stale\"\n").unwrap();
        Settings::recover_pending(&path).unwrap();
        let recovered = Settings::load_from(&path);
        assert_eq!(recovered.version, SETTINGS_VERSION);
        assert_eq!(recovered.language, "de");
        assert_eq!(recovered.ai, current.ai);
        assert!(!pending_path.exists());
    }

    #[test]
    fn the_asset_budget_is_a_setting_and_old_configs_load() {
        let old: Settings = toml::from_str("mode = \"experienced\"\nauto_mode = false\n").unwrap();
        assert_eq!(old.assets.budget(), 5.0, "a config without [assets] gets the built-in budget");
        assert_eq!(old.assets.image_model, None);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut s = Settings::default();
        s.assets.budget_usd = 12.5;
        s.assets.image_model = Some("flux-schnell".into());
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
        let odd: Settings = toml::from_str("[assets]\nbudget_usd = -3.0\n").unwrap();
        assert_eq!(odd.assets.budget(), 5.0, "a negative budget isn't used");
    }

    #[test]
    fn projects_are_listed_from_more_folders_and_a_picked_one_is_remembered() {
        let mut s = Settings { projects_dir: Some(PathBuf::from("/p")), ..Settings::default() };
        assert_eq!(s.project_locations(), [PathBuf::from("/p")]);
        assert!(!s.remember_project(Path::new("/p/notes")), "already listed: it's in the projects folder");
        assert!(s.remember_project(Path::new("/work/mine")));
        assert!(!s.remember_project(Path::new("/work/mine")), "once");
        s.project_folders.push(PathBuf::from("/p"));
        assert_eq!(s.project_locations(), [PathBuf::from("/p"), PathBuf::from("/work/mine")], "each place once");
        // Saved and read back, with the answer about a picked folder's files.
        s.existing_files = Some(crate::workspace::ExistingFiles::StartFrom);
        let back = read_settings(&toml::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.project_folders, s.project_folders);
        assert_eq!(back.existing_files, s.existing_files);
        // A version 2 config has neither, and asks.
        let v2 = read_settings("version = 2\ndeep_mode = true\n").unwrap();
        assert_eq!(v2.version, SETTINGS_VERSION);
        assert!(v2.project_folders.is_empty() && v2.existing_files.is_none() && v2.deep_mode);
    }
}
