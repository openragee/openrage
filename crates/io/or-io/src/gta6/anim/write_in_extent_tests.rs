use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_test_support::skip_install;

use super::*;
use crate::codec::anim::AnimationSource;
use crate::container::rpf::gta6_chunk_table::entry_chunks;
use crate::container::rpf::{
    read_rpf9_body, Gta6Class, Gta6Mount, GtaViKeys, Rpf9Form, Rpf9Sections, GTA6_SNAPSHOT_ENV,
};
use crate::gta6::anim::write::PAST_EXTENT;

/// Steps rising evenly from 0 to 65,535 over 70 frames.
fn ramp() -> Vec<i64> {
    (0..70).map(|f| f * 65_535 / 69).collect()
}

fn opened(q: &[i64]) -> AnimResource {
    let mut constants = floats(&[1.0, 2.0, 3.0, 0.5]);
    constants.extend(delta_channel(q));
    let spec = Spec {
        frames: q.len() as u16,
        constants,
        ..Spec::default()
    };
    let bytes = build(&spec).bytes;
    let mut resource = read(&bytes).expect("reads");
    resource.sections = Some(Arc::new(Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: bytes,
        physical: Vec::new(),
    }));
    resource
}

fn section(resource: &AnimResource) -> &[u8] {
    &resource
        .sections
        .as_ref()
        .expect("sections")
        .virtual_section
}

fn key(component: u8, frame: u16, value: f32) -> AnimEdit {
    AnimEdit::Key {
        animation: AnimAt {
            set: 0,
            animation: 0,
        },
        dof: 2,
        component,
        frame,
        value,
    }
}

#[test]
fn a_key_edit_that_fits_its_block_keeps_the_section_s_length_and_reads_back() {
    // A spike at frame 30 widens its group; taking it back out narrows the group again.
    let mut spiked = ramp();
    spiked[30] += 3;
    let resource = opened(&spiked);
    let value = section10::value(SCALE, OFFSET, ramp()[30]);
    let fitted = edit_in_extent(&resource, &key(0, 30, value)).expect("fits");
    let bytes = section(&fitted);
    assert_eq!(bytes.len(), section(&resource).len());
    assert!(bytes == section(&edit(&resource, &key(0, 30, value)).expect("edits")));
    assert!(fitted.edited);
    let again = read(bytes).expect("reads");
    let (was, now) = (the_animation(&resource), the_animation(&again));
    for frame in 0..70 {
        let want = match frame {
            30 => Some(value),
            _ => was.delta_value(2, 0, frame),
        };
        assert_eq!(now.delta_value(2, 0, frame), want, "frame {frame}");
    }
}

#[test]
fn a_key_edit_that_needs_wider_bits_is_refused_in_extent_by_name_and_rebuilt_by_edit() {
    let resource = opened(&ramp());
    // A jump to the range's foot widens the groups it falls in, so its block grows.
    let jump = key(0, 31, section10::value(SCALE, OFFSET, 0));
    let refused = edit_in_extent(&resource, &jump).expect_err("refused");
    assert!(
        matches!(&refused, AnimError::Unwritten(why) if why.contains(PAST_EXTENT)),
        "{refused}"
    );
    let rebuilt = edit(&resource, &jump).expect("edits");
    assert!(section(&rebuilt).len() > section(&resource).len());

    // An edit refused for another reason keeps its own reason.
    let other = edit_in_extent(&resource, &key(3, 31, 1.0)).expect_err("refused");
    assert!(!other.to_string().contains(PAST_EXTENT), "{other}");
}

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// The 20-byte row resource types of the clip family.
const CLIP_FAMILY: [u32; 5] = [0x43, 0x46, 0x4E, 0x66, 0x6D];
/// The 16-byte zero slot a headerless body begins with.
const SLOT: u64 = 16;
const CAP: u64 = 4 << 20;
const MAX_DEPTH: usize = 8;
/// Clear clip-family bodies, in the order a walk of the build's sorted archives meets them.
const BODIES: usize = 40;
/// Single-key edits tried per body.
const EDITS: usize = 12;

#[derive(Default)]
struct Tally {
    bodies: usize,
    edits: usize,
    outcomes: BTreeMap<String, usize>,
}

impl Tally {
    fn count(&mut self, outcome: impl Into<String>) {
        *self.outcomes.entry(outcome.into()).or_default() += 1;
    }

    fn of(&self, outcome: &str) -> usize {
        self.outcomes.get(outcome).copied().unwrap_or(0)
    }
}

/// A message with its digits masked, so reasons that differ only by a number tally together.
fn masked(message: impl std::fmt::Display) -> String {
    (message.to_string().chars())
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect()
}

/// The first [`EDITS`] section-10 components of `sections`' animations, each set at the middle
/// frame to the value the frame before it holds, and whether the edit keeps the section's length.
fn single_keys(sections: Rpf9Sections, t: &mut Tally) {
    let read = match sections.form {
        Rpf9Form::StoredRsc8 { .. } => read_stored(&sections.virtual_section),
        Rpf9Form::BlockMapped => read(&sections.virtual_section),
    };
    let Ok(mut resource) = read else {
        t.count("unread");
        return;
    };
    resource.sections = Some(Arc::new(sections));
    let mut made = 0;
    for (set, s) in resource.sets.iter().enumerate() {
        for (a, animation) in s.animations.iter().enumerate() {
            if animation.frames < 2 {
                continue;
            }
            for (dof, component) in animation.delta_components() {
                if made == EDITS {
                    return;
                }
                made += 1;
                t.edits += 1;
                let frame = animation.frames / 2;
                let Some(value) = animation.delta_value(dof, component, frame - 1) else {
                    t.count("no value");
                    continue;
                };
                let chunk = usize::from(animation.frames_per_chunk);
                let blocks: Vec<usize> = (0..animation.blocks.len())
                    .filter(|b| {
                        (usize::from(frame).checked_sub(b * chunk))
                            .is_some_and(|k| k < animation.frames_in_block(*b))
                    })
                    .collect();
                let in_section_10 = |b: &usize| {
                    (animation.blocks[*b].deltas.iter())
                        .any(|d| d.dof as usize == dof && d.component == component)
                };
                if blocks.is_empty() || !blocks.iter().all(in_section_10) {
                    t.count("mixed");
                    continue;
                }
                let change = AnimEdit::Key {
                    animation: AnimAt { set, animation: a },
                    dof,
                    component,
                    frame,
                    value,
                };
                match edit_in_extent(&resource, &change) {
                    Ok(_) => t.count("fits"),
                    Err(AnimError::Unwritten(why)) if why.contains(PAST_EXTENT) => t.count("grows"),
                    Err(e) => t.count(format!("refused: {}", masked(e))),
                }
            }
        }
    }
}

fn visit(file: &mut File, keys: &GtaViKeys, mount: &Gta6Mount, depth: usize, t: &mut Tally) {
    let extras = mount.archive.entry_extras();
    for (entry, extra) in mount.archive.entries.iter().zip(&extras) {
        if t.bodies == BODIES {
            return;
        }
        if entry.kind() == Some(Gta6Class::NestedArchive) {
            if depth < MAX_DEPTH {
                if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                    visit(file, keys, &child, depth + 1, t);
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
        t.bodies += 1;
        let sections = read_rpf9_body(&mut *file, start + SLOT, CAP)
            .ok()
            .and_then(|(find, bytes)| Rpf9Sections::of(&find, &bytes));
        match sections {
            Some(sections) => single_keys(sections, t),
            None => t.count("unread: no sections"),
        }
    }
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

/// Measured first by a node mirror of the edit over the Xbox build 1.0.2.1: of 132 edits over
/// the first 40 clear clip-family bodies, archives in sorted order, 121 fit their blocks and 11
/// grow one.
#[test]
fn single_key_edits_of_the_build_s_clips_that_fit_their_blocks() {
    let what = "single-key clip edits that keep their blocks' extent over a build";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    let mut t = Tally::default();
    for path in build_archives(&build) {
        if t.bodies == BODIES {
            break;
        }
        let mut file = File::open(&path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        visit(&mut file, &keys, &mount, 0, &mut t);
    }
    println!(
        "{} bodies, {} single-key edits: {:?}",
        t.bodies, t.edits, t.outcomes
    );
    assert_eq!(t.bodies, BODIES, "the build holds fewer clip-family bodies");
    assert_eq!(
        (t.edits, t.of("fits"), t.of("grows")),
        (132, 121, 11),
        "{:?}",
        t.outcomes
    );
}
