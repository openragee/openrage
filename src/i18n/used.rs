//! Reads the source for the texts it asks to be translated: every `tr!`, `tr_n!` and `tr_key!` with the literal it holds,
//! and every `sentence!` of the world core and of a world adapter crate, whose key the app shows through `tr`.

use std::path::{Path, PathBuf};

/// One call of a translation macro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use {
    pub file: PathBuf,
    pub line: usize,
    pub macro_name: String,
    /// The English text, which is the key.
    pub text: String,
    /// The text of the other number, for `tr_n!`.
    pub other: Option<String>,
    /// Whether the call passes arguments to put in the text, which `tr!` replaces `{name}` with.
    pub has_args: bool,
}

/// Every `.rs` file under `folder`, in path order.
pub fn rust_files(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("a folder lists").flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// The folders of the workspace at `root` whose source asks for texts: the app's, the world core's, and each world adapter crate's.
pub fn source_folders(root: &Path) -> Vec<PathBuf> {
    let domain = root.join("crates").join("domain");
    let mut adapters: Vec<PathBuf> = std::fs::read_dir(&domain)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("or-world-"))
        .map(|entry| entry.path().join("src"))
        .filter(|src| src.is_dir())
        .collect();
    adapters.sort();
    let mut folders = vec![root.join("src"), domain.join("or-world").join("src")];
    folders.extend(adapters);
    folders
}

/// Every call in the source under `folders`.
pub fn in_the_folders(folders: &[PathBuf]) -> Vec<Use> {
    folders
        .iter()
        .flat_map(|folder| rust_files(folder))
        .flat_map(|file| {
            let text = std::fs::read_to_string(&file).expect("a file reads");
            in_text(&file, &text)
        })
        .collect()
}

/// Every call in the source of the app, of the world core and of the world adapter crates.
pub fn in_the_source() -> Vec<Use> {
    in_the_folders(&source_folders(Path::new(env!("CARGO_MANIFEST_DIR"))))
}

struct Scan<'a> {
    chars: Vec<char>,
    at: usize,
    file: &'a Path,
}

impl Scan<'_> {
    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.at + ahead).copied()
    }

    fn line(&self) -> usize {
        1 + self.chars[..self.at.min(self.chars.len())]
            .iter()
            .filter(|c| **c == '\n')
            .count()
    }

    /// Past white space and comments.
    fn skip_space(&mut self) {
        loop {
            match (self.peek(0), self.peek(1)) {
                (Some(c), _) if c.is_whitespace() => self.at += 1,
                (Some('/'), Some('/')) => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.at += 1;
                    }
                }
                (Some('/'), Some('*')) => self.skip_block_comment(),
                _ => return,
            }
        }
    }

    fn skip_block_comment(&mut self) {
        let mut depth = 0;
        while self.at < self.chars.len() {
            match (self.peek(0), self.peek(1)) {
                (Some('/'), Some('*')) => {
                    depth += 1;
                    self.at += 2;
                }
                (Some('*'), Some('/')) => {
                    depth -= 1;
                    self.at += 2;
                    if depth == 0 {
                        return;
                    }
                }
                _ => self.at += 1,
            }
        }
    }

    /// The string literal at the cursor, read as the compiler reads it, and the cursor past it.
    fn string(&mut self) -> Option<String> {
        let start = self.at;
        let raw = self.peek(0) == Some('r') && matches!(self.peek(1), Some('"' | '#'));
        if raw {
            self.at += 1;
            let mut hashes = 0;
            while self.peek(0) == Some('#') {
                hashes += 1;
                self.at += 1;
            }
            if self.peek(0) != Some('"') {
                self.at = start;
                return None;
            }
            self.at += 1;
            let mut out = String::new();
            while let Some(c) = self.peek(0) {
                if c == '"' && (1..=hashes).all(|i| self.peek(i) == Some('#')) {
                    self.at += 1 + hashes;
                    return Some(out);
                }
                out.push(c);
                self.at += 1;
            }
            return None;
        }
        if self.peek(0) != Some('"') {
            return None;
        }
        self.at += 1;
        let mut out = String::new();
        while let Some(c) = self.peek(0) {
            self.at += 1;
            match c {
                '"' => return Some(out),
                '\\' => {
                    let escape = self.peek(0)?;
                    self.at += 1;
                    match escape {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        '0' => out.push('\0'),
                        '\\' | '"' | '\'' => out.push(escape),
                        'x' => {
                            let hex: String = (0..2).filter_map(|i| self.peek(i)).collect();
                            out.push(char::from(u8::from_str_radix(&hex, 16).ok()?));
                            self.at += 2;
                        }
                        'u' => {
                            let close = (0..16).find(|i| self.peek(*i) == Some('}'))?;
                            let hex: String = (1..close).filter_map(|i| self.peek(i)).collect();
                            out.push(char::from_u32(
                                u32::from_str_radix(&hex.replace('_', ""), 16).ok()?,
                            )?);
                            self.at += close + 1;
                        }
                        '\n' | '\r' => {
                            while self.peek(0).is_some_and(char::is_whitespace) {
                                self.at += 1;
                            }
                        }
                        _ => return None,
                    }
                }
                _ => out.push(c),
            }
        }
        None
    }

    /// Past a character literal, or a lifetime's tick.
    fn skip_tick(&mut self) {
        if self.peek(1) == Some('\\') {
            self.at += 2;
            while self.peek(0).is_some_and(|c| c != '\'') {
                self.at += 1;
            }
            self.at += 1;
        } else if self.peek(2) == Some('\'') {
            self.at += 3;
        } else {
            self.at += 1;
        }
    }

    fn ident(&mut self) -> String {
        let mut name = String::new();
        while let Some(c) = self.peek(0).filter(|c| c.is_alphanumeric() || *c == '_') {
            name.push(c);
            self.at += 1;
        }
        name
    }

    /// The literals of the call whose `(` the cursor stands past, and whether arguments follow them.
    fn call(&mut self, name: &str) -> Option<(String, Option<String>, bool)> {
        self.skip_space();
        let text = self.string()?;
        self.skip_space();
        let mut other = None;
        if name == "tr_n" {
            if self.peek(0)? != ',' {
                return None;
            }
            self.at += 1;
            self.skip_space();
            other = Some(self.string()?);
            self.skip_space();
        }
        let mut has_args = false;
        if self.peek(0) == Some(',') {
            self.at += 1;
            self.skip_space();
            has_args = self.peek(0) != Some(')');
        }
        Some((text, other, has_args))
    }
}

/// The calls in `source`, whose path is `file`.
pub fn in_text(file: &Path, source: &str) -> Vec<Use> {
    let mut scan = Scan {
        chars: source.chars().collect(),
        at: 0,
        file,
    };
    let mut found = Vec::new();
    while let Some(c) = scan.peek(0) {
        match (c, scan.peek(1)) {
            ('/', Some('/')) => {
                while scan.peek(0).is_some_and(|c| c != '\n') {
                    scan.at += 1;
                }
            }
            ('/', Some('*')) => scan.skip_block_comment(),
            ('"', _) => {
                if scan.string().is_none() {
                    scan.at += 1;
                }
            }
            ('r', Some('"' | '#')) if scan.at == 0 || !is_ident(scan.chars[scan.at - 1]) => {
                if scan.string().is_none() {
                    scan.at += 1;
                }
            }
            ('\'', _) => scan.skip_tick(),
            (c, _) if c.is_alphabetic() || c == '_' => {
                let preceded = scan.at > 0
                    && (is_ident(scan.chars[scan.at - 1]) || scan.chars[scan.at - 1] == '.');
                let began = scan.at;
                let name = scan.ident();
                if !preceded
                    && matches!(name.as_str(), "tr" | "tr_n" | "tr_key" | "sentence")
                    && scan.peek(0) == Some('!')
                    && scan.peek(1) == Some('(')
                {
                    scan.at += 2;
                    let mark = scan.at;
                    let line = {
                        let after = scan.at;
                        scan.at = began;
                        let line = scan.line();
                        scan.at = after;
                        line
                    };
                    if let Some((text, other, has_args)) = scan.call(&name) {
                        found.push(Use {
                            file: scan.file.to_path_buf(),
                            line,
                            macro_name: name,
                            text,
                            other,
                            has_args,
                        });
                    }
                    // The arguments are read again for the calls inside them.
                    scan.at = mark;
                }
            }
            _ => scan.at += 1,
        }
    }
    found
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(source: &str) -> Vec<(String, Option<String>, bool)> {
        in_text(Path::new("x.rs"), source)
            .into_iter()
            .map(|u| (u.text, u.other, u.has_args))
            .collect()
    }

    #[test]
    fn a_call_is_read_as_the_compiler_reads_its_literal() {
        let source = r##"
            // tr!("in a comment")
            /* tr!("in a block /* nested */") */
            let a = tr!("Open");
            let b = tr!("Saved {name}\u{2026}\n\
                         on {when}", name = x, when = y);
            let c = tr_n!("{count} file", "{count} files", n);
            let d = tr_key!(r#"A "raw" one"#);
            let e = "tr!(\"in a string\")";
            let f = format!("{}", tr!("Nested {n}", n = 1));
            let g = other_tr!("not ours");
            let h = x.tr!("not ours");
            let i = 'a'; let j = '\''; fn k<'a>(_: &'a str) {}
            let l = tr!("After the ticks",);
        "##;
        assert_eq!(
            texts(source),
            [
                ("Open".into(), None, false),
                ("Saved {name}\u{2026}\non {when}".into(), None, true),
                ("{count} file".into(), Some("{count} files".into()), true),
                ("A \"raw\" one".into(), None, false),
                ("Nested {n}".into(), None, true),
                ("After the ticks".into(), None, false),
            ]
        );
    }

    #[test]
    fn a_sentence_key_of_a_world_crate_is_read_as_a_call() {
        let source = "const A: Sentence = or_world::sentence!(\"Lights\");\nlet b = my_sentence!(\"not ours\");";
        assert_eq!(texts(source), [("Lights".into(), None, false)]);
        assert_eq!(in_text(Path::new("x.rs"), source)[0].macro_name, "sentence");
    }

    #[test]
    fn a_call_is_found_on_its_line() {
        let found = in_text(Path::new("x.rs"), "\n\nlet a = tr!(\"Open\");");
        assert_eq!(found[0].line, 3);
    }
}
