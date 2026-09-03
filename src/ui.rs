//! Rendering. Pure function of `App` state, plus the cursor placement the
//! terminal needs for inline editing.

use chrono::NaiveDateTime;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Mode, PromptKind};
use crate::doc::{dates, tags, Kind, Status};
use crate::editor::LineEditor;

pub const KEYS: &[(&str, &str)] = &[
    ("j/k ↑/↓", "move"),
    ("gg / G", "top / bottom"),
    ("[ / ]", "previous / next project"),
    ("^d / ^u", "page down / up"),
    ("Enter e a", "edit line (cursor at end)"),
    ("i", "edit line (cursor at start)"),
    ("o / O", "new task below / above"),
    ("m", "new note"),
    ("p", "new project"),
    ("Space x", "toggle done"),
    ("c", "toggle cancelled"),
    ("s", "toggle @started"),
    ("t", "toggle @today"),
    ("1 2 3", "@critical / @high / @low"),
    ("@", "add or remove a tag"),
    ("Tab / S-Tab", "indent / outdent block"),
    ("J / K", "move block down / up"),
    ("d / D", "delete line / block"),
    ("A", "archive done tasks"),
    ("u / U", "undo / redo"),
    ("z / Z", "fold project / fold all"),
    ("h", "hide or show done tasks"),
    ("/ n N", "search, next, previous"),
    ("L", "open link on line"),
    ("w / R", "save / reload"),
    ("q / Q", "quit / quit discarding"),
    ("?", "this help"),
];

pub struct Painted {
    pub list: Rect,
    pub rows: Vec<usize>,
}

pub fn draw(f: &mut Frame, app: &mut App) -> Painted {
    let area = f.area();
    let [list_area, status_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
    let rows = app.visible_rows();
    let height = list_area.height as usize;
    app.adjust_scroll(height, &rows);
    let width = list_area.width as usize;
    let now = app.now_for_display();

    let mut lines: Vec<Line> = Vec::with_capacity(height);
    let mut cursor_pos: Option<(u16, u16)> = None;
    for (row_i, &idx) in rows.iter().enumerate().skip(app.scroll).take(height) {
        let y = list_area.y + (row_i - app.scroll) as u16;
        if let Mode::Insert { editor, line, .. } = &app.mode {
            if *line == idx {
                let (l, x) = edit_row(app, idx, editor, width);
                lines.push(l);
                cursor_pos = Some((list_area.x + x, y));
                continue;
            }
        }
        lines.push(row(app, idx, idx == app.cursor, width, now));
    }
    if rows.is_empty() {
        let hint = if app.doc.is_empty() {
            "empty — press o to add a task, ? for help"
        } else {
            "everything is done or hidden — h shows done tasks"
        };
        lines.push(Line::from(Span::styled(
            hint,
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    f.render_widget(Paragraph::new(lines), list_area);

    if let Some(pos) = status(f, app, status_area) {
        cursor_pos = Some(pos);
    }
    if let Some((x, y)) = cursor_pos {
        f.set_cursor_position((x, y));
    }
    if matches!(app.mode, Mode::Help) {
        help(f, area);
    }
    Painted {
        list: list_area,
        rows,
    }
}

fn indent_cols(app: &App, idx: usize) -> usize {
    app.doc.depth(idx) * app.cfg.display_indent as usize
}

fn tag_style(name: &str, value: Option<&str>, fmt: &str, now: NaiveDateTime) -> Style {
    let n = name.to_ascii_lowercase();
    match n.as_str() {
        "today" => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        "critical" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        "high" => Style::default().fg(Color::Red),
        "low" => Style::default().fg(Color::Blue),
        "done" => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::DIM),
        "cancelled" | "lasted" | "project" => Style::default().add_modifier(Modifier::DIM),
        "started" => Style::default().fg(Color::Magenta),
        "due" => {
            if value.is_some_and(|v| dates::is_overdue(v, fmt, now)) {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Magenta)
            }
        }
        _ => Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
    }
}

/// Style every character of `text` (tags, links, search hits) and group runs
/// into spans.
fn text_spans(app: &App, text: &str, base: Style, now: NaiveDateTime) -> Vec<Span<'static>> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut styles: Vec<Style> = vec![base; chars.len()];
    let byte_to_char = |b: usize| {
        chars
            .iter()
            .position(|(i, _)| *i >= b)
            .unwrap_or(chars.len())
    };
    for t in tags::parse_tags(text) {
        let st = tag_style(&t.name, t.value.as_deref(), &app.cfg.date_format, now);
        let st = if base.add_modifier.contains(Modifier::DIM) {
            st.add_modifier(Modifier::DIM)
        } else {
            st
        };
        for s in &mut styles[byte_to_char(t.start)..byte_to_char(t.end)] {
            *s = st;
        }
    }
    if let Some((s, e)) = tags::find_url(text) {
        for st in &mut styles[byte_to_char(s)..byte_to_char(e)] {
            *st = st.add_modifier(Modifier::UNDERLINED);
        }
    }
    if let Some(q) = &app.search {
        let needle: Vec<char> = q.chars().flat_map(char::to_lowercase).collect();
        let hay: Vec<char> = chars.iter().flat_map(|(_, c)| c.to_lowercase()).collect();
        if !needle.is_empty() && hay.len() == chars.len() {
            let mut i = 0;
            while i + needle.len() <= hay.len() {
                if hay[i..i + needle.len()] == needle[..] {
                    for st in &mut styles[i..i + needle.len()] {
                        *st = Style::default().fg(Color::Black).bg(Color::Yellow);
                    }
                    i += needle.len();
                } else {
                    i += 1;
                }
            }
        }
    }
    let mut spans = Vec::new();
    let mut run = String::new();
    let mut run_style = base;
    for (k, (_, c)) in chars.iter().enumerate() {
        if k == 0 {
            run_style = styles[0];
        }
        if styles[k] != run_style {
            spans.push(Span::styled(std::mem::take(&mut run), run_style));
            run_style = styles[k];
        }
        run.push(*c);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, run_style));
    }
    spans
}

fn row(app: &App, idx: usize, is_cursor: bool, width: usize, now: NaiveDateTime) -> Line<'static> {
    let line = &app.doc.lines[idx];
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ".repeat(indent_cols(app, idx)))];
    match &line.kind {
        Kind::Project { name, suffix } => {
            let head = Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD);
            if app.is_folded(idx) {
                spans.push(Span::styled("▸ ", head));
            }
            spans.push(Span::styled(format!("{name}:"), head));
            if !suffix.is_empty() {
                spans.extend(text_spans(app, suffix, Style::default(), now));
            }
            let open = app.doc.open_tasks_in_block(idx);
            if open > 0 {
                spans.push(Span::styled(
                    format!("  {open}"),
                    Style::default().add_modifier(Modifier::DIM),
                ));
            }
        }
        Kind::Task {
            bullet,
            status,
            text,
        } => {
            let (bullet_style, text_style) = match status {
                Status::Open => (Style::default(), Style::default()),
                Status::Done => (
                    Style::default().fg(Color::Green),
                    Style::default().add_modifier(Modifier::DIM),
                ),
                Status::Cancelled => (
                    Style::default().fg(Color::Red).add_modifier(Modifier::DIM),
                    Style::default().add_modifier(Modifier::DIM | Modifier::CROSSED_OUT),
                ),
            };
            spans.push(Span::styled(format!("{bullet} "), bullet_style));
            spans.extend(text_spans(app, text, text_style, now));
        }
        Kind::Note { text } => {
            let st = if line.is_separator() {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().add_modifier(Modifier::DIM | Modifier::ITALIC)
            };
            spans.extend(text_spans(app, text, st, now));
        }
        Kind::Blank => {}
    }
    if is_cursor {
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        if used < width {
            spans.push(Span::raw(" ".repeat(width - used)));
        }
        for s in &mut spans {
            s.style = s.style.add_modifier(Modifier::REVERSED);
        }
    }
    Line::from(spans)
}

/// The row being edited: fixed prefix, then the editor buffer scrolled so the
/// cursor stays on screen. Returns the line and the cursor's x offset.
fn edit_row(app: &App, idx: usize, editor: &LineEditor, width: usize) -> (Line<'static>, u16) {
    let line = &app.doc.lines[idx];
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ".repeat(indent_cols(app, idx)))];
    match &line.kind {
        Kind::Task { bullet, .. } => spans.push(Span::raw(format!("{bullet} "))),
        Kind::Project { .. } => {}
        Kind::Note { .. } | Kind::Blank => {}
    }
    let prefix_cols: usize = spans.iter().map(|s| s.content.width()).sum();
    let avail = width.saturating_sub(prefix_cols + 1).max(1);
    let cursor_col = editor.cursor_col();
    // Drop leading characters until the cursor fits.
    let mut skipped_cols = 0;
    let mut start = 0;
    for (i, c) in editor.buf.char_indices() {
        if cursor_col - skipped_cols < avail {
            break;
        }
        skipped_cols += c.width().unwrap_or(0);
        start = i + c.len_utf8();
    }
    let shown = editor.buf[start..].to_string();
    let style = match &line.kind {
        Kind::Project { .. } => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        _ => Style::default(),
    };
    spans.push(Span::styled(shown, style));
    if let Kind::Project { .. } = &line.kind {
        spans.push(Span::styled(
            ":",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    }
    let x = (prefix_cols + cursor_col - skipped_cols) as u16;
    (Line::from(spans), x)
}

fn status(f: &mut Frame, app: &App, area: Rect) -> Option<(u16, u16)> {
    if let Mode::Prompt { kind, editor } = &app.mode {
        let label = match kind {
            PromptKind::Search => "/",
            PromptKind::Tag => "@",
        };
        let line = Line::from(vec![
            Span::styled(
                label,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(editor.buf.clone()),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return Some((
            area.x + (label.width() + editor.cursor_col()) as u16,
            area.y,
        ));
    }
    let (mode_label, mode_color) = match app.mode {
        Mode::Normal => (" NORMAL ", Color::Blue),
        Mode::Insert { .. } => (" INSERT ", Color::Green),
        Mode::Prompt { .. } => (" PROMPT ", Color::Yellow),
        Mode::Help => (" HELP ", Color::Magenta),
    };
    let mut left = vec![
        Span::styled(
            mode_label,
            Style::default()
                .fg(Color::Black)
                .bg(mode_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            app.path_display(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ];
    if app.dirty {
        left.push(Span::styled(" [+]", Style::default().fg(Color::Yellow)));
    }
    if app.changed_on_disk {
        left.push(Span::styled(
            " [changed on disk]",
            Style::default().fg(Color::Red),
        ));
    }
    if let Some(m) = &app.message {
        let st = if m.error {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::Green)
        };
        left.push(Span::raw("  "));
        left.push(Span::styled(m.text.clone(), st));
    }
    let (open, done, cancelled) = app.doc.count_tasks();
    let mut right = vec![Span::styled(
        format!("☐ {open}  ✔ {done}"),
        Style::default().add_modifier(Modifier::DIM),
    )];
    if cancelled > 0 {
        right.push(Span::styled(
            format!("  ✘ {cancelled}"),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    if app.host.is_herdr() {
        right.push(Span::raw("  "));
        right.push(Span::styled(
            " herdr ",
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ));
    }
    right.push(Span::styled(
        "  ? help ",
        Style::default().add_modifier(Modifier::DIM),
    ));
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    let fits = |spans: &[Span]| {
        spans.iter().map(|s| s.content.width()).sum::<usize>() + right_w <= area.width as usize
    };
    if !fits(&left) {
        // Narrow terminal: drop the directory, then the message.
        left[2] = Span::styled(
            app.file_name(),
            Style::default().add_modifier(Modifier::BOLD),
        );
    }
    if !fits(&left) && app.message.is_some() {
        left.truncate(left.len() - 2);
    }
    let left_w: usize = left.iter().map(|s| s.content.width()).sum();
    let gap = (area.width as usize).saturating_sub(left_w + right_w);
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(right);
    f.render_widget(Paragraph::new(Line::from(spans)), area);
    None
}

fn help(f: &mut Frame, area: Rect) {
    let rows = KEYS.len().div_ceil(2);
    let two_col = area.width >= 76;
    let inner_h = if two_col { rows } else { KEYS.len() } as u16;
    let w = if two_col { 76 } else { 40 }.min(area.width);
    let h = (inner_h + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    let key_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line> = Vec::new();
    if two_col {
        for r in 0..rows {
            let mut spans = Vec::new();
            for col in 0..2 {
                if let Some((k, d)) = KEYS.get(r + col * rows) {
                    spans.push(Span::styled(format!(" {k:<12}"), key_style));
                    spans.push(Span::raw(format!("{d:<24}")));
                }
            }
            lines.push(Line::from(spans));
        }
    } else {
        for (k, d) in KEYS {
            lines.push(Line::from(vec![
                Span::styled(format!(" {k:<12}"), key_style),
                Span::raw((*d).to_string()),
            ]));
        }
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" tsk — keys (any key closes) ")
        .title_style(Style::default().add_modifier(Modifier::BOLD));
    f.render_widget(Clear, popup);
    f.render_widget(Paragraph::new(lines).block(block), popup);
    let _ = Stylize::bold("");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::host::Host;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn app_with(src: &str) -> App {
        let path = crate::store::temp_task_file(src);
        let cfg = Config {
            autosave: false,
            ..Config::default()
        };
        App::new(path, cfg, Host::Standalone).unwrap()
    }

    fn render(app: &mut App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| {
            draw(f, app);
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_projects_tasks_and_status() {
        let mut app =
            app_with("Inbox:\n\t☐ milk @today\n\t✔ eggs @done(26-01-01 00:00)\n\t\ta note\n");
        let out = render(&mut app, 60, 8);
        assert!(out.contains("Inbox:  1"), "{out}");
        assert!(out.contains("  ☐ milk @today"), "{out}");
        assert!(out.contains("  ✔ eggs @done(26-01-01 00:00)"), "{out}");
        assert!(out.contains("    a note"), "{out}");
        assert!(out.contains("NORMAL"), "{out}");
        assert!(out.contains("☐ 1  ✔ 1"), "{out}");
    }

    #[test]
    fn insert_mode_places_cursor_after_text() {
        let mut app = app_with("☐ ab\n");
        app.handle_key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Enter,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        let mut term = Terminal::new(TestBackend::new(20, 4)).unwrap();
        term.draw(|f| {
            draw(f, &mut app);
        })
        .unwrap();
        assert_eq!(
            term.get_cursor_position().unwrap(),
            ratatui::layout::Position { x: 4, y: 0 }
        );
    }

    #[test]
    fn folded_project_shows_marker_and_help_fits_small_screens() {
        let mut app = app_with("A:\n\t☐ one\n");
        app.handle_key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Char('z'),
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        let out = render(&mut app, 40, 5);
        assert!(out.contains("▸ A:"), "{out}");
        app.handle_key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Char('?'),
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        let out = render(&mut app, 30, 6);
        assert!(out.contains("keys"), "{out}");
    }
}
