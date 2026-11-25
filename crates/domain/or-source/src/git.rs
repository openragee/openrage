//! The managed blocks of a project's `.gitignore` and `.gitattributes`: game content stays out of a repository, and source binaries are marked binary.
//!
//! ⚠ OpenRage rewrites only the lines between the two markers. Every other line of the file is a
//! person's and is kept as it is, with the line ends the file already uses.

use std::io;
use std::path::Path;

use crate::asset::{parse_folder_name, AssetManifest, MANIFEST};
use crate::game_import::SOURCES_DIR;

pub const BEGIN: &str =
    "# >>> openrage managed block: OpenRage rewrites the lines between the markers";
pub const END: &str = "# <<< openrage managed block";
/// The two files the blocks are written in.
pub const IGNORE: &str = ".gitignore";
pub const ATTRIBUTES: &str = ".gitattributes";
/// The manifests of the game a maps content keeps whole in its `manifests` folder, wherever the content lies: game content with no
/// `asset.json` to say so, which a repository ignores unless it holds the sources taken from the game.
pub const KEPT_MANIFESTS: &str = "**/manifests/*.ymf";
/// Files of a source that Git must not diff, merge or convert, whatever they hold.
const BINARY: &[&str] = &[
    "*.dds",
    "*.glb",
    "base.ydr",
    "base.ydd",
    "base.yft",
    "base.ymt",
    "base.ymap",
    "base.ytyp",
    "base.meta",
    "base.ybn",
    "base.ynd",
];
/// Images beside a DDS: binary, or Git LFS when the person asked for it.
const IMAGES: &[&str] = &[
    "*.png", "*.tga", "*.tif", "*.tiff", "*.bmp", "*.jpg", "*.jpeg", "*.psd", "*.exr", "*.hdr",
];
const LFS: &str = "filter=lfs diff=lfs merge=lfs -text";

/// What a repository of the project holds: the sources, the build, and the sources taken from the game's files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Commit {
    /// The `.src` folders of `sources/`, which a repository holds by default.
    pub sources: bool,
    /// The `build/` folder, which a repository ignores by default.
    pub build: bool,
    /// The sources whose `asset.json` says they are the game's, which a repository ignores by default.
    pub game_sources: bool,
}

impl Default for Commit {
    fn default() -> Self {
        Self {
            sources: true,
            build: false,
            game_sources: false,
        }
    }
}

/// The line of the ignore block that states a choice other than the default, in words: `# commit: sources game`.
const CHOICE: &str = "# commit:";

impl Commit {
    fn line(self) -> Option<String> {
        if self == Self::default() {
            return None;
        }
        let words: Vec<&str> = [
            (self.sources, "sources"),
            (self.build, "build"),
            (self.game_sources, "game"),
        ]
        .into_iter()
        .filter_map(|(on, word)| on.then_some(word))
        .collect();
        let words = if words.is_empty() {
            "nothing".to_string()
        } else {
            words.join(" ")
        };
        Some(format!("{CHOICE} {words}"))
    }

    fn parse(text: &str) -> Self {
        let Some(words) = (text.lines()).find_map(|line| line.trim_end().strip_prefix(CHOICE))
        else {
            return Self::default();
        };
        let has = |word: &str| words.split_whitespace().any(|w| w == word);
        Self {
            sources: has("sources"),
            build: has("build"),
            game_sources: has("game"),
        }
    }
}

/// What the managed block of `root` says the repository holds; the default where it states nothing.
pub fn commit_of(root: &Path) -> Commit {
    let text = read(&root.join(IGNORE)).unwrap_or_default();
    match block_of(&text) {
        Ok(Some((begin, end))) => Commit::parse(
            &text
                .split_inclusive('\n')
                .skip(begin + 1)
                .take(end - begin - 1)
                .collect::<String>(),
        ),
        _ => Commit::default(),
    }
}

/// Rewrites both blocks, with Git LFS for the images when `lfs`, and what the block already says the repository holds; true when a file changed.
///
/// ⚠ Both files are read and judged before either is written, so a file that cannot be rewritten stops the other too.
pub fn write(root: &Path, lfs: bool) -> io::Result<bool> {
    write_with(root, lfs, commit_of(root))
}

/// [`write`] with `commit` in place of what the block says.
pub fn write_with(root: &Path, lfs: bool, commit: Commit) -> io::Result<bool> {
    let ignore = rewrite(root, IGNORE, &ignore_block(root, commit))?;
    let attributes = rewrite(root, ATTRIBUTES, &attribute_block(lfs))?;
    let mut changed = false;
    for (name, text) in [(IGNORE, ignore), (ATTRIBUTES, attributes)] {
        if let Some(text) = text {
            std::fs::write(root.join(name), text)?;
            changed = true;
        }
    }
    Ok(changed)
}

/// Whether the attributes block sends the images through Git LFS, which the person turned on.
fn lfs_of(root: &Path) -> bool {
    let text = read(&root.join(ATTRIBUTES)).unwrap_or_default();
    block_of(&text).ok().flatten().is_some_and(|(begin, end)| {
        text.split_inclusive('\n')
            .skip(begin + 1)
            .take(end - begin - 1)
            .any(|line| line.contains("filter=lfs"))
    })
}

/// Rewrites both blocks as they are now, keeping the LFS the person turned on in the attributes block.
pub fn update(root: &Path) -> io::Result<bool> {
    read(&root.join(ATTRIBUTES))?;
    write(root, lfs_of(root))
}

/// Rewrites both blocks for what the owner chose to commit, keeping the LFS the person turned on; true when a file changed.
pub fn choose(root: &Path, commit: Commit) -> io::Result<bool> {
    write_with(root, lfs_of(root), commit)
}

/// `name` with its block replaced by `block`, or `None` when it already reads so.
fn rewrite(root: &Path, name: &str, block: &[String]) -> io::Result<Option<String>> {
    let old = read(&root.join(name))?;
    let new = replace_block(&old, block)
        .map_err(|why| io::Error::new(io::ErrorKind::InvalidData, format!("{name}: {why}")))?;
    Ok((new != old).then_some(new))
}

fn read(path: &Path) -> io::Result<String> {
    match std::fs::read_to_string(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        other => other.map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display()))),
    }
}

/// The indexes of the begin and the end marker line of `text`, none when it holds no block.
fn block_of(text: &str) -> Result<Option<(usize, usize)>, String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let is = |line: &str, marker: &str| line.trim_end_matches(['\r', '\n']) == marker;
    let Some(begin) = lines.iter().position(|l| is(l, BEGIN)) else {
        return Ok(None);
    };
    match lines[begin..].iter().position(|l| is(l, END)) {
        Some(end) => Ok(Some((begin, begin + end))),
        None => Err(format!(
            "has the line \"{BEGIN}\" and no line \"{END}\" after it, so the lines to rewrite are not known; \
             put the end line back or delete the block"
        )),
    }
}

/// `text` with the lines of `block` (markers included) in place of the block it holds, or after its last line when it holds none.
fn replace_block(text: &str, block: &[String]) -> Result<String, String> {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let body: String = block.iter().map(|l| format!("{l}{eol}")).collect();
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    Ok(match block_of(text)? {
        Some((begin, end)) => format!(
            "{}{body}{}",
            lines[..begin].concat(),
            lines[end + 1..].concat()
        ),
        None if text.is_empty() => body,
        None if text.ends_with('\n') => format!("{text}{eol}{body}"),
        None => format!("{text}{eol}{eol}{body}"),
    })
}

fn ignore_block(root: &Path, commit: Commit) -> Vec<String> {
    let mut lines = vec![BEGIN.to_string()];
    lines.extend(commit.line());
    if !commit.build {
        lines.push("/build/".to_string());
    }
    lines.push("/vfs/".to_string());
    if !commit.game_sources {
        lines.push(KEPT_MANIFESTS.to_string());
    }
    if !commit.sources {
        lines.push(format!("/{SOURCES_DIR}/"));
    } else if !commit.game_sources {
        lines.extend(
            from_game_sources(root)
                .iter()
                .map(|folder| format!("/{SOURCES_DIR}/{}/", escape(folder))),
        );
    }
    lines.push(END.to_string());
    lines
}

fn attribute_block(lfs: bool) -> Vec<String> {
    let mut lines = vec![BEGIN.to_string()];
    lines.extend(BINARY.iter().map(|p| format!("{p} binary")));
    lines.extend(IMAGES.iter().map(|p| match lfs {
        true => format!("{p} {LFS}"),
        false => format!("{p} binary"),
    }));
    lines.push(END.to_string());
    lines
}

/// The folders of `sources/` whose `asset.json` says the content is the game's, in name order.
fn from_game_sources(root: &Path) -> Vec<String> {
    let mut folders: Vec<String> = std::fs::read_dir(root.join(SOURCES_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            parse_folder_name(&name)?;
            let text = std::fs::read_to_string(entry.path().join(MANIFEST)).ok()?;
            AssetManifest::parse(&text)
                .ok()?
                .origin
                .from_game
                .then_some(name)
        })
        .collect();
    folders.sort();
    folders
}

/// `name` as a literal in an ignore file.
fn escape(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if "\\*?[]".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
