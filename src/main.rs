//! tsk — plain-text tasks in your terminal, standalone or inside herdr.

mod app;
mod config;
mod doc;
mod editor;
mod host;
mod store;
mod ui;

use std::env;
use std::fs;
use std::io::stdout;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::Rect;

use app::App;
use config::Config;
use host::Host;

#[derive(Parser)]
#[command(
    name = "tsk",
    version,
    about = "Plain-text tasks in your terminal (PlainTasks/TaskPaper format)"
)]
struct Cli {
    /// Task file to open (.todo, .taskpaper, .tasks). Created on first save.
    file: Option<PathBuf>,
    /// Don't capture the mouse this run.
    #[arg(long)]
    no_mouse: bool,
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show which task file would open, and why.
    Which,
    /// Show or create the configuration file.
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
    /// Entry points used by the herdr plugin manifest.
    Herdr {
        #[command(subcommand)]
        action: HerdrCmd,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print the config file path.
    Path,
    /// Write an example config file (refuses to overwrite).
    Init,
}

#[derive(Subcommand)]
enum HerdrCmd {
    /// Ask herdr to open the tsk pane (used by the `open` actions).
    Open {
        /// overlay, split, tab or zoomed; default is the manifest's placement.
        #[arg(long)]
        placement: Option<String>,
        /// Task file to open in the pane.
        file: Option<PathBuf>,
    },
}

fn main() {
    if let Err(e) = run() {
        eprintln!("tsk: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let host = Host::detect();
    let config_path = Config::path(&host);
    let cfg = Config::load(&config_path)?;

    match cli.command {
        Some(Cmd::Which) => {
            let (path, why) = resolve_file(cli.file.as_deref(), &host, &cfg);
            println!("{}\t({why})", path.display());
            Ok(())
        }
        Some(Cmd::Config {
            action: ConfigCmd::Path,
        }) => {
            println!("{}", config_path.display());
            Ok(())
        }
        Some(Cmd::Config {
            action: ConfigCmd::Init,
        }) => {
            if config_path.exists() {
                bail!("{} already exists", config_path.display());
            }
            Config::write_example(&config_path)?;
            println!("wrote {}", config_path.display());
            Ok(())
        }
        Some(Cmd::Herdr {
            action: HerdrCmd::Open { placement, file },
        }) => {
            let Some(h) = host.herdr() else {
                bail!("not running under herdr (HERDR_ENV is unset)")
            };
            if let Some(p) = &placement {
                if !["overlay", "split", "tab", "zoomed"].contains(&p.as_str()) {
                    bail!("placement must be overlay, split, tab or zoomed");
                }
            }
            let cwd = h.context_cwd();
            h.open_pane(
                "main",
                placement.as_deref(),
                cwd.as_deref(),
                file.as_deref(),
            )
        }
        None => {
            let (path, _) = resolve_file(cli.file.as_deref(), &host, &cfg);
            remember_last(&host, &path);
            let mut app = App::new(path, cfg, host)?;
            if cli.no_mouse {
                app.mouse = false;
            }
            run_tui(&mut app)
        }
    }
}

/// Which file to open, in priority order: the argument, `$TSK_FILE`, the
/// configured file, a task file in the current directory, the last file
/// opened, and finally `~/todo.todo`.
fn resolve_file(arg: Option<&Path>, host: &Host, cfg: &Config) -> (PathBuf, &'static str) {
    if let Some(p) = arg {
        return (p.to_path_buf(), "argument");
    }
    if let Some(p) = env::var_os("TSK_FILE").filter(|v| !v.is_empty()) {
        return (PathBuf::from(p), "TSK_FILE");
    }
    if let Some(f) = &cfg.file {
        return (store::expand_tilde(f), "config");
    }
    if let Ok(cwd) = env::current_dir() {
        if !host.is_plugin_root(&cwd) {
            if let Some(p) = store::find_task_file(&cwd) {
                return (p, "found in current directory");
            }
        }
    }
    if let Some(p) = last_file(host).filter(|p| p.exists()) {
        return (p, "last opened");
    }
    (
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("todo.todo"),
        "default",
    )
}

fn last_file_path(host: &Host) -> PathBuf {
    host.state_dir().join("last_file")
}

fn last_file(host: &Host) -> Option<PathBuf> {
    fs::read_to_string(last_file_path(host))
        .ok()
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| !p.as_os_str().is_empty())
}

fn remember_last(host: &Host, path: &Path) {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map(|d| d.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let target = last_file_path(host);
    if let Some(dir) = target.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(target, abs.display().to_string());
}

fn run_tui(app: &mut App) -> Result<()> {
    let mut terminal = ratatui::init();
    let mut out = stdout();
    if app.mouse {
        let _ = execute!(out, EnableMouseCapture);
    }
    let _ = execute!(out, EnableBracketedPaste);
    let result = event_loop(&mut terminal, app);
    let _ = execute!(out, DisableBracketedPaste);
    if app.mouse {
        let _ = execute!(out, DisableMouseCapture);
    }
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    let mut list = Rect::default();
    let mut rows: Vec<usize> = Vec::new();
    loop {
        terminal
            .draw(|f| {
                let painted = ui::draw(f, app);
                list = painted.list;
                rows = painted.rows;
            })
            .context("drawing")?;
        if app.quit {
            return Ok(());
        }
        if event::poll(Duration::from_millis(250)).context("polling input")? {
            match event::read().context("reading input")? {
                Event::Key(k) if k.kind != KeyEventKind::Release => app.handle_key(k),
                Event::Mouse(m) => app.handle_mouse(m, list.y, list.height, &rows),
                Event::Paste(s) => app.handle_paste(&s),
                _ => {}
            }
        }
        app.tick();
    }
}
