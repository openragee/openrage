//! Install-gated: the clips of a sample of a build state at 0x38 the signature their fields give,
//! and an edited clip saves with the one its edited fields give.
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use or_test_support::skip_install;

use super::*;
use crate::container::rpf::gta6_chunk_table::entry_chunks;
use crate::container::rpf::{
    read_rpf9_body, Gta6Class, Gta6Mount, GtaViKeys, Rpf9Form, Rpf9Sections, GTA6_SNAPSHOT_ENV,
};

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// The 20-byte row resource types of the clip family.
const CLIP_FAMILY: [u32; 5] = [0x43, 0x46, 0x4E, 0x66, 0x6D];
/// The 16-byte zero slot a headerless body begins with.
const SLOT: u64 = 16;
const CAP: u64 = 4 << 20;
const MAX_DEPTH: usize = 8;
/// Clip-family bodies read from each archive of the build's root, so the sample reaches past the
/// first archive.
const PER_ARCHIVE: usize = 40;
/// The walk stops at this many seconds, whatever it has read.
const BUDGET: Duration = Duration::from_secs(180);

#[derive(Default)]
struct Tally {
    bodies: usize,
    clips: usize,
    holding: usize,
    one: usize,
    groups: usize,
    /// Group entries whose warp list is not empty.
    warped: usize,
    /// Edits made, each of a clip whose fields it changed.
    edited: usize,
    misses: Vec<String>,
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// Every `*.rpf` directly under `root`, then every `pack/*/*.rpf`, sorted.
fn build_archives(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut rpfs_in = |dir: &Path| {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rpf") {
                out.push(path);
            }
        }
    };
    rpfs_in(root);
    for pack in std::fs::read_dir(root.join("pack"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if pack.path().is_dir() {
            rpfs_in(&pack.path());
        }
    }
    out.sort();
    out
}

/// A window or rate edit of the first clip of `resource` it can be made on, which changes a field
/// the clip's signature follows from.
fn an_edit(resource: &AnimResource) -> Option<(usize, AnimEdit)> {
    (resource.clips.iter().enumerate()).find_map(|(clip, c)| match &c.play {
        ClipPlay::One {
            start, end, rate, ..
        } => Some((
            clip,
            AnimEdit::ClipWindow {
                clip,
                start: *start,
                end: *end,
                rate: rate * 0.5,
            },
        )),
        ClipPlay::Groups { groups, .. } => {
            let (group, entry, e) = (groups.iter().enumerate()).find_map(|(g, group)| {
                (group.entries.iter().enumerate())
                    .find(|(_, e)| e.animation.is_some() && e.end > e.start)
                    .map(|(i, e)| (g, i, e))
            })?;
            Some((
                clip,
                AnimEdit::EntryWindow {
                    clip,
                    group,
                    entry,
                    start: e.start,
                    end: e.start + (e.end - e.start) * 0.5,
                    frame_rate: None,
                },
            ))
        }
        ClipPlay::Unresolved => None,
    })
}

fn tally(sections: Rpf9Sections, at: &str, t: &mut Tally) {
    let read = match sections.form {
        Rpf9Form::StoredRsc8 { .. } => read_stored(&sections.virtual_section),
        Rpf9Form::BlockMapped => read(&sections.virtual_section),
    };
    let Ok(mut resource) = read else {
        return;
    };
    let signed = match clip_signatures(&sections.virtual_section) {
        Ok(signed) => signed,
        Err(e) => {
            t.misses.push(format!("{at}: {e}"));
            return;
        }
    };
    t.bodies += 1;
    for (c, (clip, (stored, computed))) in resource.clips.iter().zip(&signed).enumerate() {
        t.clips += 1;
        match &clip.play {
            ClipPlay::One { .. } => t.one += 1,
            ClipPlay::Groups { groups, .. } => {
                t.groups += 1;
                t.warped += (groups.iter().flat_map(|g| &g.entries))
                    .filter(|e| !e.warp_points.is_empty())
                    .count();
            }
            ClipPlay::Unresolved => {}
        }
        if stored == computed {
            t.holding += 1;
        } else {
            t.misses.push(format!(
                "{at} clip {c}: states {stored:#010X}, its fields give {computed:#010X}"
            ));
        }
    }
    let Some((clip, change)) = an_edit(&resource) else {
        return;
    };
    let before = signed.get(clip).map(|s| s.1);
    resource.sections = Some(Arc::new(sections));
    // An edit refused for its own reason (an entry's frame rate unknown, say) is no miss.
    let Ok(edited) = edit(&resource, &change) else {
        return;
    };
    let out = &(edited.sections.as_ref())
        .expect("sections")
        .virtual_section;
    match clip_signatures(out).map(|s| s.get(clip).copied()) {
        Ok(Some((stored, computed))) if stored == computed && Some(computed) != before => {
            t.edited += 1;
        }
        other => t.misses.push(format!(
            "{at} clip {clip} after {change:?}: {other:?}, was {before:?}"
        )),
    }
}

fn visit(
    file: &mut File,
    keys: &GtaViKeys,
    mount: &Gta6Mount,
    (depth, until): (usize, Instant),
    t: &mut Tally,
) {
    let extras = mount.archive.entry_extras();
    for (entry, extra) in mount.archive.entries.iter().zip(&extras) {
        if t.bodies == PER_ARCHIVE || Instant::now() > until {
            return;
        }
        if entry.kind() == Some(Gta6Class::NestedArchive) {
            if depth < MAX_DEPTH {
                if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                    visit(file, keys, &child, (depth + 1, until), t);
                }
            }
            continue;
        }
        let (None, Some(start), Some(row)) =
            (&entry.cipher, mount.absolute_start(entry), extra.nested)
        else {
            continue;
        };
        if !CLIP_FAMILY.contains(&row.resource_type)
            || row.stored_size() > CAP
            || !matches!(
                entry_chunks(&mount.archive, entry.index as usize),
                Ok(Some(_))
            )
        {
            continue;
        }
        let sections = read_rpf9_body(&mut *file, start + SLOT, CAP)
            .ok()
            .and_then(|(find, bytes)| Rpf9Sections::of(&find, &bytes));
        if let Some(sections) = sections {
            tally(sections, &format!("entry {}", entry.index), t);
        }
    }
}

/// Measured first by a node mirror of the rule: 356 of 356 clips of the build hold (19 of one
/// animation, 337 of groups, 198 of those hashing a warp list).
#[test]
fn the_clips_of_a_sample_of_the_build_state_the_signature_their_fields_give() {
    let what = "GTA VI clip signatures over a sample of a build";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    let until = Instant::now() + BUDGET;
    let mut total = Tally::default();
    for path in build_archives(&build) {
        if Instant::now() > until {
            break;
        }
        let mut file = File::open(&path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        let mut t = Tally::default();
        visit(&mut file, &keys, &mount, (0, until), &mut t);
        total.bodies += t.bodies;
        total.clips += t.clips;
        total.holding += t.holding;
        total.one += t.one;
        total.groups += t.groups;
        total.warped += t.warped;
        total.edited += t.edited;
        total.misses.extend(t.misses);
    }
    let t = total;
    println!(
        "{} bodies, {} clips ({} of one animation, {} of groups, {} warp lists), {} holding, {} edited",
        t.bodies, t.clips, t.one, t.groups, t.warped, t.holding, t.edited
    );
    assert!(
        t.misses.is_empty(),
        "{:?}",
        &t.misses[..t.misses.len().min(20)]
    );
    assert_eq!(t.holding, t.clips);
    assert!(
        t.groups > 0 && t.warped > 0,
        "the sample hashed no warp list"
    );
    assert!(t.edited > 0, "no clip of the sample took an edit");
}
