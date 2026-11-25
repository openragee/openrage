//! `rpf scan-expressions`: a GTA VI expression set's tracks, group by group, and a track's id
//! written, or a track added or removed, and saved as a loose resource that `rpf scan-open` reads
//! again.

use std::fs;

use or_io::container::rpf::{
    read_rpf9_body, write_rpf9_resource, Rpf9BodyKind, Rpf9Sections, Rpf9ZlibRun,
};
use or_io::gta6::anim::{
    add_expression_track, read_expressions, remove_expression_track, track_label,
    write_edited_expressions, ExpressionGroup, Expressions, EXPRESSIONS_SIGNATURE,
};

use crate::project::Failure::{self, Run, Usage};

const USAGE: &str = "usage: rpf scan-expressions <archive.rpf> <offset> [--limit <bytes>] \
     [--id <group> <track> <hash> --out <file> | --add <group> <track> <hash> --out <file> \
     | --remove <group> <track> --out <file>]   (group: inputs, bones, controls, facial-outputs \
     or outputs; --add facial-outputs writes the inputs' track of that hash, track 0 where the set \
     writes none)";

/// What an edit of the set does to track `track` of its group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    /// The track's id becomes the hash.
    Id(u32),
    /// A copy of the track is added under the hash; of a bone, its pair.
    Add(u32),
    /// The track is removed; of a bone, its pair.
    Remove,
}

/// An edit the arguments name: the group, the track, the change and the file written.
type Edit<'a> = (ExpressionGroup, usize, Change, &'a String);

/// The archive, the offset and the edit the arguments after `scan-expressions` name.
fn parsed(args: &[String]) -> Option<(&String, &String, Option<Edit<'_>>)> {
    let (path, offset, rest) = match args {
        [path, offset, rest @ ..] => (path, offset, rest),
        _ => return None,
    };
    let (flag, group, track, hash, out) = match rest {
        [] => return Some((path, offset, None)),
        [flag, group, track, hash, out_flag, out] if out_flag == "--out" => {
            (flag, group, track, Some(hash), out)
        }
        [flag, group, track, out_flag, out] if out_flag == "--out" => {
            (flag, group, track, None, out)
        }
        _ => return None,
    };
    let group = group_named(group)?;
    let track: usize = track.parse().ok()?;
    let hash = match hash {
        Some(hash) => Some(parse_hash(hash)?),
        None => None,
    };
    let change = match (flag.as_str(), hash) {
        ("--id", Some(hash)) => Change::Id(hash),
        ("--add", Some(hash)) => Change::Add(hash),
        ("--remove", None) => Change::Remove,
        _ => return None,
    };
    Some((path, offset, Some((group, track, change, out))))
}

/// The group a word names, as the usage spells it.
fn group_named(word: &str) -> Option<ExpressionGroup> {
    (ExpressionGroup::ALL.into_iter()).find(|g| group_word(*g) == word)
}

fn group_word(group: ExpressionGroup) -> &'static str {
    match group {
        ExpressionGroup::Inputs => "inputs",
        ExpressionGroup::Bones => "bones",
        ExpressionGroup::Controls => "controls",
        ExpressionGroup::FacialOutputs => "facial-outputs",
        ExpressionGroup::Outputs => "outputs",
    }
}

/// A hash typed as `hash_` and hex digits, `0x` and hex digits, or a decimal number.
fn parse_hash(word: &str) -> Option<u32> {
    let hex = (word.strip_prefix("hash_")).or_else(|| word.strip_prefix("0x"));
    match hex {
        Some(digits) => u32::from_str_radix(digits, 16).ok(),
        None => word.parse().ok(),
    }
}

pub(crate) fn cmd_scan_expressions(args: &[String]) -> Result<(), Failure> {
    let (limit, args) = super::rpf::split_limit(args).map_err(Usage)?;
    let (path, offset, edit) = parsed(&args).ok_or_else(|| Usage(USAGE.to_string()))?;
    let offset = super::rpf::parse_offset(offset).map_err(Usage)?;
    if let Some((.., out)) = edit.filter(|(.., out)| super::navmesh_edit::same_file(path, out)) {
        return Err(Usage(format!(
            "the output {out} is the input: name another file"
        )));
    }
    let file = fs::File::open(path).map_err(|e| Run(format!("open({path}): {e}")))?;
    let (find, bytes) = read_rpf9_body(file, offset, limit).map_err(|e| Run(e.to_string()))?;
    let signature = match find.kind {
        Rpf9BodyKind::Zlib(Rpf9ZlibRun { signature, .. }) => signature,
        _ => None,
    };
    if !signature.is_some_and(|s| s.to_string() == EXPRESSIONS_SIGNATURE) {
        return Err(Run(format!(
            "the body at {offset:#X} is no expression set: its signature is not {EXPRESSIONS_SIGNATURE}"
        )));
    }
    let mut sections = Rpf9Sections::of(&find, &bytes)
        .ok_or_else(|| Run(format!("the body at {offset:#X} states no sections")))?;
    let set = read_expressions(&sections.virtual_section)
        .map_err(|e| Run(format!("the expression reader does not read it: {e}")))?;
    print_tables(&set);
    let Some((group, track, change, out)) = edit else {
        return Ok(());
    };
    let (set, done) = edited(set, group, track, change).map_err(Run)?;
    let physical = sections.physical.len();
    write_edited_expressions(&mut sections.virtual_section, physical, &set)
        .map_err(|e| Run(e.to_string()))?;
    let written = write_rpf9_resource(&sections).map_err(|e| Run(e.to_string()))?;
    fs::write(out, &written).map_err(|e| Run(format!("write({out}): {e}")))?;
    let how = match change {
        Change::Id(_) => "written where it lies",
        Change::Add(_) | Change::Remove => "the set laid out anew",
    };
    println!(
        "  wrote {} bytes to {out}: {done}, {how}; `rpf scan-open {out} 0` reads it again",
        written.len()
    );
    Ok(())
}

/// `set` with the change made to track `track` of `group`, and what was done.
// An add to an empty group D takes no track of it: group A's track of the hash is written there.
fn edited(
    mut set: Expressions,
    group: ExpressionGroup,
    track: usize,
    change: Change,
) -> Result<(Expressions, String), String> {
    let word = group_word(group);
    if matches!(change, Change::Id(_)) && (group.of(&set)).is_none_or(|g| track >= g.tracks.len()) {
        return Err(format!("the set holds no track {track} among its {word}"));
    }
    let done = match change {
        Change::Id(hash) => {
            if let Some(held) = (group.of_mut(&mut set)).and_then(|g| g.tracks.get_mut(track)) {
                held.0 = hash;
            }
            format!("track {track} of the {word} now names hash_{hash:08X}")
        }
        Change::Add(hash) if group == ExpressionGroup::FacialOutputs => {
            set = add_expression_track(&set, group, track, hash).map_err(|e| e.to_string())?;
            format!("the inputs' hash_{hash:08X} is added to the {word}")
        }
        Change::Add(hash) => {
            set = add_expression_track(&set, group, track, hash).map_err(|e| e.to_string())?;
            format!("a copy of track {track} of the {word} is added under hash_{hash:08X}")
        }
        Change::Remove => {
            set = remove_expression_track(&set, group, track).map_err(|e| e.to_string())?;
            format!("track {track} of the {word} is removed")
        }
    };
    Ok((set, done))
}

/// Each group's tracks, a line each: its index, id, track word, components or byte or mask, and
/// slots.
fn print_tables(set: &Expressions) {
    println!(
        "  expressions   key {:#010X}, whose name is not known; it holds no program",
        set.key
    );
    for group in ExpressionGroup::ALL {
        let Some(tracks) = group.of(set) else {
            println!("  {:<15} none", group_word(group));
            continue;
        };
        let side = if group.reads() { "reads" } else { "writes" };
        println!(
            "  {:<15} {} track(s) it {side}",
            group_word(group),
            tracks.tracks.len()
        );
        for (k, (id, word)) in tracks.tracks.iter().enumerate() {
            let stated = match group.mask(set, k) {
                Some(mask) => format!("mask {mask:#04X}"),
                None => format!(
                    "component(s) {}",
                    tracks.components.get(k).copied().unwrap_or_default()
                ),
            };
            let slots: Vec<String> = (group.slots(set, k).unwrap_or_default().iter())
                .map(u16::to_string)
                .collect();
            println!(
                "    {k:>4} hash_{id:08X} {:<18} {stated}, slot(s) {}",
                track_label(*word),
                slots.join(", ")
            );
        }
    }
    if !set.control_slots.is_empty() {
        println!(
            "  further input slots of the unnamed word: {:?}, their words {:08X?}",
            set.control_slots, set.control_words
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_and_a_hash_are_read_as_the_usage_spells_them() {
        for group in ExpressionGroup::ALL {
            assert_eq!(group_named(group_word(group)), Some(group));
        }
        assert_eq!(group_named("faces"), None);
        assert_eq!(parse_hash("hash_0BADF00D"), Some(0x0BAD_F00D));
        assert_eq!(parse_hash("0x10"), Some(16));
        assert_eq!(parse_hash("17"), Some(17));
        assert_eq!(parse_hash("hash_xyz"), None);
    }

    #[test]
    fn an_id_an_add_and_a_remove_are_read_as_the_usage_spells_them() {
        let words = |line: &str| -> Vec<String> { line.split(' ').map(str::to_string).collect() };
        let change = |line: &str| {
            let args = words(line);
            parsed(&args).map(|(_, _, edit)| edit.map(|(g, t, c, out)| (g, t, c, out.clone())))
        };
        let out = || "o.rsc".to_string();
        assert_eq!(change("a.rpf 0x10"), Some(None));
        assert_eq!(
            change("a.rpf 0x10 --id inputs 1 hash_0000000A --out o.rsc"),
            Some(Some((ExpressionGroup::Inputs, 1, Change::Id(10), out())))
        );
        assert_eq!(
            change("a.rpf 0x10 --add bones 2 0x1F --out o.rsc"),
            Some(Some((ExpressionGroup::Bones, 2, Change::Add(31), out())))
        );
        assert_eq!(
            change("a.rpf 0x10 --remove outputs 0 --out o.rsc"),
            Some(Some((ExpressionGroup::Outputs, 0, Change::Remove, out())))
        );
        for refused in [
            "a.rpf",
            "a.rpf 0x10 --remove inputs 0 7 --out o.rsc",
            "a.rpf 0x10 --add inputs 0 --out o.rsc",
            "a.rpf 0x10 --add faces 0 7 --out o.rsc",
            "a.rpf 0x10 --id inputs 0 7 o.rsc",
        ] {
            assert_eq!(change(refused), None, "{refused}");
        }
    }

    #[test]
    fn a_set_that_writes_no_facial_control_takes_its_first_as_track_0_and_nothing_past_it() {
        use or_io::gta6::anim::ExpressionTracks;
        let none = || ExpressionTracks {
            tracks: Vec::new(),
            slots: Vec::new(),
            components: Vec::new(),
        };
        let facial = or_io::gta6::hash::name_hash32(b"facialTranslation");
        let set = Expressions {
            key: 7,
            inputs: ExpressionTracks {
                tracks: vec![(0xA1, facial), (0xA2, facial)],
                slots: vec![0, 1, 2],
                components: vec![1, 2],
            },
            bones: none(),
            bone_masks: Vec::new(),
            controls: none(),
            control_slots: Vec::new(),
            control_words: Vec::new(),
            facial_outputs: none(),
            outputs: None,
            output_masks: Vec::new(),
        };
        let d = ExpressionGroup::FacialOutputs;
        let (added, done) = edited(set.clone(), d, 0, Change::Add(0xA2)).expect("added");
        assert_eq!(added.facial_outputs.tracks, [(0xA2, facial)]);
        assert_eq!(added.facial_outputs.slots, [0, 1]);
        assert!(done.contains("hash_000000A2"), "{done}");
        for (track, change) in [
            (1, Change::Add(0xA1)),
            (0, Change::Id(5)),
            (0, Change::Remove),
        ] {
            assert!(edited(set.clone(), d, track, change).is_err(), "{change:?}");
        }
    }
}
