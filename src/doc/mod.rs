//! The task document: a PlainTasks/TaskPaper file parsed line by line.
//!
//! Every line keeps its original text until it is edited, so a file that is
//! opened and saved without changes is byte-for-byte identical. Only lines
//! that were actually modified are re-rendered from their parsed form.

pub mod dates;
pub mod ops;
pub mod tags;

use std::fmt;

pub const DEFAULT_OPEN_BULLET: &str = "☐";
pub const DEFAULT_DONE_BULLET: &str = "✔";
pub const DEFAULT_CANCELLED_BULLET: &str = "✘";

/// Bullets recognised as tasks. Anything else is a note (or a project when it
/// ends with a colon).
const OPEN_BULLETS: &[&str] = &[
    "☐", "❍", "❑", "■", "□", "▪", "▫", "–", "—", "≡", "→", "›", "-", "+", "*", "[ ]",
];
const DONE_BULLETS: &[&str] = &["✔", "✓", "[x]", "[X]"];
const CANCELLED_BULLETS: &[&str] = &["✘", "✗", "[-]"];

/// The horizontal rule PlainTasks writes above the archive.
pub const ARCHIVE_SEPARATOR: &str =
    "＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Open,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// `Name:` optionally followed by tags (`Name: @tag`).
    Project {
        name: String,
        suffix: String,
    },
    /// `☐ text @tag` — `text` includes the tags.
    Task {
        bullet: String,
        status: Status,
        text: String,
    },
    /// Any other non-blank line.
    Note {
        text: String,
    },
    Blank,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Stable identity across edits, used for fold state and cursor restore.
    pub id: u64,
    /// Leading whitespace exactly as found (or generated).
    pub indent: String,
    pub kind: Kind,
    /// The original source line, kept until the line is modified.
    raw: Option<String>,
}

impl Line {
    pub fn is_project(&self) -> bool {
        matches!(self.kind, Kind::Project { .. })
    }

    pub fn is_task(&self) -> bool {
        matches!(self.kind, Kind::Task { .. })
    }

    pub fn is_blank(&self) -> bool {
        matches!(self.kind, Kind::Blank)
    }

    pub fn status(&self) -> Option<Status> {
        match &self.kind {
            Kind::Task { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// The part of the line a user edits: project name, task text, note text.
    pub fn text(&self) -> &str {
        match &self.kind {
            Kind::Project { name, .. } => name,
            Kind::Task { text, .. } | Kind::Note { text } => text,
            Kind::Blank => "",
        }
    }

    pub fn set_text(&mut self, new: &str) {
        match &mut self.kind {
            Kind::Project { name, .. } => *name = new.to_string(),
            Kind::Task { text, .. } | Kind::Note { text } => *text = new.to_string(),
            Kind::Blank => {
                if !new.is_empty() {
                    self.kind = Kind::Note {
                        text: new.to_string(),
                    };
                }
            }
        }
        self.raw = None;
    }

    pub fn touch(&mut self) {
        self.raw = None;
    }

    /// Render the line as it will be written to disk.
    pub fn render(&self) -> String {
        if let Some(raw) = &self.raw {
            return raw.clone();
        }
        match &self.kind {
            Kind::Project { name, suffix } => format!("{}{}:{}", self.indent, name, suffix),
            Kind::Task { bullet, text, .. } => {
                if text.is_empty() {
                    format!("{}{}", self.indent, bullet)
                } else {
                    format!("{}{} {}", self.indent, bullet, text)
                }
            }
            Kind::Note { text } => format!("{}{}", self.indent, text),
            Kind::Blank => String::new(),
        }
    }

    /// Whether this note-style line is a decorative separator.
    pub fn is_separator(&self) -> bool {
        match &self.kind {
            Kind::Note { text } => {
                let t = text.trim();
                t.starts_with("---") || t.starts_with("＿＿＿") || t.starts_with("___")
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Document {
    pub lines: Vec<Line>,
    /// One level of indentation, detected from the file (`"\t"`, `"  "`, ...).
    pub unit: String,
    pub trailing_newline: bool,
    pub crlf: bool,
    next_id: u64,
}

fn split_indent(line: &str) -> (&str, &str) {
    let n = line.len() - line.trim_start_matches([' ', '\t']).len();
    line.split_at(n)
}

fn match_bullet(rest: &str) -> Option<(&str, Status)> {
    let candidates = OPEN_BULLETS
        .iter()
        .map(|b| (*b, Status::Open))
        .chain(DONE_BULLETS.iter().map(|b| (*b, Status::Done)))
        .chain(CANCELLED_BULLETS.iter().map(|b| (*b, Status::Cancelled)));
    for (bullet, status) in candidates {
        if let Some(after) = rest.strip_prefix(bullet) {
            if after.is_empty() || after.starts_with([' ', '\t']) {
                return Some((bullet, status));
            }
        }
    }
    None
}

/// `Name:` or `Name: @tag @tag`. The colon must not follow whitespace.
fn match_project(rest: &str) -> Option<(String, String)> {
    let trimmed = rest.trim_end();
    let mut body = trimmed;
    while let Some(ws) = body.rfind(char::is_whitespace) {
        let token = &body[ws..].trim_start();
        if token.len() > 1 && token.starts_with('@') {
            body = body[..ws].trim_end();
        } else {
            break;
        }
    }
    let name = body.strip_suffix(':')?;
    if name.is_empty() || name.ends_with(char::is_whitespace) {
        return None;
    }
    Some((name.to_string(), trimmed[body.len()..].to_string()))
}

pub fn parse_kind(rest: &str) -> Kind {
    if rest.trim().is_empty() {
        return Kind::Blank;
    }
    if let Some((bullet, status)) = match_bullet(rest) {
        let text = rest[bullet.len()..].trim_start().trim_end().to_string();
        return Kind::Task {
            bullet: bullet.to_string(),
            status,
            text,
        };
    }
    if let Some((name, suffix)) = match_project(rest) {
        return Kind::Project { name, suffix };
    }
    Kind::Note {
        text: rest.trim_end().to_string(),
    }
}

fn detect_unit(lines: &[Line], fallback: &str) -> String {
    if lines.iter().any(|l| l.indent.contains('\t')) {
        return "\t".to_string();
    }
    let min_spaces = lines
        .iter()
        .filter(|l| !l.is_blank())
        .map(|l| l.indent.len())
        .filter(|n| *n > 0)
        .min();
    match min_spaces {
        Some(n) => " ".repeat(n),
        None => fallback.to_string(),
    }
}

impl Document {
    pub fn parse(src: &str, fallback_unit: &str) -> Document {
        let crlf = src.contains("\r\n");
        let body = if crlf {
            src.replace("\r\n", "\n")
        } else {
            src.to_string()
        };
        let trailing_newline = body.ends_with('\n') || body.is_empty();
        let text = body.strip_suffix('\n').unwrap_or(&body);
        let mut lines = Vec::new();
        let mut next_id = 1;
        if !text.is_empty() || !body.is_empty() && !trailing_newline {
            for raw in text.split('\n') {
                let (indent, rest) = split_indent(raw);
                lines.push(Line {
                    id: next_id,
                    indent: indent.to_string(),
                    kind: parse_kind(rest),
                    raw: Some(raw.to_string()),
                });
                next_id += 1;
            }
        }
        let unit = detect_unit(&lines, fallback_unit);
        Document {
            lines,
            unit,
            trailing_newline,
            crlf,
            next_id,
        }
    }

    pub fn serialize(&self) -> String {
        let nl = if self.crlf { "\r\n" } else { "\n" };
        let mut out = String::new();
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                out.push_str(nl);
            }
            out.push_str(&line.render());
        }
        if self.trailing_newline && !self.lines.is_empty() {
            out.push_str(nl);
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Width in columns of one space-based indent unit (tabs count as one level).
    fn unit_width(&self) -> usize {
        if self.unit == "\t" {
            0
        } else {
            self.unit.len().max(1)
        }
    }

    pub fn depth_of_indent(&self, indent: &str) -> usize {
        let tabs = indent.matches('\t').count();
        let spaces = indent.len() - tabs;
        let uw = self.unit_width();
        let space_levels = if spaces == 0 {
            0
        } else if uw == 0 {
            spaces.div_ceil(4)
        } else {
            spaces.div_ceil(uw)
        };
        tabs + space_levels
    }

    pub fn depth(&self, i: usize) -> usize {
        self.depth_of_indent(&self.lines[i].indent)
    }

    pub fn indent_for(&self, depth: usize) -> String {
        self.unit.repeat(depth)
    }

    pub fn make_line(&mut self, depth: usize, kind: Kind) -> Line {
        let id = self.next_id;
        self.next_id += 1;
        Line {
            id,
            indent: self.indent_for(depth),
            kind,
            raw: None,
        }
    }

    /// Index of the last line belonging to `i`'s block: `i` plus every
    /// following non-blank line indented deeper than it. Blank lines inside
    /// the block are included; trailing blanks are not.
    pub fn block_end(&self, i: usize) -> usize {
        let d = self.depth(i);
        let mut end = i;
        for j in i + 1..self.lines.len() {
            if self.lines[j].is_blank() {
                continue;
            }
            if self.depth(j) > d {
                end = j;
            } else {
                break;
            }
        }
        end
    }

    /// Nearest project line above `i` that encloses it (shallower than `i`).
    pub fn enclosing_project(&self, i: usize) -> Option<usize> {
        let d = self.depth(i);
        let mut limit = d;
        for j in (0..i).rev() {
            let line = &self.lines[j];
            if line.is_blank() {
                continue;
            }
            let dj = self.depth(j);
            if dj < limit {
                if line.is_project() {
                    return Some(j);
                }
                limit = dj;
            }
        }
        None
    }

    /// Names of the projects enclosing `i`, outermost first.
    pub fn project_path(&self, i: usize) -> Vec<String> {
        let mut path = Vec::new();
        let mut cur = self.enclosing_project(i);
        while let Some(p) = cur {
            if let Kind::Project { name, .. } = &self.lines[p].kind {
                path.push(name.clone());
            }
            cur = self.enclosing_project(p);
        }
        path.reverse();
        path
    }

    /// Line index of the top-level project with this name, if present.
    pub fn find_project(&self, name: &str) -> Option<usize> {
        self.lines
            .iter()
            .enumerate()
            .find_map(|(i, l)| match &l.kind {
                Kind::Project { name: n, .. } if n == name && self.depth(i) == 0 => Some(i),
                _ => None,
            })
    }

    pub fn index_of_id(&self, id: u64) -> Option<usize> {
        self.lines.iter().position(|l| l.id == id)
    }

    pub fn count_tasks(&self) -> (usize, usize, usize) {
        let mut open = 0;
        let mut done = 0;
        let mut cancelled = 0;
        for l in &self.lines {
            match l.status() {
                Some(Status::Open) => open += 1,
                Some(Status::Done) => done += 1,
                Some(Status::Cancelled) => cancelled += 1,
                None => {}
            }
        }
        (open, done, cancelled)
    }

    /// Open tasks directly or indirectly inside the block starting at `i`.
    pub fn open_tasks_in_block(&self, i: usize) -> usize {
        (i + 1..=self.block_end(i))
            .filter(|&j| self.lines[j].status() == Some(Status::Open))
            .count()
    }
}

impl fmt::Display for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.serialize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Inbox:\n\t☐ Write the parser @today\n\t✔ Pick a name @done(26-09-02 10:00)\n\t\tnested note\n\n- dash task\nSomeday:\n\t✘ nope @cancelled(26-09-02 10:00)\n";

    /// The tutorial that ships with PlainTasks touches every construct the
    /// format has, with irregular indentation on purpose.
    #[test]
    fn plaintasks_tutorial_roundtrips_and_classifies() {
        let src = include_str!("../../tests/fixtures/plaintasks-tutorial.todo");
        let doc = Document::parse(src, "\t");
        assert_eq!(doc.serialize(), src);
        let projects: Vec<&str> = doc
            .lines
            .iter()
            .filter_map(|l| match &l.kind {
                Kind::Project { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert!(projects.contains(&"How to Use PlainTasks"), "{projects:?}");
        assert!(projects.contains(&"Projects"));
        assert!(projects.contains(&"Tagging"));
        assert!(projects.contains(&"Archive"));
        let (open, done, _) = doc.count_tasks();
        assert!(open > 20, "open={open}");
        assert_eq!(done, 1);
        let archived = doc
            .lines
            .iter()
            .find(|l| l.status() == Some(Status::Done))
            .unwrap();
        assert_eq!(
            tags::tag_value(archived.text(), "done").as_deref(),
            Some("12-09-07 07:30")
        );
        // Every line under a project is deeper than that project.
        let tasks = doc.find_project("Tasks").unwrap();
        assert!(doc.block_end(tasks) > tasks + 10);
        assert!(!doc
            .lines
            .iter()
            .any(|l| matches!(&l.kind, Kind::Note { text } if text.starts_with('☐'))));
    }

    #[test]
    fn roundtrips_untouched_files() {
        for src in [
            SAMPLE,
            "",
            "no newline at end",
            "  weird   spacing  \n\n\n",
            "a:\r\n\t☐ b\r\n",
        ] {
            let doc = Document::parse(src, "\t");
            assert_eq!(doc.serialize(), src, "roundtrip of {src:?}");
        }
    }

    #[test]
    fn classifies_lines() {
        let doc = Document::parse(SAMPLE, "\t");
        assert!(matches!(&doc.lines[0].kind, Kind::Project { name, .. } if name == "Inbox"));
        assert!(
            matches!(&doc.lines[1].kind, Kind::Task { status: Status::Open, text, .. } if text == "Write the parser @today")
        );
        assert!(matches!(
            &doc.lines[2].kind,
            Kind::Task {
                status: Status::Done,
                ..
            }
        ));
        assert!(matches!(&doc.lines[3].kind, Kind::Note { text } if text == "nested note"));
        assert!(doc.lines[4].is_blank());
        assert!(matches!(&doc.lines[5].kind, Kind::Task { bullet, .. } if bullet == "-"));
        assert!(matches!(
            &doc.lines[7].kind,
            Kind::Task {
                status: Status::Cancelled,
                ..
            }
        ));
        assert_eq!(doc.unit, "\t");
        assert_eq!(doc.depth(3), 2);
    }

    #[test]
    fn projects_may_carry_tags_and_notes_may_contain_colons() {
        assert!(
            matches!(parse_kind("Work: @focus"), Kind::Project { name, suffix } if name == "Work" && suffix == " @focus")
        );
        assert!(matches!(parse_kind("see: the thing"), Kind::Note { .. }));
        assert!(matches!(parse_kind("--- ✄ -----------"), Kind::Note { .. }));
        assert!(matches!(parse_kind("trailing space :"), Kind::Note { .. }));
        assert!(matches!(
            parse_kind("[ ] bracket task"),
            Kind::Task {
                status: Status::Open,
                ..
            }
        ));
        assert!(matches!(
            parse_kind("[x] bracket done"),
            Kind::Task {
                status: Status::Done,
                ..
            }
        ));
    }

    #[test]
    fn detects_space_indentation() {
        let doc = Document::parse("A:\n  ☐ one\n    ☐ two\n", "\t");
        assert_eq!(doc.unit, "  ");
        assert_eq!(doc.depth(1), 1);
        assert_eq!(doc.depth(2), 2);
        assert_eq!(doc.block_end(0), 2);
        assert_eq!(doc.block_end(1), 2);
    }

    #[test]
    fn project_paths_and_blocks() {
        let doc = Document::parse("A:\n\tB:\n\t\t☐ deep\n\n\t☐ shallow\nC:\n", "\t");
        assert_eq!(doc.project_path(2), vec!["A", "B"]);
        assert_eq!(doc.project_path(4), vec!["A"]);
        assert_eq!(doc.block_end(0), 4);
        assert_eq!(doc.block_end(1), 2);
        assert_eq!(doc.find_project("C"), Some(5));
        assert_eq!(doc.open_tasks_in_block(0), 2);
    }

    #[test]
    fn edited_lines_rerender() {
        let mut doc = Document::parse("☐   spaced   \n", "\t");
        assert_eq!(doc.serialize(), "☐   spaced   \n");
        doc.lines[0].set_text("spaced");
        assert_eq!(doc.serialize(), "☐ spaced\n");
    }
}
