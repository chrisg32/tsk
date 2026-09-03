//! Reading and writing the task file, plus locating one when none was named.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};

pub const EXTENSIONS: &[&str] = &["todo", "taskpaper", "tasks", "todolist"];

pub struct Loaded {
    pub text: String,
    pub mtime: Option<SystemTime>,
    pub existed: bool,
}

pub fn load(path: &Path) -> Result<Loaded> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Loaded {
            text,
            mtime: mtime(path),
            existed: true,
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Loaded {
            text: String::new(),
            mtime: None,
            existed: false,
        }),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Write via a temporary file in the same directory and rename over the
/// target, so a crash mid-write never leaves a truncated task list.
pub fn save(path: &Path, text: &str) -> Result<SystemTime> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "tsk".into());
    let tmp = dir.join(format!(".{name}.tsk-{}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        #[cfg(unix)]
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&tmp, meta.permissions());
        }
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.with_context(|| format!("writing {}", path.display()))?;
    Ok(mtime(path).unwrap_or_else(SystemTime::now))
}

pub fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if s == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(s)
}

/// The task file in `dir`, if there is exactly one obvious candidate:
/// `todo.todo` wins, otherwise the alphabetically first file with a known
/// extension.
pub fn find_task_file(dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    let mut candidates: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
        })
        .collect();
    if let Some(p) = candidates
        .iter()
        .find(|p| p.file_name().is_some_and(|n| n == "todo.todo"))
    {
        return Some(p.clone());
    }
    candidates.sort();
    candidates.into_iter().next()
}

/// A fresh task file for tests. Unique per process *and* per call, so
/// parallel tests never share a path.
#[cfg(test)]
pub fn temp_task_file(contents: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("tsk-test-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.todo");
    fs::write(&path, contents).unwrap();
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_roundtrip_and_missing_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("tsk-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("todo.todo");
        let missing = load(&path).unwrap();
        assert!(!missing.existed);
        assert_eq!(missing.text, "");
        save(&path, "A:\n\t☐ b\n").unwrap();
        let got = load(&path).unwrap();
        assert!(got.existed);
        assert_eq!(got.text, "A:\n\t☐ b\n");
        assert!(got.mtime.is_some());
        assert!(!fs::read_dir(path.parent().unwrap()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
        assert_eq!(find_task_file(path.parent().unwrap()), Some(path.clone()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tilde_expansion() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_tilde("~/x.todo"), home.join("x.todo"));
        assert_eq!(expand_tilde("/abs/x.todo"), PathBuf::from("/abs/x.todo"));
        assert_eq!(expand_tilde("rel.todo"), PathBuf::from("rel.todo"));
    }
}
