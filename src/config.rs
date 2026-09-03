//! User configuration (`config.toml`) — read from the herdr plugin config dir
//! when running under herdr, otherwise from the XDG config dir.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::doc::ops::Prefs;
use crate::doc::{DEFAULT_CANCELLED_BULLET, DEFAULT_DONE_BULLET, DEFAULT_OPEN_BULLET};
use crate::host::Host;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// File to open when none is given on the command line. `~` is expanded.
    pub file: Option<String>,
    /// Bullets written for new/toggled tasks. Any recognised bullet is read.
    pub open_bullet: String,
    pub done_bullet: String,
    pub cancelled_bullet: String,
    /// strftime format for `@done(...)`, `@started(...)`, and friends.
    pub date_format: String,
    /// Indentation for files that don't have any yet: "tab", or a number of spaces.
    pub indent: String,
    /// Write the file after every change. When off, `w` saves and `q` refuses
    /// to quit with unsaved changes (`Q` discards them).
    pub autosave: bool,
    /// Capture mouse clicks and scrolling. Defaults to on standalone and off
    /// under herdr, whose own mouse handling is the point.
    pub mouse: Option<bool>,
    /// Columns per indentation level on screen.
    pub display_indent: u16,
    /// Name of the project completed tasks are archived into.
    pub archive_project: String,
    /// Show done and cancelled tasks. Toggled at runtime with `h`.
    pub show_done: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            file: None,
            open_bullet: DEFAULT_OPEN_BULLET.into(),
            done_bullet: DEFAULT_DONE_BULLET.into(),
            cancelled_bullet: DEFAULT_CANCELLED_BULLET.into(),
            date_format: "%y-%m-%d %H:%M".into(),
            indent: "tab".into(),
            autosave: true,
            mouse: None,
            display_indent: 2,
            archive_project: "Archive".into(),
            show_done: true,
        }
    }
}

pub const EXAMPLE: &str = r#"# tsk configuration. Every key is optional; these are the defaults.

# File to open when none is given on the command line.
# file = "~/todo.todo"

# Bullets written for new and toggled tasks. Any PlainTasks bullet is read.
open_bullet = "☐"
done_bullet = "✔"
cancelled_bullet = "✘"

# strftime format used inside @done(...), @started(...), @cancelled(...).
date_format = "%y-%m-%d %H:%M"

# Indentation for files that have none yet: "tab" or a number of spaces.
indent = "tab"

# Save after every change. When false, `w` saves and `q` refuses to quit dirty.
autosave = true

# Capture the mouse. Unset means: on standalone, off under herdr.
# mouse = true

# Screen columns per indentation level.
display_indent = 2

# Project that completed tasks are archived into with `A`.
archive_project = "Archive"

# Show done and cancelled tasks on startup (toggle with `h`).
show_done = true
"#;

impl Config {
    pub fn path(host: &Host) -> PathBuf {
        host.config_dir().join("config.toml")
    }

    pub fn load(path: &Path) -> Result<Config> {
        if !path.exists() {
            return Ok(Config::default());
        }
        let text =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn write_example(path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, EXAMPLE).with_context(|| format!("writing {}", path.display()))
    }

    pub fn prefs(&self) -> Prefs {
        Prefs {
            open_bullet: self.open_bullet.clone(),
            done_bullet: self.done_bullet.clone(),
            cancelled_bullet: self.cancelled_bullet.clone(),
            date_format: self.date_format.clone(),
            archive_project: self.archive_project.clone(),
        }
    }

    /// The indent unit used for files with no indentation of their own.
    pub fn indent_unit(&self) -> String {
        match self.indent.trim() {
            "tab" | "\t" | "" => "\t".to_string(),
            s => match s.parse::<usize>() {
                Ok(n) if n > 0 => " ".repeat(n.min(16)),
                _ => s.to_string(),
            },
        }
    }

    pub fn mouse_enabled(&self, host: &Host) -> bool {
        self.mouse.unwrap_or(!host.is_herdr())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_parses_to_defaults() {
        let parsed: Config = toml::from_str(EXAMPLE).unwrap();
        let d = Config::default();
        assert_eq!(parsed.open_bullet, d.open_bullet);
        assert_eq!(parsed.date_format, d.date_format);
        assert_eq!(parsed.autosave, d.autosave);
        assert_eq!(parsed.display_indent, d.display_indent);
        assert_eq!(parsed.mouse, None);
    }

    #[test]
    fn indent_units() {
        let mut c = Config::default();
        assert_eq!(c.indent_unit(), "\t");
        c.indent = "2".into();
        assert_eq!(c.indent_unit(), "  ");
        c.indent = "4".into();
        assert_eq!(c.indent_unit(), "    ");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(toml::from_str::<Config>("bogus = 1").is_err());
    }
}
