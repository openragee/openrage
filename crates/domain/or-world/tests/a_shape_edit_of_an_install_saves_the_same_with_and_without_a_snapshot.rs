//! One type file in fifty of an install takes an archetype added by copy and removed again, one map in fifty an
//! entity removed and put back, the first interior of one type file in ten a room, a portal and an attachment added
//! by copy and removed again, and one `.ymt` in twenty, where it is a ped variation, a drawable added by copy, one
//! built there, and both removed again, each saved, read back and saved again, in a process that installed the staged
//! schema snapshot as the app installs it and in one that installed none, and the two processes write the same bytes.
//!
//! The install is `OPENRAGE_GAME_ROOT` (`;`-separated roots), opened read only. The process with no snapshot is this
//! binary run again as a child: an installed snapshot is state of the whole process and is never removed.
//!
//! ⚠ A map saved without the last record of a class no longer states that class, and an RBM class carries a signature
//! nothing derives: the record put back into the file as it was saved is refused for it where no snapshot is
//! installed. Those are counted apart and are no failure; the same record put back before the file is read again
//! saves the bytes read. They are also the one place the two processes may part: an installed snapshot still gives a
//! class the file's own table does not hold, so there the record is laid out by the snapshot's class, or refused for
//! a member that class lacks. How many do is counted and said.

mod support;

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_io::decl::File;
use or_io::meta::binary::synth::name_hash;
use or_io::meta::binary::type_tables;
use or_io::meta::typed::{ped_types, world_types};
use or_io::meta::{
    CBaseArchetypeDefAny, CMapData, CMapTypes, CPVDrawblData, CPVTextureData, CPedVariationInfo,
    FieldType, MetaDocument, SchemaLibrary,
};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV, SCHEMA_ENV};
use or_title::game::Game;

const CHILD_TEST: &str = "measure_for_a_parent_with_no_snapshot";
const CHILD_ENV: &str = "OPENRAGE_SHAPE_EDITS_CHILD";
const BEGIN: &str = "\u{1e}measure begins\u{1e}";
const END: &str = "\u{1e}measure ends\u{1e}";

/// One file in this many of its kind takes an archetype or loses an entity.
const ONE_IN: usize = 50;
/// One type file in this many is looked into for an interior to edit: about one type file in five holds one.
const ONE_IN_FOR_AN_INTERIOR: usize = 10;
/// One `.ymt` in this many is looked into for a ped variation to edit.
const ONE_IN_FOR_A_PED: usize = 20;

fn crypto_for(root: &Path) -> Option<Arc<dyn RpfCryptoProvider>> {
    let exe = Game::detect_from_root(root).find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    Some(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))))
}

/// What a save wrote, as its length and a digest two runs of this binary agree on.
fn written(bytes: &[u8]) -> String {
    // ⚠ `DefaultHasher::new` has fixed keys, so the parent and its child digest alike.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{}:{:016x}", bytes.len(), hasher.finish())
}

/// What one edit of one file came to.
enum Edited {
    /// What each save wrote.
    Saved(Vec<String>),
    /// The file holds nothing to edit.
    Nothing,
    /// The record put back into the file as saved without it is of a class that file no longer states: refused for
    /// the `signature` of an RBM class, or for a `member` the installed snapshot's class lacks.
    Unstated(&'static str, Vec<String>),
}

/// An edit, or the step that failed with its sentence.
type Outcome = Result<Edited, String>;

/// An edit of a file's bytes: its name, the class of the record it copies or takes, which kind of file takes it (a
/// type file is 0, a map 1 and a `.ymt` 2), one file in how many is sampled, and what makes it.
type Edit = (
    &'static str,
    &'static str,
    usize,
    usize,
    fn(&[u8]) -> Outcome,
);

/// What a record whose class the file does not state was refused for, where `refused` is such a refusal.
fn for_a_class_the_file_lacks(refused: &or_io::Error) -> Option<&'static str> {
    let why = refused.to_string();
    if why.contains("copied here cannot be saved into this file") && why.contains("signature") {
        Some("signature")
    } else if why.contains("does not state the class it is laid out as") {
        Some("member")
    } else {
        None
    }
}

fn step<T>(what: &str, done: Result<T, or_io::Error>) -> Result<T, String> {
    done.map_err(|e| format!("{what}: {e}"))
}

/// An archetype added by a copy of the first, then removed again.
fn add_and_remove_an_archetype(bytes: &[u8]) -> Outcome {
    let types = step("opens", CMapTypes::from_bytes(bytes))?;
    let Some(first) = types.archetypes.first().cloned() else {
        return Ok(Edited::Nothing);
    };
    let untouched = step("the file as read saves", types.to_bytes())?;
    let mut added = types.clone();
    added.archetypes.push(first);
    let with = step("the copy saves", added.to_bytes())?;
    // The copy taken out before the file is read again, as an undo does, the save is the one of the file as read.
    added.archetypes.pop();
    if step("the copy taken back saves", added.to_bytes())? != untouched {
        return Err("the copy taken back does not save the bytes of the file as read".into());
    }
    added.archetypes.push(types.archetypes[0].clone());
    let back = step("the copy reads back", CMapTypes::from_bytes(&with))?;
    if back != added {
        return Err("the copy reads back as another value".into());
    }
    if step("the copy saves again", back.to_bytes())? != with {
        return Err("the copy is not at its fixed point".into());
    }
    let mut removed = back;
    removed.archetypes.pop();
    let without = step("the removal saves", removed.to_bytes())?;
    let back = step("the removal reads back", CMapTypes::from_bytes(&without))?;
    if back != types {
        return Err("the removal reads back as another value than the file read".into());
    }
    if step("the removal saves again", back.to_bytes())? != without {
        return Err("the removal is not at its fixed point".into());
    }
    Ok(Edited::Saved(vec![written(&with), written(&without)]))
}

/// The last entity removed, then put back.
fn remove_and_put_back_an_entity(bytes: &[u8]) -> Outcome {
    let map = step("opens", CMapData::from_bytes(bytes))?;
    let mut removed = map.clone();
    let Some(taken) = removed.entities.pop() else {
        return Ok(Edited::Nothing);
    };
    // ⚠ A map whose last entity is the LOD parent of another is refused its removal, and rightly: not an edit to count.
    let last = removed.entities.len();
    let is_parent = (removed.entities.iter())
        .any(|e| e.flags & (1 << 3) == 0 && usize::try_from(e.parent_index) == Ok(last));
    if is_parent {
        return Ok(Edited::Nothing);
    }
    let untouched = step("the file as read saves", map.to_bytes())?;
    let without = step("the removal saves", removed.to_bytes())?;
    // The entity put back before the file is read again, as an undo does, the save is the one of the file as read.
    removed.entities.push(taken.clone());
    if step("the removal taken back saves", removed.to_bytes())? != untouched {
        return Err("the removal taken back does not save the bytes of the file as read".into());
    }
    removed.entities.pop();
    let back = step("the removal reads back", CMapData::from_bytes(&without))?;
    if back != removed {
        return Err("the removal reads back as another value".into());
    }
    if step("the removal saves again", back.to_bytes())? != without {
        return Err("the removal is not at its fixed point".into());
    }
    let mut restored = back;
    restored.entities.push(taken);
    let saved = restored.to_bytes();
    if let Some(what) = saved.as_ref().err().and_then(for_a_class_the_file_lacks) {
        return Ok(Edited::Unstated(what, vec![written(&without)]));
    }
    let with = step("the entity put back saves", saved)?;
    let back = step(
        "the entity put back reads back",
        CMapData::from_bytes(&with),
    )?;
    if back != map {
        return Err("the entity put back reads back as another value than the file read".into());
    }
    if step("the entity put back saves again", back.to_bytes())? != with {
        return Err("the entity put back is not at its fixed point".into());
    }
    Ok(Edited::Saved(vec![written(&without), written(&with)]))
}

/// The first interior of `types` that holds a room, and where it is.
fn first_interior(types: &CMapTypes) -> Option<usize> {
    types.archetypes.iter().position(
        |a| matches!(a, CBaseArchetypeDefAny::CMloArchetypeDef(mlo) if !mlo.rooms.is_empty()),
    )
}

/// The first interior that holds a room takes a copy of its first room, of its first portal where it holds one, and
/// an attachment of its first entity to the copied room where it holds one; then each is taken out again.
fn add_and_remove_a_room_a_portal_and_an_attachment(bytes: &[u8]) -> Outcome {
    let types = step("opens", CMapTypes::from_bytes(bytes))?;
    let Some(at) = first_interior(&types) else {
        return Ok(Edited::Nothing);
    };
    let untouched = step("the file as read saves", types.to_bytes())?;
    let mut added = types.clone();
    let CBaseArchetypeDefAny::CMloArchetypeDef(mlo) = &mut added.archetypes[at] else {
        return Err("the interior found is no interior".into());
    };
    let (rooms, portals) = (mlo.rooms.len(), mlo.portals.len());
    let mut room = mlo.rooms[0].clone();
    // ⚠ An attachment names an entity by its place in the interior, so one is added only where an entity stands there.
    if !mlo.entities.is_empty() && !room.attached_objects.contains(&0) {
        room.attached_objects.push(0);
    }
    mlo.rooms.push(room);
    if let Some(portal) = mlo.portals.first().cloned() {
        mlo.portals.push(portal);
    }
    let with = step("the copies save", added.to_bytes())?;
    let back = step("the copies read back", CMapTypes::from_bytes(&with))?;
    if back != added {
        return Err("the copies read back as another value".into());
    }
    if step("the copies save again", back.to_bytes())? != with {
        return Err("the copies are not at their fixed point".into());
    }
    // Each copy taken out before the file is read again, as an undo does, the save is the one of the file as read.
    let take_out = |edited: &mut CMapTypes| {
        if let CBaseArchetypeDefAny::CMloArchetypeDef(mlo) = &mut edited.archetypes[at] {
            mlo.rooms.truncate(rooms);
            mlo.portals.truncate(portals);
        }
    };
    take_out(&mut added);
    if step("the copies taken back save", added.to_bytes())? != untouched {
        return Err("the copies taken back do not save the bytes of the file as read".into());
    }
    let mut removed = back;
    take_out(&mut removed);
    let without = step("the removal saves", removed.to_bytes())?;
    let back = step("the removal reads back", CMapTypes::from_bytes(&without))?;
    if back != types {
        return Err("the removal reads back as another value than the file read".into());
    }
    if step("the removal saves again", back.to_bytes())? != without {
        return Err("the removal is not at its fixed point".into());
    }
    Ok(Edited::Saved(vec![written(&with), written(&without)]))
}

/// The drawables of the component slot `slot` of a ped variation.
fn drawables(info: &mut CPedVariationInfo, slot: usize) -> &mut Vec<CPVDrawblData> {
    &mut info.a_component_data3[slot].a_drawbl_data3
}

/// The first component slot of a ped variation that holds a drawable takes a copy of its first drawable, then a
/// drawable built here; then both are taken out again.
fn add_and_remove_a_drawable(bytes: &[u8]) -> Outcome {
    // ⚠ A `.ymt` is a ped variation only where its root says so: a scenario region or a creature's tags is no file to edit.
    let root = MetaDocument::parse(&SchemaLibrary::default(), bytes.to_vec())
        .map(|doc| name_hash(&doc.instance.type_name));
    if root.ok() != Some(name_hash("CPedVariationInfo")) {
        return Ok(Edited::Nothing);
    }
    let info = step("opens", CPedVariationInfo::from_bytes(bytes))?;
    let holds = |c: &or_io::meta::CPVComponentData| !c.a_drawbl_data3.is_empty();
    let Some(slot) = info.a_component_data3.iter().position(holds) else {
        return Ok(Edited::Nothing);
    };
    let held = info.a_component_data3[slot].a_drawbl_data3.len();
    let untouched = step("the file as read saves", info.to_bytes())?;
    let mut added = info.clone();
    let copy = drawables(&mut added, slot)[0].clone();
    drawables(&mut added, slot).push(copy.clone());
    added.restate_counts();
    let with = step("the copy saves", added.to_bytes())?;
    // The copy taken out before the file is read again, as an undo does, the save is the one of the file as read.
    drawables(&mut added, slot).pop();
    added.restate_counts();
    if step("the copy taken back saves", added.to_bytes())? != untouched {
        return Err("the copy taken back does not save the bytes of the file as read".into());
    }
    drawables(&mut added, slot).push(copy);
    added.restate_counts();
    let back = step("the copy reads back", CPedVariationInfo::from_bytes(&with))?;
    if back != added {
        return Err("the copy reads back as another value".into());
    }
    if step("the copy saves again", back.to_bytes())? != with {
        return Err("the copy is not at its fixed point".into());
    }
    // ⚠ A record built here holds no tree: it is conformed to a class, and the root leaves its layout for a synthesised one.
    let mut built = back;
    let mut texture = CPVTextureData::default();
    texture.distribution = 255;
    let mut drawable = CPVDrawblData::default();
    drawable.a_tex_data.push(texture);
    drawables(&mut built, slot).push(drawable);
    built.restate_counts();
    let with_built = step("the drawable built here saves", built.to_bytes())?;
    let back = step(
        "the drawable built here reads back",
        CPedVariationInfo::from_bytes(&with_built),
    )?;
    if back != built {
        return Err("the drawable built here reads back as another value".into());
    }
    if step("the drawable built here saves again", back.to_bytes())? != with_built {
        return Err("the drawable built here is not at its fixed point".into());
    }
    let mut removed = back;
    drawables(&mut removed, slot).truncate(held);
    removed.restate_counts();
    let without = step("the removal saves", removed.to_bytes())?;
    let back = step(
        "the removal reads back",
        CPedVariationInfo::from_bytes(&without),
    )?;
    if back != info {
        return Err("the removal reads back as another value than the file read".into());
    }
    if step("the removal saves again", back.to_bytes())? != without {
        return Err("the removal is not at its fixed point".into());
    }
    Ok(Edited::Saved(vec![
        written(&with),
        written(&with_built),
        written(&without),
    ]))
}

/// Whether the type table of `bytes` states a class the ped family declares in code with another set of members than
/// the declaration, which is what a save that leaves its layout lays the class out by; `None` where it states none.
fn states_a_ped_class_otherwise_than_declared(bytes: &[u8]) -> Option<bool> {
    let members = |class: &or_io::meta::StructDef| {
        // The bytes of a class no member names are no member.
        let named = (class.fields.iter()).filter(|f| !matches!(f.ty, FieldType::Pad(_)));
        let mut members: Vec<u32> = named.map(|f| name_hash(&f.name)).collect();
        members.sort_unstable();
        members
    };
    let doc = MetaDocument::parse(&SchemaLibrary::default(), bytes.to_vec()).ok()?;
    let tables = type_tables::of_layout(doc.layout.as_ref()?);
    let declared = ped_types::declared();
    let mut apart = None;
    for (name, stated) in &tables.structs {
        let key = name_hash(name);
        let found = (declared.structs.iter()).find(|(declared, _)| name_hash(declared) == key);
        if let Some((_, declared)) = found {
            apart = Some(apart.unwrap_or(false) || members(stated) != members(declared));
        }
    }
    apart
}

/// What `world_types::first_member_missing` rests on, said of a file of the install: a tree as it was read lacks no
/// member its own table states, so a save that fails is made again as one of a copy only where a copy is held.
fn lacks_a_member_as_read(bytes: &[u8]) -> Option<String> {
    let doc = MetaDocument::parse(&SchemaLibrary::default(), bytes.to_vec()).ok()?;
    let (class, member) = world_types::first_member_missing(doc.layout.as_ref()?, &doc.instance)?;
    Some(format!(
        "the file as read lacks {member:#010X}, a member its own table states for `{class}`"
    ))
}

/// Whether the type table of `bytes` states `class` with another set of members than the installed snapshot declares
/// for it; `None` where the table states no such class or the snapshot declares none.
fn states_other_members_than_the_snapshot(bytes: &[u8], class: &str) -> Option<bool> {
    let members = |class: &[String]| {
        let mut members: Vec<u32> = class.iter().map(|name| name_hash(name)).collect();
        members.sort_unstable();
        members
    };
    let declared = or_io::meta::default_library().resolve_any(class)?;
    let declared: Vec<String> = declared.fields.iter().map(|f| f.name.clone()).collect();
    let doc = MetaDocument::parse(&SchemaLibrary::default(), bytes.to_vec()).ok()?;
    let tables = type_tables::of_layout(doc.layout.as_ref()?);
    let key = name_hash(class);
    let stated = tables
        .structs
        .iter()
        .find(|(name, _)| name_hash(name) == key)?;
    let stated: Vec<String> = stated.1.fields.iter().map(|f| f.name.clone()).collect();
    Some(members(&stated) != members(&declared))
}

/// What the type table of a sampled file is held against: which files are counted, and what a counted file that
/// differs does.
type Against = (&'static str, &'static str);

const THE_SNAPSHOT: Against = (
    "states the class of the record",
    "state it with another set of members than the staged snapshot declares",
);
const THE_DECLARATION: Against = (
    "states a class the ped family declares in code",
    "state one with another set of members than its declaration",
);

/// How many sampled files of an install are counted against a source of classes, and how many of those differ from
/// it, by install, edit and source.
type Differing = BTreeMap<(String, String, Against), (usize, usize)>;

/// One line for each sampled file of each install: the install, the edit, the file and what the edit came to.
fn measure() -> Option<(Vec<String>, Differing)> {
    let mut lines = Vec::new();
    let mut differing = Differing::new();
    let mut mounted = 0;
    for root in game_roots() {
        let Some(install) = or_title::GtaInstall::from_path(&root) else {
            continue;
        };
        let Some(crypto) = crypto_for(&root) else {
            continue;
        };
        let Ok(game) = or_title::build_mounted_game(&install, crypto, or_title::DlcCutoff::All)
        else {
            continue;
        };
        mounted += 1;
        let mut files: [BTreeMap<String, String>; 3] = Default::default();
        game.for_each_file(|logical, _container, leaf| {
            let leaf = leaf.to_ascii_lowercase();
            let kinds = [".ytyp", ".ymap", ".ymt"];
            let Some(kind) = kinds.iter().position(|ext| leaf.ends_with(ext)) else {
                return;
            };
            files[kind].insert(logical.to_ascii_lowercase(), logical.to_string());
        });
        let edits: [Edit; 4] = [
            (
                "archetype",
                "CBaseArchetypeDef",
                0,
                ONE_IN,
                add_and_remove_an_archetype,
            ),
            (
                "entity",
                "CEntityDef",
                1,
                ONE_IN,
                remove_and_put_back_an_entity,
            ),
            (
                "interior",
                "CMloRoomDef",
                0,
                ONE_IN_FOR_AN_INTERIOR,
                add_and_remove_a_room_a_portal_and_an_attachment,
            ),
            (
                "drawable",
                "CPVDrawblData",
                2,
                ONE_IN_FOR_A_PED,
                add_and_remove_a_drawable,
            ),
        ];
        for (edit, class, kind, one_in, apply) in edits {
            for path in files[kind].values().step_by(one_in) {
                let bytes = game.vfs.open(path);
                let other = (bytes.as_deref())
                    .and_then(|bytes| states_other_members_than_the_snapshot(bytes, class));
                let apart = (bytes.as_deref())
                    .filter(|_| edit == "drawable")
                    .and_then(states_a_ped_class_otherwise_than_declared);
                for (other, against) in [(other, THE_SNAPSHOT), (apart, THE_DECLARATION)] {
                    if let Some(other) = other {
                        let key = (root.display().to_string(), edit.to_string(), against);
                        let (stating, differ) = differing.entry(key).or_default();
                        *stating += 1;
                        *differ += usize::from(other);
                    }
                }
                let outcome = match bytes {
                    Some(bytes) => match lacks_a_member_as_read(&bytes) {
                        Some(lacks) => Err(lacks),
                        None => apply(&bytes),
                    },
                    None => Err("does not open".into()),
                };
                let outcome = match outcome {
                    Ok(Edited::Saved(saves)) => format!("ok {}", saves.join(" ")),
                    Ok(Edited::Nothing) => "none".into(),
                    Ok(Edited::Unstated(what, saves)) => {
                        format!("unstated-{what} {}", saves.join(" "))
                    }
                    Err(why) => format!("failed {}", why.replace(['\n', '\t'], " ")),
                };
                lines.push(format!("{}\t{edit}\t{path}\t{outcome}", root.display()));
            }
        }
    }
    (mounted > 0).then_some((lines, differing))
}

/// Not a test of its own: the measure a parent runs in a child that installs no snapshot.
#[test]
fn measure_for_a_parent_with_no_snapshot() {
    use std::io::Write;
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    assert!(
        !or_io::meta::has_default_library(),
        "the child installs no snapshot"
    );
    let (lines, _) = measure().unwrap_or_default();
    assert!(!or_io::meta::has_default_library());
    let mut out = std::io::stdout();
    let _ = write!(out, "{BEGIN}{}{END}", lines.join("\n"));
    let _ = out.flush();
}

fn measured_with_no_snapshot() -> Vec<String> {
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, "1")
        .env(SCHEMA_ENV, "none")
        .output()
        .expect("this test binary runs as a child");
    let text = String::from_utf8_lossy(&output.stdout);
    let error = String::from_utf8_lossy(&output.stderr);
    let (_, rest) = text
        .split_once(BEGIN)
        .unwrap_or_else(|| panic!("the child printed no start marker: {text}{error}"));
    let (printed, _) = rest
        .split_once(END)
        .unwrap_or_else(|| panic!("the child panicked: {rest}{error}"));
    printed.lines().map(str::to_string).collect()
}

/// What the sampled files of one install came to under one edit.
#[derive(Default)]
struct Tally {
    saved: usize,
    nothing: usize,
    unstated: usize,
    failed: Vec<String>,
}

fn tally(lines: &[String]) -> BTreeMap<(String, String), Tally> {
    let mut tally: BTreeMap<_, Tally> = BTreeMap::new();
    for line in lines {
        let mut parts = line.splitn(4, '\t');
        let (Some(install), Some(edit), Some(_), Some(outcome)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            panic!("a line of the measure has four parts: {line}");
        };
        let entry = tally
            .entry((install.to_string(), edit.to_string()))
            .or_default();
        match outcome.split(' ').next() {
            Some("none") => entry.nothing += 1,
            Some("ok") => entry.saved += 1,
            Some(unstated) if unstated.starts_with("unstated-") => entry.unstated += 1,
            _ => entry.failed.push(line.clone()),
        }
    }
    tally
}

/// What `lines` came to, said once for each install and edit, and every line that failed.
fn report(how: &str, lines: &[String]) -> Vec<String> {
    let mut failed = Vec::new();
    for ((install, edit), of) in tally(lines) {
        let edited = of.saved + of.unstated + of.failed.len();
        eprintln!(
            "{how}: {install}: {edit}: of {edited} sampled files edited, {} saved, read back and reached their fixed point at each step, {} were refused the record put back for a class the file no longer states, {} failed ({} more held nothing to edit)",
            of.saved,
            of.unstated,
            of.failed.len(),
            of.nothing
        );
        // The sentences that failed, each once with how many files it failed, whatever the file.
        let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
        for line in &of.failed {
            let reason = line.rsplit('\t').next().unwrap_or_default();
            // An index between brackets is of the file, and is left out.
            let mut bracketed = false;
            let reason = reason.chars().filter(|c| {
                bracketed = (bracketed || *c == '[') && *c != ']';
                !(bracketed && c.is_ascii_digit())
            });
            *reasons.entry(reason.take(240).collect()).or_default() += 1;
        }
        for (reason, files) in reasons {
            eprintln!("    {files} files: {reason}");
        }
        // A sample that edited nothing would pass with no edit made. ⚠ Said with the failures at the end, and never by
        // an assertion here: the tallies of the edits after this one are what tells one cause from another.
        if edited < 10 || of.saved < 10 {
            failed.push(format!(
                "{how}: {install}: {edit}: {edited} files edited and {} saved, where a sample edits and saves ten at least",
                of.saved
            ));
        }
        failed.extend(of.failed);
    }
    failed
}

/// ⚠ The install is opened read only and nothing here writes into one.
#[test]
fn a_sample_of_an_installs_typed_files_takes_a_shape_edit_with_and_without_the_staged_snapshot() {
    const WHAT: &str =
        "a_sample_of_an_installs_typed_files_takes_a_shape_edit_with_and_without_the_staged_snapshot";
    if std::env::var_os(CHILD_ENV).is_some() {
        return;
    }
    if game_roots().is_empty() {
        skip_install(
            WHAT,
            format_args!(
                "no install root; set {GAME_ROOT_ENV} to a ;-separated list of install directories"
            ),
        );
        return;
    }
    let without = measured_with_no_snapshot();
    if without.is_empty() {
        skip_install(WHAT, "no install root mounted");
        return;
    }
    let mut failed = report("with no snapshot", &without);

    if support::schema_or_skip(WHAT).is_some() {
        assert!(or_io::meta::has_default_library());
        let (with, differing) = measure().expect("the install the child mounted mounts here");
        failed.extend(report("with the staged snapshot", &with));
        // What the run with the snapshot proves of an install: nothing more than the run with none, where no sampled
        // file states a class otherwise than the snapshot declares it.
        for ((install, edit, (counted, differs)), (stating, differ)) in &differing {
            eprintln!(
                "{install}: {edit}: of {stating} sampled files whose type table {counted}, {differ} {differs}"
            );
        }
        assert_eq!(with.len(), without.len(), "the same sample both times");
        let mut parted: BTreeMap<String, usize> = BTreeMap::new();
        for (with, without) in with.iter().zip(&without) {
            if with == without {
                continue;
            }
            // ⚠ The one place the two may part: the first save is the same, and the record put back is of a class
            // the file no longer states, which the process with no snapshot refused.
            let outcome = |line: &str| {
                let outcome = line.splitn(4, '\t').nth(3).unwrap_or_default();
                let mut parts = outcome.split(' ');
                (
                    parts.next().unwrap_or_default().to_string(),
                    parts.next().unwrap_or_default().to_string(),
                )
            };
            let ((how, first_save), (how_without, first_save_without)) =
                (outcome(with), outcome(without));
            let was_refused = how_without == "unstated-signature";
            if was_refused && first_save == first_save_without && how != "failed" {
                *parted.entry(how).or_insert(0usize) += 1;
            } else {
                failed.push(format!(
                    "the snapshot changed what was written:\n    with:    {with}\n    without: {without}"
                ));
            }
        }
        eprintln!(
            "of {} lines, {} part between the two processes, each at a record put back into a file that no longer states its class and refused for its signature with no snapshot: {parted:?} with one",
            with.len(),
            parted.values().sum::<usize>()
        );
    }
    assert!(
        failed.is_empty(),
        "{} problems:\n  {}",
        failed.len(),
        failed
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
