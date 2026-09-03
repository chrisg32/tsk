//! Application state and key handling. Rendering lives in `ui`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use crate::config::Config;
use crate::doc::ops::Prefs;
use crate::doc::{dates, tags, Document, Status};
use crate::editor::LineEditor;
use crate::host::Host;
use crate::store;

const UNDO_LIMIT: usize = 200;
const MESSAGE_TTL: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Search,
    Tag,
}

#[derive(Debug, Clone)]
pub enum Mode {
    Normal,
    /// Editing the text of `line` in place. `created` lines are removed again
    /// if the edit is cancelled or committed empty.
    Insert {
        editor: LineEditor,
        line: usize,
        created: bool,
    },
    Prompt {
        kind: PromptKind,
        editor: LineEditor,
    },
    Help,
}

struct Snapshot {
    doc: Document,
    cursor: usize,
}

pub struct Message {
    pub text: String,
    pub error: bool,
    at: Instant,
}

pub struct App {
    pub doc: Document,
    pub path: PathBuf,
    pub cfg: Config,
    pub prefs: Prefs,
    pub host: Host,
    pub cursor: usize,
    pub scroll: usize,
    pub folded: HashSet<u64>,
    pub mode: Mode,
    pub dirty: bool,
    pub changed_on_disk: bool,
    pub search: Option<String>,
    pub show_done: bool,
    pub mouse: bool,
    pub quit: bool,
    pub message: Option<Message>,
    saved_mtime: Option<SystemTime>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    pending_g: bool,
    last_check: Instant,
}

impl App {
    pub fn new(path: PathBuf, cfg: Config, host: Host) -> anyhow::Result<App> {
        let loaded = store::load(&path)?;
        let doc = Document::parse(&loaded.text, &cfg.indent_unit());
        let prefs = cfg.prefs();
        let mouse = cfg.mouse_enabled(&host);
        let show_done = cfg.show_done;
        let mut app = App {
            doc,
            path,
            cfg,
            prefs,
            host,
            cursor: 0,
            scroll: 0,
            folded: HashSet::new(),
            mode: Mode::Normal,
            dirty: false,
            changed_on_disk: false,
            search: None,
            show_done,
            mouse,
            quit: false,
            message: None,
            saved_mtime: loaded.mtime,
            undo: Vec::new(),
            redo: Vec::new(),
            pending_g: false,
            last_check: Instant::now(),
        };
        if !loaded.existed {
            app.info(format!("new file: {}", app.path.display()));
        }
        app.ensure_cursor_visible();
        Ok(app)
    }

    // ----- messages ---------------------------------------------------------

    pub fn info(&mut self, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            error: false,
            at: Instant::now(),
        });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            error: true,
            at: Instant::now(),
        });
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }

    // ----- visibility -------------------------------------------------------

    /// Line indexes currently shown, in order, honouring folds and `show_done`.
    pub fn visible_rows(&self) -> Vec<usize> {
        let mut rows = Vec::with_capacity(self.doc.len());
        let mut i = 0;
        while i < self.doc.len() {
            let line = &self.doc.lines[i];
            if !self.show_done && matches!(line.status(), Some(Status::Done | Status::Cancelled)) {
                i = self.doc.block_end(i) + 1;
                continue;
            }
            rows.push(i);
            if line.is_project() && self.folded.contains(&line.id) {
                i = self.doc.block_end(i) + 1;
                continue;
            }
            i += 1;
        }
        rows
    }

    pub fn is_folded(&self, i: usize) -> bool {
        self.folded.contains(&self.doc.lines[i].id)
    }

    fn ensure_cursor_visible(&mut self) {
        let rows = self.visible_rows();
        if rows.is_empty() {
            self.cursor = 0;
            return;
        }
        if rows.contains(&self.cursor) {
            return;
        }
        // Snap to the nearest visible line at or above the cursor.
        self.cursor = rows
            .iter()
            .rev()
            .find(|&&r| r < self.cursor)
            .or_else(|| rows.first())
            .copied()
            .unwrap_or(0);
    }

    /// Keep the cursor's row inside a viewport of `height` rows.
    pub fn adjust_scroll(&mut self, height: usize, rows: &[usize]) {
        let pos = rows.iter().position(|&r| r == self.cursor).unwrap_or(0);
        if height == 0 {
            return;
        }
        if pos < self.scroll {
            self.scroll = pos;
        } else if pos >= self.scroll + height {
            self.scroll = pos + 1 - height;
        }
        let max_scroll = rows.len().saturating_sub(height);
        self.scroll = self.scroll.min(max_scroll);
    }

    fn move_by(&mut self, delta: isize) {
        let rows = self.visible_rows();
        if rows.is_empty() {
            return;
        }
        let pos = rows.iter().position(|&r| r == self.cursor).unwrap_or(0) as isize;
        let next = (pos + delta).clamp(0, rows.len() as isize - 1) as usize;
        self.cursor = rows[next];
    }

    fn move_to_row(&mut self, which: fn(&[usize]) -> Option<&usize>) {
        if let Some(&r) = which(&self.visible_rows()) {
            self.cursor = r;
        }
    }

    fn unfold_ancestors(&mut self, i: usize) {
        let mut cur = self.doc.enclosing_project(i);
        while let Some(p) = cur {
            self.folded.remove(&self.doc.lines[p].id);
            cur = self.doc.enclosing_project(p);
        }
    }

    /// Move to `i`, unfolding whatever hides it.
    pub fn jump_to(&mut self, i: usize) {
        if i >= self.doc.len() {
            return;
        }
        self.unfold_ancestors(i);
        if matches!(
            self.doc.lines[i].status(),
            Some(Status::Done | Status::Cancelled)
        ) {
            self.show_done = true;
        }
        self.cursor = i;
    }

    fn next_project(&mut self, forward: bool) {
        let rows = self.visible_rows();
        let pos = rows.iter().position(|&r| r == self.cursor).unwrap_or(0);
        let target = if forward {
            rows.iter()
                .skip(pos + 1)
                .find(|&&r| self.doc.lines[r].is_project())
        } else {
            rows.iter()
                .take(pos)
                .rev()
                .find(|&&r| self.doc.lines[r].is_project())
        };
        if let Some(&r) = target {
            self.cursor = r;
        }
    }

    // ----- undo / mutation --------------------------------------------------

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            doc: self.doc.clone(),
            cursor: self.cursor,
        }
    }

    /// Run a change with undo support. `f` returns whether anything changed.
    fn mutate<F: FnOnce(&mut App) -> bool>(&mut self, f: F) -> bool {
        let snap = self.snapshot();
        if !f(self) {
            return false;
        }
        self.undo.push(snap);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.after_change();
        true
    }

    fn after_change(&mut self) {
        self.dirty = true;
        self.ensure_cursor_visible();
        if self.cfg.autosave {
            self.save();
        }
    }

    fn restore(&mut self, snap: Snapshot) {
        self.doc = snap.doc;
        self.cursor = snap.cursor.min(self.doc.len().saturating_sub(1));
        self.after_change();
    }

    fn undo(&mut self) {
        match self.undo.pop() {
            Some(snap) => {
                let current = self.snapshot();
                self.redo.push(current);
                self.restore(snap);
            }
            None => self.info("nothing to undo"),
        }
    }

    fn redo(&mut self) {
        match self.redo.pop() {
            Some(snap) => {
                let current = self.snapshot();
                self.undo.push(current);
                self.restore(snap);
            }
            None => self.info("nothing to redo"),
        }
    }

    // ----- file -------------------------------------------------------------

    pub fn save(&mut self) -> bool {
        match store::save(&self.path, &self.doc.serialize()) {
            Ok(mtime) => {
                self.saved_mtime = Some(mtime);
                self.dirty = false;
                self.changed_on_disk = false;
                true
            }
            Err(e) => {
                self.error(format!("save failed: {e:#}"));
                false
            }
        }
    }

    pub fn reload(&mut self) {
        match store::load(&self.path) {
            Ok(loaded) => {
                // Folds are keyed by line id; remember them by (depth, name) instead.
                let folded_keys: Vec<(usize, String)> = self
                    .folded
                    .iter()
                    .filter_map(|id| self.doc.index_of_id(*id))
                    .map(|i| (self.doc.depth(i), self.doc.lines[i].text().to_string()))
                    .collect();
                self.doc = Document::parse(&loaded.text, &self.cfg.indent_unit());
                self.folded = self
                    .doc
                    .lines
                    .iter()
                    .enumerate()
                    .filter(|(i, l)| {
                        l.is_project()
                            && folded_keys.contains(&(self.doc.depth(*i), l.text().to_string()))
                    })
                    .map(|(_, l)| l.id)
                    .collect();
                self.saved_mtime = loaded.mtime;
                self.dirty = false;
                self.changed_on_disk = false;
                self.undo.clear();
                self.redo.clear();
                self.cursor = self.cursor.min(self.doc.len().saturating_sub(1));
                self.ensure_cursor_visible();
            }
            Err(e) => self.error(format!("reload failed: {e:#}")),
        }
    }

    /// Called every few hundred milliseconds: notice edits made by other programs.
    pub fn tick(&mut self) {
        if let Some(m) = &self.message {
            if m.at.elapsed() > MESSAGE_TTL {
                self.message = None;
            }
        }
        if self.last_check.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.last_check = Instant::now();
        if matches!(self.mode, Mode::Insert { .. }) {
            return;
        }
        let on_disk = store::mtime(&self.path);
        if on_disk.is_none() || on_disk == self.saved_mtime {
            return;
        }
        if self.dirty {
            if !self.changed_on_disk {
                self.changed_on_disk = true;
                self.error("file changed on disk — R reloads (discarding local changes)");
            }
        } else {
            self.reload();
            self.info("reloaded: file changed on disk");
        }
    }

    // ----- editing ----------------------------------------------------------

    fn start_edit(&mut self, line: usize, at_end: bool, created: bool) {
        if line >= self.doc.len() {
            return;
        }
        let editor = LineEditor::new(self.doc.lines[line].text(), at_end);
        self.cursor = line;
        self.mode = Mode::Insert {
            editor,
            line,
            created,
        };
    }

    fn commit_edit(&mut self) {
        let Mode::Insert {
            editor,
            line,
            created,
        } = std::mem::replace(&mut self.mode, Mode::Normal)
        else {
            return;
        };
        let mut text = editor.buf.trim().to_string();
        if self.doc.lines[line].is_project() {
            text = text.trim_end_matches(':').trim_end().to_string();
        }
        if created {
            if text.is_empty() {
                if let Some(snap) = self.undo.pop() {
                    self.doc = snap.doc;
                    self.cursor = snap.cursor.min(self.doc.len().saturating_sub(1));
                    self.ensure_cursor_visible();
                }
            } else {
                self.doc.set_line_text(line, &text);
                self.after_change();
            }
        } else if text.is_empty() {
            self.mutate(|app| {
                app.cursor = app.doc.delete_line(line);
                true
            });
        } else if text != self.doc.lines[line].text() {
            self.mutate(|app| {
                app.doc.set_line_text(line, &text);
                true
            });
        }
    }

    fn cancel_edit(&mut self) {
        let Mode::Insert { created, .. } = std::mem::replace(&mut self.mode, Mode::Normal) else {
            return;
        };
        if created {
            if let Some(snap) = self.undo.pop() {
                self.doc = snap.doc;
                self.cursor = snap.cursor.min(self.doc.len().saturating_sub(1));
                self.ensure_cursor_visible();
            }
        }
    }

    /// Insert a line via `f` (which returns its index) and start editing it.
    fn create_and_edit<F: FnOnce(&mut Document, usize, &Prefs) -> usize>(&mut self, f: F) {
        let cursor = self.cursor;
        let prefs = self.prefs.clone();
        let mut new_index = 0;
        self.mutate(|app| {
            new_index = f(&mut app.doc, cursor, &prefs);
            true
        });
        if let Some(p) = self.doc.enclosing_project(new_index) {
            self.folded.remove(&self.doc.lines[p].id);
        }
        self.unfold_ancestors(new_index);
        self.start_edit(new_index, true, true);
    }

    fn toggle_fold(&mut self) {
        if self.doc.is_empty() {
            return;
        }
        let target = if self.doc.lines[self.cursor].is_project() {
            Some(self.cursor)
        } else {
            self.doc.enclosing_project(self.cursor)
        };
        let Some(p) = target else { return };
        let id = self.doc.lines[p].id;
        if !self.folded.remove(&id) {
            self.folded.insert(id);
        }
        self.cursor = p;
    }

    fn toggle_fold_all(&mut self) {
        if self.folded.is_empty() {
            self.folded = self
                .doc
                .lines
                .iter()
                .filter(|l| l.is_project())
                .map(|l| l.id)
                .collect();
        } else {
            self.folded.clear();
        }
        self.ensure_cursor_visible();
    }

    fn search_matches(&self, query: &str) -> Vec<usize> {
        let q = query.to_lowercase();
        self.doc
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| !q.is_empty() && l.render().to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect()
    }

    fn search_step(&mut self, forward: bool) {
        let Some(q) = self.search.clone() else {
            self.info("no search — press / to search");
            return;
        };
        let matches = self.search_matches(&q);
        if matches.is_empty() {
            self.error(format!("no match for '{q}'"));
            return;
        }
        let target = if forward {
            matches
                .iter()
                .find(|&&m| m > self.cursor)
                .or_else(|| matches.first())
        } else {
            matches
                .iter()
                .rev()
                .find(|&&m| m < self.cursor)
                .or_else(|| matches.last())
        };
        if let Some(&t) = target {
            self.jump_to(t);
        }
    }

    fn open_url(&mut self) {
        if self.doc.is_empty() {
            return;
        }
        let text = self.doc.lines[self.cursor].render();
        let Some((s, e)) = tags::find_url(&text) else {
            self.info("no link on this line");
            return;
        };
        let url = text[s..e].to_string();
        let result = if cfg!(target_os = "macos") {
            Command::new("open")
                .arg(&url)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        } else if cfg!(windows) {
            Command::new("cmd")
                .args(["/C", "start", "", &url])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        } else {
            Command::new("xdg-open")
                .arg(&url)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        };
        match result {
            Ok(_) => self.info(format!("opened {url}")),
            Err(e) => self.error(format!("could not open link: {e}")),
        }
    }

    fn request_quit(&mut self, force: bool) {
        if self.dirty && !force {
            if self.cfg.autosave {
                if !self.save() {
                    return;
                }
            } else {
                self.error("unsaved changes — w to save, Q to quit anyway");
                return;
            }
        }
        self.quit = true;
    }

    fn archive(&mut self) {
        let prefs = self.prefs.clone();
        let mut count = 0;
        let changed = self.mutate(|app| {
            count = app.doc.archive(&prefs);
            count > 0
        });
        if changed {
            self.info(format!(
                "archived {count} task{}",
                if count == 1 { "" } else { "s" }
            ));
        } else {
            self.info("nothing to archive");
        }
    }

    // ----- input ------------------------------------------------------------

    pub fn handle_key(&mut self, key: KeyEvent) {
        match &self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Insert { .. } => self.handle_insert_key(key),
            Mode::Prompt { .. } => self.handle_prompt_key(key),
            Mode::Help => {
                self.mode = Mode::Normal;
            }
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        let one_line: String = text.lines().collect::<Vec<_>>().join(" ");
        match &mut self.mode {
            Mode::Insert { editor, .. } | Mode::Prompt { editor, .. } => {
                editor.insert_str(&one_line)
            }
            _ => {}
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.pending_g {
            self.pending_g = false;
            if key.code == KeyCode::Char('g') {
                self.move_to_row(<[usize]>::first);
            }
            return;
        }
        let empty = self.doc.is_empty();
        let prefs = self.prefs.clone();
        let now = dates::now();
        let cursor = self.cursor;
        match (key.code, ctrl) {
            (KeyCode::Char('q'), false) => self.request_quit(false),
            (KeyCode::Char('Q'), false) => self.request_quit(true),
            (KeyCode::Char('c'), true) => self.request_quit(true),
            (KeyCode::Char('?'), false) => self.mode = Mode::Help,
            (KeyCode::Esc, _) => {
                self.search = None;
                self.message = None;
            }

            (KeyCode::Char('j'), false) | (KeyCode::Down, _) => self.move_by(1),
            (KeyCode::Char('k'), false) | (KeyCode::Up, _) => self.move_by(-1),
            (KeyCode::Char('d'), true) | (KeyCode::PageDown, _) => self.move_by(10),
            (KeyCode::Char('u'), true) | (KeyCode::PageUp, _) => self.move_by(-10),
            (KeyCode::Char('g'), false) => self.pending_g = true,
            (KeyCode::Home, _) => self.move_to_row(<[usize]>::first),
            (KeyCode::Char('G'), false) | (KeyCode::End, _) => self.move_to_row(<[usize]>::last),
            (KeyCode::Char(']'), false) => self.next_project(true),
            (KeyCode::Char('['), false) => self.next_project(false),

            (KeyCode::Enter, _) | (KeyCode::Char('e'), false) | (KeyCode::Char('a'), false)
                if !empty =>
            {
                self.start_edit(cursor, true, false)
            }
            (KeyCode::Char('i'), false) if !empty => self.start_edit(cursor, false, false),
            (KeyCode::Char('o'), false) => {
                self.create_and_edit(|d, c, p| d.insert_task_below(c, p))
            }
            (KeyCode::Char('O'), false) => {
                self.create_and_edit(|d, c, p| d.insert_task_above(c, p))
            }
            (KeyCode::Char('m'), false) => self.create_and_edit(|d, c, _| d.insert_note_below(c)),
            (KeyCode::Char('p'), false) => {
                self.create_and_edit(|d, c, _| d.insert_project_below(c))
            }

            (KeyCode::Char(' '), false) | (KeyCode::Char('x'), false) if !empty => {
                self.mutate(|app| app.doc.toggle_done(cursor, &prefs, now));
            }
            (KeyCode::Char('c'), false) if !empty => {
                self.mutate(|app| app.doc.toggle_cancelled(cursor, &prefs, now));
            }
            (KeyCode::Char('s'), false) if !empty => {
                self.mutate(|app| app.doc.toggle_started(cursor, &prefs, now));
            }
            (KeyCode::Char('t'), false) if !empty => {
                self.mutate(|app| app.doc.toggle_tag(cursor, "today", None));
            }
            (KeyCode::Char('1'), false) if !empty => {
                self.mutate(|app| app.doc.set_priority(cursor, "critical"));
            }
            (KeyCode::Char('2'), false) if !empty => {
                self.mutate(|app| app.doc.set_priority(cursor, "high"));
            }
            (KeyCode::Char('3'), false) if !empty => {
                self.mutate(|app| app.doc.set_priority(cursor, "low"));
            }
            (KeyCode::Char('@'), false) if !empty => {
                self.mode = Mode::Prompt {
                    kind: PromptKind::Tag,
                    editor: LineEditor::default(),
                };
            }
            (KeyCode::Char('/'), false) => {
                let initial = self.search.clone().unwrap_or_default();
                self.mode = Mode::Prompt {
                    kind: PromptKind::Search,
                    editor: LineEditor::new(&initial, true),
                };
            }
            (KeyCode::Char('n'), false) => self.search_step(true),
            (KeyCode::Char('N'), false) => self.search_step(false),

            (KeyCode::Char('A'), false) => self.archive(),
            (KeyCode::Char('D'), false) if !empty => {
                self.mutate(|app| {
                    app.cursor = app.doc.delete_block(cursor);
                    true
                });
            }
            (KeyCode::Char('d'), false) if !empty => {
                self.mutate(|app| {
                    app.cursor = app.doc.delete_line(cursor);
                    true
                });
            }
            (KeyCode::Char('u'), false) => self.undo(),
            (KeyCode::Char('U'), false) | (KeyCode::Char('r'), true) => self.redo(),

            (KeyCode::Tab, _) | (KeyCode::Char('>'), false) if !empty => {
                self.mutate(|app| app.doc.indent_block(cursor));
            }
            (KeyCode::BackTab, _) | (KeyCode::Char('<'), false) if !empty => {
                self.mutate(|app| app.doc.outdent_block(cursor));
            }
            (KeyCode::Char('J'), false) if !empty => {
                self.mutate(|app| match app.doc.move_block_down(cursor) {
                    Some(i) => {
                        app.cursor = i;
                        true
                    }
                    None => false,
                });
            }
            (KeyCode::Char('K'), false) if !empty => {
                self.mutate(|app| match app.doc.move_block_up(cursor) {
                    Some(i) => {
                        app.cursor = i;
                        true
                    }
                    None => false,
                });
            }

            (KeyCode::Char('z'), false) => self.toggle_fold(),
            (KeyCode::Char('Z'), false) => self.toggle_fold_all(),
            (KeyCode::Char('h'), false) => {
                self.show_done = !self.show_done;
                self.ensure_cursor_visible();
                self.info(if self.show_done {
                    "showing done tasks"
                } else {
                    "hiding done tasks"
                });
            }
            (KeyCode::Char('w'), false) => {
                if self.save() {
                    self.info(format!("saved {}", self.file_name()));
                }
            }
            (KeyCode::Char('R'), false) => {
                self.reload();
                self.info(format!("reloaded {}", self.file_name()));
            }
            (KeyCode::Char('L'), false) => self.open_url(),
            _ => {}
        }
    }

    fn handle_insert_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Enter => self.commit_edit(),
            KeyCode::Esc => self.cancel_edit(),
            _ => {
                let Mode::Insert { editor, .. } = &mut self.mode else {
                    return;
                };
                edit_key(editor, key.code, ctrl, alt);
            }
        }
    }

    fn handle_prompt_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let Mode::Prompt { kind, editor } = &mut self.mode else {
            return;
        };
        let kind = *kind;
        match key.code {
            KeyCode::Esc => {
                if kind == PromptKind::Search {
                    self.search = None;
                }
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                let text = editor.buf.trim().to_string();
                self.mode = Mode::Normal;
                match kind {
                    PromptKind::Search => {
                        if text.is_empty() {
                            self.search = None;
                        } else {
                            self.search = Some(text);
                            let matches =
                                self.search_matches(self.search.as_deref().unwrap_or_default());
                            match matches
                                .iter()
                                .find(|&&m| m >= self.cursor)
                                .or_else(|| matches.first())
                            {
                                Some(&m) => self.jump_to(m),
                                None => self.error("no match"),
                            }
                        }
                    }
                    PromptKind::Tag => {
                        let cursor = self.cursor;
                        let raw = text.trim_start_matches('@');
                        if raw.is_empty() {
                            return;
                        }
                        let (name, value) = match raw.split_once('(') {
                            Some((n, v)) => (
                                n.trim().to_string(),
                                Some(v.trim_end_matches(')').to_string()),
                            ),
                            None => (raw.to_string(), None),
                        };
                        self.mutate(|app| app.doc.toggle_tag(cursor, &name, value.as_deref()));
                    }
                }
            }
            _ => {
                edit_key(editor, key.code, ctrl, alt);
                if kind == PromptKind::Search {
                    let q = editor.buf.clone();
                    self.search = if q.is_empty() { None } else { Some(q) };
                }
            }
        }
    }

    /// `rows` and `top` describe the list currently on screen so clicks map to lines.
    pub fn handle_mouse(&mut self, ev: MouseEvent, top: u16, height: u16, rows: &[usize]) {
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        match ev.kind {
            MouseEventKind::ScrollDown => self.move_by(3),
            MouseEventKind::ScrollUp => self.move_by(-3),
            MouseEventKind::Down(MouseButton::Left) => {
                if ev.row < top || ev.row >= top + height {
                    return;
                }
                let idx = self.scroll + (ev.row - top) as usize;
                if let Some(&line) = rows.get(idx) {
                    if self.cursor == line && self.doc.lines[line].is_project() {
                        self.toggle_fold();
                    } else {
                        self.cursor = line;
                    }
                }
            }
            _ => {}
        }
    }

    pub fn path_display(&self) -> String {
        shorten_home(&self.path)
    }

    pub fn now_for_display(&self) -> chrono::NaiveDateTime {
        dates::now()
    }
}

fn edit_key(editor: &mut LineEditor, code: KeyCode, ctrl: bool, alt: bool) {
    match (code, ctrl, alt) {
        (KeyCode::Char(c), false, false) => editor.insert(c),
        (KeyCode::Backspace, false, false) => editor.backspace(),
        (KeyCode::Backspace, true, _)
        | (KeyCode::Backspace, _, true)
        | (KeyCode::Char('w'), true, _) => editor.delete_word_back(),
        (KeyCode::Delete, _, _) => editor.delete(),
        (KeyCode::Left, true, _) | (KeyCode::Left, _, true) | (KeyCode::Char('b'), _, true) => {
            editor.word_left()
        }
        (KeyCode::Right, true, _) | (KeyCode::Right, _, true) | (KeyCode::Char('f'), _, true) => {
            editor.word_right()
        }
        (KeyCode::Left, _, _) | (KeyCode::Char('b'), true, _) => editor.left(),
        (KeyCode::Right, _, _) | (KeyCode::Char('f'), true, _) => editor.right(),
        (KeyCode::Home, _, _) | (KeyCode::Char('a'), true, _) => editor.home(),
        (KeyCode::End, _, _) | (KeyCode::Char('e'), true, _) => editor.end(),
        (KeyCode::Char('k'), true, _) => editor.kill_to_end(),
        (KeyCode::Char('u'), true, _) => editor.kill_to_start(),
        _ => {}
    }
}

pub fn shorten_home(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn app_with(src: &str) -> App {
        let path = crate::store::temp_task_file(src);
        let cfg = Config {
            autosave: false,
            ..Config::default()
        };
        App::new(path, cfg, Host::Standalone).unwrap()
    }

    #[test]
    fn folding_hides_children_and_snaps_cursor() {
        let mut app = app_with("A:\n\t☐ one\n\t☐ two\nB:\n");
        app.cursor = 2;
        app.handle_key(key('z'));
        assert_eq!(app.cursor, 0);
        assert_eq!(app.visible_rows(), vec![0, 3]);
        app.handle_key(key('j'));
        assert_eq!(app.cursor, 3);
        app.handle_key(key('Z'));
        assert_eq!(app.visible_rows(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn create_edit_commit_and_undo() {
        let mut app = app_with("A:\n\t☐ one\n");
        app.cursor = 1;
        app.handle_key(key('o'));
        assert!(matches!(app.mode, Mode::Insert { created: true, .. }));
        for c in "two".chars() {
            app.handle_key(key(c));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.doc.serialize(), "A:\n\t☐ one\n\t☐ two\n");
        assert!(app.dirty);
        app.handle_key(key('u'));
        assert_eq!(app.doc.serialize(), "A:\n\t☐ one\n");
        app.handle_key(key('U'));
        assert_eq!(app.doc.serialize(), "A:\n\t☐ one\n\t☐ two\n");
    }

    #[test]
    fn cancelled_creation_leaves_no_trace() {
        let mut app = app_with("A:\n\t☐ one\n");
        app.handle_key(key('o'));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.doc.serialize(), "A:\n\t☐ one\n");
        assert!(app.undo.is_empty());
        app.handle_key(key('o'));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.doc.serialize(), "A:\n\t☐ one\n");
    }

    #[test]
    fn hiding_done_tasks_and_searching() {
        let mut app = app_with("A:\n\t✔ done @done(26-01-01 00:00)\n\t☐ open\n");
        app.handle_key(key('h'));
        assert_eq!(app.visible_rows(), vec![0, 2]);
        app.handle_key(key('/'));
        for c in "done".chars() {
            app.handle_key(key(c));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.cursor, 1);
        assert!(app.show_done);
    }

    #[test]
    fn quit_refuses_when_dirty_without_autosave() {
        let mut app = app_with("☐ a\n");
        app.handle_key(key(' '));
        app.handle_key(key('q'));
        assert!(!app.quit);
        app.handle_key(key('w'));
        assert!(!app.dirty);
        app.handle_key(key('q'));
        assert!(app.quit);
        assert!(std::fs::read_to_string(&app.path)
            .unwrap()
            .starts_with("✔ a @done("));
    }

    #[test]
    fn external_changes_are_picked_up() {
        let mut app = app_with("☐ a\n");
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&app.path, "☐ b\n").unwrap();
        // Force the mtime to differ even on coarse filesystems.
        let later = SystemTime::now() + Duration::from_secs(5);
        let _ = std::fs::File::open(&app.path).and_then(|f| f.set_modified(later));
        app.last_check = Instant::now() - Duration::from_secs(1);
        app.tick();
        assert_eq!(app.doc.lines[0].text(), "b");
    }
}
