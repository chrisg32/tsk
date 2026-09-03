//! Where are we running? herdr is detected from the environment it injects
//! into plugin commands; everything else is "standalone". Nothing outside this
//! module needs to know the difference beyond a few paths and an optional
//! handle for calling back into herdr.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone)]
pub struct HerdrHost {
    pub bin: PathBuf,
    pub plugin_id: String,
    pub root: Option<PathBuf>,
    pub config_dir: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub context: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub enum Host {
    Standalone,
    Herdr(HerdrHost),
}

fn env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

impl Host {
    pub fn detect() -> Host {
        if env::var("HERDR_ENV").as_deref() != Ok("1") {
            return Host::Standalone;
        }
        let Some(bin) = env_path("HERDR_BIN_PATH") else {
            return Host::Standalone;
        };
        let context = env::var("HERDR_PLUGIN_CONTEXT_JSON")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        Host::Herdr(HerdrHost {
            bin,
            plugin_id: env::var("HERDR_PLUGIN_ID").unwrap_or_else(|_| "tsk".into()),
            root: env_path("HERDR_PLUGIN_ROOT"),
            config_dir: env_path("HERDR_PLUGIN_CONFIG_DIR"),
            state_dir: env_path("HERDR_PLUGIN_STATE_DIR"),
            context,
        })
    }

    pub fn is_herdr(&self) -> bool {
        matches!(self, Host::Herdr(_))
    }

    pub fn herdr(&self) -> Option<&HerdrHost> {
        match self {
            Host::Herdr(h) => Some(h),
            Host::Standalone => None,
        }
    }

    pub fn config_dir(&self) -> PathBuf {
        if let Some(dir) = self.herdr().and_then(|h| h.config_dir.clone()) {
            return dir;
        }
        if let Some(dir) = env_path("XDG_CONFIG_HOME") {
            return dir.join("tsk");
        }
        if cfg!(windows) {
            if let Some(dir) = dirs::config_dir() {
                return dir.join("tsk");
            }
        }
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config")
            .join("tsk")
    }

    pub fn state_dir(&self) -> PathBuf {
        if let Some(dir) = self.herdr().and_then(|h| h.state_dir.clone()) {
            return dir;
        }
        if let Some(dir) = env_path("XDG_STATE_HOME") {
            return dir.join("tsk");
        }
        if cfg!(windows) {
            if let Some(dir) = dirs::data_local_dir() {
                return dir.join("tsk");
            }
        }
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local")
            .join("state")
            .join("tsk")
    }

    /// Under herdr, commands start in the plugin checkout, which is never where
    /// the user's task file lives. Treat that directory as "no cwd".
    pub fn is_plugin_root(&self, dir: &Path) -> bool {
        self.herdr()
            .and_then(|h| h.root.as_deref())
            .is_some_and(|root| same_dir(root, dir))
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

impl HerdrHost {
    /// Best guess at the directory the user is working in, from the
    /// invocation context herdr hands to actions.
    pub fn context_cwd(&self) -> Option<PathBuf> {
        let ctx = self.context.as_ref()?;
        for key in ["focused_pane", "pane", "workspace", "worktree"] {
            if let Some(cwd) = ctx
                .get(key)
                .and_then(|v| v.get("cwd"))
                .and_then(|v| v.as_str())
            {
                return Some(PathBuf::from(cwd));
            }
        }
        if let Some(cwd) = ctx.get("cwd").and_then(|v| v.as_str()) {
            return Some(PathBuf::from(cwd));
        }
        find_key(ctx, "cwd").map(PathBuf::from)
    }

    /// Ask herdr to open our pane entrypoint. `placement` is one of the CLI's
    /// overlay/split/tab/zoomed; `None` uses the manifest default.
    pub fn open_pane(
        &self,
        entrypoint: &str,
        placement: Option<&str>,
        cwd: Option<&Path>,
        file: Option<&Path>,
    ) -> Result<()> {
        let mut cmd = Command::new(&self.bin);
        cmd.args([
            "plugin",
            "pane",
            "open",
            "--plugin",
            &self.plugin_id,
            "--entrypoint",
            entrypoint,
            "--focus",
        ]);
        if let Some(p) = placement {
            cmd.args(["--placement", p]);
            if p == "split" {
                cmd.args(["--direction", "right"]);
            }
        }
        if let Some(dir) = cwd {
            cmd.arg("--cwd").arg(dir);
        }
        if let Some(f) = file {
            cmd.arg("--env").arg(format!("TSK_FILE={}", f.display()));
        }
        let status = cmd
            .status()
            .with_context(|| format!("running {}", self.bin.display()))?;
        if !status.success() {
            bail!("herdr plugin pane open exited with {status}");
        }
        Ok(())
    }
}

fn find_key<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(s) = map.get(key).and_then(|v| v.as_str()) {
                return Some(s);
            }
            map.values().find_map(|v| find_key(v, key))
        }
        serde_json::Value::Array(items) => items.iter().find_map(|v| find_key(v, key)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn herdr_with(ctx: &str) -> HerdrHost {
        HerdrHost {
            bin: PathBuf::from("herdr"),
            plugin_id: "tsk".into(),
            root: None,
            config_dir: None,
            state_dir: None,
            context: serde_json::from_str(ctx).ok(),
        }
    }

    #[test]
    fn context_cwd_prefers_the_focused_pane() {
        let h = herdr_with(r#"{"workspace":{"cwd":"/ws"},"focused_pane":{"cwd":"/pane"}}"#);
        assert_eq!(h.context_cwd(), Some(PathBuf::from("/pane")));
        let h = herdr_with(r#"{"workspace":{"cwd":"/ws"}}"#);
        assert_eq!(h.context_cwd(), Some(PathBuf::from("/ws")));
        let h = herdr_with(r#"{"nested":{"deeper":{"cwd":"/deep"}}}"#);
        assert_eq!(h.context_cwd(), Some(PathBuf::from("/deep")));
        let h = herdr_with(r#"{}"#);
        assert_eq!(h.context_cwd(), None);
    }
}
