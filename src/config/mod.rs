//! Loading and validation of `config.toml`. See `docs/config.md`.

pub mod keymap;

use keymap::{KeyBinding, KeySpec};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const KNOWN_THEMES: &[&str] = &["pastel"];

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// Seconds between automatic refreshes; 0 means manual only.
    pub refresh_interval_secs: u64,
    pub default_remote: String,
    pub nerd_fonts: bool,
    pub theme: String,
    /// Raw `[keybindings]` table; parsed into `keybindings` by [`Config::parse`].
    #[serde(rename = "keybindings")]
    raw_keybindings: BTreeMap<String, KeySpec>,
    #[serde(skip)]
    pub keybindings: BTreeMap<String, Vec<KeyBinding>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            refresh_interval_secs: 0,
            default_remote: "origin".to_string(),
            nerd_fonts: true,
            theme: "pastel".to_string(),
            raw_keybindings: BTreeMap::new(),
            keybindings: BTreeMap::new(),
        }
    }
}

impl Config {
    /// Parse and validate config text. Errors are human-readable.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut config: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        if !KNOWN_THEMES.contains(&config.theme.as_str()) {
            return Err(format!(
                "unknown theme {:?} (available: {})",
                config.theme,
                KNOWN_THEMES.join(", ")
            ));
        }
        for (intent, spec) in &config.raw_keybindings {
            let bindings = spec
                .parse()
                .map_err(|e| format!("[keybindings] {intent}: {e}"))?;
            config.keybindings.insert(intent.clone(), bindings);
        }
        Ok(config)
    }

    /// Load from `path`; a missing file yields defaults.
    pub fn load_from(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Load from the default (or `TRELLIS_CONFIG`) location.
    pub fn load() -> Result<Self, String> {
        match config_path() {
            Some(path) => Self::load_from(&path),
            None => Ok(Self::default()),
        }
    }
}

/// Resolve the config file path.
pub fn config_path() -> Option<PathBuf> {
    resolve_path(|k| std::env::var_os(k), cfg!(windows))
}

fn resolve_path(env: impl Fn(&str) -> Option<OsString>, windows: bool) -> Option<PathBuf> {
    let non_empty = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(p) = non_empty("TRELLIS_CONFIG") {
        return Some(p);
    }
    let base = if windows {
        non_empty("APPDATA")?
    } else if let Some(xdg) = non_empty("XDG_CONFIG_HOME") {
        xdg
    } else {
        non_empty("HOME")?.join(".config")
    };
    Some(base.join("trellis").join("config.toml"))
}

static CONFIG: OnceLock<Config> = OnceLock::new();

/// Install the process-wide config (first call wins).
pub fn install(config: Config) {
    let _ = CONFIG.set(config);
}

/// The installed config, or defaults if none was installed.
pub fn get() -> &'static Config {
    CONFIG.get_or_init(Config::default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn empty_is_default() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn parses_full_config() {
        let c = Config::parse(
            r#"
refresh_interval_secs = 30
default_remote = "upstream"
nerd_fonts = false
theme = "pastel"
[keybindings]
back = "q"
move_down = ["j", "down"]
half_page_down = "ctrl+d"
"#,
        )
        .unwrap();
        assert_eq!(c.refresh_interval_secs, 30);
        assert_eq!(c.default_remote, "upstream");
        assert!(!c.nerd_fonts);
        assert_eq!(c.keybindings["move_down"].len(), 2);
        assert_eq!(
            c.keybindings["half_page_down"][0],
            KeyBinding::new(KeyCode::Char('d'), KeyModifiers::CONTROL)
        );
    }

    #[test]
    fn unknown_field_is_error() {
        let e = Config::parse("nerd_font = true").unwrap_err();
        assert!(e.contains("nerd_font"), "{e}");
    }

    #[test]
    fn bad_key_string_is_error() {
        let e = Config::parse("[keybindings]\nback = \"wat\"").unwrap_err();
        assert!(e.contains("back") && e.contains("wat"), "{e}");
    }

    #[test]
    fn unknown_theme_is_error() {
        let e = Config::parse("theme = \"neon\"").unwrap_err();
        assert!(e.contains("neon"), "{e}");
    }

    #[test]
    fn invalid_toml_is_error() {
        assert!(Config::parse("= =").is_err());
    }

    #[test]
    fn missing_file_is_default() {
        let dir = tempfile::tempdir().unwrap();
        let c = Config::load_from(&dir.path().join("nope.toml")).unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn error_names_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        std::fs::write(&p, "bogus = 1").unwrap();
        let e = Config::load_from(&p).unwrap_err();
        assert!(e.contains("config.toml") && e.contains("bogus"), "{e}");
    }

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn path_resolution() {
        assert_eq!(
            resolve_path(env(&[("TRELLIS_CONFIG", "/x.toml")]), false),
            Some(PathBuf::from("/x.toml"))
        );
        assert_eq!(
            resolve_path(env(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/h")]), false),
            Some(PathBuf::from("/xdg/trellis/config.toml"))
        );
        assert_eq!(
            resolve_path(env(&[("HOME", "/h")]), false),
            Some(PathBuf::from("/h/.config/trellis/config.toml"))
        );
        assert_eq!(
            resolve_path(env(&[("APPDATA", "/ad")]), true),
            Some(PathBuf::from("/ad/trellis/config.toml"))
        );
    }
}
