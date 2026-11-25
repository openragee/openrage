//! A server's configuration read as far as the order it starts resources in: its `ensure` and `start` lines, and the files an `exec` line includes.
//!
//! A configuration is a list of console commands a server runs. This reads three of them and runs nothing, so a resource a script or
//! a person starts later is not known here.
//!
//! ⚠ Nothing is refused but a configuration that cannot be read at all. A line that is none of the three is kept in
//! [`Config::unread`], and an included file that was not read is kept in [`Config::includes_unread`]: the order is the whole order
//! only while that second list is empty ([`Config::is_whole`]).

use std::path::{Component, Path, PathBuf};

use super::{text_of, Files, Scan};
use crate::resource::inside;

/// A line that starts a resource, or every resource of a category in brackets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    /// The resource or the category, as the line names it.
    pub resource: String,
    /// The file the line is in, under the server's folder with `/`, or as `@resource/path` for a file of a resource.
    pub file: String,
    /// The line, from 1.
    pub line: usize,
}

/// A run of lines that are no command this reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lines {
    pub file: String,
    pub first: usize,
    pub last: usize,
}

/// Why an included file was not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotRead {
    /// Its path is absolute or climbs out of the server's folder, and is not followed.
    Escapes,
    /// It is not a file under the server's folder that opens, with the reason.
    Unreadable(String),
}

/// An `exec` line whose file was not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Include {
    /// The path as the line states it.
    pub stated: String,
    pub file: String,
    pub line: usize,
    pub why: NotRead,
}

/// What a configuration starts, and what of it was not read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// The configuration, as it was named.
    pub file: PathBuf,
    /// The files read, the configuration first, each under the server's folder with `/`, or as `@resource/path` for a file of a resource.
    pub read: Vec<String>,
    /// Every line that starts a resource, in the order a server runs them.
    pub starts: Vec<Start>,
    pub unread: Vec<Lines>,
    pub includes_unread: Vec<Include>,
}

/// The configuration itself did not open.
#[derive(Debug, thiserror::Error)]
#[error("the configuration {} cannot be read: {why}", file.display())]
pub struct Unreadable {
    pub file: PathBuf,
    pub why: String,
}

#[derive(Clone)]
enum Command {
    Start(String),
    Include(String),
}

/// One file being run: its commands, and the next of them.
struct Running {
    file: String,
    commands: Vec<(usize, Command)>,
    next: usize,
}

impl Config {
    /// Whether every file the configuration includes was read, without which the order is not the whole order.
    pub fn is_whole(&self) -> bool {
        self.includes_unread.is_empty()
    }

    /// [`Config::read`], with a path that starts with `@` and the name of a resource of `scan` read inside that resource.
    pub fn read_beside(file: &Path, files: &dyn Files, scan: &Scan) -> Result<Self, Unreadable> {
        Self::read_in(file, files, Some(scan))
    }

    /// Read the configuration at `file` and the files it includes, each once; the server's folder is the one that holds `file`.
    pub fn read(file: &Path, files: &dyn Files) -> Result<Self, Unreadable> {
        Self::read_in(file, files, None)
    }

    fn read_in(file: &Path, files: &dyn Files, scan: Option<&Scan>) -> Result<Self, Unreadable> {
        let folder = match file.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = text_of(files, file).map_err(|why| Unreadable {
            file: file.to_path_buf(),
            why,
        })?;
        let mut config = Self {
            file: file.to_path_buf(),
            ..Self::default()
        };
        let first = config.run(name, &text);
        // ⚠ An included file runs where its line stands, and files include files to any depth: they wait on a list and not on the stack.
        let mut running = vec![first];
        while let Some(at) = running.last_mut() {
            let Some((line, command)) = at.commands.get(at.next).cloned() else {
                running.pop();
                continue;
            };
            at.next += 1;
            let file = at.file.clone();
            match command {
                Command::Start(resource) => config.starts.push(Start {
                    resource,
                    file,
                    line,
                }),
                Command::Include(stated) => {
                    match config.include(&folder, &stated, files, scan) {
                        Ok(Some(included)) => running.push(included),
                        // Read before: by the first line that starts a resource, running it again changes no order.
                        Ok(None) => {}
                        Err(why) => config.includes_unread.push(Include {
                            stated,
                            file,
                            line,
                            why,
                        }),
                    }
                }
            }
        }
        Ok(config)
    }

    /// The commands of the file `stated` names, where it is a file under `folder`, or inside a resource of `scan`, that was not read before.
    fn include(
        &mut self,
        folder: &Path,
        stated: &str,
        files: &dyn Files,
        scan: Option<&Scan>,
    ) -> Result<Option<Running>, NotRead> {
        // `@name/path` is a file of the resource `name`, wherever its folder is among the categories.
        let (mut at, stated, of) = match stated.strip_prefix('@') {
            None => (folder.to_path_buf(), stated, String::new()),
            Some(named) => {
                let (resource, rest) = named.split_once(['/', '\\']).unwrap_or((named, ""));
                let at = resource_folder(scan, resource).map_err(NotRead::Unreadable)?;
                (at, rest, format!("@{resource}/"))
            }
        };
        let inside = inside(stated).ok_or(NotRead::Escapes)?;
        let name = format!("{of}{}", inside.to_string_lossy().replace('\\', "/"));
        if self.read.contains(&name) {
            return Ok(None);
        }
        for part in inside.components() {
            let Component::Normal(part) = part else {
                continue;
            };
            at.push(part);
            match std::fs::symlink_metadata(&at) {
                Ok(found) if found.file_type().is_symlink() => {
                    return Err(NotRead::Unreadable(
                        "its path goes through a link, which is not followed".to_string(),
                    ))
                }
                Ok(_) => {}
                Err(why) => return Err(NotRead::Unreadable(why.to_string())),
            }
        }
        if !at.is_file() {
            return Err(NotRead::Unreadable("it is not a file".to_string()));
        }
        let text = text_of(files, &at).map_err(NotRead::Unreadable)?;
        Ok(Some(self.run(name, &text)))
    }

    /// Keep `file` as read and its lines that are no command, and give its commands to run.
    fn run(&mut self, file: String, text: &str) -> Running {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut commands = Vec::new();
        // Whether a command was read since the last unread line, which ends a run of them.
        let mut read_since = true;
        for (index, line) in text.lines().enumerate() {
            let number = index + 1;
            for words in commands_of(line) {
                let command = match words.as_slice() {
                    [verb, name] if is_verb(verb, &["ensure", "start"]) => {
                        Command::Start(name.clone())
                    }
                    [verb, path] if is_verb(verb, &["exec"]) => Command::Include(path.clone()),
                    _ => {
                        match self.unread.last_mut() {
                            Some(run) if !read_since && run.file == file => run.last = number,
                            _ => self.unread.push(Lines {
                                file: file.clone(),
                                first: number,
                                last: number,
                            }),
                        }
                        read_since = false;
                        continue;
                    }
                };
                commands.push((number, command));
                read_since = true;
            }
        }
        self.read.push(file.clone());
        Running {
            file,
            commands,
            next: 0,
        }
    }
}

/// The folder of the one resource of `scan` called `name`, or why there is none to read a file in.
fn resource_folder(scan: Option<&Scan>, name: &str) -> Result<PathBuf, String> {
    let Some(scan) = scan else {
        return Err(
            "it names a file of a resource, and no resources were read beside the configuration"
                .to_string(),
        );
    };
    let mut held = scan.resources.iter().filter(|r| r.name == name);
    match (held.next(), held.count()) {
        (None, _) => Err(format!("no resource of the name `{name}` was read")),
        (Some(only), 0) => Ok(scan.folder.join(&only.path)),
        (Some(_), more) => Err(format!(
            "{} folders have the resource name `{name}`",
            more + 1
        )),
    }
}

fn is_verb(word: &str, verbs: &[&str]) -> bool {
    verbs.iter().any(|verb| word.eq_ignore_ascii_case(verb))
}

/// The commands of one line, each as its words: split at `;`, a word in double quotes kept whole, and a `#` or `//` comment left out.
fn commands_of(line: &str) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word: Option<String> = None;
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' => quoted = false,
                c => word.get_or_insert_with(String::new).push(c),
            }
            continue;
        }
        match c {
            '"' => {
                quoted = true;
                word.get_or_insert_with(String::new);
            }
            ';' => {
                words.extend(word.take());
                if !words.is_empty() {
                    commands.push(std::mem::take(&mut words));
                }
            }
            c if c.is_whitespace() => words.extend(word.take()),
            '#' if word.is_none() => break,
            '/' if word.is_none() && chars.peek() == Some(&'/') => break,
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word.take());
    if !words.is_empty() {
        commands.push(words);
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<Vec<String>> {
        commands_of(line)
    }

    #[test]
    fn a_line_is_split_into_commands_and_words_and_a_comment_is_left_out() {
        assert_eq!(words("ensure harbour"), [["ensure", "harbour"]]);
        assert_eq!(
            words("  ensure \"my maps\" # the maps"),
            [["ensure", "my maps"]]
        );
        assert_eq!(
            words("start a; ensure b ;; // and no more"),
            [["start", "a"], ["ensure", "b"]]
        );
        assert_eq!(
            words("sv_hostname \"a # b; c // d\""),
            [["sv_hostname", "a # b; c // d"]]
        );
        assert_eq!(words("set x \"\""), [["set", "x", ""]]);
        assert_eq!(words("exec a/b#c.cfg"), [["exec", "a/b#c.cfg"]]);
        for nothing in ["", "   ", "# ensure a", "// ensure a", ";"] {
            assert_eq!(words(nothing), Vec::<Vec<String>>::new(), "{nothing:?}");
        }
    }
}
