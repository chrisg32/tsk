//! Editing operations. Every method works on line indexes and leaves the
//! document consistent; undo is handled by the caller snapshotting first.

use chrono::NaiveDateTime;

use super::dates;
use super::tags;
use super::{Document, Kind, Line, Status};

/// The bits of user configuration the document operations need.
#[derive(Debug, Clone)]
pub struct Prefs {
    pub open_bullet: String,
    pub done_bullet: String,
    pub cancelled_bullet: String,
    pub date_format: String,
    pub archive_project: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            open_bullet: super::DEFAULT_OPEN_BULLET.into(),
            done_bullet: super::DEFAULT_DONE_BULLET.into(),
            cancelled_bullet: super::DEFAULT_CANCELLED_BULLET.into(),
            date_format: "%y-%m-%d %H:%M".into(),
            archive_project: "Archive".into(),
        }
    }
}

pub const PRIORITY_TAGS: &[&str] = &["critical", "high", "low"];

impl Document {
    /// Set a task's status, maintaining the `@done`/`@cancelled`/`@lasted` tags
    /// the way PlainTasks does. Returns false if the line is not a task.
    pub fn set_status(&mut self, i: usize, new: Status, prefs: &Prefs, now: NaiveDateTime) -> bool {
        let line = &mut self.lines[i];
        let Kind::Task {
            bullet,
            status,
            text,
        } = &mut line.kind
        else {
            return false;
        };
        if *status == new {
            return false;
        }
        let stamp = dates::format(now, &prefs.date_format);
        let mut t = tags::remove_tag(text, "done");
        t = tags::remove_tag(&t, "cancelled");
        t = tags::remove_tag(&t, "lasted");
        match new {
            Status::Open => {
                *bullet = prefs.open_bullet.clone();
            }
            Status::Done | Status::Cancelled => {
                let name = if new == Status::Done {
                    "done"
                } else {
                    "cancelled"
                };
                *bullet = if new == Status::Done {
                    prefs.done_bullet.clone()
                } else {
                    prefs.cancelled_bullet.clone()
                };
                t = tags::add_tag(&t, name, Some(&stamp));
                if let Some(started) = tags::tag_value(&t, "started") {
                    if let Some(from) = dates::parse(&started, &prefs.date_format) {
                        t = tags::add_tag(&t, "lasted", Some(&dates::lasted(from, now)));
                    }
                }
            }
        }
        *status = new;
        *text = t;
        line.touch();
        true
    }

    pub fn toggle_done(&mut self, i: usize, prefs: &Prefs, now: NaiveDateTime) -> bool {
        match self.lines[i].status() {
            Some(Status::Done) => self.set_status(i, Status::Open, prefs, now),
            Some(_) => self.set_status(i, Status::Done, prefs, now),
            None => false,
        }
    }

    pub fn toggle_cancelled(&mut self, i: usize, prefs: &Prefs, now: NaiveDateTime) -> bool {
        match self.lines[i].status() {
            Some(Status::Cancelled) => self.set_status(i, Status::Open, prefs, now),
            Some(_) => self.set_status(i, Status::Cancelled, prefs, now),
            None => false,
        }
    }

    /// Add the tag if absent, remove it if present. Works on tasks, notes and
    /// project suffixes.
    pub fn toggle_tag(&mut self, i: usize, name: &str, value: Option<&str>) -> bool {
        let line = &mut self.lines[i];
        match &mut line.kind {
            Kind::Task { text, .. } | Kind::Note { text } => {
                *text = if tags::has_tag(text, name) {
                    tags::remove_tag(text, name)
                } else {
                    tags::add_tag(text, name, value)
                };
            }
            Kind::Project { suffix, .. } => {
                let s = suffix.trim().to_string();
                let next = if tags::has_tag(&s, name) {
                    tags::remove_tag(&s, name)
                } else {
                    tags::add_tag(&s, name, value)
                };
                *suffix = if next.is_empty() {
                    String::new()
                } else {
                    format!(" {next}")
                };
            }
            Kind::Blank => return false,
        }
        line.touch();
        true
    }

    /// `@started(now)` on, or off again if already started.
    pub fn toggle_started(&mut self, i: usize, prefs: &Prefs, now: NaiveDateTime) -> bool {
        if !self.lines[i].is_task() {
            return false;
        }
        let stamp = dates::format(now, &prefs.date_format);
        self.toggle_tag(i, "started", Some(&stamp))
    }

    /// Set one of critical/high/low (clearing the others), or clear all when
    /// the task already carries `which`.
    pub fn set_priority(&mut self, i: usize, which: &str) -> bool {
        let Kind::Task { text, .. } = &mut self.lines[i].kind else {
            return false;
        };
        let had = tags::has_tag(text, which);
        let mut t = text.clone();
        for p in PRIORITY_TAGS {
            t = tags::remove_tag(&t, p);
        }
        if !had {
            t = tags::add_tag(&t, which, None);
        }
        *text = t;
        self.lines[i].touch();
        true
    }

    pub fn set_line_text(&mut self, i: usize, text: &str) {
        self.lines[i].set_text(text);
    }

    fn set_depth(&mut self, i: usize, depth: usize) {
        let indent = self.indent_for(depth);
        let line = &mut self.lines[i];
        line.indent = indent;
        line.touch();
    }

    pub fn indent_block(&mut self, i: usize) -> bool {
        if self.lines[i].is_blank() {
            return false;
        }
        let end = self.block_end(i);
        for j in i..=end {
            if !self.lines[j].is_blank() {
                let d = self.depth(j);
                self.set_depth(j, d + 1);
            }
        }
        true
    }

    pub fn outdent_block(&mut self, i: usize) -> bool {
        if self.lines[i].is_blank() || self.depth(i) == 0 {
            return false;
        }
        let end = self.block_end(i);
        for j in i..=end {
            if !self.lines[j].is_blank() {
                let d = self.depth(j);
                self.set_depth(j, d.saturating_sub(1));
            }
        }
        true
    }

    /// Start index of the sibling block immediately above `i`, if any.
    fn prev_sibling(&self, i: usize) -> Option<usize> {
        let d = self.depth(i);
        for j in (0..i).rev() {
            if self.lines[j].is_blank() {
                continue;
            }
            let dj = self.depth(j);
            if dj == d {
                return Some(j);
            }
            if dj < d {
                return None;
            }
        }
        None
    }

    fn next_sibling(&self, i: usize) -> Option<usize> {
        let d = self.depth(i);
        for j in self.block_end(i) + 1..self.lines.len() {
            if self.lines[j].is_blank() {
                continue;
            }
            let dj = self.depth(j);
            if dj == d {
                return Some(j);
            }
            if dj < d {
                return None;
            }
        }
        None
    }

    /// Swap the block at `i` with its previous sibling. Returns the block's new start.
    pub fn move_block_up(&mut self, i: usize) -> Option<usize> {
        if self.lines[i].is_blank() {
            return None;
        }
        let prev = self.prev_sibling(i)?;
        let end = self.block_end(i);
        let block: Vec<Line> = self.lines.drain(i..=end).collect();
        for (k, l) in block.into_iter().enumerate() {
            self.lines.insert(prev + k, l);
        }
        Some(prev)
    }

    pub fn move_block_down(&mut self, i: usize) -> Option<usize> {
        if self.lines[i].is_blank() {
            return None;
        }
        let next = self.next_sibling(i)?;
        let next_end = self.block_end(next);
        let end = self.block_end(i);
        let block: Vec<Line> = self.lines.drain(i..=end).collect();
        let len = block.len();
        // After draining, the next block starts `len` lines earlier.
        let insert_at = next_end + 1 - len;
        for (k, l) in block.into_iter().enumerate() {
            self.lines.insert(insert_at + k, l);
        }
        Some(insert_at)
    }

    pub fn insert_at(&mut self, idx: usize, depth: usize, kind: Kind) -> usize {
        let line = self.make_line(depth, kind);
        let idx = idx.min(self.lines.len());
        self.lines.insert(idx, line);
        idx
    }

    /// Depth a new sibling of `i` should get, and where it goes.
    fn sibling_slot(&self, i: usize) -> (usize, usize) {
        if self.lines.is_empty() {
            return (0, 0);
        }
        let line = &self.lines[i];
        if line.is_project() {
            return (i + 1, self.depth(i) + 1);
        }
        if line.is_blank() {
            // Continue whatever came before the gap.
            let prev = (0..i).rev().find(|&j| !self.lines[j].is_blank());
            return match prev {
                Some(p) if self.lines[p].is_project() => (i + 1, self.depth(p) + 1),
                Some(p) => (i + 1, self.depth(p)),
                None => (i + 1, 0),
            };
        }
        (self.block_end(i) + 1, self.depth(i))
    }

    pub fn insert_task_below(&mut self, i: usize, prefs: &Prefs) -> usize {
        let (idx, depth) = self.sibling_slot(i);
        let kind = Kind::Task {
            bullet: prefs.open_bullet.clone(),
            status: Status::Open,
            text: String::new(),
        };
        self.insert_at(idx, depth, kind)
    }

    pub fn insert_task_above(&mut self, i: usize, prefs: &Prefs) -> usize {
        let depth = if self.lines.is_empty() {
            0
        } else {
            self.depth(i)
        };
        let kind = Kind::Task {
            bullet: prefs.open_bullet.clone(),
            status: Status::Open,
            text: String::new(),
        };
        self.insert_at(i, depth, kind)
    }

    pub fn insert_note_below(&mut self, i: usize) -> usize {
        let (idx, depth) = if self.lines.is_empty() {
            (0, 0)
        } else if self.lines[i].is_task() {
            // A note directly under the task, above any existing children.
            (i + 1, self.depth(i) + 1)
        } else {
            self.sibling_slot(i)
        };
        self.insert_at(
            idx,
            depth,
            Kind::Note {
                text: String::new(),
            },
        )
    }

    pub fn insert_project_below(&mut self, i: usize) -> usize {
        let (idx, depth) = if self.lines.is_empty() {
            (0, 0)
        } else if self.lines[i].is_project() {
            (self.block_end(i) + 1, self.depth(i))
        } else {
            match self.enclosing_project(i) {
                Some(p) => (self.block_end(p) + 1, self.depth(p)),
                None => (self.block_end(i) + 1, 0),
            }
        };
        self.insert_at(
            idx,
            depth,
            Kind::Project {
                name: String::new(),
                suffix: String::new(),
            },
        )
    }

    /// Remove the block at `i`. Returns a sensible cursor index afterwards.
    pub fn delete_block(&mut self, i: usize) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        let end = if self.lines[i].is_blank() {
            i
        } else {
            self.block_end(i)
        };
        self.lines.drain(i..=end);
        i.min(self.lines.len().saturating_sub(1))
    }

    pub fn delete_line(&mut self, i: usize) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        self.lines.remove(i);
        i.min(self.lines.len().saturating_sub(1))
    }

    /// Move every done/cancelled task (with its notes and subtasks) to the top
    /// of the archive project, tagging each with `@project(A / B)` so it can be
    /// traced back. Returns how many tasks were archived.
    pub fn archive(&mut self, prefs: &Prefs) -> usize {
        let archive_idx = self.find_project(&prefs.archive_project);
        let limit = archive_idx.unwrap_or(self.lines.len());
        let mut blocks: Vec<(usize, usize)> = Vec::new();
        let mut i = 0;
        while i < limit {
            let line = &self.lines[i];
            if matches!(line.status(), Some(Status::Done) | Some(Status::Cancelled)) {
                let end = self.block_end(i);
                blocks.push((i, end));
                i = end + 1;
            } else {
                i += 1;
            }
        }
        if blocks.is_empty() {
            return 0;
        }
        // Tag before moving so project paths are still intact.
        let mut moved: Vec<Vec<Line>> = Vec::new();
        for &(start, end) in &blocks {
            let path = self.project_path(start).join(" / ");
            if !path.is_empty() {
                if let Kind::Task { text, .. } = &mut self.lines[start].kind {
                    if !tags::has_tag(text, "project") {
                        *text = tags::add_tag(text, "project", Some(&path));
                        self.lines[start].touch();
                    }
                }
            }
            let base = self.depth(start);
            let mut block = Vec::with_capacity(end - start + 1);
            for j in start..=end {
                let mut l = self.lines[j].clone();
                if !l.is_blank() {
                    let rel = self.depth(j).saturating_sub(base);
                    l.indent = self.indent_for(1 + rel);
                    l.touch();
                }
                block.push(l);
            }
            moved.push(block);
        }
        for &(start, end) in blocks.iter().rev() {
            self.lines.drain(start..=end);
        }
        let archive_idx = match self.find_project(&prefs.archive_project) {
            Some(a) => a,
            None => {
                let n = self.lines.len();
                if n > 0 && !self.lines[n - 1].is_blank() {
                    self.insert_at(n, 0, Kind::Blank);
                }
                let n = self.lines.len();
                self.insert_at(
                    n,
                    0,
                    Kind::Note {
                        text: super::ARCHIVE_SEPARATOR.to_string(),
                    },
                );
                let n = self.lines.len();
                self.insert_at(
                    n,
                    0,
                    Kind::Project {
                        name: prefs.archive_project.clone(),
                        suffix: String::new(),
                    },
                )
            }
        };
        let mut at = archive_idx + 1;
        let count = moved.len();
        for block in moved {
            for l in block {
                self.lines.insert(at, l);
                at += 1;
            }
        }
        if !self.trailing_newline {
            self.trailing_newline = true;
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn doc(src: &str) -> Document {
        Document::parse(src, "\t")
    }

    #[test]
    fn toggling_done_stamps_and_unstamps() {
        let p = Prefs::default();
        let mut d = doc("☐ milk @today\n");
        assert!(d.toggle_done(0, &p, at("2026-09-02 14:30")));
        assert_eq!(d.serialize(), "✔ milk @today @done(26-09-02 14:30)\n");
        assert!(d.toggle_done(0, &p, at("2026-09-02 14:31")));
        assert_eq!(d.serialize(), "☐ milk @today\n");
    }

    #[test]
    fn done_after_started_records_lasted() {
        let p = Prefs::default();
        let mut d = doc("☐ work @started(26-09-02 10:00)\n");
        d.toggle_done(0, &p, at("2026-09-02 12:05"));
        assert_eq!(
            d.serialize(),
            "✔ work @started(26-09-02 10:00) @done(26-09-02 12:05) @lasted(2h05m)\n"
        );
        d.toggle_done(0, &p, at("2026-09-02 12:06"));
        assert_eq!(d.serialize(), "☐ work @started(26-09-02 10:00)\n");
    }

    #[test]
    fn cancel_and_done_replace_each_other() {
        let p = Prefs::default();
        let mut d = doc("- thing\n");
        d.toggle_cancelled(0, &p, at("2026-01-01 00:00"));
        assert_eq!(d.serialize(), "✘ thing @cancelled(26-01-01 00:00)\n");
        d.toggle_done(0, &p, at("2026-01-01 00:01"));
        assert_eq!(d.serialize(), "✔ thing @done(26-01-01 00:01)\n");
    }

    #[test]
    fn priorities_are_exclusive_and_toggle_off() {
        let mut d = doc("☐ a @low\n");
        d.set_priority(0, "high");
        assert_eq!(d.serialize(), "☐ a @high\n");
        d.set_priority(0, "high");
        assert_eq!(d.serialize(), "☐ a\n");
    }

    #[test]
    fn indent_and_outdent_move_whole_blocks() {
        let mut d = doc("A:\n\t☐ one\n\t\tnote\n\t☐ two\n");
        d.indent_block(1);
        assert_eq!(d.serialize(), "A:\n\t\t☐ one\n\t\t\tnote\n\t☐ two\n");
        d.outdent_block(1);
        assert_eq!(d.serialize(), "A:\n\t☐ one\n\t\tnote\n\t☐ two\n");
        assert!(!d.outdent_block(0));
    }

    #[test]
    fn move_blocks_between_siblings() {
        let mut d = doc("A:\n\t☐ one\n\t\tnote\n\t☐ two\n\t☐ three\n");
        assert_eq!(d.move_block_down(1), Some(2));
        assert_eq!(d.serialize(), "A:\n\t☐ two\n\t☐ one\n\t\tnote\n\t☐ three\n");
        assert_eq!(d.move_block_up(2), Some(1));
        assert_eq!(d.serialize(), "A:\n\t☐ one\n\t\tnote\n\t☐ two\n\t☐ three\n");
        assert_eq!(d.move_block_up(1), None);
        assert_eq!(d.move_block_down(4), None);
    }

    #[test]
    fn inserting_tasks_picks_the_right_slot() {
        let p = Prefs::default();
        let mut d = doc("A:\n\t☐ one\n\t\tnote\n\t☐ two\n");
        let i = d.insert_task_below(1, &p);
        assert_eq!(i, 3);
        d.set_line_text(i, "one-b");
        assert_eq!(d.serialize(), "A:\n\t☐ one\n\t\tnote\n\t☐ one-b\n\t☐ two\n");
        let i = d.insert_task_below(0, &p);
        assert_eq!(i, 1);
        d.set_line_text(i, "first");
        assert_eq!(
            d.serialize(),
            "A:\n\t☐ first\n\t☐ one\n\t\tnote\n\t☐ one-b\n\t☐ two\n"
        );
        let i = d.insert_task_above(2, &p);
        d.set_line_text(i, "zero");
        assert_eq!(d.lines[2].text(), "zero");
        assert_eq!(d.depth(2), 1);
    }

    #[test]
    fn inserting_projects_and_notes() {
        let p = Prefs::default();
        let mut d = doc("A:\n\t☐ one\nB:\n\t☐ two\n");
        let i = d.insert_project_below(1);
        d.set_line_text(i, "A2");
        assert_eq!(d.serialize(), "A:\n\t☐ one\nA2:\nB:\n\t☐ two\n");
        let i = d.insert_note_below(1);
        d.set_line_text(i, "a note");
        assert_eq!(d.serialize(), "A:\n\t☐ one\n\t\ta note\nA2:\nB:\n\t☐ two\n");
        let mut empty = doc("");
        let i = empty.insert_task_below(0, &p);
        empty.set_line_text(i, "hello");
        assert_eq!(empty.serialize(), "☐ hello\n");
    }

    #[test]
    fn delete_block_removes_descendants() {
        let mut d = doc("A:\n\t☐ one\n\t\tnote\n\t☐ two\n");
        let c = d.delete_block(1);
        assert_eq!(c, 1);
        assert_eq!(d.serialize(), "A:\n\t☐ two\n");
    }

    #[test]
    fn archive_moves_done_tasks_with_project_tags() {
        let p = Prefs::default();
        let mut d = doc("Inbox:\n\t☐ open\n\t✔ done @done(26-09-02 10:00)\n\t\tits note\n\tWork:\n\t\t✘ nope @cancelled(26-09-02 10:00)\n");
        assert_eq!(d.archive(&p), 2);
        assert_eq!(
            d.serialize(),
            "Inbox:\n\t☐ open\n\tWork:\n\n＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿\nArchive:\n\t✔ done @done(26-09-02 10:00) @project(Inbox)\n\t\tits note\n\t✘ nope @cancelled(26-09-02 10:00) @project(Inbox / Work)\n"
        );
        // Archiving again with nothing new is a no-op; existing archive stays.
        assert_eq!(d.archive(&p), 0);
        // New done tasks go to the top of the archive.
        d.insert_task_below(1, &p);
        d.set_line_text(2, "later");
        d.toggle_done(2, &p, at("2026-09-03 09:00"));
        assert_eq!(d.archive(&p), 1);
        let a = d.find_project("Archive").unwrap();
        assert_eq!(
            d.lines[a + 1].text(),
            "later @done(26-09-03 09:00) @project(Inbox)"
        );
    }

    #[test]
    fn tags_on_projects_live_in_the_suffix() {
        let mut d = doc("Work:\n");
        d.toggle_tag(0, "focus", None);
        assert_eq!(d.serialize(), "Work: @focus\n");
        d.toggle_tag(0, "focus", None);
        assert_eq!(d.serialize(), "Work:\n");
    }
}
