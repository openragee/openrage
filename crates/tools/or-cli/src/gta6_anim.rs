//! `rpf scan-open` on a GTA VI animation resource: what its reader states, structure and counts only.

use std::collections::BTreeMap;

use or_io::codec::anim::AnimationSource;
use or_io::gta6::anim::{
    known_name, read, read_stored, reencode, AnimAt, AnimResource, Animation, Motion, Record,
    RootKind, DELTA_SECTION,
};

use crate::project::Failure::{self, Run, Usage};

/// A quaternion this far from unit length is off it, unless a reconstruct clamped it.
const UNIT_SLACK: f32 = 0.01;
const CLAMP_SLACK: f32 = 0.05;

/// The resource whose virtual section is `section`, `stored` whole in an RSC8 or not.
pub(crate) fn print(section: &[u8], stored: bool) -> Result<(), String> {
    let resource = if stored {
        read_stored(section)
    } else {
        read(section)
    }
    .map_err(|e| format!("GTA VI's animation reader does not read it: {e}"))?;
    let root = match resource.root {
        RootKind::ClipDictionary => "clip dictionary",
        RootKind::AnimationSet => "animation set",
        RootKind::MotionDictionary => "motion dictionary",
    };
    println!(
        "  root          {root} {:?}, key {:#010X}, word {} at 0x6C",
        resource.name, resource.key, resource.root_word
    );
    if let Some([v, p]) = resource.block_map {
        println!("  block map     {v} virtual and {p} physical block(s)");
    }
    if resource.root == RootKind::ClipDictionary {
        println!(
            "  sets          {} in this body, {} stored in another body and named by key{}",
            resource.sets.len(),
            resource.external_sets.len(),
            keys(&resource.external_sets)
        );
        println!(
            "  key set       {} animation key(s)",
            resource.animation_keys.len()
        );
    }
    for (s, set) in resource.sets.iter().enumerate() {
        println!(
            "  set {s}         {:?}, key {:#010X}, {} animation(s)",
            set.name,
            set.key,
            set.animations.len()
        );
        for (a, animation) in set.animations.iter().enumerate() {
            print_animation(a, animation)?;
        }
    }
    print_clips(&resource);
    print_motions(&resource);
    crate::gta6_matcher::print_matchers(&resource);
    if let Some(region) = &resource.end_region {
        println!(
            "  end region    {} name(s) {:?}, then the word {:#010X}, whose role is not known",
            region.names.len(),
            region.names,
            region.word
        );
    }
    let (channels, blocks) = resource
        .sets
        .iter()
        .flat_map(|s| &s.animations)
        .flat_map(|a| &a.blocks)
        .fold((0, 0), |(c, b), block| {
            let n = usize::from(block.sections[DELTA_SECTION]);
            (c + n, b + usize::from(n > 0))
        });
    println!(
        "  section 10    {channels} channel(s) in {blocks} block(s), each decoded at every frame; a value between two frames is their linear blend, as GTA V's formats blend"
    );
    match reencode(section, stored) {
        Ok(written) => println!(
            "  re-encode     every block written through the channel encoder is {} the section as read",
            if written == section { "equal to" } else { "not equal to" }
        ),
        Err(e) => println!("  re-encode     not written: {e}"),
    }
    println!(
        "  timing        duration = (frames - 1) / rate; a clip stored beside its audio runs longer than the audio, so its duration is not the audio's length"
    );
    println!(
        "  not read      what a motion database's row halves mean, what a data attribute's bytes hold, and what attribute types 16, 18, 21 and 22 are for; any engine name"
    );
    Ok(())
}

/// An expression set's key and its tracks, counted by track.
pub(crate) fn print_expressions(section: &[u8]) -> Result<(), String> {
    let set = or_io::gta6::anim::read_expressions(section)
        .map_err(|e| format!("the expression reader does not read it: {e}"))?;
    let tally = |tracks: &or_io::gta6::anim::ExpressionTracks| {
        let mut by: BTreeMap<String, usize> = BTreeMap::new();
        for (_, track) in &tracks.tracks {
            *by.entry(or_io::gta6::anim::track_label(*track))
                .or_default() += 1;
        }
        let by: Vec<String> = by.into_iter().map(|(t, n)| format!("{n} {t}")).collect();
        format!(
            "{} track(s) ({}), {} slot(s)",
            tracks.tracks.len(),
            by.join(", "),
            tracks.slots.len()
        )
    };
    println!(
        "  expressions   key {:#010X}, whose name is not known",
        set.key
    );
    println!("  inputs        {}", tally(&set.inputs));
    println!("  bone inputs   {}", tally(&set.bones));
    println!(
        "  other inputs  {}, {} slot(s) more",
        tally(&set.controls),
        set.control_slots.len()
    );
    println!("  facial out    {}", tally(&set.facial_outputs));
    match &set.outputs {
        Some(outputs) => println!("  outputs       {}", tally(outputs)),
        None => println!("  outputs       none"),
    }
    println!(
        "  not stated    how what it reads becomes what it writes: the set holds no other data"
    );
    Ok(())
}

/// Each drawable entry's skeleton of a dictionary: its bones and those named as face-rig bones.
pub(crate) fn print_dictionary_skeletons(section: &[u8]) {
    match or_io::gta6::headerless::read_dictionary_skeletons(section) {
        Ok(skeletons) => {
            for (code, skeleton) in skeletons {
                let face = (skeleton.bones.iter())
                    .filter(|b| {
                        let name = b.name.to_ascii_lowercase();
                        ["face_", "facial_", "fb_"]
                            .iter()
                            .any(|p| name.starts_with(p))
                    })
                    .count();
                println!(
                    "  skeleton of entry {code:#010X}  {} bone(s), {face} of them named as face-rig bones",
                    skeleton.bones.len()
                );
            }
        }
        Err(e) => println!("  skeletons     not read: {e}"),
    }
}

/// Each stream's loudness records and the animation resource its chunk stores, as counts and names.
pub(crate) fn print_awc_chunks(awc: &or_io::awc::AwcFile) {
    for stream in &awc.streams {
        match stream.loudness() {
            Some(Ok(records)) => println!(
                "  stream {:#010X}  {} loudness record(s), positions {:?}",
                stream.id,
                records.len(),
                records
                    .first()
                    .zip(records.last())
                    .map(|(a, b)| [a.position, b.position])
            ),
            Some(Err(e)) => println!("  stream {:#010X}  loudness not read: {e}", stream.id),
            None => {}
        }
        let read = stream.animation().map(|rsc| {
            let rsc = rsc.map_err(|e| e.to_string())?;
            let section = &rsc.data()[..rsc.virtual_size().min(rsc.data().len())];
            read_stored(section).map_err(|e| e.to_string())
        });
        match read {
            Some(Ok(resource)) => {
                println!(
                "  stream {:#010X}  stores animation resource {:?}: {} clip(s), {} animation(s)",
                stream.id,
                resource.name,
                resource.clips.len(),
                resource.sets.iter().map(|s| s.animations.len()).sum::<usize>()
            )
            }
            Some(Err(e)) => println!(
                "  stream {:#010X}  its animation chunk is not read: {e}",
                stream.id
            ),
            None => {}
        }
    }
}

fn keys(keys: &[u32]) -> String {
    if keys.is_empty() {
        return String::new();
    }
    let shown: Vec<String> = keys.iter().take(8).map(|k| format!("{k:#010X}")).collect();
    let more = if keys.len() > 8 { ", ..." } else { "" };
    format!(": {}{more}", shown.join(", "))
}

fn print_animation(a: usize, animation: &Animation) -> Result<(), String> {
    let rate = animation
        .rate()
        .map_or_else(|| "no rate".to_string(), |r| format!("{r} fps"));
    let deltas = (0..animation.dofs.len())
        .filter(|&d| animation.in_delta_section(d))
        .count();
    println!(
        "    animation {a}  {:?}, key {:#010X}, flags {:#06X}, {} frames, {} s, {rate}, {} DOF(s), {deltas} of them with a channel in section {DELTA_SECTION}",
        animation.name,
        animation.key,
        animation.flags,
        animation.frames,
        animation.duration,
        animation.dofs.len()
    );
    for (d, dof) in animation.dofs.iter().enumerate() {
        let track = dof
            .track_name()
            .map_or_else(|| format!("hash_{:08X}", dof.track), str::to_string);
        println!(
            "      dof {d}  id hash_{:08X}, track {track}, type {}",
            dof.id, dof.kind
        );
    }
    for (b, block) in animation.blocks.iter().enumerate() {
        println!(
            "      block {b}  channels per directory section {:?}",
            block.sections
        );
    }
    let (samples, off) = quaternions(animation)?;
    if samples > 0 {
        println!(
            "      quaternions with a section-10 component: {samples} sampled at whole frames, {off} off unit length by more than {UNIT_SLACK}"
        );
    }
    Ok(())
}

/// Each type-0 DOF with a section-10 channel, sampled at every frame of each block that holds it:
/// the samples, and those off unit length.
fn quaternions(animation: &Animation) -> Result<(usize, usize), String> {
    let (mut samples, mut off) = (0, 0);
    let chunk = usize::from(animation.frames_per_chunk);
    let mut sampler = animation.sampler();
    // Frame by frame, so the sampler reads each frame's words once for every DOF.
    for (b, block) in animation.blocks.iter().enumerate() {
        let held: Vec<usize> = (0..animation.dofs.len())
            .filter(|&d| animation.dofs[d].kind == 0 && block.holds_delta(d))
            .collect();
        for f in 0..animation.frames_in_block(b) {
            let time = animation.internal_frame_to_time((b * chunk + f) as f32);
            for &d in &held {
                let value = sampler.sample(d, time).map_err(|e| e.to_string())?;
                let length = value.as_slice().iter().map(|x| x * x).sum::<f32>().sqrt();
                samples += 1;
                let clamped = value.as_slice().contains(&0.0) && length <= 1.0 + CLAMP_SLACK;
                off += usize::from((1.0 - length).abs() > UNIT_SLACK && !clamped);
            }
        }
    }
    Ok((samples, off))
}

fn named(hash: u32) -> String {
    known_name(hash).map_or_else(
        || format!("hash_{hash:08X}"),
        |n| format!("{n} ({hash:#010X})"),
    )
}

/// Records by name, and attributes by type word, over `records`.
fn tally<'a>(
    records: impl Iterator<Item = &'a Record>,
) -> (BTreeMap<String, usize>, BTreeMap<u16, usize>) {
    let (mut names, mut types) = (BTreeMap::new(), BTreeMap::new());
    for record in records {
        *names.entry(named(record.name)).or_insert(0) += 1;
        for attribute in &record.attributes {
            *types.entry(attribute.value.type_word()).or_insert(0) += 1;
        }
    }
    (names, types)
}

fn print_tally(what: &str, records: Vec<&Record>) {
    let count = records.len();
    let (names, types) = tally(records.into_iter());
    let names: Vec<String> = names.iter().map(|(n, c)| format!("{n} {c}")).collect();
    let types: Vec<String> = types.iter().map(|(t, c)| format!("type {t} {c}")).collect();
    println!("  {what:<13} {count}: {}", names.join(", "));
    println!("    attributes  {}", types.join(", "));
}

fn print_clips(resource: &AnimResource) {
    if resource.root != RootKind::ClipDictionary {
        return;
    }
    println!("  clips         {}", resource.clips.len());
    let name = |at: &AnimAt| format!("set {} animation {}", at.set, at.animation);
    for clip in &resource.clips {
        use or_io::gta6::anim::ClipPlay;
        let plays = match &clip.play {
            ClipPlay::One {
                animation,
                key,
                start,
                end,
                rate,
            } => format!(
                "{} (key {key:#010X}), {start} to {end} s at rate {rate}",
                name(animation)
            ),
            ClipPlay::Groups { total, groups } => {
                let entries: usize = groups.iter().map(|g| g.entries.len()).sum();
                let external = groups
                    .iter()
                    .flat_map(|g| &g.entries)
                    .filter(|e| e.animation.is_none())
                    .count();
                format!(
                    "{} group(s), {entries} entries ({external} in sets another body stores), {total} s in all",
                    groups.len()
                )
            }
            ClipPlay::Unresolved => {
                "an entry points at no animation of the sets, so what it plays is not known".into()
            }
        };
        println!(
            "    {:?}  {plays}; key {:#018X}, {} propert(ies), {} tag(s)",
            clip.name,
            clip.key,
            clip.properties.len(),
            clip.tags.len()
        );
    }
    print_tally(
        "tags",
        resource.clips.iter().flat_map(|c| &c.tags).collect(),
    );
    print_tally(
        "properties",
        resource.clips.iter().flat_map(|c| &c.properties).collect(),
    );
}

fn print_motions(resource: &AnimResource) {
    if resource.root != RootKind::MotionDictionary {
        return;
    }
    println!(
        "  motions       {}, drawing on {} clip dictionar(ies) by key{}",
        resource.motions.len(),
        resource.dictionaries.len(),
        keys(&resource.dictionaries)
    );
    let mut curves = 0;
    for motion in &resource.motions {
        curves += print_motion(motion);
    }
    println!("  curves        {curves} (time warps and phase curves)");
    let tags: Vec<&Record> = resource
        .motions
        .iter()
        .flat_map(|m| m.groups.iter().flat_map(|g| &g.tags))
        .collect();
    print_tally("tags", tags);
    print_tally(
        "properties",
        resource
            .motions
            .iter()
            .flat_map(|m| &m.properties)
            .collect(),
    );
}

/// One motion's line; returns its curves.
fn print_motion(motion: &Motion) -> usize {
    use or_io::gta6::anim::Typed;
    let groups: Vec<String> = motion
        .groups
        .iter()
        .map(|g| {
            format!(
                "clips {}+{}, lists {}+{}, curve type {}, range type {}",
                g.first_clip,
                g.clips,
                g.first_weight_list,
                g.weight_lists,
                g.curve.type_word(),
                g.range.type_word()
            )
        })
        .collect();
    let objects = &motion.parameter_objects;
    println!(
        "    motion {:?}  key {:#010X}, kind {}, {} clip(s) by key, {} weight list(s), {} parameter(s), parameter objects of types {objects:?}; groups: {}",
        motion.name,
        motion.key,
        motion.kind,
        motion.clips.len(),
        motion.weight_lists.len(),
        motion.parameters.len(),
        groups.join("; ")
    );
    for (g, group) in motion.groups.iter().enumerate() {
        match &group.range {
            Typed::Rows(db) => println!(
                "      group {g} database  {} key(s), {} table(s) of {} row(s) in all, {} grid(s), {} weight list(s); a row is four items of four f16 halves, whose meaning is not known; columns {}",
                db.keys.len(),
                db.tables.len(),
                db.tables.iter().map(Vec::len).sum::<usize>(),
                db.grids.len(),
                db.lists,
                column_names(motion, &db.keys)
            ),
            Typed::Table(db) => println!(
                "      group {g} database  type 5, {} row(s), {} tetrahedra; a row is 16 f16 halves, whose meaning is not known; columns {}",
                db.rows.len(),
                db.tetrahedra.len(),
                column_names(motion, &db.columns)
            ),
            Typed::Simplices(db) => println!(
                "      group {g} database  type 8, {} table(s) of {} row(s) in all, {} simplices, {} weight list(s); a row is 16 f16 halves, whose meaning is not known; columns {}",
                db.tables.len(),
                db.tables.iter().map(Vec::len).sum::<usize>(),
                db.simplices.len(),
                db.lists,
                column_names(motion, &db.columns)
            ),
            _ => {}
        }
    }
    let phase = |t: &Typed| match t {
        Typed::PhaseCurves(c) => c.len(),
        _ => 0,
    };
    motion.time_warps.len()
        + motion
            .groups
            .iter()
            .map(|g| phase(&g.curve) + phase(&g.range))
            .sum::<usize>()
}

/// Each column of a database of `motion` as the parameter and component it names.
fn column_names(motion: &Motion, columns: &[or_io::gta6::anim::Column]) -> String {
    let mut shown = Vec::new();
    for &(hash, words) in columns {
        let column = or_io::gta6::anim::column_parameter(&motion.properties, (hash, words));
        shown.push(column.map_or_else(|_| format!("{hash:#010X} {words:?}"), |p| p.label(named)));
    }
    shown.join(", ")
}

/// `rpf scan-anim`: an animation body as a JSON report, or bound to the skeleton of a scanned
/// drawable or fragment body, with the binding report and a pose, as `inspect anim` does a `.ycd`'s.
pub(crate) fn cmd_scan_anim(args: &[String], limit: u64) -> Result<(), Failure> {
    const USAGE: &str = "usage: rpf scan-anim <archive.rpf> <offset> [--json] [--on <archive.rpf> <offset> [--clip <name>] [--at <seconds>]]";
    let mut rest = Vec::new();
    let (mut json, mut on, mut clip, mut at) = (false, None, None, None);
    let mut i = 0;
    while i < args.len() {
        let next = |k: usize| (args.get(i + k).cloned()).ok_or_else(|| Usage(USAGE.to_string()));
        match args[i].as_str() {
            "--json" => json = true,
            "--on" => {
                on = Some((next(1)?, next(2)?));
                i += 2;
            }
            "--clip" => {
                clip = Some(next(1)?);
                i += 1;
            }
            "--at" => {
                let word = next(1)?;
                at = Some(
                    word.parse::<f32>()
                        .map_err(|_| Usage(format!("--at takes seconds, not {word}")))?,
                );
                i += 1;
            }
            other => rest.push(other.to_string()),
        }
        i += 1;
    }
    let [path, offset] = rest.as_slice() else {
        return Err(Usage(USAGE.to_string()));
    };
    for word in std::iter::once(offset).chain(on.iter().map(|(_, offset)| offset)) {
        super::rpf::parse_offset(word).map_err(Usage)?;
    }
    let (section, stored) = animation_section(path, offset, limit)?;
    let resource = if stored {
        read_stored(&section)
    } else {
        read(&section)
    }
    .map_err(|e| format!("GTA VI's animation reader does not read it: {e}"))?;
    if json {
        let report = serde_json::to_string_pretty(&report(&resource)).map_err(|e| e.to_string())?;
        println!("{report}");
    }
    match on {
        Some((archive, offset)) => {
            print_binding(&resource, &archive, &offset, limit, clip.as_deref(), at).map_err(Run)
        }
        None if json => Ok(()),
        None => Err(Usage(format!(
            "{USAGE}: say --json, or --on and the body of a skeleton"
        ))),
    }
}

fn body(
    path: &str,
    offset: &str,
    limit: u64,
) -> Result<(or_io::container::rpf::Rpf9ScanFind, Vec<u8>), String> {
    let offset = super::rpf::parse_offset(offset)?;
    let file = std::fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    or_io::container::rpf::read_rpf9_body(&file, offset, limit).map_err(|e| e.to_string())
}

/// The virtual section of the animation resource at `offset`, and whether it is stored whole in an RSC8.
fn animation_section(path: &str, offset: &str, limit: u64) -> Result<(Vec<u8>, bool), String> {
    use or_io::container::rpf::{rsc8::Rsc8, Rpf9BodyKind, Rpf9Content};
    let (find, bytes) = body(path, offset, limit)?;
    match find.kind {
        Rpf9BodyKind::Zlib(run) if run.content == Rpf9Content::ClipRoot => {
            let resource = run.resource.ok_or("this run starts no headerless resource")?;
            Ok((resource.sections(&bytes).0.to_vec(), false))
        }
        Rpf9BodyKind::StoredRsc8 { version: or_io::gta6::anim::RSC8_VERSION, .. } => {
            let rsc = Rsc8::open(&bytes).map_err(|e| e.to_string())?;
            Ok((rsc.data()[..rsc.virtual_size().min(rsc.data().len())].to_vec(), true))
        }
        _ => Err("the body there is no GTA VI animation resource: a zlib clip root or an RSC8 of version 65".into()),
    }
}

/// The skeleton of the drawable or fragment body at `offset`.
fn skeleton_at(
    path: &str,
    offset: &str,
    limit: u64,
) -> Result<or_io::gta6::headerless::Skeleton, String> {
    use or_io::container::rpf::{Rpf9BodyKind, Rpf9Role};
    let (find, bytes) = body(path, offset, limit)?;
    let Rpf9BodyKind::Zlib(run) = find.kind else {
        return Err("the body there is no headerless resource".into());
    };
    let resource = run
        .resource
        .ok_or("this run starts no headerless resource")?;
    let section = resource.sections(&bytes).0;
    // A body of no known role is read as a drawable, then as a fragment.
    let fragment = run.signature.and_then(|s| s.role()) == Some(Rpf9Role::DrawableWithBound);
    let read = |fragment| or_io::gta6::headerless::read_skeleton(section, fragment);
    let skeleton = match read(fragment) {
        Ok(Some(s)) => Some(s),
        _ if !fragment => read(true).ok().flatten(),
        _ => None,
    };
    skeleton
        .ok_or_else(|| "the body there states no skeleton GTA VI's skeleton reader reads".into())
}

/// An animation a clip plays, its window's start and its time, in seconds of the animation.
type Layer<'a> = (&'a Animation, f32, f32);

/// Each animation `resource`'s clip `name`, or its first, plays at `time` of the clip, and the
/// keys of the entries another body stores.
fn layers_of<'a>(
    resource: &'a AnimResource,
    name: Option<&str>,
    time: f32,
) -> Result<(String, Vec<Layer<'a>>, Vec<u32>), String> {
    use or_io::gta6::anim::ClipPlay;
    let animation = |at: AnimAt| {
        resource
            .sets
            .get(at.set)
            .and_then(|s| s.animations.get(at.animation))
    };
    let clip = match name {
        Some(name) => resource.clips.iter().find(|c| {
            c.name == name
                || c.name.ends_with(&format!("/{name}"))
                || c.name == format!("pack:/{name}")
        }),
        None => resource.clips.first(),
    };
    let Some(clip) = clip else {
        let found = resource
            .sets
            .iter()
            .flat_map(|s| &s.animations)
            .find(|a| name.is_none_or(|n| a.name.contains(n)));
        let a =
            found.ok_or_else(|| format!("no clip or animation named {}", name.unwrap_or("")))?;
        return Ok((
            a.name.clone(),
            vec![(a, 0.0, time.clamp(0.0, a.duration))],
            Vec::new(),
        ));
    };
    let mut layers = Vec::new();
    let mut waiting = Vec::new();
    match &clip.play {
        ClipPlay::One {
            animation: at,
            start,
            end,
            rate,
            ..
        } => {
            let a = animation(*at).ok_or("the clip's animation is not in its sets")?;
            let t = (start + rate * time).clamp(start.min(*end), end.max(*start));
            layers.push((a, *start, t));
        }
        ClipPlay::Groups { groups, .. } => {
            for layer in or_anim::gta6::layers_at(groups, time) {
                let entry = &groups[layer.group].entries[layer.entry];
                match entry.animation.and_then(animation) {
                    Some(a) => layers.push((a, entry.start, layer.time)),
                    None => waiting.push(entry.key),
                }
            }
        }
        ClipPlay::Unresolved => {
            return Err(format!(
                "clip {:?} points at no animation of the sets",
                clip.name
            ))
        }
    }
    Ok((clip.name.clone(), layers, waiting))
}

fn print_binding(
    resource: &AnimResource,
    archive: &str,
    offset: &str,
    limit: u64,
    clip: Option<&str>,
    at: Option<f32>,
) -> Result<(), String> {
    use or_anim::gta6::{pose_layers, posed_matrices, Binding, Skipped};
    let source = skeleton_at(archive, offset, limit)?;
    let skeleton = or_anim::gta6::skeleton(&source);
    let (name, layers, waiting) = layers_of(resource, clip, at.unwrap_or(0.0))?;
    println!("\n=== ANIMATION BINDING ===\n");
    println!(
        "Skeleton:       {} bones, from {archive} at {offset}",
        source.bones.len()
    );
    println!("Clip:           {name}");
    if !waiting.is_empty() {
        println!(
            "  {} layer(s) play animations of sets another body stores, not bound: {}",
            waiting.len(),
            keys(&waiting)
        );
    }
    let bindings: Vec<Binding> = layers
        .iter()
        .map(|(a, _, _)| Binding::build(&source, a))
        .collect();
    for ((animation, start, time), binding) in layers.iter().zip(&bindings) {
        let (mut none, mut other) = (0, 0);
        for s in &binding.skipped {
            match s {
                Skipped::NoSuchBone { .. } => none += 1,
                Skipped::NotABoneTrack { .. } => other += 1,
            }
        }
        println!("{}", animation.name);
        println!(
            "  {} of {} DOF(s) move {} bone(s); {none} name no bone of the skeleton, {other} are no bone's rotation, translation or scale",
            animation.dofs.len() - binding.skipped.len() - usize::from(binding.mover_translation.is_some()) - usize::from(binding.mover_rotation.is_some()),
            animation.dofs.len(),
            binding.bones.len(),
        );
        let mover = |d: Option<usize>| d.map_or_else(|| "none".to_string(), |d| format!("DOF {d}"));
        println!(
            "  mover         translation {}, rotation {}",
            mover(binding.mover_translation),
            mover(binding.mover_rotation)
        );
        println!("  window        from {start} s, at {time} s");
    }
    let Some(at) = at else {
        return Ok(());
    };
    let posed: Vec<_> = (bindings.iter().zip(&layers))
        .map(|(b, (a, start, time))| (b, *a, *start, *time))
        .collect();
    let pose = pose_layers(&skeleton, &posed).map_err(|e| e.to_string())?;
    let (world, _) = posed_matrices(&pose, &skeleton);
    let mut bones: Vec<u16> = bindings
        .iter()
        .flat_map(|b| b.bones.iter().map(|d| d.bone))
        .collect();
    bones.sort_unstable();
    bones.dedup();
    println!("\nPose at {at} s of the clip, each posed bone in the model's space:");
    for bone in bones {
        let (Some(b), Some(m)) = (
            source.bones.get(usize::from(bone)),
            world.get(usize::from(bone)),
        ) else {
            continue;
        };
        let p = m.w_axis;
        println!("  {:<24} ({:.4}, {:.4}, {:.4})", b.name, p.x, p.y, p.z);
    }
    let root = pose.root_motion.translation;
    println!(
        "  root motion   ({:.4}, {:.4}, {:.4})",
        root.x, root.y, root.z
    );
    Ok(())
}

/// What the reader states of `resource`, field by field, as `convert info --json` gives a `.ycd`'s.
fn report(resource: &AnimResource) -> serde_json::Value {
    use or_io::gta6::anim::ClipPlay;
    use serde_json::json;
    let root = match resource.root {
        RootKind::ClipDictionary => "clip dictionary",
        RootKind::AnimationSet => "animation set",
        RootKind::MotionDictionary => "motion dictionary",
    };
    let animation = |a: &Animation| {
        let dofs: Vec<_> = (a.dofs.iter().enumerate())
            .map(|(d, dof)| json!({ "id": dof.id, "track": dof.track_label(), "type": dof.kind, "section_10": a.in_delta_section(d) }))
            .collect();
        json!({
            "name": a.name, "key": a.key, "references": a.references, "flags": a.flags,
            "project_flags": a.project_flags, "signature": a.signature, "max_block_size": a.max_block_size,
            "frames": a.frames, "frames_per_chunk": a.frames_per_chunk, "duration": a.duration,
            "rate": a.rate(), "dofs": dofs,
            "blocks": a.blocks.iter().map(|b| b.sections.to_vec()).collect::<Vec<_>>(),
        })
    };
    let records = |r: &[Record]| r.iter().map(|r| named(r.name)).collect::<Vec<_>>();
    let clips: Vec<_> = (resource.clips.iter())
        .map(|c| {
            let play = match &c.play {
                ClipPlay::One { animation, key, start, end, rate } => json!({
                    "type": 1, "set": animation.set, "animation": animation.animation, "key": key,
                    "start": start, "end": end, "rate": rate,
                }),
                ClipPlay::Groups { total, groups } => json!({
                    "type": 2, "total": total,
                    "groups": groups.iter().map(|g| json!({
                        "duration": g.duration,
                        "entries": g.entries.iter().map(|e| json!({
                            "set": e.animation.map(|a| a.set), "animation": e.animation.map(|a| a.animation),
                            "key": e.key, "start": e.start, "end": e.end, "rate": e.rate,
                            "warp_points": e.warp_points,
                        })).collect::<Vec<_>>(),
                    })).collect::<Vec<_>>(),
                }),
                ClipPlay::Unresolved => json!({ "unresolved": true }),
            };
            json!({ "name": c.name, "key": c.key, "play": play, "tags": records(&c.tags), "properties": records(&c.properties) })
        })
        .collect();
    let motions: Vec<_> = (resource.motions.iter())
        .map(|m| json!({ "name": m.name, "key": m.key, "kind": m.kind, "clips": m.clips, "parameters": m.parameters.len(), "groups": m.groups.len() }))
        .collect();
    json!({
        "root": root, "name": resource.name, "key": resource.key, "root_word": resource.root_word,
        "word_at_60": resource.word_at_60, "word_at_98": resource.word_at_98,
        "block_map": resource.block_map,
        "sets": resource.sets.iter().map(|s| json!({
            "name": s.name, "key": s.key, "animations": s.animations.iter().map(animation).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "external_sets": resource.external_sets,
        "animation_keys": resource.animation_keys.len(),
        "clips": clips,
        "motions": motions,
        "matchers": crate::gta6_matcher::report(resource),
        "dictionaries": resource.dictionaries,
        "end_region": resource.end_region.as_ref().map(|r| json!({ "names": r.names, "word": r.word })),
    })
}
