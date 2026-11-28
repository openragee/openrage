//! `rpf9 tree-install`: every path of an install, merged across its archives in the order the game
//! mounts them, and `rpf9 pack`: one archive printed whole, each nested archive indented under its row.

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::container::rpf::gta6_chunk_table::{cell_offset, decode};
use or_io::container::rpf::gta6_kinds::entry_kinds;
use or_io::container::rpf::{
    Gta6Archive, Gta6Class, Gta6Entry, Gta6EntryExtra, Gta6Mount, GtaViKeys, NameResolver,
};
use or_title::dlc::{SetupContentType, SetupData};
use or_title::gta6_mount::table::Table;
use or_title::gta6_mount::{read_documents_by_table, Document, Registry};

use super::{
    build_archives, counts, load_keys, load_resolver, take_common, usage, Common, CENSUS_MAX_DEPTH,
};

/// The archive that gives a path (its place in [`Tree::archives`]), the path's kind and real size,
/// and how many archives mounted before it also name the path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Given {
    pub(super) archive: usize,
    pub(super) kind: String,
    pub(super) size: Option<u64>,
    pub(super) shadowed: usize,
}

/// An install's paths, each given by the last archive in mount order that names it.
#[derive(Debug, Default)]
pub(super) struct Tree {
    /// The archives, in mount order.
    pub(super) archives: Vec<PathBuf>,
    pub(super) paths: BTreeMap<String, Given>,
    /// Archives and nested tables that could not be opened.
    pub(super) refused: usize,
}

pub(super) fn tree_install(args: &[String]) -> ExitCode {
    let (common, root) = match one_target(args, "a build root") {
        Ok(parsed) => parsed,
        Err(code) => return code,
    };
    let (keys, resolver) = match keys_and_names(&common) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    let root = Path::new(root);
    match do_tree_install(root, &keys, &resolver) {
        Ok(tree) => {
            for line in tree_lines(root, &tree) {
                println!("{line}");
            }
            let shared = tree.paths.values().filter(|g| g.shadowed > 0).count();
            eprintln!(
                "{} path(s) from {} archive(s) in mount order; {shared} named by more than one \
                 archive; {} table(s) could not be opened",
                tree.paths.len(),
                tree.archives.len(),
                tree.refused
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

pub(super) fn pack(args: &[String]) -> ExitCode {
    let (common, archive) = match one_target(args, "an archive") {
        Ok(parsed) => parsed,
        Err(code) => return code,
    };
    let (keys, resolver) = match keys_and_names(&common) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    match pack_lines(Path::new(archive), &keys, &resolver) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The shared options and the one positional a verb takes.
fn one_target<'a>(args: &'a [String], what: &str) -> Result<(Common, &'a str), ExitCode> {
    let mut common = Common::default();
    let mut target = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if take_common(&mut common, arg, &mut it)? {
            continue;
        }
        if arg.starts_with("--") {
            return Err(usage(&format!("unknown option {arg}")));
        }
        if target.replace(arg.as_str()).is_some() {
            return Err(usage(&format!("give only {what}")));
        }
    }
    let target = target.ok_or_else(|| usage(&format!("give {what}")))?;
    Ok((common, target))
}

fn keys_and_names(common: &Common) -> Result<(GtaViKeys, NameResolver), ExitCode> {
    let keys = load_keys(common.keys_path.as_deref(), common.exe_path.as_deref()).map_err(|e| {
        eprintln!("{e}");
        ExitCode::FAILURE
    })?;
    Ok((keys, load_resolver(common.names_dir.as_deref())))
}

// --- tree-install -----------------------------------------------------------

/// The longest entry read for a pack's setup, which is a few lines of XML.
// ponytail: a size bound, so a pack of many gigabytes is not inflated whole to find its setup.
const SETUP_LIMIT: u64 = 64 << 10;

/// The packs the setups in the archives of `archives` under `pack/` state, each read through the
/// archive's table; an archive whose table or documents do not read gives no pack.
pub(super) fn pack_registry(root: &Path, archives: &[PathBuf], keys: &GtaViKeys) -> Registry {
    let packs = archives.iter().filter(|path| path.parent() != Some(root));
    Registry::of(
        packs
            .filter_map(|path| Some((path.clone(), setups_of(path, keys)?)))
            .collect(),
    )
}

fn setups_of(path: &Path, keys: &GtaViKeys) -> Option<Vec<Document>> {
    let read = Table::open(path, keys).and_then(|table| {
        let small = (table.entries().iter())
            .filter(|entry| entry.stated <= SETUP_LIMIT)
            .copied()
            .collect();
        read_documents_by_table(path, &Table::of_entries(small), &|| false)
    });
    match read {
        Ok(found) => Some(
            (found.into_iter())
                .map(|found| found.document)
                .filter(|document| matches!(document, Document::Setup(_)))
                .collect(),
        ),
        Err(e) => {
            eprintln!("  {}: no setup read: {e}", path.display());
            None
        }
    }
}

/// What the game sorts packs by: the setup's type, then its order, then its minorOrder, whatever the
/// type; two setups of equal keys tie, and the game keeps them in the order it registered them.
fn mount_key(setup: &SetupData) -> (u8, i32, i32) {
    // ponytail: the game compares the type enum's numeric value, which was not read; this is the
    // rank the registry gives the types.
    let rank = match setup.content_type {
        SetupContentType::TitleUpdate => 0,
        SetupContentType::LevelPack => 1,
        SetupContentType::CompatPack => 2,
        SetupContentType::PaidPack => 3,
        SetupContentType::Unknown => 4,
    };
    (rank, setup.order, setup.minor_order)
}

/// `archives` in the order the game mounts them: those at `root` first (the base content, mounted at
/// start-up), each by path; then the folders under `pack/` by the [`mount_key`] of the setup each
/// holds, a folder of no setup after every other; packs that tie in the order of `listed`, the pack
/// folders in the order the game's pack lists register them, a folder it does not name after every
/// one it does; then by path, as are the archives of one folder. A folder whose every setup is named
/// as one registered before it (in that order, ignoring case) is dropped, as the game drops a pack
/// of a name it holds.
pub(super) fn mount_order(
    root: &Path,
    mut archives: Vec<PathBuf>,
    registry: &Registry,
    listed: &[PathBuf],
) -> Vec<PathBuf> {
    let key_of = |archive: &Path| {
        (registry.packs.iter())
            .filter(|pack| pack.archive.parent() == archive.parent())
            .map(|pack| mount_key(&pack.setup))
            .min()
    };
    let registered = |archive: &Path| {
        (listed.iter())
            .position(|folder| archive.parent() == Some(folder.as_path()))
            .unwrap_or(usize::MAX)
    };
    // ponytail: the first registered of two packs of one name is kept; the game keeps one by a bit
    // of the pack (+0x1F8) that was not read.
    let mut packs: Vec<_> = registry.packs.iter().collect();
    packs.sort_by_key(|pack| (registered(&pack.archive), pack.archive.clone()));
    let (mut names, mut kept, mut repeated) = (HashSet::new(), HashSet::new(), HashSet::new());
    for pack in packs {
        let name = pack.setup.name_hash.to_ascii_lowercase();
        match name.is_empty() || names.insert(name) {
            true => kept.insert(pack.archive.parent()),
            false => repeated.insert(pack.archive.parent()),
        };
    }
    archives.retain(|archive| {
        let folder = archive.parent();
        folder == Some(root) || kept.contains(&folder) || !repeated.contains(&folder)
    });
    archives.sort_by_cached_key(|archive| {
        if archive.parent() == Some(root) {
            return (false, false, None, 0, archive.clone());
        }
        let key = key_of(archive);
        (
            true,
            key.is_none(),
            key,
            registered(archive),
            archive.clone(),
        )
    });
    archives
}

/// Every path of the install at `root`, a nested archive's entries under its name as a folder, each
/// given by the last archive in [`mount_order`] that names it.
pub(super) fn do_tree_install(
    root: &Path,
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<Tree, String> {
    let found = build_archives(root);
    let registry = pack_registry(root, &found, keys);
    // ponytail: the game's two pack lists were not found in the build, so no folder is listed and
    // packs that tie mount by path.
    let archives = mount_order(root, found, &registry, &[]);
    if archives.is_empty() {
        return Err(format!("no .rpf under {}", root.display()));
    }
    let mut tree = Tree::default();
    for (at, path) in archives.iter().enumerate() {
        let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        match Gta6Mount::open_root(&mut file, keys) {
            Ok(mut mount) => {
                mount.archive.resolve_names(resolver);
                let mut merge = Merge {
                    file: &mut file,
                    keys,
                    resolver,
                    tree: &mut tree,
                    archive: at,
                };
                merge.mount(&mount, "", 0);
            }
            Err(e) => {
                eprintln!("  {}: {e}", path.display());
                tree.refused += 1;
            }
        }
    }
    tree.archives = archives;
    Ok(tree)
}

/// One archive file's paths merged into the tree, through its nested archives.
struct Merge<'a> {
    file: &'a mut File,
    keys: &'a GtaViKeys,
    resolver: &'a NameResolver,
    tree: &'a mut Tree,
    archive: usize,
}

impl Merge<'_> {
    fn mount(&mut self, mount: &Gta6Mount, prefix: &str, depth: usize) {
        let kinds = entry_kinds(&mount.archive);
        for (entry, kind) in mount.archive.entries.iter().zip(kinds) {
            let path = format!("{prefix}{}", entry.display_name().replace('\\', "/"));
            // A nested archive is a folder; one that does not open is listed as the entry it is.
            // ⚠ Only a row whose class says archive is read for its head: reading every row's is
            // past 15 minutes over a build.
            let says_archive =
                entry.class.map(Gta6Class::from_byte) == Some(Gta6Class::NestedArchive);
            let begins = match says_archive {
                true => mount.entry_begins_archive(&mut *self.file, entry),
                false => Ok(false),
            };
            match begins {
                Ok(false) => {}
                Ok(true) if depth < CENSUS_MAX_DEPTH => {
                    match mount.open_nested(&mut *self.file, entry, self.keys) {
                        Ok(mut child) => {
                            child.archive.resolve_names(self.resolver);
                            self.mount(&child, &format!("{path}/"), depth + 1);
                            continue;
                        }
                        Err(_) => self.tree.refused += 1,
                    }
                }
                Ok(true) | Err(_) => self.tree.refused += 1,
            }
            let shadowed = (self.tree.paths.get(&path)).map_or(0, |was| was.shadowed + 1);
            let given = Given {
                archive: self.archive,
                kind: kind.into_owned(),
                size: entry.real,
                shadowed,
            };
            self.tree.paths.insert(path, given);
        }
    }
}

/// One line per path: the path, the archive that gives it (relative to `root`), its kind and real
/// size, and how many earlier archives it is over.
pub(super) fn tree_lines(root: &Path, tree: &Tree) -> Vec<String> {
    (tree.paths.iter())
        .map(|(path, given)| {
            let archive = &tree.archives[given.archive];
            let archive = archive.strip_prefix(root).unwrap_or(archive);
            let size = given
                .size
                .map_or_else(|| "-".to_string(), |s| s.to_string());
            let over = match given.shadowed {
                0 => String::new(),
                n => format!("\tover {n} earlier"),
            };
            format!(
                "{path}\t{}\t{}\t{size}{over}",
                archive.display(),
                given.kind
            )
        })
        .collect()
}

// --- pack -------------------------------------------------------------------

/// The archive at `path` whole: a count line, then one line per row (index, name or hash, kind,
/// stored and real size, cipher, row shape), each nested archive's rows indented under its own.
pub(super) fn pack_lines(
    path: &Path,
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<Vec<String>, String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut mount = Gta6Mount::open_root(&mut file, keys).map_err(|e| e.to_string())?;
    mount.archive.resolve_names(resolver);
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    let mut lines = vec![format!("{name}: {}", counts(&mount))];
    pack_rows(&mut file, &mount, keys, resolver, 1, &mut lines);
    Ok(lines)
}

fn pack_rows(
    file: &mut File,
    mount: &Gta6Mount,
    keys: &GtaViKeys,
    resolver: &NameResolver,
    depth: usize,
    lines: &mut Vec<String>,
) {
    let indent = "  ".repeat(depth);
    let size = |s: Option<u64>| s.map_or_else(|| "-".to_string(), |s| s.to_string());
    let extras = mount.archive.entry_extras();
    let kinds = entry_kinds(&mount.archive);
    for (at, (entry, kind)) in mount.archive.entries.iter().zip(&kinds).enumerate() {
        let cipher = (entry.cipher.as_ref()).map_or_else(
            || "clear".to_string(),
            |c| format!("cipher type {} key {:#010x}", c.cipher_type, c.key_id),
        );
        lines.push(format!(
            "{indent}{:>6}  {}  {kind}  stored {}  real {}  {cipher}  {}",
            entry.index,
            entry.display_name(),
            size(entry.stored),
            size(entry.real),
            row_shape(&mount.archive, extras.get(at), entry, at),
        ));
        match mount.entry_begins_archive(&mut *file, entry) {
            Ok(false) => {}
            Ok(true) if depth > CENSUS_MAX_DEPTH => lines.push(format!(
                "{indent}  (nested past depth {CENSUS_MAX_DEPTH}; not opened)"
            )),
            Ok(true) => match mount.open_nested(&mut *file, entry, keys) {
                Ok(mut child) => {
                    child.archive.resolve_names(resolver);
                    lines.push(format!(
                        "{indent}  {}: {}",
                        entry.display_name(),
                        counts(&child)
                    ));
                    pack_rows(file, &child, keys, resolver, depth + 1, lines);
                }
                Err(e) => lines.push(format!("{indent}  nested archive refused: {e}")),
            },
            Err(e) => lines.push(format!("{indent}  body not probed: {e}")),
        }
    }
}

/// A row's shape: a 24-byte row by its class byte, a 20-byte row by its type code, or none, and the
/// mode and chunk count of its chunk table where it has one.
fn row_shape(
    archive: &Gta6Archive,
    extra: Option<&Gta6EntryExtra>,
    entry: &Gta6Entry,
    at: usize,
) -> String {
    let row = match (entry.class, extra.and_then(|x| x.nested)) {
        (Some(class), _) => format!("row24 class {class:#04x}"),
        (None, Some(row)) => format!("row20 type {:#04x}", row.resource_type),
        (None, None) => "no row".to_string(),
    };
    let Some(heap) = cell_offset(archive, at).map(|cell| &archive.toc.bytes[cell..]) else {
        return row;
    };
    let Some(&cell) = heap.first_chunk::<8>() else {
        return row;
    };
    match decode(cell, heap) {
        Ok(table) => format!(
            "{row}, chunk table mode {} of {} chunk(s)",
            table.mode,
            table.chunks.len()
        ),
        Err(e) => format!("{row}, chunk table refused: {e}"),
    }
}

#[cfg(test)]
#[path = "rpf9_install_tests.rs"]
mod tests;
