//! `@tag` and `@tag(value)` parsing over a task's text.
//!
//! A tag starts with `@` at the beginning of the text or after whitespace, so
//! an email address is never mistaken for one. The value is everything up to
//! the first `)`; PlainTasks does not nest parentheses and neither do we.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub value: Option<String>,
    /// Byte range of the whole tag (including `@` and any `(value)`).
    pub start: usize,
    pub end: usize,
}

/// Tags whose value may be separated from the name by spaces.
const SPACED_VALUE_TAGS: &[&str] = &[
    "done",
    "cancelled",
    "started",
    "lasted",
    "created",
    "toggle",
    "due",
    "est",
];

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
}

pub fn parse_tags(text: &str) -> Vec<Tag> {
    let mut tags = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'@' {
            i += 1;
            continue;
        }
        let preceded_ok = i == 0 || text[..i].ends_with(char::is_whitespace);
        if !preceded_ok {
            i += 1;
            continue;
        }
        let name_start = i + 1;
        let mut name_end = name_start;
        for (off, c) in text[name_start..].char_indices() {
            if is_name_char(c) {
                name_end = name_start + off + c.len_utf8();
            } else {
                break;
            }
        }
        if name_end == name_start {
            i += 1;
            continue;
        }
        let mut end = name_end;
        let mut value = None;
        let name = &text[name_start..name_end];
        // Older PlainTasks wrote `@done (date)`; accept the gap for date tags.
        let paren_at = if bytes.get(name_end) == Some(&b'(') {
            Some(name_end)
        } else if SPACED_VALUE_TAGS
            .iter()
            .any(|t| t.eq_ignore_ascii_case(name))
        {
            let after = &text[name_end..];
            let ws = after.len() - after.trim_start_matches(' ').len();
            (ws > 0 && after[ws..].starts_with('(')).then_some(name_end + ws)
        } else {
            None
        };
        if let Some(open) = paren_at {
            if let Some(close) = text[open + 1..].find(')') {
                value = Some(text[open + 1..open + 1 + close].to_string());
                end = open + 1 + close + 1;
            }
        }
        tags.push(Tag {
            name: text[name_start..name_end].to_string(),
            value,
            start: i,
            end,
        });
        i = end.max(i + 1);
    }
    tags
}

pub fn find_tag<'a>(tags: &'a [Tag], name: &str) -> Option<&'a Tag> {
    tags.iter().find(|t| t.name.eq_ignore_ascii_case(name))
}

pub fn has_tag(text: &str, name: &str) -> bool {
    find_tag(&parse_tags(text), name).is_some()
}

pub fn tag_value(text: &str, name: &str) -> Option<String> {
    find_tag(&parse_tags(text), name).and_then(|t| t.value.clone())
}

/// Remove every occurrence of `@name` (with or without a value) together with
/// the whitespace that preceded it.
pub fn remove_tag(text: &str, name: &str) -> String {
    let tags = parse_tags(text);
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for t in tags.iter().filter(|t| t.name.eq_ignore_ascii_case(name)) {
        let mut keep_end = t.start;
        // Eat the whitespace run before the tag so we don't leave doubles.
        while keep_end > pos && text[..keep_end].ends_with(char::is_whitespace) {
            keep_end -= text[..keep_end]
                .chars()
                .next_back()
                .map_or(1, char::len_utf8);
        }
        out.push_str(&text[pos..keep_end]);
        pos = t.end;
    }
    out.push_str(&text[pos..]);
    // A tag at the very start leaves a leading space behind.
    out.trim_start().trim_end().to_string()
}

/// Append `@name` or `@name(value)` to the end of the text.
pub fn add_tag(text: &str, name: &str, value: Option<&str>) -> String {
    let mut out = text.trim_end().to_string();
    if !out.is_empty() {
        out.push(' ');
    }
    out.push('@');
    out.push_str(name);
    if let Some(v) = value {
        out.push('(');
        out.push_str(v);
        out.push(')');
    }
    out
}

/// Byte range of the first `http://` or `https://` URL in the text, if any.
pub fn find_url(text: &str) -> Option<(usize, usize)> {
    let start = ["https://", "http://"]
        .iter()
        .filter_map(|p| text.find(p))
        .min()?;
    let rest = &text[start..];
    let mut end = rest
        .find(|c: char| c.is_whitespace() || c == ')' || c == '>' || c == '"')
        .map_or(text.len(), |off| start + off);
    // Trailing punctuation is almost never part of the link.
    while end > start
        && matches!(
            text.as_bytes()[end - 1],
            b'.' | b',' | b';' | b':' | b'!' | b'?'
        )
    {
        end -= 1;
    }
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_valued_tags() {
        let tags = parse_tags("buy milk @today @due(24-01-05) mail me@example.com");
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].name, "today");
        assert_eq!(tags[0].value, None);
        assert_eq!(tags[1].name, "due");
        assert_eq!(tags[1].value.as_deref(), Some("24-01-05"));
        assert_eq!(
            &"buy milk @today @due(24-01-05) mail me@example.com"[tags[1].start..tags[1].end],
            "@due(24-01-05)"
        );
    }

    #[test]
    fn accepts_space_before_value_for_date_tags() {
        let tags = parse_tags("learn @done (12-09-07 07:30)");
        assert_eq!(tags[0].value.as_deref(), Some("12-09-07 07:30"));
        assert_eq!(remove_tag("learn @done (12-09-07 07:30)", "done"), "learn");
        // Only date tags get that leniency; a parenthesised aside stays text.
        let tags = parse_tags("x @home (call first)");
        assert_eq!(tags[0].value, None);
    }

    #[test]
    fn unterminated_value_is_a_bare_tag() {
        let tags = parse_tags("x @due(oops");
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "due");
        assert_eq!(tags[0].value, None);
    }

    #[test]
    fn removes_tag_and_surrounding_space() {
        assert_eq!(
            remove_tag("buy milk @today @high", "today"),
            "buy milk @high"
        );
        assert_eq!(remove_tag("@today buy milk", "today"), "buy milk");
        assert_eq!(
            remove_tag("buy milk @done(24-01-05 10:00)", "done"),
            "buy milk"
        );
        assert_eq!(remove_tag("buy milk", "done"), "buy milk");
    }

    #[test]
    fn adds_and_sets_tags() {
        assert_eq!(add_tag("buy milk ", "today", None), "buy milk @today");
        assert_eq!(add_tag("", "today", None), "@today");
    }

    #[test]
    fn finds_urls() {
        let t = "see https://example.com/a?b=1. now";
        let (s, e) = find_url(t).unwrap();
        assert_eq!(&t[s..e], "https://example.com/a?b=1");
        assert!(find_url("nothing here").is_none());
    }
}
