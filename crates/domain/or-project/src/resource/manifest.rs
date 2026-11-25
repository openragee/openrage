//! The manifest of a resource, read as far as a check needs: its lines of the form `key 'value'`, and the lines that are something else.
//!
//! A manifest is a Lua program a server runs. This reads the statements real manifests are made of (a key followed by text, by a table
//! of text, or by either in brackets) and runs nothing, so a value a manifest computes is not known here.
//!
//! ⚠ Nothing is refused. What is not a statement of that form is skipped to the end of the last line read of it, and the lines skipped
//! are kept in [`Manifest::unread`]: a claim that a manifest lacks a line holds only while that list is empty.

use std::collections::BTreeMap;

/// The manifest a server reads first, and the older name it still reads.
pub const NAMES: [&str; 2] = [crate::plan::MANIFEST, "__resource.lua"];

/// The words of Lua that begin a statement no manifest line is.
const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// The key of the line of a FiveM manifest that declares the resource a map, which every reader and writer of that line spells from here.
pub const MAP_KEY: &str = "this_is_a_map";

/// One statement of a manifest: a key and the text after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directive {
    pub key: String,
    /// Each text after the key, a table's entries in their order.
    pub values: Vec<String>,
    /// The line the key is on, from 1.
    pub line: usize,
}

/// A `data_file` line: the type the game is told and the path it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataFile<'a> {
    pub file_type: &'a str,
    pub path: &'a str,
    pub line: usize,
}

/// What a manifest states, and what of it was not read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    pub directives: Vec<Directive>,
    /// The first and last line of each run of lines that is no statement this reads, from 1.
    pub unread: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Name(String),
    Text(String),
    Open(char),
    Close(char),
    Separator,
    Other,
}

struct Lexed {
    token: Token,
    line: usize,
}

impl Manifest {
    /// Read the text of a manifest; what is not understood is in [`Manifest::unread`].
    pub fn read(text: &str) -> Self {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let tokens = lex(text);
        let mut manifest = Self::default();
        // Whether a statement was read since the last unread line, which ends a run of them.
        let mut read_since = true;
        let mut at = 0;
        while at < tokens.len() {
            if tokens[at].token == Token::Separator {
                at += 1;
                continue;
            }
            match directive(&tokens, at) {
                Ok((found, next, odd)) => {
                    manifest.directives.push(found);
                    read_since = true;
                    for line in odd {
                        manifest.skipped(line, &mut read_since);
                    }
                    at = next;
                }
                // ⚠ Stepped over as far as it was read and never read again from inside, so the work stays in proportion to the text.
                Err(last) => {
                    manifest.skipped(tokens[at].line, &mut read_since);
                    manifest.skipped(tokens[last].line, &mut read_since);
                    at = skip(&tokens, last);
                }
            }
        }
        manifest
    }

    /// Keep `line` as unread: in the run before it when no statement was read since, else as a new run.
    fn skipped(&mut self, line: usize, read_since: &mut bool) {
        match self.unread.last_mut() {
            Some(run) if !*read_since => run.1 = run.1.max(line),
            _ => self.unread.push((line, line)),
        }
        *read_since = false;
    }

    fn values_of<'a>(&'a self, keys: &'a [&str]) -> impl Iterator<Item = (usize, &'a str)> {
        (self.directives.iter())
            .filter(|d| keys.contains(&d.key.as_str()))
            .flat_map(|d| d.values.iter().map(|v| (d.line, v.as_str())))
    }

    /// The value of the version line, `fx_version` or the older `resource_manifest_version`.
    pub fn version(&self) -> Option<&str> {
        self.values_of(&["fx_version", "resource_manifest_version"])
            .map(|(_, value)| value)
            .next()
    }

    /// The games the manifest names, by `game` or `games`.
    pub fn games(&self) -> Vec<&str> {
        self.values_of(&["game", "games"]).map(|(_, v)| v).collect()
    }

    /// Each entry of `file` and `files` with its statement's line; an entry may be a glob.
    pub fn files(&self) -> Vec<(usize, &str)> {
        self.values_of(&["file", "files"]).collect()
    }

    /// Each `data_file` line that states a type and a path.
    pub fn data_files(&self) -> Vec<DataFile<'_>> {
        (self.directives.iter())
            .filter(|d| d.key == "data_file")
            .filter_map(|d| match d.values.as_slice() {
                [file_type, path] => Some(DataFile {
                    file_type,
                    path,
                    line: d.line,
                }),
                _ => None,
            })
            .collect()
    }

    /// Each `data_file` line that states anything but a type and a path: its line, and how many texts it states.
    pub fn data_files_unread(&self) -> Vec<(usize, usize)> {
        (self.directives.iter())
            .filter(|d| d.key == "data_file" && d.values.len() != 2)
            .map(|d| (d.line, d.values.len()))
            .collect()
    }

    /// Whether a line declares the resource a map.
    pub fn is_map(&self) -> bool {
        self.directives.iter().any(|d| d.key == MAP_KEY)
    }
}

/// Whether an entry names files by a pattern and not one file.
pub fn is_glob(entry: &str) -> bool {
    entry.contains('*')
}

/// The statement that begins at `at`, the index after it, and the lines of table entries that are not text; when it is not `key` and text, the index of the last token read of it.
fn directive(tokens: &[Lexed], at: usize) -> Result<(Directive, usize, Vec<usize>), usize> {
    let Token::Name(key) = &tokens[at].token else {
        return Err(at);
    };
    if KEYWORDS.contains(&key.as_str()) {
        return Err(at);
    }
    let mut values = Vec::new();
    let mut odd = Vec::new();
    let mut next = at + 1;
    let mut taken = 0;
    while let Some(after) = argument(tokens, next, &mut values, &mut odd) {
        next = after;
        taken += 1;
    }
    // ⚠ Text joined to something else on its line (`'a' .. b`) is not the value the manifest states.
    let joined = tokens.get(next).is_some_and(|t| {
        t.line == tokens[next - 1].line && !matches!(t.token, Token::Separator | Token::Name(_))
    });
    if taken == 0 || joined {
        return Err(next - 1);
    }
    let found = Directive {
        key: key.clone(),
        values,
        line: tokens[at].line,
    };
    Ok((found, next, odd))
}

/// One argument at `at`: text, a table of text, or either in round brackets; the index after it.
fn argument(
    tokens: &[Lexed],
    at: usize,
    values: &mut Vec<String>,
    odd: &mut Vec<usize>,
) -> Option<usize> {
    // ⚠ The brackets are counted and not recursed into: a manifest may open any number of them, and the stack is not theirs to spend.
    let opened = (tokens[at..].iter())
        .take_while(|t| t.token == Token::Open('('))
        .count();
    let mut inner = Vec::new();
    let mut inner_odd = Vec::new();
    let after = match &tokens.get(at + opened)?.token {
        Token::Text(text) => {
            inner.push(text.clone());
            at + opened + 1
        }
        Token::Open('{') => table(tokens, at + opened, &mut inner, &mut inner_odd),
        _ => return None,
    };
    let closed = tokens.get(after..after + opened)?;
    if closed.iter().any(|t| t.token != Token::Close(')')) {
        return None;
    }
    values.extend(inner);
    odd.extend(inner_odd);
    Some(after + opened)
}

/// The text entries of the table that opens at `at`, and the index after its closing brace or after the last token.
fn table(tokens: &[Lexed], at: usize, values: &mut Vec<String>, odd: &mut Vec<usize>) -> usize {
    let mut next = at + 1;
    loop {
        let Some(lexed) = tokens.get(next) else {
            odd.push(tokens[at].line);
            return next;
        };
        let ends_entry = |t: Option<&Lexed>| {
            t.is_none_or(|t| matches!(t.token, Token::Separator | Token::Close('}')))
        };
        match &lexed.token {
            Token::Close('}') => return next + 1,
            Token::Separator => next += 1,
            Token::Text(text) if ends_entry(tokens.get(next + 1)) => {
                values.push(text.clone());
                next += 1;
            }
            _ => {
                odd.push(lexed.line);
                let mut depth = 0usize;
                while let Some(inside) = tokens.get(next) {
                    match inside.token {
                        Token::Open(_) => depth += 1,
                        Token::Close(_) if depth == 0 => break,
                        Token::Close(_) => depth -= 1,
                        Token::Separator if depth == 0 => break,
                        _ => {}
                    }
                    next += 1;
                }
            }
        }
    }
}

/// Step over what begins at `at` and is no statement this reads, to the end of its line; the index after it.
fn skip(tokens: &[Lexed], at: usize) -> usize {
    let line = tokens[at].line;
    (at..tokens.len())
        .find(|&next| tokens[next].line > line)
        .unwrap_or(tokens.len())
}

fn lex(text: &str) -> Vec<Lexed> {
    let chars: Vec<char> = text.chars().collect();
    let closers = last_closers(&chars);
    let mut tokens = Vec::new();
    let mut line = 1;
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        let started = line;
        let token = match c {
            '\n' => {
                line += 1;
                at += 1;
                continue;
            }
            c if c.is_whitespace() => {
                at += 1;
                continue;
            }
            '-' if chars.get(at + 1) == Some(&'-') => {
                at = match long_bracket(&chars, at + 2, &closers) {
                    Some((_, end)) => {
                        line += newlines(&chars[at..end]);
                        end
                    }
                    None => (at..chars.len())
                        .find(|&i| chars[i] == '\n')
                        .unwrap_or(chars.len()),
                };
                continue;
            }
            '\'' | '"' => match quoted(&chars, at) {
                Some((text, end)) => {
                    at = end;
                    Token::Text(text)
                }
                None => {
                    at += 1;
                    Token::Other
                }
            },
            '[' => match long_bracket(&chars, at, &closers) {
                Some((text, end)) => {
                    line += newlines(&chars[at..end]);
                    at = end;
                    Token::Text(text)
                }
                None => {
                    at += 1;
                    Token::Other
                }
            },
            '{' | '(' => {
                at += 1;
                Token::Open(c)
            }
            '}' | ')' => {
                at += 1;
                Token::Close(c)
            }
            ',' | ';' => {
                at += 1;
                Token::Separator
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let end = (at..chars.len())
                    .find(|&i| !(chars[i].is_ascii_alphanumeric() || chars[i] == '_'))
                    .unwrap_or(chars.len());
                let name: String = chars[at..end].iter().collect();
                at = end;
                Token::Name(name)
            }
            _ => {
                at += 1;
                Token::Other
            }
        };
        tokens.push(Lexed {
            token,
            line: started,
        });
    }
    tokens
}

fn newlines(chars: &[char]) -> usize {
    chars.iter().filter(|c| **c == '\n').count()
}

/// The text between the quote at `at` and the same quote on its line, and the index after it; none when the line ends first.
fn quoted(chars: &[char], at: usize) -> Option<(String, usize)> {
    let quote = chars[at];
    let mut text = String::new();
    let mut next = at + 1;
    loop {
        match *chars.get(next)? {
            '\n' => return None,
            c if c == quote => return Some((text, next + 1)),
            '\\' => {
                let escaped = *chars.get(next + 1)?;
                // A backslash before anything but a quote or itself is kept, as a path of Windows spells it.
                if !matches!(escaped, '\\' | '\'' | '"') {
                    text.push('\\');
                }
                if escaped == '\n' {
                    return None;
                }
                text.push(escaped);
                next += 2;
            }
            c => {
                text.push(c);
                next += 1;
            }
        }
    }
}

/// Where the last `]]`, `]=]` and so on of the text begins, by its count of `=`.
fn last_closers(chars: &[char]) -> BTreeMap<usize, usize> {
    let mut last = BTreeMap::new();
    let mut at = 0;
    while at < chars.len() {
        if chars[at] != ']' {
            at += 1;
            continue;
        }
        let level = chars[at + 1..].iter().take_while(|c| **c == '=').count();
        if chars.get(at + 1 + level) == Some(&']') {
            last.insert(level, at);
        }
        // The `]` that ends one closer may begin the next.
        at += 1 + level;
    }
    last
}

/// The text of the long bracket `[[…]]` or `[=[…]=]` that opens at `at`, and the index after it; none when none opens there or it never closes.
fn long_bracket(
    chars: &[char],
    at: usize,
    closers: &BTreeMap<usize, usize>,
) -> Option<(String, usize)> {
    if chars.get(at) != Some(&'[') {
        return None;
    }
    let level = chars[at + 1..].iter().take_while(|c| **c == '=').count();
    let body = at + 1 + level;
    if chars.get(body) != Some(&'[') {
        return None;
    }
    let body = body + 1;
    // ⚠ Without this a text of brackets that never close is searched to its end once for each of them.
    if closers.get(&level).is_none_or(|last| *last < body) {
        return None;
    }
    let mut close = vec![']'];
    close.extend(std::iter::repeat_n('=', level));
    close.push(']');
    let end = (body..chars.len()).find(|&i| chars[i..].starts_with(&close))?;
    Some((chars[body..end].iter().collect(), end + close.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directive<'a>(manifest: &'a Manifest, key: &str) -> &'a Directive {
        (manifest.directives.iter())
            .find(|d| d.key == key)
            .unwrap_or_else(|| panic!("no `{key}` in {manifest:#?}"))
    }

    #[test]
    fn each_spelling_a_real_manifest_writes_is_read() {
        let text = "\u{feff}-- a map of the harbour\n\
            fx_version 'cerulean'\n\
            game \"gta5\"\n\
            --[[ a comment\n\
            over two lines ]]\n\
            files {\n\
            \x20 'data/a.meta', -- the first\n\
            \x20 \"data/b.meta\";\n\
            \x20 [[data/c.meta]],\n\
            \x20 'data/**/*.xml',\n\
            }\n\
            file 'data/d.meta'\n\
            data_file 'DLC_ITYP_REQUEST' \"stream/props.ytyp\"\n\
            data_file('HANDLING_FILE')('data/handling.meta')\n\
            games { 'gta5', 'rdr3' }\n\
            this_is_a_map 'yes'\n\
            client_scripts({ 'client/a.lua', 'client/b.lua' })\n\
            author 'it\\'s me' description \"a \\\"map\\\"\"\n";
        let manifest = Manifest::read(text);

        assert_eq!(manifest.unread, [], "{manifest:#?}");
        assert_eq!(manifest.version(), Some("cerulean"));
        assert_eq!(manifest.games(), ["gta5", "gta5", "rdr3"]);
        assert!(manifest.is_map());
        let files: Vec<&str> = manifest.files().into_iter().map(|(_, f)| f).collect();
        assert_eq!(
            files,
            [
                "data/a.meta",
                "data/b.meta",
                "data/c.meta",
                "data/**/*.xml",
                "data/d.meta"
            ]
        );
        assert!(is_glob(files[3]) && !is_glob(files[0]));
        assert_eq!(directive(&manifest, "files").line, 6);
        assert_eq!(
            manifest.data_files(),
            [
                DataFile {
                    file_type: "DLC_ITYP_REQUEST",
                    path: "stream/props.ytyp",
                    line: 13
                },
                DataFile {
                    file_type: "HANDLING_FILE",
                    path: "data/handling.meta",
                    line: 14
                },
            ]
        );
        assert_eq!(
            directive(&manifest, "client_scripts").values,
            ["client/a.lua", "client/b.lua"]
        );
        assert_eq!(directive(&manifest, "author").values, ["it's me"]);
        assert_eq!(directive(&manifest, "description").values, ["a \"map\""]);
    }

    #[test]
    fn the_older_version_line_is_a_version_line() {
        let manifest =
            Manifest::read("resource_manifest_version '44febabe-d386-4d18-afbe-5e627f4af937'\n");
        assert_eq!(
            manifest.version(),
            Some("44febabe-d386-4d18-afbe-5e627f4af937")
        );
        assert_eq!(Manifest::read("game 'gta5'\n").version(), None);
    }

    #[test]
    fn what_is_not_a_line_of_a_manifest_is_skipped_by_its_lines_and_refuses_nothing() {
        let text = "local version = 'cerulean'\n\
            fx_version(version)\n\
            local list = {\n\
            \x20 'a.meta',\n\
            \x20 'b.meta',\n\
            }\n\
            game 'gta5'\n\
            for _, name in ipairs(list) do files { 'loop.meta' } end\n\
            if GetConvar('maps', '') == 'on' then\n\
            \x20 this_is_a_map 'yes'\n\
            end\n\
            files { 'kept.meta', prefix .. 'x.meta', ['odd'] = { 'y' }, 'also.meta' }\n\
            ui_page 'html/' .. page\n\
            fx_version 'unclosed\n\
            return { 'returned.meta' }\n\
            files(\n\
            \x20 'first.meta',\n\
            \x20 'second.meta'\n\
            )\n\
            author 'someone'\n\
            files { 'lone.meta' .. suffix, 'after.meta' }\n\
            files { 'open.meta',\n";
        let manifest = Manifest::read(text);

        assert_eq!(
            manifest.unread,
            [(1, 6), (8, 9), (11, 11), (12, 19), (21, 21), (22, 22)],
            "{manifest:#?}"
        );
        assert_eq!(manifest.version(), None, "a computed version is not known");
        assert_eq!(manifest.games(), ["gta5"]);
        assert!(manifest.is_map(), "a line inside a block is still a line");
        let files: Vec<&str> = manifest.files().into_iter().map(|(_, f)| f).collect();
        assert_eq!(
            files,
            ["kept.meta", "also.meta", "after.meta", "open.meta"],
            "an entry that is not text alone is left out, the table's other entries are kept, and nothing is read of a line that begins as something else, of a word of the language or of brackets that hold more than one text"
        );
    }

    /// More brackets than a thread of a test has stack for, were each a call.
    const DEEP: usize = 200_000;

    #[test]
    fn brackets_opened_to_any_depth_cost_no_stack() {
        let open = format!("files{}'a.meta'\ngame 'gta5'\n", "(".repeat(DEEP));
        let manifest = Manifest::read(&open);
        assert_eq!(manifest.unread, [(1, 1)]);
        assert_eq!(manifest.files(), []);
        assert_eq!(manifest.games(), ["gta5"]);

        let closed = format!("files{}'a.meta'{}\n", "(".repeat(DEEP), ")".repeat(DEEP));
        let manifest = Manifest::read(&closed);
        assert_eq!(manifest.unread, []);
        assert_eq!(manifest.files(), [(1, "a.meta")]);
    }

    /// Enough lines that reading the rest of the text again for each of them does not end while a test is waited for.
    const LINES: usize = 100_000;

    #[test]
    fn a_text_that_opens_on_every_line_what_it_never_closes_is_read_once() {
        let comments = "--[[ never closed\n".repeat(LINES);
        assert_eq!(Manifest::read(&comments), Manifest::default());

        let strings = "name [==[ never closed\n".repeat(LINES);
        let manifest = Manifest::read(&strings);
        assert_eq!(manifest.directives, []);
        assert_eq!(manifest.unread, [(1, LINES)]);

        // Each table closes, and is joined to something that makes its statement none this reads.
        let tables = "files {\n".repeat(3 * LINES) + &"} .. tail\n".repeat(3 * LINES);
        let manifest = Manifest::read(&tables);
        assert_eq!(manifest.directives, []);
        assert_eq!(manifest.unread, [(1, 6 * LINES)]);
    }

    #[test]
    fn a_file_that_is_no_manifest_at_all_reads_as_nothing_but_unread_lines() {
        for text in [
            "{ \"fx_version\": \"cerulean\", \"files\": [\"a\"] }\n",
            "<Resource version=\"1\"/>\n",
            "\0\u{1}\u{2}binary",
            "[[never closed",
            "",
        ] {
            let manifest = Manifest::read(text);
            assert_eq!(manifest.directives, [], "{text:?}");
            assert_eq!(manifest.unread.is_empty(), text.is_empty(), "{text:?}");
        }
    }
}
