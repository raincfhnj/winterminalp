use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::prefix::{
    KeyChord, LogicalKey, PrefixConfig, default_prefix_chord, is_reserved_system_chord,
    shortcut_specs,
};
use crate::{AppError, AppResult};

pub const CONFIG_SCHEMA_VERSION: u32 = 2;
const LEGACY_CONFIG_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_PREFIX_TIMEOUT_MS: u64 = 1_500;
pub const DISABLED_SHORTCUT: &str = "disabled";
const MIN_PREFIX_TIMEOUT_MS: u64 = 250;
const MAX_PREFIX_TIMEOUT_MS: u64 = 5_000;
const MAX_DIVIDER_HIT_SLOP_PX: u8 = 32;
const MIN_PANE_GEOMETRY_POLL_INTERVAL_MS: u64 = 50;
const MAX_PANE_GEOMETRY_POLL_INTERVAL_MS: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MouseResizeConfig {
    /// Enables native Windows Terminal divider dragging through UI Automation.
    pub enabled: bool,
    /// Extra clickable pixels on either side of the visible native divider.
    ///
    /// 8px matches the grab tolerance users expect from native window borders
    /// while staying below the smallest pane's half-width.
    pub divider_hit_slop_px: u8,
    /// Refresh cadence for the disposable native pane geometry snapshot.
    pub geometry_poll_interval_ms: u64,
}

impl Default for MouseResizeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            divider_hit_slop_px: 8,
            geometry_poll_interval_ms: 100,
        }
    }
}

impl MouseResizeConfig {
    fn validate(self) -> AppResult<()> {
        if self.divider_hit_slop_px > MAX_DIVIDER_HIT_SLOP_PX {
            return Err(AppError::InvalidConfiguration(format!(
                "mouse_resize.divider_hit_slop_px must be between 0 and {MAX_DIVIDER_HIT_SLOP_PX}"
            )));
        }
        if !(MIN_PANE_GEOMETRY_POLL_INTERVAL_MS..=MAX_PANE_GEOMETRY_POLL_INTERVAL_MS)
            .contains(&self.geometry_poll_interval_ms)
        {
            return Err(AppError::InvalidConfiguration(format!(
                "mouse_resize.geometry_poll_interval_ms must be between {MIN_PANE_GEOMETRY_POLL_INTERVAL_MS} and {MAX_PANE_GEOMETRY_POLL_INTERVAL_MS}"
            )));
        }
        Ok(())
    }

    #[must_use]
    pub const fn geometry_poll_interval(self) -> Duration {
        Duration::from_millis(self.geometry_poll_interval_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ControllerConfig {
    pub schema_version: u32,
    pub prefix_timeout_ms: u64,
    pub launch_terminal_on_start: bool,
    #[serde(default = "default_prefix")]
    pub prefix: String,
    #[serde(default = "default_shortcuts")]
    pub shortcuts: BTreeMap<String, String>,
    #[serde(default)]
    pub mouse_resize: MouseResizeConfig,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            prefix_timeout_ms: DEFAULT_PREFIX_TIMEOUT_MS,
            launch_terminal_on_start: true,
            prefix: default_prefix(),
            shortcuts: default_shortcuts(),
            mouse_resize: MouseResizeConfig::default(),
        }
    }
}

impl ControllerConfig {
    pub fn load(path: &Path) -> AppResult<Self> {
        let source = fs::read_to_string(path)
            .map_err(|error| AppError::io("read controller config", path, error))?;
        let mut config: Self = toml::from_str(&source).map_err(|error| {
            AppError::InvalidConfiguration(format!("{}: {error}", path.display()))
        })?;
        config.migrate_schema();
        config.validate()?;
        Ok(config)
    }

    pub fn load_or_create(path: &Path) -> AppResult<Self> {
        if path.exists() {
            return Self::load(path);
        }

        let config = Self::default();
        config.validate()?;
        let parent = path.parent().ok_or_else(|| {
            AppError::InvalidConfiguration(format!(
                "configuration path has no parent: {}",
                path.display()
            ))
        })?;
        fs::create_dir_all(parent)
            .map_err(|error| AppError::io("create controller config directory", parent, error))?;

        let serialized = toml::to_string_pretty(&config).map_err(|error| {
            AppError::InvalidConfiguration(format!("serialize default configuration: {error}"))
        })?;
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut file) => {
                file.write_all(serialized.as_bytes())
                    .and_then(|()| file.sync_all())
                    .map_err(|error| AppError::io("write controller config", path, error))?;
                Ok(config)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Self::load(path),
            Err(error) => Err(AppError::io("create controller config", path, error)),
        }
    }

    pub fn validate(&self) -> AppResult<()> {
        // `prefix_config` validates the scalar fields before compiling chords.
        let _ = self.prefix_config()?;
        Ok(())
    }

    fn validate_scalar_fields(&self) -> AppResult<()> {
        if !matches!(
            self.schema_version,
            LEGACY_CONFIG_SCHEMA_VERSION | CONFIG_SCHEMA_VERSION
        ) {
            return Err(AppError::InvalidConfiguration(format!(
                "unsupported schema_version {}; expected {} or legacy {}",
                self.schema_version, CONFIG_SCHEMA_VERSION, LEGACY_CONFIG_SCHEMA_VERSION
            )));
        }
        if !(MIN_PREFIX_TIMEOUT_MS..=MAX_PREFIX_TIMEOUT_MS).contains(&self.prefix_timeout_ms) {
            return Err(AppError::InvalidConfiguration(format!(
                "prefix_timeout_ms must be between {MIN_PREFIX_TIMEOUT_MS} and {MAX_PREFIX_TIMEOUT_MS}"
            )));
        }
        self.mouse_resize.validate()?;
        Ok(())
    }

    pub fn prefix_config(&self) -> AppResult<PrefixConfig> {
        self.validate_scalar_fields()?;
        let prefix_chord = parse_configured_chord("prefix", &self.prefix)?;
        if !prefix_chord.modifiers.has_ctrl_or_alt() {
            return Err(AppError::InvalidConfiguration(
                "prefix must include ctrl or alt so ordinary typing is never captured".to_owned(),
            ));
        }
        if prefix_chord.key == LogicalKey::Escape || is_reserved_system_chord(prefix_chord) {
            return Err(AppError::InvalidConfiguration(format!(
                "prefix {:?} is reserved by Windows or by Prefix cancellation",
                self.prefix
            )));
        }

        let known_names = shortcut_specs()
            .iter()
            .map(|spec| spec.name)
            .collect::<HashSet<_>>();
        let unknown_names = self
            .shortcuts
            .keys()
            .filter(|name| !known_names.contains(name.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !unknown_names.is_empty() {
            return Err(AppError::InvalidConfiguration(format!(
                "unknown shortcut name(s): {}",
                unknown_names.join(", ")
            )));
        }

        let mut bindings = HashMap::new();
        let mut owners = HashMap::<KeyChord, &'static str>::new();
        for spec in shortcut_specs() {
            let configured = self.shortcuts.get(spec.name);
            let chord = match configured.map(String::as_str) {
                Some(value) if value.trim().eq_ignore_ascii_case(DISABLED_SHORTCUT) => {
                    if !spec.allow_disabled {
                        return Err(AppError::InvalidConfiguration(format!(
                            "shortcut {:?} cannot be disabled; assign another chord instead",
                            spec.name
                        )));
                    }
                    continue;
                }
                Some(value) => parse_configured_chord(spec.name, value)?,
                None => spec.default_chord,
            };

            if chord.key == LogicalKey::Escape {
                return Err(AppError::InvalidConfiguration(format!(
                    "shortcut {:?} cannot use Escape because Escape cancels Prefix mode",
                    spec.name
                )));
            }
            if is_reserved_system_chord(chord) {
                return Err(AppError::InvalidConfiguration(format!(
                    "shortcut {:?} uses reserved system chord {chord}",
                    spec.name
                )));
            }
            if let Some(existing) = owners.insert(chord, spec.name) {
                return Err(AppError::InvalidConfiguration(format!(
                    "shortcuts {existing:?} and {:?} both use {chord}",
                    spec.name
                )));
            }
            bindings.insert(chord, spec.command);
        }

        Ok(PrefixConfig::with_bindings(
            self.prefix_timeout(),
            prefix_chord,
            bindings,
        ))
    }

    pub fn to_pretty_toml(&self) -> AppResult<String> {
        self.validate()?;
        let mut expanded = self.clone();
        expanded.schema_version = CONFIG_SCHEMA_VERSION;
        let mut effective_shortcuts = default_shortcuts();
        effective_shortcuts.extend(self.shortcuts.clone());
        expanded.shortcuts = effective_shortcuts;
        toml::to_string_pretty(&expanded).map_err(|error| {
            AppError::InvalidConfiguration(format!("serialize controller configuration: {error}"))
        })
    }

    #[must_use]
    pub const fn prefix_timeout(&self) -> Duration {
        Duration::from_millis(self.prefix_timeout_ms)
    }

    fn migrate_schema(&mut self) {
        if self.schema_version == LEGACY_CONFIG_SCHEMA_VERSION {
            self.schema_version = CONFIG_SCHEMA_VERSION;
        }
    }
}

fn default_prefix() -> String {
    default_prefix_chord().to_string()
}

fn default_shortcuts() -> BTreeMap<String, String> {
    shortcut_specs()
        .iter()
        .map(|spec| (spec.name.to_owned(), spec.default_chord.to_string()))
        .collect()
}

fn parse_configured_chord(field: &str, value: &str) -> AppResult<KeyChord> {
    value.parse::<KeyChord>().map_err(|error| {
        AppError::InvalidConfiguration(format!("invalid chord for {field:?}: {value:?}: {error}"))
    })
}

pub fn default_app_data_dir() -> AppResult<PathBuf> {
    let local_app_data = env::var_os("LOCALAPPDATA").ok_or_else(|| {
        AppError::InvalidConfiguration("LOCALAPPDATA is not available".to_owned())
    })?;
    Ok(PathBuf::from(local_app_data).join("WinTerminalPP"))
}

pub fn default_config_path() -> AppResult<PathBuf> {
    Ok(default_app_data_dir()?.join("config.toml"))
}

#[cfg(test)]
mod tests {
    use crate::TerminalAction;
    use crate::prefix::ShortcutCommand;

    use super::*;

    #[test]
    fn creates_and_reloads_default_configuration() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("nested").join("config.toml");

        let created = ControllerConfig::load_or_create(&path)
            .expect("default configuration should be created");
        let loaded = ControllerConfig::load(&path).expect("configuration should reload");

        assert_eq!(created, ControllerConfig::default());
        assert_eq!(loaded, created);
    }

    #[test]
    fn rejects_out_of_range_timeout() {
        let config = ControllerConfig {
            prefix_timeout_ms: MIN_PREFIX_TIMEOUT_MS - 1,
            ..ControllerConfig::default()
        };

        assert!(matches!(
            config.validate(),
            Err(AppError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn rejects_out_of_range_mouse_resize_settings() {
        let too_wide = ControllerConfig {
            mouse_resize: MouseResizeConfig {
                divider_hit_slop_px: MAX_DIVIDER_HIT_SLOP_PX + 1,
                ..MouseResizeConfig::default()
            },
            ..ControllerConfig::default()
        };
        assert!(
            too_wide
                .validate()
                .expect_err("oversized divider hit target must fail")
                .to_string()
                .contains("divider_hit_slop_px")
        );

        let too_frequent = ControllerConfig {
            mouse_resize: MouseResizeConfig {
                geometry_poll_interval_ms: MIN_PANE_GEOMETRY_POLL_INTERVAL_MS - 1,
                ..MouseResizeConfig::default()
            },
            ..ControllerConfig::default()
        };
        assert!(
            too_frequent
                .validate()
                .expect_err("overly frequent geometry polling must fail")
                .to_string()
                .contains("geometry_poll_interval_ms")
        );
    }

    #[test]
    fn does_not_overwrite_an_existing_invalid_file() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("config.toml");
        fs::write(&path, "schema_version = 99\n").expect("invalid fixture should be written");

        assert!(ControllerConfig::load_or_create(&path).is_err());
        assert_eq!(
            fs::read_to_string(path).expect("fixture should remain readable"),
            "schema_version = 99\n"
        );
    }

    #[test]
    fn legacy_config_loads_with_effective_default_shortcuts() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("config.toml");
        fs::write(
            &path,
            "schema_version = 1\nprefix_timeout_ms = 1500\nlaunch_terminal_on_start = true\n",
        )
        .expect("legacy fixture should be written");

        let config = ControllerConfig::load(&path).expect("legacy configuration should load");
        let runtime = config
            .prefix_config()
            .expect("default shortcut configuration should compile");

        assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
        assert_eq!(
            fs::read_to_string(path).expect("legacy fixture should remain readable"),
            "schema_version = 1\nprefix_timeout_ms = 1500\nlaunch_terminal_on_start = true\n"
        );
        assert_eq!(config.prefix, "ctrl+b");
        assert_eq!(config.mouse_resize, MouseResizeConfig::default());
        assert_eq!(config.shortcuts.len(), shortcut_specs().len());
        assert_eq!(runtime.prefix_chord, default_prefix_chord());
        assert_eq!(runtime.bindings.len(), shortcut_specs().len());
    }

    #[test]
    fn custom_prefix_and_shortcut_override_compile() {
        let mut config = ControllerConfig {
            prefix: "alt+a".to_owned(),
            ..ControllerConfig::default()
        };
        config
            .shortcuts
            .insert("new_tab".to_owned(), "t".to_owned());
        config
            .shortcuts
            .insert("rename_tab".to_owned(), DISABLED_SHORTCUT.to_owned());

        let runtime = config
            .prefix_config()
            .expect("custom shortcut configuration should compile");

        assert_eq!(runtime.prefix_chord, "alt+a".parse().expect("valid chord"));
        assert_eq!(
            runtime.bindings.get(&"t".parse().expect("valid chord")),
            Some(&ShortcutCommand::Terminal(TerminalAction::NewTab))
        );
        assert!(
            !runtime
                .bindings
                .contains_key(&"comma".parse().expect("valid chord"))
        );
    }

    #[test]
    fn partial_shortcut_table_inherits_unspecified_defaults() {
        let config: ControllerConfig = toml::from_str(
            r#"
schema_version = 1
prefix_timeout_ms = 800
launch_terminal_on_start = true
prefix = "ctrl+a"

[shortcuts]
new_tab = "t"
"#,
        )
        .expect("partial shortcut table should deserialize");

        let runtime = config
            .prefix_config()
            .expect("partial shortcut table should compile");

        assert_eq!(config.shortcuts.len(), 1);
        assert_eq!(runtime.bindings.len(), shortcut_specs().len());
        assert_eq!(
            runtime.bindings.get(&"left".parse().expect("valid chord")),
            Some(&ShortcutCommand::Terminal(TerminalAction::FocusPane {
                direction: crate::Direction::Left,
            }))
        );
    }

    #[test]
    fn duplicate_and_unknown_shortcuts_are_rejected() {
        let mut duplicate = ControllerConfig::default();
        duplicate
            .shortcuts
            .insert("new_tab".to_owned(), "n".to_owned());
        let duplicate_error = duplicate.validate().expect_err("duplicate chord must fail");
        assert!(duplicate_error.to_string().contains("both use n"));

        let mut unknown = ControllerConfig::default();
        unknown
            .shortcuts
            .insert("launch_spaceship".to_owned(), "s".to_owned());
        let unknown_error = unknown.validate().expect_err("unknown shortcut must fail");
        assert!(unknown_error.to_string().contains("launch_spaceship"));
    }

    #[test]
    fn unsafe_prefix_and_disabled_shutdown_are_rejected() {
        let plain_prefix = ControllerConfig {
            prefix: "b".to_owned(),
            ..ControllerConfig::default()
        };
        assert!(
            plain_prefix
                .validate()
                .expect_err("plain prefix must fail")
                .to_string()
                .contains("must include ctrl or alt")
        );

        let mut no_shutdown = ControllerConfig::default();
        no_shutdown
            .shortcuts
            .insert("shutdown".to_owned(), DISABLED_SHORTCUT.to_owned());
        assert!(
            no_shutdown
                .validate()
                .expect_err("shutdown cannot be disabled")
                .to_string()
                .contains("cannot be disabled")
        );

        let mut system_shortcut = ControllerConfig::default();
        system_shortcut
            .shortcuts
            .insert("new_tab".to_owned(), "alt+f4".to_owned());
        assert!(
            system_shortcut
                .validate()
                .expect_err("system shortcut must fail")
                .to_string()
                .contains("reserved system chord")
        );
    }

    #[test]
    fn pretty_toml_materializes_the_complete_shortcut_table() {
        let config: ControllerConfig = toml::from_str(
            "schema_version = 1\nprefix_timeout_ms = 900\nlaunch_terminal_on_start = false\n",
        )
        .expect("legacy shape should deserialize");

        let rendered = config
            .to_pretty_toml()
            .expect("effective configuration should serialize");

        assert!(rendered.contains("schema_version = 2"));
        assert!(rendered.contains("prefix = \"ctrl+b\""));
        assert!(rendered.contains("[shortcuts]"));
        assert!(rendered.contains("new_tab = \"c\""));
        assert!(rendered.contains("shutdown = \"q\""));
        assert!(rendered.contains("[mouse_resize]"));
        assert!(rendered.contains("divider_hit_slop_px = 8"));

        let reparsed: ControllerConfig =
            toml::from_str(&rendered).expect("rendered configuration should parse again");
        assert_eq!(
            reparsed
                .prefix_config()
                .expect("rendered semantics should compile"),
            config
                .prefix_config()
                .expect("source semantics should compile")
        );
    }
}
