use egui::Color32;
use ropey::Rope;
use std::path::Path;

use crate::ui::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Plain,
    Xml,
    Json,
    Ini,
    IdeIpl,
    Log,
}

impl Language {
    pub fn from_path(path: Option<&Path>) -> Self {
        let Some(ext) = path
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
        else {
            return Language::Plain;
        };
        match ext.as_str() {
            "xml" | "meta" | "ymt" | "ymf" | "ymap" | "ytyp" => Language::Xml,
            "json" => Language::Json,
            "ini" | "vdf" | "sps" | "ugc" => Language::Ini,
            "ide" | "ipl" => Language::IdeIpl,
            _ => Language::Plain,
        }
    }
}

pub fn detect_log(rope: &Rope) -> bool {
    let line_count = rope.len_lines().min(64);
    for li in 0..line_count {
        let line = rope.line(li);
        let s: std::borrow::Cow<str> = line.into();
        let trimmed = s.trim_start();
        if trimmed.is_empty() || trimmed == "\n" {
            continue;
        }
        return looks_like_timestamp_prefix(trimmed);
    }
    false
}

pub fn looks_like_timestamp_prefix(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.first() == Some(&b'[') {
        if let Some(end) = s.find(']') {
            return s[1..end].contains(':');
        }
    }
    if bytes.len() >= 19
        && bytes[0..4].iter().all(|b| b.is_ascii_digit())
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(|b| b.is_ascii_digit())
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(|b| b.is_ascii_digit())
        && (bytes[10] == b' ' || bytes[10] == b'T')
        && bytes[11..13].iter().all(|b| b.is_ascii_digit())
        && bytes[13] == b':'
        && bytes[14..16].iter().all(|b| b.is_ascii_digit())
        && bytes[16] == b':'
        && bytes[17..19].iter().all(|b| b.is_ascii_digit())
    {
        return true;
    }
    if bytes.len() >= 8
        && bytes[0..2].iter().all(|b| b.is_ascii_digit())
        && bytes[2] == b':'
        && bytes[3..5].iter().all(|b| b.is_ascii_digit())
        && bytes[5] == b':'
        && bytes[6..8].iter().all(|b| b.is_ascii_digit())
    {
        return true;
    }
    false
}

pub struct Token<'a> {
    pub text: &'a str,
    pub color: Color32,
}

pub fn highlight_line<'a>(language: Language, line: &'a str) -> Vec<Token<'a>> {
    match language {
        Language::Plain => vec![Token {
            text: line,
            color: theme::TEXT_PRIMARY.get(),
        }],
        Language::Xml => highlight_xml(line),
        Language::Json => highlight_json(line),
        Language::Ini => highlight_ini(line),
        Language::IdeIpl => highlight_ide_ipl(line),
        Language::Log => highlight_log(line),
    }
}

fn push_token<'a>(tokens: &mut Vec<Token<'a>>, text: &'a str, color: Color32) {
    if !text.is_empty() {
        tokens.push(Token { text, color });
    }
}

pub(crate) fn highlight_xml(line: &str) -> Vec<Token<'_>> {
    let mut tokens: Vec<Token<'_>> = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if line[i..].starts_with("<!--") {
            let end = line[i + 4..]
                .find("-->")
                .map(|p| i + 4 + p + 3)
                .unwrap_or(bytes.len());
            push_token(&mut tokens, &line[i..end], theme::SYNTAX_COMMENT.get());
            i = end;
            continue;
        }
        if line[i..].starts_with("<![CDATA[") {
            let end = line[i + 9..]
                .find("]]>")
                .map(|p| i + 9 + p + 3)
                .unwrap_or(bytes.len());
            push_token(&mut tokens, &line[i..end], theme::SYNTAX_STRING.get());
            i = end;
            continue;
        }
        if line[i..].starts_with("<?") {
            let end = line[i + 2..]
                .find("?>")
                .map(|p| i + 2 + p + 2)
                .unwrap_or(bytes.len());
            push_token(&mut tokens, &line[i..end], theme::SYNTAX_TAG.get());
            i = end;
            continue;
        }
        if line[i..].starts_with("<!") {
            let end = line[i + 2..]
                .find('>')
                .map(|p| i + 2 + p + 1)
                .unwrap_or(bytes.len());
            push_token(&mut tokens, &line[i..end], theme::SYNTAX_KEYWORD.get());
            i = end;
            continue;
        }
        if bytes[i] == b'<' {
            let tag_end = find_tag_close(&line[i + 1..]).map_or(bytes.len(), |p| i + 1 + p + 1);
            let after_lt = i + 1;
            let (name_start, lt_run_end) = if bytes.get(after_lt) == Some(&b'/') {
                (after_lt + 1, after_lt + 1)
            } else {
                (after_lt, after_lt)
            };
            push_token(&mut tokens, &line[i..lt_run_end], theme::SYNTAX_PUNCT.get());
            let name_end = line[name_start..]
                .char_indices()
                .find(|(_, ch)| ch.is_whitespace() || *ch == '>' || *ch == '/')
                .map(|(p, _)| name_start + p)
                .unwrap_or(tag_end);
            push_token(
                &mut tokens,
                &line[name_start..name_end],
                theme::SYNTAX_TAG.get(),
            );

            let mut j = name_end;
            while j < tag_end {
                let ch = bytes[j];
                if ch == b'>' {
                    push_token(&mut tokens, &line[j..j + 1], theme::SYNTAX_PUNCT.get());
                    j += 1;
                    break;
                }
                if ch == b'/' {
                    let upto = (j + 2).min(tag_end);
                    push_token(&mut tokens, &line[j..upto], theme::SYNTAX_PUNCT.get());
                    j = upto;
                    continue;
                }
                // ⚠ The character and not its first byte: the search for a word's end below stops at
                // any space, and one outside ASCII would otherwise never be stepped over.
                if line[j..].starts_with(char::is_whitespace) {
                    let ws_end = line[j..]
                        .char_indices()
                        .find(|(_, c)| !c.is_whitespace())
                        .map(|(p, _)| j + p)
                        .unwrap_or(tag_end);
                    push_token(&mut tokens, &line[j..ws_end], theme::TEXT_PRIMARY.get());
                    j = ws_end;
                    continue;
                }
                if ch == b'"' || ch == b'\'' {
                    let close = line[j + 1..]
                        .find(ch as char)
                        .map(|p| j + 1 + p + 1)
                        .unwrap_or(tag_end);
                    push_token(&mut tokens, &line[j..close], theme::SYNTAX_STRING.get());
                    j = close;
                    continue;
                }
                if ch == b'=' {
                    push_token(&mut tokens, &line[j..j + 1], theme::SYNTAX_PUNCT.get());
                    j += 1;
                    continue;
                }
                let attr_end = line[j..]
                    .char_indices()
                    .find(|(_, c)| c.is_whitespace() || *c == '=' || *c == '>' || *c == '/')
                    .map(|(p, _)| j + p)
                    .unwrap_or(tag_end);
                push_token(&mut tokens, &line[j..attr_end], theme::SYNTAX_ATTR.get());
                j = attr_end;
            }
            i = j.max(tag_end);
            continue;
        }
        let next = line[i..].find('<').map(|p| i + p).unwrap_or(bytes.len());
        push_token(&mut tokens, &line[i..next], theme::TEXT_PRIMARY.get());
        i = next;
    }
    if tokens.is_empty() {
        tokens.push(Token {
            text: line,
            color: theme::TEXT_PRIMARY.get(),
        });
    }
    tokens
}

fn find_tag_close(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch == b'"' || ch == b'\'' {
            let close = s[i + 1..].find(ch as char).map(|p| i + 1 + p + 1);
            i = close.unwrap_or(bytes.len());
            continue;
        }
        if ch == b'>' {
            return Some(i);
        }
        i += 1;
    }
    None
}

pub(crate) fn highlight_json(line: &str) -> Vec<Token<'_>> {
    let mut tokens: Vec<Token<'_>> = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'"' {
            let mut end = i + 1;
            let mut escaped = false;
            while end < bytes.len() {
                let ch = bytes[end];
                if escaped {
                    escaped = false;
                } else if ch == b'\\' {
                    escaped = true;
                } else if ch == b'"' {
                    end += 1;
                    break;
                }
                end += 1;
            }
            let is_key = line[end..]
                .chars()
                .find(|c| !c.is_whitespace())
                .map(|c| c == ':')
                .unwrap_or(false);
            push_token(
                &mut tokens,
                &line[i..end],
                if is_key {
                    theme::SYNTAX_ATTR.get()
                } else {
                    theme::SYNTAX_STRING.get()
                },
            );
            i = end;
        } else if c.is_ascii_digit()
            || (c == b'-' && bytes.get(i + 1).is_some_and(|b| b.is_ascii_digit()))
        {
            let end = line[i..]
                .char_indices()
                .find(|(_, ch)| !(ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+' | 'e' | 'E')))
                .map(|(p, _)| i + p)
                .unwrap_or(bytes.len());
            push_token(&mut tokens, &line[i..end], theme::SYNTAX_NUMBER.get());
            i = end;
        } else if line[i..].starts_with("true")
            || line[i..].starts_with("false")
            || line[i..].starts_with("null")
        {
            let len = if line[i..].starts_with("false") { 5 } else { 4 };
            push_token(&mut tokens, &line[i..i + len], theme::SYNTAX_KEYWORD.get());
            i += len;
        } else if matches!(c, b'{' | b'}' | b'[' | b']' | b':' | b',') {
            push_token(&mut tokens, &line[i..i + 1], theme::SYNTAX_PUNCT.get());
            i += 1;
        } else {
            let end = line[i..]
                .char_indices()
                .find(|(_, ch)| {
                    matches!(ch, '"' | '{' | '}' | '[' | ']' | ':' | ',') || ch.is_ascii_digit()
                })
                .map(|(p, _)| i + p)
                .unwrap_or(bytes.len());
            let end = end.max(i + 1);
            push_token(&mut tokens, &line[i..end], theme::TEXT_PRIMARY.get());
            i = end;
        }
    }
    if tokens.is_empty() {
        tokens.push(Token {
            text: line,
            color: theme::TEXT_PRIMARY.get(),
        });
    }
    tokens
}

pub(crate) fn highlight_ini(line: &str) -> Vec<Token<'_>> {
    let trimmed = line.trim_start();
    if trimmed.starts_with(';') || trimmed.starts_with('#') {
        return vec![Token {
            text: line,
            color: theme::SYNTAX_COMMENT.get(),
        }];
    }
    if trimmed.starts_with('[') && trimmed.contains(']') {
        return vec![Token {
            text: line,
            color: theme::SYNTAX_TAG.get(),
        }];
    }
    if let Some(eq) = line.find('=') {
        return vec![
            Token {
                text: &line[..eq],
                color: theme::SYNTAX_ATTR.get(),
            },
            Token {
                text: &line[eq..eq + 1],
                color: theme::SYNTAX_PUNCT.get(),
            },
            Token {
                text: &line[eq + 1..],
                color: theme::SYNTAX_STRING.get(),
            },
        ];
    }
    vec![Token {
        text: line,
        color: theme::TEXT_PRIMARY.get(),
    }]
}

const IDE_IPL_KEYWORDS: &[&str] = &[
    "objs", "tobj", "weap", "hier", "anim", "peds", "cars", "cull", "inst", "path", "mlo", "grge",
    "enex", "pick", "jump", "tcyc", "auzo", "mult", "occl", "zone", "blok", "txdp", "amat", "end",
];

pub(crate) fn highlight_ide_ipl(line: &str) -> Vec<Token<'_>> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') || trimmed.starts_with("//") {
        return vec![Token {
            text: line,
            color: theme::SYNTAX_COMMENT.get(),
        }];
    }
    let mut out: Vec<Token<'_>> = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if ch.is_whitespace() || ch == ',' {
            let end = line[i..]
                .char_indices()
                .find(|(_, c)| !(c.is_whitespace() || *c == ','))
                .map(|(p, _)| i + p)
                .unwrap_or(bytes.len());
            push_token(&mut out, &line[i..end], theme::TEXT_PRIMARY.get());
            i = end;
            continue;
        }
        let end = line[i..]
            .char_indices()
            .find(|(_, c)| c.is_whitespace() || *c == ',')
            .map(|(p, _)| i + p)
            .unwrap_or(bytes.len());
        let tok = &line[i..end];
        let color = classify_ide_ipl_token(tok);
        push_token(&mut out, tok, color);
        i = end;
    }
    if out.is_empty() {
        out.push(Token {
            text: line,
            color: theme::TEXT_PRIMARY.get(),
        });
    }
    out
}

fn classify_ide_ipl_token(tok: &str) -> Color32 {
    if IDE_IPL_KEYWORDS
        .iter()
        .any(|kw| kw.eq_ignore_ascii_case(tok))
    {
        return theme::SYNTAX_KEYWORD.get();
    }
    if is_numeric_literal(tok) {
        return theme::SYNTAX_NUMBER.get();
    }
    theme::TEXT_PRIMARY.get()
}

fn is_numeric_literal(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    if bytes[0] == b'-' || bytes[0] == b'+' {
        i = 1;
    }
    if i >= bytes.len() {
        return false;
    }
    let mut saw_digit = false;
    let mut saw_dot = false;
    let mut saw_exp = false;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_digit() {
            saw_digit = true;
        } else if c == b'.' && !saw_dot && !saw_exp {
            saw_dot = true;
        } else if (c == b'e' || c == b'E') && saw_digit && !saw_exp {
            saw_exp = true;
            if let Some(&next) = bytes.get(i + 1) {
                if next == b'+' || next == b'-' {
                    i += 1;
                }
            }
        } else {
            return false;
        }
        i += 1;
    }
    saw_digit
}

pub(crate) fn highlight_log(line: &str) -> Vec<Token<'_>> {
    let mut out: Vec<Token<'_>> = Vec::new();
    let bytes = line.as_bytes();

    let ts_end = if bytes.first() == Some(&b'[') {
        line.find(']').map(|p| p + 1).unwrap_or(0)
    } else if looks_like_timestamp_prefix(line) {
        line.find([' ', '\t']).unwrap_or(bytes.len())
    } else {
        0
    };
    if ts_end > 0 {
        push_token(&mut out, &line[..ts_end], theme::SYNTAX_NUMBER.get());
    }

    let mut i = ts_end;
    while i < bytes.len() {
        let ch = bytes[i];
        // ⚠ The character and not its first byte, as in `highlight_xml`.
        if line[i..].starts_with(char::is_whitespace)
            || matches!(ch, b'[' | b']' | b'(' | b')' | b':' | b',')
        {
            let width = line[i..].chars().next().map_or(1, char::len_utf8);
            push_token(&mut out, &line[i..i + width], theme::TEXT_PRIMARY.get());
            i += width;
            continue;
        }
        if ch == b'"' || ch == b'\'' {
            let close = line[i + 1..]
                .find(ch as char)
                .map(|p| i + 1 + p + 1)
                .unwrap_or(bytes.len());
            push_token(&mut out, &line[i..close], theme::SYNTAX_STRING.get());
            i = close;
            continue;
        }
        let end = line[i..]
            .char_indices()
            .find(|(_, c)| {
                c.is_whitespace() || matches!(c, '[' | ']' | '(' | ')' | ':' | ',' | '"' | '\'')
            })
            .map(|(p, _)| i + p)
            .unwrap_or(bytes.len());
        let tok = &line[i..end];
        let color = classify_log_token(tok);
        push_token(&mut out, tok, color);
        i = end;
    }

    if out.is_empty() {
        out.push(Token {
            text: line,
            color: theme::TEXT_PRIMARY.get(),
        });
    }
    out
}

fn classify_log_token(tok: &str) -> Color32 {
    if tok.eq_ignore_ascii_case("error") || tok.eq_ignore_ascii_case("fatal") {
        return theme::SYNTAX_LEVEL_ERROR.get();
    }
    if tok.eq_ignore_ascii_case("warn") || tok.eq_ignore_ascii_case("warning") {
        return theme::SYNTAX_LEVEL_WARN.get();
    }
    if tok.eq_ignore_ascii_case("info")
        || tok.eq_ignore_ascii_case("debug")
        || tok.eq_ignore_ascii_case("trace")
    {
        return theme::SYNTAX_LEVEL_INFO.get();
    }
    if is_numeric_literal(tok) {
        return theme::SYNTAX_NUMBER.get();
    }
    theme::TEXT_PRIMARY.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors(toks: &[Token<'_>]) -> Vec<(String, Color32)> {
        toks.iter().map(|t| (t.text.to_string(), t.color)).collect()
    }

    fn joined_text(toks: &[Token<'_>]) -> String {
        toks.iter().map(|t| t.text).collect()
    }

    /// ⚠ A space outside ASCII is several bytes, none of which is a space taken alone.
    #[test]
    fn a_space_outside_ascii_ends_a_word_and_the_line_is_all_there() {
        let _palette = crate::ui::theme::serial();
        for line in ["<a\u{a0}b=\"1\">", "<\u{a0}>", "<Item\u{2028}/>"] {
            assert_eq!(joined_text(&highlight_xml(line)), line);
        }
        for line in ["warn\u{a0}ing: x", "\u{a0}", "a\u{3000}b"] {
            assert_eq!(joined_text(&highlight_log(line)), line);
        }
    }

    #[test]
    fn xml_simple_open_close() {
        let _palette = crate::ui::theme::serial();
        let toks = highlight_xml("<foo>");
        assert_eq!(joined_text(&toks), "<foo>");
        assert!(toks
            .iter()
            .any(|t| t.text == "foo" && t.color == theme::SYNTAX_TAG.get()));
    }

    #[test]
    fn xml_self_closing_with_attr() {
        let _palette = crate::ui::theme::serial();
        let toks = highlight_xml(r#"<Item Name="hi"/>"#);
        assert_eq!(joined_text(&toks), r#"<Item Name="hi"/>"#);
        let palette = colors(&toks);
        assert!(palette
            .iter()
            .any(|(t, c)| t == "Item" && *c == theme::SYNTAX_TAG.get()));
        assert!(palette
            .iter()
            .any(|(t, c)| t == "Name" && *c == theme::SYNTAX_ATTR.get()));
        assert!(palette
            .iter()
            .any(|(t, c)| t == "\"hi\"" && *c == theme::SYNTAX_STRING.get()));
    }

    #[test]
    fn xml_comment() {
        let _palette = crate::ui::theme::serial();
        let toks = highlight_xml("<!-- hi -->");
        assert_eq!(joined_text(&toks), "<!-- hi -->");
        assert_eq!(toks[0].color, theme::SYNTAX_COMMENT.get());
    }

    #[test]
    fn xml_cdata() {
        let _palette = crate::ui::theme::serial();
        let toks = highlight_xml("<![CDATA[raw <text>]]>");
        assert_eq!(joined_text(&toks), "<![CDATA[raw <text>]]>");
        assert_eq!(toks[0].color, theme::SYNTAX_STRING.get());
    }

    #[test]
    fn xml_processing_instruction() {
        let _palette = crate::ui::theme::serial();
        let toks = highlight_xml(r#"<?xml version="1.0"?>"#);
        assert_eq!(joined_text(&toks), r#"<?xml version="1.0"?>"#);
        assert_eq!(toks[0].color, theme::SYNTAX_TAG.get());
    }

    #[test]
    fn xml_close_with_quoted_gt() {
        let toks = highlight_xml(r#"<a x=">"/>"#);
        assert_eq!(joined_text(&toks), r#"<a x=">"/>"#);
    }

    #[test]
    fn json_keys_vs_values() {
        let _palette = crate::ui::theme::serial();
        let toks = highlight_json(r#"{"name": "value"}"#);
        let palette = colors(&toks);
        assert!(palette
            .iter()
            .any(|(t, c)| t == "\"name\"" && *c == theme::SYNTAX_ATTR.get()));
        assert!(palette
            .iter()
            .any(|(t, c)| t == "\"value\"" && *c == theme::SYNTAX_STRING.get()));
    }

    #[test]
    fn ini_section_and_kv() {
        let _palette = crate::ui::theme::serial();
        let s = "[section]";
        assert_eq!(highlight_ini(s)[0].color, theme::SYNTAX_TAG.get());
        let toks = highlight_ini("key=value");
        assert_eq!(toks[0].color, theme::SYNTAX_ATTR.get());
        assert_eq!(toks[1].text, "=");
        assert_eq!(toks[2].color, theme::SYNTAX_STRING.get());
    }

    #[test]
    fn ide_keyword_and_number() {
        let _palette = crate::ui::theme::serial();
        let toks = highlight_ide_ipl("inst, 1234");
        let palette = colors(&toks);
        assert!(palette
            .iter()
            .any(|(t, c)| t == "inst" && *c == theme::SYNTAX_KEYWORD.get()));
        assert!(palette
            .iter()
            .any(|(t, c)| t == "1234" && *c == theme::SYNTAX_NUMBER.get()));
    }

    #[test]
    fn log_levels() {
        let _palette = crate::ui::theme::serial();
        let toks = highlight_log("[12:34:56] ERROR something broke");
        let palette = colors(&toks);
        assert!(palette
            .iter()
            .any(|(t, c)| t == "ERROR" && *c == theme::SYNTAX_LEVEL_ERROR.get()));
    }

    #[test]
    fn detect_log_iso() {
        assert!(looks_like_timestamp_prefix("2024-01-02 12:34:56 hello"));
        assert!(looks_like_timestamp_prefix("[12:34:56] hi"));
        assert!(looks_like_timestamp_prefix("12:34:56 hi"));
        assert!(!looks_like_timestamp_prefix("hello world"));
    }
}
