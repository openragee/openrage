//! `openrage-cli navmesh disable|delete` over a scanned or loose GTA VI cell: one polygon proper
//! cut off or deleted as the editor does it, the cell written as a loose resource that
//! `rpf scan-open` reads at offset 0, and each `--neighbour` body whose special links it renumbered
//! written beside it as `<offset>.rsc`.

use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::path::Path;
use std::process::ExitCode;

use or_io::container::rpf::{
    navigation_cells, read_rpf9_body, write_rpf9_resource, Gta6Mount, GtaViKeys, Rpf9BodyKind,
    Rpf9Role, Rpf9Sections,
};
use or_io::gta6::headerless::{
    cut_edges, delete_polygon, deletion, disable_polygon, disabling, links_into, polygon_cut,
    read_navmesh, relinked, write_links, Border, NavMesh, NavRebuild, NOWHERE,
};

const USAGE: &str = "navmesh disable|delete <archive.rpf|file> <offset> <polygon> <out> \
                     [--neighbour <offset>]... [--limit <bytes>]";

pub(crate) fn run(verb: &str, args: &[String]) -> ExitCode {
    let (limit, neighbours, path, offset, polygon, out) = match parse(args) {
        Ok(parsed) => parsed,
        Err(why) => return super::usage_error(&why),
    };
    if super::navmesh_edit::same_file(&path, &out) {
        return super::usage_error(&format!(
            "the output is the input {path}: name another file"
        ));
    }
    // With no key the table is not read, and every neighbouring cell may hold a body.
    let keys = super::gta6_keys::load(None, None).ok();
    let edited = edit_body(
        verb,
        &path,
        offset,
        polygon,
        &neighbours,
        limit,
        keys.as_ref(),
    );
    let (saved, lines, beside) = match edited {
        Ok(edited) => edited,
        Err(why) => {
            eprintln!("Error: {path} at {offset:#X}: {why}");
            return ExitCode::FAILURE;
        }
    };
    let mut files = vec![(out.clone(), saved)];
    for (at, bytes) in beside {
        let to = Path::new(&out).with_file_name(format!("{at:#X}.rsc"));
        let to = to.to_string_lossy().into_owned();
        let same = super::navmesh_edit::same_file;
        if same(&path, &to) || same(&out, &to) {
            eprintln!(
                "Error: the neighbouring body at {at:#X} would be written over {to}: name another output"
            );
            return ExitCode::FAILURE;
        }
        files.push((to, bytes));
    }
    for line in lines {
        println!("{line}");
    }
    if let Err(why) = write_all(&files) {
        eprintln!("Error: {why}");
        return ExitCode::FAILURE;
    }
    for (to, bytes) in &files {
        println!("wrote {to} ({} bytes)", bytes.len());
    }
    println!("This checks the cells' bytes. It does not test that GTA VI loads them.");
    ExitCode::SUCCESS
}

/// Each of `files` written, every one first beside its place, so a write that fails changes none:
/// a cell renumbered alone names polygons its neighbours' links do not.
fn write_all(files: &[(String, Vec<u8>)]) -> Result<(), String> {
    let part = |to: &str| format!("{to}.part");
    for (i, (to, bytes)) in files.iter().enumerate() {
        if let Err(e) = std::fs::write(part(to), bytes) {
            for (written, _) in &files[..i] {
                let _ = std::fs::remove_file(part(written));
            }
            return Err(format!("write {to}: {e}; no file was changed"));
        }
    }
    for (i, (to, _)) in files.iter().enumerate() {
        if let Err(e) = std::fs::rename(part(to), to) {
            let done: Vec<&str> = files[..i].iter().map(|(to, _)| to.as_str()).collect();
            return Err(format!(
                "write {to}: {e}; [{}] hold the edit, and the rest are left as .part files",
                done.join(", ")
            ));
        }
    }
    Ok(())
}

/// The cell written, what the edit did, and each neighbouring body it changed by its offset.
type Edited = (Vec<u8>, Vec<String>, Vec<(u64, Vec<u8>)>);

type Parsed = (u64, Vec<u64>, String, u64, u64, String);

/// The `--limit`, every `--neighbour` offset, the path, the offset, the polygon and the output.
fn parse(args: &[String]) -> Result<Parsed, String> {
    let (limit, mut rest) = super::rpf::split_limit(args)?;
    let mut neighbours = Vec::new();
    while let (Some(word), next) = super::rpf::split_value(&rest, "--neighbour")? {
        neighbours.push(super::rpf::parse_offset(&word)?);
        rest = next;
    }
    let [path, offset, polygon, out] = rest.as_slice() else {
        return Err(USAGE.to_string());
    };
    let at = super::rpf::parse_offset(offset)?;
    let Ok(number) = polygon.parse::<u64>() else {
        return Err(format!("'{polygon}' is not a polygon number"));
    };
    Ok((limit, neighbours, path.clone(), at, number, out.clone()))
}

/// The cell body at `offset` of the file at `path` and its sections.
fn cell_at(file: &File, offset: u64, limit: u64) -> Result<(Rpf9Sections, NavMesh), String> {
    let (find, bytes) = read_rpf9_body(file, offset, limit).map_err(|e| e.to_string())?;
    let role = match find.kind {
        Rpf9BodyKind::Zlib(run) => run.signature.and_then(|s| s.role()),
        _ => None,
    };
    if role != Some(Rpf9Role::NavMesh) {
        return Err(format!(
            "the body at {offset:#X} is no GTA VI navigation cell"
        ));
    }
    let sections = Rpf9Sections::of(&find, &bytes)
        .ok_or_else(|| format!("the body at {offset:#X} is not a resource"))?;
    let mesh = read_navmesh(&sections.virtual_section).map_err(|e| e.to_string())?;
    Ok((sections, mesh))
}

/// Indices as a person reads a list of them.
fn listed(items: impl IntoIterator<Item = impl ToString>) -> String {
    let items: Vec<String> = items.into_iter().map(|i| i.to_string()).collect();
    items.join(", ")
}

/// The loose resource the edit makes of the cell at `offset`, read back, what it did, and each
/// neighbouring body whose special links it renumbered, as a loose resource.
fn edit_body(
    verb: &str,
    path: &str,
    offset: u64,
    polygon: u64,
    neighbours: &[u64],
    limit: u64,
    keys: Option<&GtaViKeys>,
) -> Result<Edited, String> {
    let file = File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let (mut sections, cell) = cell_at(&file, offset, limit)?;
    let polygon = usize::try_from(polygon).map_err(|_| format!("no polygon {polygon}"))?;
    let (mut near, mut stored) = (Vec::new(), Vec::new());
    for &at in neighbours {
        let (s, body) = cell_at(&file, at, limit).map_err(|e| format!("--neighbour: {e}"))?;
        if !across(&cell).any(|c| c == body.cell) {
            return Err(format!(
                "--neighbour {at:#X} is cell {}, which is not across a border or at a corner of cell {}",
                body.cell, cell.cell
            ));
        }
        near.push(body);
        stored.push((at, s));
    }
    let bodied = |missing: Vec<u32>| Ok::<_, String>(with_bodies(&file, missing, keys));
    let (mut lines, made) = match verb {
        "disable" => disable(&mut sections, &cell, polygon, &near, &bodied)?,
        _ => delete(&mut sections, &cell, polygon, &near, &bodied)?,
    };
    let saved = loose(&sections)?;
    let mut beside = Vec::new();
    if let Some(made) = made {
        for ((at, mut s), body) in stored.into_iter().zip(&near) {
            let renumbered = relinked(body, &cell, &made).map_err(|e| e.to_string())?;
            let Some(links) = renumbered else { continue };
            write_links(&mut s, links).map_err(|e| format!("--neighbour {at:#X}: {e}"))?;
            let said = format!(
                "special links of the neighbouring cell {} renumbered",
                body.cell
            );
            lines.push(said);
            beside.push((at, loose(&s)?));
        }
    }
    Ok((saved, lines, beside))
}

/// `s` as a loose resource, once it reads back as a cell.
fn loose(s: &Rpf9Sections) -> Result<Vec<u8>, String> {
    let saved = write_rpf9_resource(s).map_err(|e| e.to_string())?;
    let (find, bytes) = read_rpf9_body(Cursor::new(&saved), 0, u64::MAX)
        .map_err(|e| format!("the written resource does not read back: {e}"))?;
    let back = Rpf9Sections::of(&find, &bytes).ok_or("the written resource does not read back")?;
    read_navmesh(&back.virtual_section)
        .map_err(|e| format!("the written cell does not read back: {e}"))?;
    Ok(saved)
}

/// What an edit said, and the cell it made again where it removed a record.
type Said = (Vec<String>, Option<NavRebuild>);

/// Which of the cells it is given the input holds a body of.
type Bodied<'a> = &'a dyn Fn(Vec<u32>) -> Result<Vec<u32>, String>;

/// The cells of `missing` the table of the archive `file` may hold a navigation row of, ciphered
/// rows and nested archives among them, as no special link names a cell with no body; every one
/// where no key opens a table or `file` is no archive. A byte scan sees no ciphered body, so none
/// is made.
fn with_bodies(file: &File, missing: Vec<u32>, keys: Option<&GtaViKeys>) -> Vec<u32> {
    let mut f = file;
    let Some(keys) = keys.filter(|_| !missing.is_empty()) else {
        return missing;
    };
    let Ok(mount) = Gta6Mount::open_root(&mut f, keys) else {
        return missing;
    };
    held_of(&mut f, &mount, keys, missing)
}

/// The cells of `missing` the table `mount` holds a navigation row of; every one where a row's
/// cell does not read, as it may be any of them.
fn held_of<R: Read + Seek>(
    src: &mut R,
    mount: &Gta6Mount,
    keys: &GtaViKeys,
    missing: Vec<u32>,
) -> Vec<u32> {
    match navigation_cells(src, mount, keys) {
        Ok(cells) => missing.into_iter().filter(|c| cells.contains(c)).collect(),
        Err(e) => {
            tracing::warn!(
                "whether the archive holds the neighbouring cells {} is not known: {e}",
                listed(&missing)
            );
            missing
        }
    }
}

/// That the archive's table holds no navigation row of the cells `none`, where there are any.
fn bodiless_line(none: &[u32]) -> Option<String> {
    (!none.is_empty()).then(|| {
        format!(
            "the archive's table holds no navigation row of the neighbouring cells {}, so no special link of theirs ends in this cell",
            listed(none)
        )
    })
}

/// Polygon proper `polygon` cut off as the editor cuts it: every edge into or out of it that names a
/// polygon of this cell written as leading nowhere, and the side records naming it chained again.
fn disable(
    sections: &mut Rpf9Sections,
    cell: &NavMesh,
    polygon: usize,
    near: &[NavMesh],
    bodied: Bodied<'_>,
) -> Result<Said, String> {
    let cut = polygon_cut(cell, polygon).map_err(|e| e.to_string())?;
    let others: Vec<&NavMesh> = near.iter().collect();
    // A body of no cell has no side record to chain again.
    let made = (cell.cell != 0)
        .then(|| disabling(cell, polygon, &others))
        .transpose()
        .map_err(|e| e.to_string())?;
    let reaching = |m: &NavMesh| m.edges.iter().filter(|e| e.mesh == m.cell).count();
    // A side record that drops a corner moves the arrays after it.
    let rechained = made.filter(|m| m.mesh.indices.len() != cell.indices.len());
    let mut removed = None;
    // Whether the archive was found to hold no body of each cell not given.
    let mut bodiless = false;
    let cuts = if let Some(made) = rechained {
        let removes = made.records.len() < cell.polygons.len();
        if removes {
            let given: Vec<u32> = near.iter().map(|b| b.cell).collect();
            let missing: Vec<u32> = across(cell).filter(|c| !given.contains(c)).collect();
            let held = bodied(missing)?;
            if !held.is_empty() {
                return Err(format!(
                    "polygon {polygon} is not disabled while the bodies of the neighbouring cells {} are not given with --neighbour <offset>: a side record it empties goes, and their special links may number the records after it",
                    listed(&held)
                ));
            }
            bodiless = true;
        }
        disable_polygon(sections, polygon, &others).map_err(|e| e.to_string())?;
        let cuts = reaching(cell).saturating_sub(reaching(&made.mesh));
        removed = removes.then_some(made);
        cuts
    } else {
        let edges: Vec<usize> = (cut.edges.iter().copied())
            .filter(|&j| cell.edges.get(j).is_some_and(|e| e.mesh == cell.cell))
            .collect();
        cut_edges(&mut sections.virtual_section, &edges).map_err(|e| e.to_string())?;
        edges.len()
    };
    let mut lines = vec![match cuts {
        0 => format!("polygon {polygon} has no edge into or out of it left to cut"),
        n => format!("polygon {polygon} disabled: {n} edges now lead nowhere"),
    }];
    if !cut.links.is_empty() {
        lines.push(format!(
            "special links {} of this cell still end in it",
            listed(&cut.links)
        ));
    }
    for body in near {
        let links = links_into(body, cell.cell, polygon);
        if !links.is_empty() {
            lines.push(format!(
                "special links {} of the neighbouring cell {} still end in it",
                listed(&links),
                body.cell
            ));
        }
    }
    let read: Vec<u32> = near.iter().map(|b| b.cell).collect();
    let unread = across(cell).filter(|c| !read.contains(c));
    let unread: Vec<u32> = unread.collect();
    if bodiless {
        lines.extend(bodiless_line(&unread));
    } else if !unread.is_empty() {
        lines.push(format!(
            "the bodies of the neighbouring cells {} were not given with --neighbour, so whether their special links end in it is not known",
            listed(&unread)
        ));
    }
    Ok((lines, removed))
}

/// The cells across `cell`'s borders and at its corners; none for a body of no cell, which no other
/// body links into.
fn across(cell: &NavMesh) -> impl Iterator<Item = u32> {
    let cells = match cell.cell {
        0 => Vec::new(),
        c => Border::around(c),
    };
    cells.into_iter()
}

/// Polygon proper `polygon` deleted as the editor deletes it, once every cell across a border or at
/// a corner that the archive holds a body of is given, since their special links number this
/// cell's polygons.
fn delete(
    sections: &mut Rpf9Sections,
    cell: &NavMesh,
    polygon: usize,
    near: &[NavMesh],
    bodied: Bodied<'_>,
) -> Result<Said, String> {
    let given: Vec<u32> = near.iter().map(|b| b.cell).collect();
    let missing: Vec<u32> = across(cell).filter(|c| !given.contains(c)).collect();
    let held = bodied(missing.clone())?;
    if !held.is_empty() {
        return Err(format!(
            "polygon {polygon} is not deleted while the bodies of the neighbouring cells {} are not given with --neighbour <offset>: their special links may number this cell's polygons",
            listed(&held)
        ));
    }
    let all: Vec<u32> = across(cell).collect();
    for (i, l) in cell.special_links.iter().enumerate() {
        for mesh in [l.from_mesh, l.to_mesh] {
            if mesh != cell.cell && mesh != NOWHERE && !all.contains(&mesh) {
                return Err(format!(
                    "polygon {polygon} is not deleted: special link {i} ends in cell {mesh}, which is not across a border or at a corner of this one"
                ));
            }
        }
    }
    let others: Vec<&NavMesh> = near.iter().collect();
    let made = deletion(cell, polygon, &others).map_err(|e| e.to_string())?;
    let after = delete_polygon(sections, polygon, &others).map_err(|e| e.to_string())?;
    let mut lines = vec![format!(
        "polygon {polygon} deleted: {} polygons remain",
        after.polygons.len()
    )];
    let gone = (cell.special_links.len()).saturating_sub(after.special_links.len());
    if gone > 0 {
        lines.push(format!(
            "{gone} special links of this cell ended in a record that went, and were deleted"
        ));
    }
    lines.extend(bodiless_line(&missing));
    Ok((lines, Some(made)))
}

// Shared with the `rpf9` tests, which use parts of it this file does not.
#[cfg(test)]
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "rpf9_fixture.rs"]
mod fixture;

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
    use or_io::container::rpf::gta6_content::KNOWN_FRAMINGS;
    use or_io::container::rpf::{
        navigation_cell, Gta6Cipher, Rpf9Form, NAVIGATION_MESH, RPF9_SLOT,
    };
    use or_io::gta6::hash::name_hash64;
    use or_io::gta6::headerless::NavEdge;

    /// The cell's grid place, which the built cell states at 0xC8.
    const CELL: u32 = 256 + 512 * 256;

    #[test]
    fn a_neighbour_that_cannot_be_written_leaves_the_cell_unwritten() {
        let dir = tempfile::tempdir().expect("tempdir");
        let at = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
        let files = [
            (at("cell.rsc"), b"cell".to_vec()),
            (at("no such folder/0x10.rsc"), b"neighbour".to_vec()),
        ];
        let why = write_all(&files).expect_err("the neighbour's folder is missing");
        assert!(why.contains("no file was changed"), "{why}");
        let left: Vec<_> = std::fs::read_dir(dir.path()).expect("listed").collect();
        assert!(left.is_empty(), "{left:?}");
        let files = [(at("cell.rsc"), b"cell".to_vec())];
        write_all(&files).expect("written");
        assert_eq!(std::fs::read(at("cell.rsc")).expect("read"), b"cell");
    }

    fn put(v: &mut [u8], at: usize, bytes: &[u8]) {
        v[at..at + bytes.len()].copy_from_slice(bytes);
    }

    fn pointer(v: &mut [u8], at: usize, to: usize) {
        put(v, at, &(0x5000_0000 + to as u64).to_le_bytes());
    }

    /// The virtual section of a cell of one water triangle over 128 m, at grid place `cell`, whose
    /// first edge leads back into the triangle.
    fn cell_section(cell: u32) -> Vec<u8> {
        let mut v = vec![0u8; 0x360];
        // A split array at `at` of one chunk: header, chunk row, start, then the items.
        let list = |v: &mut Vec<u8>, at: usize, items: &[u8], stride: usize| {
            let count = (items.len() / stride) as u64;
            put(v, at + 8, &count.to_le_bytes());
            pointer(v, at + 0x10, at + 0x30);
            pointer(v, at + 0x18, at + 0x40);
            put(v, at + 0x20, &1u64.to_le_bytes());
            pointer(v, at + 0x30, at + 0x50);
            put(v, at + 0x38, &count.to_le_bytes());
            put(v, at + 0x50, items);
        };
        pointer(&mut v, 0x08, 0xF0);
        v[0xF8] = 1;
        put(&mut v, 0x10, &9u32.to_le_bytes());
        put(&mut v, 0x14, &0x0001_0015u32.to_le_bytes());
        for at in [0x20, 0x34, 0x48] {
            put(&mut v, at, &1.0f32.to_le_bytes());
        }
        let vertices: Vec<u8> = [0u16, 0, 0, 65535, 0, 0, 0, 65535, 0]
            .iter()
            .flat_map(|q| q.to_le_bytes())
            .collect();
        list(&mut v, 0x100, &vertices, 6);
        list(&mut v, 0x170, &[0, 0, 1, 0, 2, 0], 2);
        let edges = [[0xFF; 4], [0; 4], [0; 4]].repeat(3).concat();
        list(&mut v, 0x1D0, &edges, 12);
        let mut polygon = [0u8; 44];
        polygon[4] = 3;
        polygon[8] = 0x80;
        list(&mut v, 0x250, &polygon, 44);
        for (slot, at) in [
            (0x70, 0x100),
            (0x80, 0x170),
            (0x88, 0x1D0),
            (0x98, 0x250),
            (0xA0, 0x2D0),
        ] {
            pointer(&mut v, slot, at);
        }
        put(&mut v, 0x90, &3u32.to_le_bytes());
        for (at, x) in [(0x2E0, 128.0f32), (0x2E4, 128.0), (0x2E8, 4.0)] {
            put(&mut v, at, &x.to_le_bytes());
        }
        pointer(&mut v, 0x2D0 + 0x2C, 0x330);
        pointer(&mut v, 0x330 + 0x08, 0x350);
        v[0x330 + 0x18] = 1;
        for (at, word) in [
            (0xC0, 3),
            (0xC4, 1),
            (0xC8, cell),
            (0xCC, 6 * 3 + 14 * 3 + 44),
        ] {
            put(&mut v, at, &word.to_le_bytes());
        }
        // The adjacency list's items start 0x50 into its header at 0x1D0.
        put(&mut v, 0x220, &cell.to_le_bytes());
        put(&mut v, 0x224, &0u16.to_le_bytes());
        put(&mut v, 0x226, &0x1207u16.to_le_bytes());
        v
    }

    /// The cell at `cell` as a loose resource in `dir`.
    fn loose(dir: &std::path::Path, cell: u32) -> String {
        let sections = Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section: cell_section(cell),
            physical: Vec::new(),
        };
        let path = dir.join(format!("cell{cell}.rsc"));
        std::fs::write(&path, write_rpf9_resource(&sections).expect("written")).expect("saved");
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn a_disabled_scanned_cell_saves_its_edge_into_the_polygon_cut_and_its_flags_kept() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = loose(dir.path(), CELL);
        let (saved, lines, beside) =
            edit_body("disable", &path, 0, 0, &[], u64::MAX, None).expect("disabled");
        assert!(beside.is_empty(), "no neighbour given");
        assert_eq!(lines[0], "polygon 0 disabled: 1 edges now lead nowhere");
        assert!(
            lines
                .iter()
                .any(|l| l.contains("not given with --neighbour")),
            "{lines:?}"
        );
        let (find, bytes) = read_rpf9_body(Cursor::new(&saved), 0, u64::MAX).expect("it reads");
        let back = Rpf9Sections::of(&find, &bytes).expect("a resource");
        let cell = read_navmesh(&back.virtual_section).expect("the cell reads");
        assert_eq!(
            cell.edges[0],
            NavEdge {
                mesh: NOWHERE,
                polygon: 0xFFFF,
                word_06: 0x1200,
                flags: 0,
            }
        );
        assert_eq!(cell.polygons[0].flags, 0x80, "the water bit as read");
        let (_, again, _) = edit_body("disable", &path, 0, 0, &[], u64::MAX, None).expect("said");
        assert!(
            again[0].contains("polygon 0 disabled"),
            "the input is untouched"
        );
    }

    #[test]
    fn a_deletion_without_every_neighbouring_cell_is_refused_naming_them() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = loose(dir.path(), CELL);
        let why = edit_body("delete", &path, 0, 0, &[], u64::MAX, None).expect_err("refused");
        let cells = [CELL + 1, CELL - 1, CELL + 512, CELL - 512].map(|c| c.to_string());
        let corners = [CELL + 513, CELL - 511, CELL + 511, CELL - 513].map(|c| c.to_string());
        let named = format!("{}, {}", cells.join(", "), corners.join(", "));
        assert!(why.contains(&named), "{why}");
    }

    #[test]
    fn a_body_at_a_corner_of_the_cell_is_taken_as_a_neighbour() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let corner = CELL + 512 + 1;
        let read = |p: String| std::fs::read(p).expect("read");
        // A read refuses a file that starts with a resource and holds more, so the cell and the
        // corner body sit at 0x1000-aligned offsets past a leading slot, as they do in an archive.
        let mut bytes = vec![0u8; 0x1000];
        let main = bytes.len() as u64;
        bytes.extend(read(loose(dir.path(), CELL)));
        bytes.resize(bytes.len().next_multiple_of(0x1000), 0);
        let at = bytes.len() as u64;
        bytes.extend(read(loose(dir.path(), corner)));
        let path = dir.path().join("both.rsc");
        std::fs::write(&path, &bytes).expect("saved");
        let path = path.to_string_lossy().into_owned();
        let (_, lines, _) =
            edit_body("disable", &path, main, 0, &[at], u64::MAX, None).expect("disabled");
        let unread = (lines.iter())
            .find(|l| l.contains("not given with --neighbour"))
            .expect("the seven others are named");
        assert!(!unread.contains(&corner.to_string()), "{unread}");
        assert!(unread.contains(&(CELL - 513).to_string()), "{unread}");
    }

    #[test]
    fn a_body_of_no_cell_asks_for_no_neighbouring_cell() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = loose(dir.path(), 0);
        // Its one polygon going would empty it: refused for that, not for a neighbour.
        let why = edit_body("delete", &path, 0, 0, &[], u64::MAX, None).expect_err("refused");
        assert!(why.contains("a cell of no polygon"), "{why}");
        let (_, lines, _) =
            edit_body("disable", &path, 0, 0, &[], u64::MAX, None).expect("disabled");
        assert!(
            !lines.iter().any(|l| l.contains("--neighbour")),
            "{lines:?}"
        );
    }

    #[test]
    fn a_neighbour_not_across_a_border_a_side_record_and_an_output_that_is_the_input_are_refused() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = loose(dir.path(), CELL);
        let why = edit_body("disable", &path, 0, 0, &[0], u64::MAX, None).expect_err("refused");
        assert!(why.contains("not across a border"), "{why}");
        let why = edit_body("disable", &path, 0, 1, &[], u64::MAX, None).expect_err("refused");
        assert!(why.contains("side record"), "{why}");
        let words = ["disable", path.as_str(), "0", "0", path.as_str()].map(String::from);
        assert_eq!(super::super::navmesh_edit::run(&words), ExitCode::from(2));
        let words = [path.as_str(), "0", "x", "out.rsc"].map(String::from);
        assert!(parse(&words)
            .expect_err("refused")
            .contains("polygon number"));
    }

    /// The virtual section of a cell at grid place `cell` of two water triangles over 128 m whose
    /// edges lead nowhere, so either goes alone.
    fn two_triangles(cell: u32) -> Vec<u8> {
        let mut v = vec![0u8; 0x3B0];
        // A split array at `at` of one chunk: header, chunk row, start, then the items.
        let list = |v: &mut Vec<u8>, at: usize, items: &[u8], stride: usize| {
            let count = (items.len() / stride) as u64;
            put(v, at + 8, &count.to_le_bytes());
            pointer(v, at + 0x10, at + 0x30);
            pointer(v, at + 0x18, at + 0x40);
            put(v, at + 0x20, &1u64.to_le_bytes());
            pointer(v, at + 0x30, at + 0x50);
            put(v, at + 0x38, &count.to_le_bytes());
            put(v, at + 0x50, items);
        };
        pointer(&mut v, 0x08, 0xF0);
        v[0xF8] = 1;
        put(&mut v, 0x10, &9u32.to_le_bytes());
        put(&mut v, 0x14, &0x0001_0015u32.to_le_bytes());
        for at in [0x20, 0x34, 0x48] {
            put(&mut v, at, &1.0f32.to_le_bytes());
        }
        let vertices: Vec<u8> = [0u16, 0, 0, 65535, 0, 0, 0, 65535, 0, 65535, 65535, 0]
            .iter()
            .flat_map(|q| q.to_le_bytes())
            .collect();
        list(&mut v, 0x100, &vertices, 6);
        list(&mut v, 0x170, &[0, 0, 1, 0, 2, 0, 1, 0, 3, 0, 2, 0], 2);
        let edges = [[0xFF; 4], [0; 4], [0; 4]].repeat(6).concat();
        list(&mut v, 0x1D0, &edges, 12);
        // The second triangle's corners start at index 3.
        let mut polygons = [0u8; 88];
        (polygons[4], polygons[8]) = (3, 0x80);
        (polygons[44 + 4], polygons[44 + 8]) = (0x33, 0x80);
        list(&mut v, 0x270, &polygons, 44);
        for (slot, at) in [
            (0x70, 0x100),
            (0x80, 0x170),
            (0x88, 0x1D0),
            (0x98, 0x270),
            (0xA0, 0x320),
        ] {
            pointer(&mut v, slot, at);
        }
        put(&mut v, 0x90, &6u32.to_le_bytes());
        for (at, x) in [(0x330, 128.0f32), (0x334, 128.0), (0x338, 4.0)] {
            put(&mut v, at, &x.to_le_bytes());
        }
        // The quadtree's one leaf names both triangles.
        pointer(&mut v, 0x320 + 0x2C, 0x380);
        pointer(&mut v, 0x380 + 0x08, 0x3A0);
        v[0x380 + 0x18] = 2;
        put(&mut v, 0x3A0, &[0, 0, 1, 0]);
        for (at, word) in [
            (0xC0, 4),
            (0xC4, 2),
            (0xC8, cell),
            (0xCC, 6 * 4 + 14 * 6 + 44 * 2),
        ] {
            put(&mut v, at, &word.to_le_bytes());
        }
        v
    }

    /// `v`, a section of [`cell_section`], with one special link from its polygon 0 to polygon
    /// `to` of cell `cell`, filed under its polygon 0.
    fn with_link(mut v: Vec<u8>, to: u16, cell: u32) -> Vec<u8> {
        let own = v[0xC8..0xCC].to_vec();
        let at = v.len();
        v.resize(at + 0x30, 0);
        put(&mut v, at + 0x0E, &to.to_le_bytes());
        put(&mut v, at + 0x10, &own);
        put(&mut v, at + 0x14, &cell.to_le_bytes());
        // The index list after the link, naming it once.
        pointer(&mut v, 0xA8, at);
        pointer(&mut v, 0xB0, at + 0x20);
        put(&mut v, 0xD4, &1u32.to_le_bytes());
        put(&mut v, 0xD8, &1u32.to_le_bytes());
        put(&mut v, 0xCC, &(6 * 3 + 14 * 3 + 44 + 32u32).to_le_bytes());
        // Polygon 0's run of the index list, at its record's +0x20: from 0, one entry.
        put(&mut v, 0x2A0 + 0x20, &1u32.to_le_bytes());
        v
    }

    /// The fixture's archive key, which its synthetic snapshot holds.
    const KEY: Rpf9IndexKey = Rpf9IndexKey {
        key_id: 0xDEAD_BEEF,
        iv: [0x33; 16],
    };

    /// An archive built in `dir` of `cells`, each a path and a cell's virtual section under a
    /// navigation row; its path, and each cell's body offset in order.
    fn table_of(dir: &Path, cells: &[(&str, Vec<u8>)]) -> (String, Vec<u64>) {
        let keys = super::fixture::keys();
        let entries: Vec<(String, Rpf9NewBody)> = (cells.iter())
            .map(|(name, v)| {
                let s = Rpf9Sections {
                    form: Rpf9Form::BlockMapped,
                    virtual_section: v.clone(),
                    physical: Vec::new(),
                };
                let loose = write_rpf9_resource(&s).expect("written");
                let body = Rpf9NewBody::Resource {
                    type_code: NAVIGATION_MESH,
                    loose,
                };
                ((*name).to_owned(), body)
            })
            .collect();
        let plan = build(&entries, &KEY, &keys).expect("the archive builds");
        let mut bytes = Vec::new();
        (plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut bytes)).expect("laid out");
        let mount = Gta6Mount::open_root(&mut Cursor::new(&bytes), &keys).expect("it opens");
        let offsets = (cells.iter())
            .map(|(name, _)| {
                let hash = name_hash64(name.as_bytes());
                let row = (mount.archive.entries.iter()).find(|e| e.name64 == hash);
                mount.absolute_start(row.expect("listed")).expect("a start") + RPF9_SLOT
            })
            .collect();
        let path = dir.join("cells.rpf");
        std::fs::write(&path, &bytes).expect("saved");
        (path.to_string_lossy().into_owned(), offsets)
    }

    #[test]
    fn a_deletion_takes_a_cell_the_table_holds_no_navigation_row_of_as_linking_nowhere() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let east_body = with_link(cell_section(CELL + 1), 1, CELL);
        let cells = [
            ("nav/main.ynv", two_triangles(CELL)),
            ("nav/east.ynv", east_body),
        ];
        let (path, at) = table_of(dir.path(), &cells);
        let (main, east) = (at[0], at[1]);
        let keys = super::fixture::keys();
        let keys = Some(&keys);

        // With no key no table is read, so every neighbouring cell is still wanted.
        let why = edit_body("delete", &path, main, 0, &[], u64::MAX, None).expect_err("refused");
        let all: Vec<String> = Border::around(CELL).iter().map(u32::to_string).collect();
        assert!(why.contains(&all.join(", ")), "{why}");
        // The table holds a row of the cell east, so it alone is still wanted.
        let why = edit_body("delete", &path, main, 0, &[], u64::MAX, keys).expect_err("refused");
        let wanted = format!("neighbouring cells {} are not given", CELL + 1);
        assert!(why.contains(&wanted), "{why}");

        let (saved, lines, beside) =
            edit_body("delete", &path, main, 0, &[east], u64::MAX, keys).expect("deleted");
        assert_eq!(lines[0], "polygon 0 deleted: 1 polygons remain");
        for said in [
            "holds no navigation row of the neighbouring cells",
            "renumbered",
        ] {
            assert!(lines.iter().any(|l| l.contains(said)), "{said}: {lines:?}");
        }
        let back = |bytes: &[u8]| {
            let (find, read) = read_rpf9_body(Cursor::new(bytes), 0, u64::MAX).expect("it reads");
            let s = Rpf9Sections::of(&find, &read).expect("a resource");
            read_navmesh(&s.virtual_section).expect("the cell reads")
        };
        assert_eq!(back(&saved).polygons.len(), 1);
        assert_eq!(beside.len(), 1);
        assert_eq!(beside[0].0, east);
        let m = back(&beside[0].1);
        let l = &m.special_links[0];
        assert_eq!((l.from_polygon, l.to_polygon, l.to_mesh), (0, 0, CELL));
    }

    #[test]
    fn a_neighbour_in_a_ciphered_row_blocks_a_deletion_and_one_the_table_does_not_hold_does_not() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let cells = [
            ("nav/main.ynv", two_triangles(CELL)),
            ("nav/east.ynv", cell_section(CELL + 1)),
        ];
        let (path, at) = table_of(dir.path(), &cells);
        let keys = super::fixture::keys();
        let file = File::open(&path).expect("opened");
        let mut mount = Gta6Mount::open_root(&mut &file, &keys).expect("it opens");
        let (east, west) = (CELL + 1, CELL - 1);
        assert_eq!(held_of(&mut &file, &mount, &keys, vec![east, west]), [east]);

        // The fixture's snapshot holds no content key, so a ciphered row's cell does not read, and
        // it may be any of them.
        let hash = name_hash64(b"nav/east.ynv");
        let row = (mount.archive.entries.iter_mut()).find(|e| e.name64 == hash);
        row.expect("listed").cipher = Some(Gta6Cipher {
            key_id: 0x0BAD_0001,
            cipher_type: 17,
            iv: [0; 16],
            framing: KNOWN_FRAMINGS[0],
        });
        assert_eq!(
            held_of(&mut &file, &mount, &keys, vec![east, west]),
            [east, west]
        );
        let (mut sections, cell) = cell_at(&file, at[0], u64::MAX).expect("the cell reads");
        let bodied = |m: Vec<u32>| Ok::<_, String>(held_of(&mut &file, &mount, &keys, m));
        let why = delete(&mut sections, &cell, 0, &[], &bodied).expect_err("refused");
        let refused = format!("{west}, {}", CELL + 512);
        assert!(why.contains("are not given with --neighbour"), "{why}");
        assert!(why.contains(&refused), "{why}");
    }

    /// Navigation rows of the build, ciphered or not, each read as the cell they state.
    #[test]
    fn the_build_s_navigation_rows_each_state_a_cell() {
        const MOST: usize = 40;
        let set = ["OPENRAGE_GTA6_KEYS", "OPENRAGE_GTA6_EXE"].map(std::env::var_os);
        let build = std::env::var_os("OPENRAGE_GTA6_BUILD").map(std::path::PathBuf::from);
        let build = build.filter(|p| p.is_dir() && set.iter().all(Option::is_some));
        let keys = build
            .as_ref()
            .and_then(|_| crate::gta6_keys::load(None, None).ok());
        let (Some(build), Some(keys)) = (build, keys) else {
            or_test_support::skip_install(
                "gta6 navigation rows",
                "set OPENRAGE_GTA6_KEYS, OPENRAGE_GTA6_EXE and OPENRAGE_GTA6_BUILD",
            );
            return;
        };
        let listed = std::fs::read_dir(&build).expect("the build lists");
        let mut archives: Vec<_> = (listed.flatten().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "rpf"))
            .collect();
        archives.sort();
        let mut read = 0usize;
        for disk in archives {
            let mut file = File::open(&disk).expect("opened");
            let Ok(root) = Gta6Mount::open_root(&mut file, &keys) else {
                continue;
            };
            let mut stack = vec![(root, 0usize)];
            while let Some((mount, depth)) = stack.pop() {
                let extras = mount.archive.entry_extras();
                for (i, entry) in mount.archive.entries.iter().enumerate() {
                    if read == MOST {
                        return;
                    }
                    if mount
                        .entry_begins_archive(&mut file, entry)
                        .unwrap_or(false)
                    {
                        if depth < 3 {
                            if let Ok(child) = mount.open_nested(&mut file, entry, &keys) {
                                stack.push((child, depth + 1));
                            }
                        }
                        continue;
                    }
                    let row = extras.get(i).and_then(|x| x.nested);
                    if row.is_some_and(|r| r.resource_type == NAVIGATION_MESH) {
                        navigation_cell(&mut file, &mount, entry, &keys)
                            .unwrap_or_else(|e| panic!("{}: {e}", disk.display()));
                        read += 1;
                    }
                }
            }
        }
        if read == 0 {
            or_test_support::skip_install("gta6 navigation rows", "no navigation row was met");
        }
    }
}
