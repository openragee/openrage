//! `openrage-cli maps changes`: what each map file and type file of a project's maps content changed of the base its source keeps,
//! by object (`or_maps::changes`).

use std::fmt::Write as _;
use std::process::ExitCode;

use or_maps::changes::{
    EntityChange, EntityRef, MapChanges, Matched, MemberChange, PackSources, SourceChanges, Stated,
    TypesChanges,
};
use or_maps::facts::{Fact, Name};
use or_maps::{Depth, Maps, Role};
use serde_json::{json, Value};

use crate::maps::maps_of;
use crate::project::{parse, Failure};
use Failure::Run;

pub(crate) fn run(args: &[String]) -> Result<ExitCode, Failure> {
    let (said, compared) = report(args)?;
    print!("{said}");
    Ok(if compared {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// What `maps changes` prints for `args`, and whether every map file and type file was compared.
pub(crate) fn report(args: &[String]) -> Result<(String, bool), Failure> {
    let json_out = args.iter().any(|a| a == "--json");
    let rest: Vec<String> = args.iter().filter(|a| *a != "--json").cloned().collect();
    let (positional, flags) = parse(&rest, &["--module"])?;
    let (project, modules) = maps_of(&positional, &flags)?;
    let (mut text, mut reports, mut compared) = (String::new(), Vec::new(), true);
    for id in modules {
        let module = (project.container.module(&id))
            .ok_or_else(|| Run(format!("{} has no maps content `{id}`", positional[0])))?;
        let folder = or_maps::module_folder(&project.folder(), module)
            .map_err(|e| Run(format!("content `{id}`: {e}")))?;
        let model =
            or_maps::Maps::read(&folder).map_err(|e| Run(format!("content `{id}`: {e}")))?;
        let facts = or_maps::inspect(&folder, &model, Depth::Stat);
        let compared_role = |path: &&str| {
            let role = facts.source(path).and_then(|source| source.role());
            matches!(role, Some(Role::Map | Role::Types))
        };
        // Read once, so that a link of a map file into its parent map is compared by the entity it names there.
        let read = PackSources::read(
            &folder,
            or_maps::facts::named_paths(&model).filter(compared_role),
        );
        let mut sources = Vec::new();
        let _ = writeln!(text, "content {id}:");
        for path in or_maps::facts::named_paths(&model) {
            let Some(source) = facts.source(path) else {
                compared = false;
                let why = match facts.get(path) {
                    Some(Fact::Unreadable(why)) => why.as_str(),
                    Some(Fact::Missing) => "is not in the folder of its content",
                    _ => "leaves the folder of its content",
                };
                let _ = writeln!(text, "  {path}: {why}");
                sources.push(json!({ "source": path, "error": why }));
                continue;
            };
            if !matches!(source.role(), Some(Role::Map | Role::Types)) {
                continue;
            }
            let file = source.file_name();
            match read.changes(path) {
                Ok(changes) => {
                    let _ = writeln!(text, "  {path} ({file}): {}", counts(&changes));
                    match &changes {
                        SourceChanges::Map(map) => {
                            map_file_said(&mut text, &model, map);
                            entities_said(&mut text, &model, map, "    ");
                        }
                        SourceChanges::Types(types) => {
                            file_said(&mut text, &model, &types.root, types.other);
                            archetypes_said(&mut text, &model, types);
                        }
                    }
                    let mut entry =
                        serde_json::to_value(&changes).map_err(|e| Run(e.to_string()))?;
                    name_hashes(&mut entry, &model);
                    entry["source"] = json!(path);
                    entry["file"] = json!(file);
                    sources.push(entry);
                }
                Err(why) => {
                    compared = false;
                    let _ = writeln!(text, "  {path} ({file}): {why}");
                    sources.push(json!({ "source": path, "file": file, "error": why.to_string() }));
                }
            }
        }
        reports.push(json!({ "module": id, "sources": sources }));
    }
    if json_out {
        text = serde_json::to_string_pretty(&reports).map_err(|e| Run(e.to_string()))?;
        text.push('\n');
    }
    Ok((text, compared))
}

fn counts(changes: &SourceChanges) -> String {
    let (added, deleted, changed, ambiguous, unchanged) = match changes {
        SourceChanges::Map(c) => (
            c.added.len(),
            c.deleted.len(),
            c.changed.len(),
            c.ambiguous.len(),
            c.unchanged,
        ),
        SourceChanges::Types(c) => (
            c.added.len(),
            c.deleted.len(),
            c.changed.len(),
            c.ambiguous.len(),
            c.unchanged,
        ),
    };
    format!("{added} added, {deleted} deleted, {changed} changed, {ambiguous} ambiguous, {unchanged} unchanged")
}

fn numbers(values: &[f32]) -> String {
    let said: Vec<String> = values.iter().map(ToString::to_string).collect();
    format!("({})", said.join(", "))
}

fn indexes(of: impl Iterator<Item = usize>) -> String {
    let said: Vec<String> = of.map(|i| i.to_string()).collect();
    said.join(", ")
}

/// The text of a name: what the file gave, else what a person typed into the pack for its hash, else the hash as a source spells it.
fn shown(model: &Maps, name: &Name) -> String {
    match &name.text {
        Some(text) => text.clone(),
        None => model.name_of(name.hash).unwrap_or_else(|| name.shown()),
    }
}

/// Give each name of a report that has no text the one a person typed into the pack for its hash.
fn name_hashes(report: &mut Value, model: &Maps) {
    match report {
        Value::Array(items) => items.iter_mut().for_each(|v| name_hashes(v, model)),
        Value::Object(members) => {
            // A name is the one object of a report that holds a hash and a text and nothing else.
            let hash = (members.len() == 2 && members.get("text").is_some_and(Value::is_null))
                .then(|| members.get("hash").and_then(Value::as_u64))
                .flatten()
                .and_then(|hash| u32::try_from(hash).ok());
            match hash.and_then(|hash| model.name_of(hash)) {
                Some(text) => members["text"] = json!(text),
                None => members.values_mut().for_each(|v| name_hashes(v, model)),
            }
        }
        _ => {}
    }
}

fn entity(model: &Maps, entity: &EntityRef) -> String {
    format!(
        "{} at {}, guid {}",
        shown(model, &entity.archetype),
        numbers(&entity.position),
        entity.guid
    )
}

fn parent(index: i32) -> String {
    match index {
        index if index < 0 => "none".to_string(),
        index => index.to_string(),
    }
}

/// The members a record states differently, each with what it stated and what it states, and a word for what no member names.
fn members_said(model: &Maps, changes: &[MemberChange], other: bool) -> Vec<String> {
    let mut said: Vec<String> = (changes.iter())
        .map(|c| {
            let (from, to) = (stated(model, &c.from), stated(model, &c.to));
            format!("{} {from} to {to}", c.member)
        })
        .collect();
    if other {
        said.push("a member no typed record names".to_string());
    }
    said
}

/// The line that says what a file states of itself differently, when it does.
fn file_said(text: &mut String, model: &Maps, root: &[MemberChange], other: bool) {
    let said = members_said(model, root, other);
    if !said.is_empty() {
        let _ = writeln!(text, "    the file  {}", said.join("; "));
    }
}

/// [`file_said`] of a map, whose families of records that differ are named by the members that hold them, between its own members and the word for what none names.
fn map_file_said(text: &mut String, model: &Maps, map: &or_maps::changes::MapChanges) {
    let mut said = members_said(model, &map.root, false);
    if !map.records.is_empty() {
        said.push(format!(
            "{} changed, which is not listed record by record",
            map.records.join(", ")
        ));
    }
    said.extend(members_said(model, &[], map.other));
    if !said.is_empty() {
        let _ = writeln!(text, "    the file  {}", said.join("; "));
    }
}

fn change(model: &Maps, change: &EntityChange) -> String {
    match change {
        EntityChange::Moved { from, to } => {
            format!("moved from {} to {}", numbers(from), numbers(to))
        }
        EntityChange::Turned { from, to } => {
            format!("turned from {} to {}", numbers(from), numbers(to))
        }
        EntityChange::Scaled { from, to } => {
            format!("scaled from {} to {}", numbers(from), numbers(to))
        }
        EntityChange::Archetype { from, to } => {
            format!("archetype {} to {}", shown(model, from), shown(model, to))
        }
        EntityChange::LodDist { from, to } => format!("lodDist {from} to {to}"),
        EntityChange::Flags { from, to } => format!("flags {from} to {to}"),
        // The two numbers are indexes of two files, so one number can name two entities.
        EntityChange::Parent { from, to } if from == to => {
            format!("parent {} is another entity", parent(*to))
        }
        EntityChange::Parent { from, to } => {
            format!("parent {} to {}", parent(*from), parent(*to))
        }
        EntityChange::Guid { from, to } => format!("guid {from} to {to}"),
        EntityChange::Members { names } => names.join(", "),
        EntityChange::Other => "a member no typed record names".to_string(),
    }
}

/// A line for each entity added, deleted, changed and ambiguous, each after `indent`.
fn entities_said(text: &mut String, model: &Maps, changes: &MapChanges, indent: &str) {
    for added in &changes.added {
        let _ = writeln!(
            text,
            "{indent}added     entity {}: {}",
            added.index,
            entity(model, added)
        );
    }
    for deleted in &changes.deleted {
        let _ = writeln!(
            text,
            "{indent}deleted   entity {} of the base: {}",
            deleted.index,
            entity(model, deleted)
        );
    }
    for kept in &changes.changed {
        let found = match kept.matched {
            Matched::Guid => "",
            Matched::Place => ", found at its place",
            Matched::Nearest => ", found as the nearest of its archetype",
        };
        let said: Vec<String> = kept.changes.iter().map(|c| change(model, c)).collect();
        let _ = writeln!(
            text,
            "{indent}changed   entity {} ({} of the base): {}{found}: {}",
            kept.now.index,
            kept.base.index,
            entity(model, &kept.base),
            said.join("; ")
        );
    }
    for group in &changes.ambiguous {
        let _ = writeln!(
            text,
            "{indent}ambiguous {}: entity {} of the base and entity {} now cannot be told apart",
            shown(model, &group.archetype),
            indexes(group.base.iter().map(|e| e.index)),
            indexes(group.now.iter().map(|e| e.index)),
        );
    }
}

fn stated(model: &Maps, value: &Stated) -> String {
    match value {
        Stated::Float(value) => value.to_string(),
        Stated::Number(value) => value.to_string(),
        Stated::Vector(value) => numbers(value),
        Stated::Name(name) => shown(model, name),
        Stated::Text(text) if text.is_empty() => "(none)".to_string(),
        Stated::Text(text) => text.clone(),
    }
}

fn archetypes_said(text: &mut String, model: &Maps, changes: &TypesChanges) {
    for added in &changes.added {
        let _ = writeln!(
            text,
            "    added     archetype {}: {}",
            added.index,
            shown(model, &added.name)
        );
    }
    for deleted in &changes.deleted {
        let _ = writeln!(
            text,
            "    deleted   archetype {} of the base: {}",
            deleted.index,
            shown(model, &deleted.name)
        );
    }
    for kept in &changes.changed {
        let mut said = members_said(model, &kept.changes, kept.other);
        if let Some(inside) = &kept.entities {
            said.push(format!(
                "entities: {}",
                counts(&SourceChanges::Map(inside.clone()))
            ));
        }
        let _ = writeln!(
            text,
            "    changed   archetype {} ({} of the base): {}: {}",
            kept.now.index,
            kept.base.index,
            shown(model, &kept.base.name),
            said.join("; ")
        );
        if let Some(inside) = &kept.entities {
            entities_said(text, model, inside, "      ");
        }
    }
    for group in &changes.ambiguous {
        let _ = writeln!(
            text,
            "    ambiguous {}: archetype {} of the base and archetype {} now cannot be told apart",
            shown(model, &group.name),
            indexes(group.base.iter().copied()),
            indexes(group.now.iter().copied()),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use or_io::meta::instance::HashedString;
    use or_io::meta::typed::{
        CBaseArchetypeDefAny, CEntityDef, CEntityDefAny, CMapTypes, CMloArchetypeDef,
    };
    use or_maps::changes::{entity_changes, types_changes};
    use or_maps::edit::{
        add_archetype, add_entity, edit_map, edit_types, place_entity, pose_of, remove_entity,
    };
    use or_maps::import::{add_files, Incoming};
    use or_maps::sample::{map_file, types_file, Archetype, Entity};

    use super::*;

    /// A project whose maps content holds a map file of two crates, a bench and twin trees, and a type file of two crates.
    fn a_project(dir: &Path) -> PathBuf {
        std::fs::write(
            dir.join("harbour.openrage-proj"),
            r#"{
  "documents": [],
  "game": "both",
  "modules": [{ "id": "maps", "path": "maps", "type": "maps" }],
  "open": [],
  "outputs": [{ "id": "fivem", "kind": "fivem-resource", "name": "harbour", "take": ["maps"] }],
  "schema": 4
}"#,
        )
        .unwrap();
        let map = map_file(
            "harbour",
            &[
                Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1),
                Entity::at("prop_crate_b", [14.0, 20.0, 0.5], 2),
                Entity::at("prop_bench", [18.0, 20.0, 0.5], 3),
                Entity::at("prop_tree", [30.0, 30.0, 0.0], 7),
                Entity::at("prop_tree", [34.0, 30.0, 0.0], 7),
            ],
            ([0.0; 3], [100.0; 3]),
        );
        let types = types_file(
            "harbour_types",
            &[
                Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3]),
                Archetype::new("prop_crate_b", [-2.0; 3], [2.0; 3]),
            ],
        );
        let added = add_files(
            &dir.join("maps"),
            &[
                Incoming {
                    name: "harbour.ymap",
                    bytes: &map,
                    from_game: true,
                    virtual_path: Some("x64a.rpf/levels/harbour.ymap"),
                },
                Incoming {
                    name: "harbour_types.ytyp",
                    bytes: &types,
                    from_game: true,
                    virtual_path: Some("x64a.rpf/levels/harbour_types.ytyp"),
                },
            ],
            None,
        )
        .unwrap();
        assert_eq!(added.refused, Vec::<(String, String)>::new());
        dir.join("harbour.openrage-proj")
    }

    fn both(file: &Path) -> (String, Value, bool) {
        let path = file.to_str().unwrap().to_string();
        let run = |args: &[String]| match report(args) {
            Ok(said) => said,
            Err(Failure::Usage(why) | Run(why)) => panic!("{why}"),
        };
        let (text, compared) = run(std::slice::from_ref(&path));
        let (json, compared_json) = run(&[path, "--json".to_string()]);
        assert_eq!(compared, compared_json);
        (text, serde_json::from_str(&json).unwrap(), compared)
    }

    /// One change a report says: how deep it is (1 for an entity of an interior), the word that starts its line, and the index it names first.
    type Said = (usize, String, usize);

    /// The changes the lines of a report say, each indented by four spaces and two more for each level under.
    fn said<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<Said> {
        lines
            .map(|l| {
                let depth = (l.len() - l.trim_start().len() - 4) / 2;
                let mut words = l.split_whitespace();
                let word = match words.next().unwrap_or_default() {
                    "the" => "file".to_string(),
                    word => word.to_string(),
                };
                // An ambiguous group is named first; every other line names the index right after `entity` or `archetype`.
                let index = (words.clone())
                    .skip_while(|w| !matches!(*w, "entity" | "archetype"))
                    .nth(1)
                    .and_then(|w| w.trim_end_matches([':', ',']).parse().ok())
                    .unwrap_or(usize::MAX);
                (depth, word, index)
            })
            .collect()
    }

    /// The changes the text of a report says of one source.
    fn said_of(text: &str, source: &str) -> Vec<Said> {
        let mut lines = text
            .lines()
            .skip_while(|l| !l.starts_with(&format!("  {source} (")));
        lines.next();
        said(lines.take_while(|l| l.starts_with("    ")))
    }

    /// The changes the JSON of one comparison lists, as [`said`] gives the lines of the text.
    fn listed(entry: &Value, depth: usize) -> Vec<Said> {
        let index = |value: &Value| value.as_u64().expect("an index") as usize;
        let each = |key: &str| entry[key].as_array().into_iter().flatten();
        let mut out = Vec::new();
        if each("root").next().is_some() || entry["other"] == true {
            out.push((depth, "file".to_string(), usize::MAX));
        }
        for added in each("added") {
            out.push((depth, "added".to_string(), index(&added["index"])));
        }
        for deleted in each("deleted") {
            out.push((depth, "deleted".to_string(), index(&deleted["index"])));
        }
        for kept in each("changed") {
            out.push((depth, "changed".to_string(), index(&kept["now"]["index"])));
            if kept.get("entities").is_some() {
                out.extend(listed(&kept["entities"], depth + 1));
            }
        }
        for group in each("ambiguous") {
            // A group of entities lists each with its index, and a group of archetypes lists the indexes.
            let first = &group["base"][0];
            let first = index(first.get("index").unwrap_or(first));
            out.push((depth, "ambiguous".to_string(), first));
        }
        out
    }

    /// The changes the JSON of a report lists of one source.
    fn listed_of(report: &Value, source: &str) -> Vec<Said> {
        let entry = (report[0]["sources"].as_array().into_iter().flatten())
            .find(|s| s["source"] == source)
            .unwrap_or_else(|| panic!("{source} is not in {report}"));
        listed(entry, 0)
    }

    const MAP: &str = "harbour.metadata.src";
    const TYPES: &str = "harbour_types.metadata.src";

    #[test]
    fn the_json_of_the_changes_lists_the_changes_the_text_lists() {
        let dir = tempfile::tempdir().unwrap();
        let file = a_project(dir.path());

        let (text, json, compared) = both(&file);
        assert!(compared);
        assert_eq!(said_of(&text, MAP), []);
        assert_eq!(listed_of(&json, MAP), []);
        assert!(
            text.contains("0 added, 0 deleted, 0 changed, 0 ambiguous, 5 unchanged"),
            "{text}"
        );

        // The first crate taken out, the second moved, a crate added, a twin tree moved and the flags of the map changed; and in the
        // type file a box grown, an archetype taken out, one added and the name of the file changed.
        let maps = dir.path().join("maps");
        edit_map(&maps.join(MAP), |map| {
            let mut moved = pose_of(&map.entities[1]);
            moved.position.x += 2.0;
            place_entity(map, 1, moved)?;
            let mut tree = pose_of(&map.entities[4]);
            tree.position.y += 3.0;
            place_entity(map, 4, tree)?;
            remove_entity(map, 0)?;
            let donor = map.entities[0].clone();
            let mut stands = pose_of(&donor);
            stands.position.x += 40.0;
            map.content_flags = 66;
            add_entity(map, &donor, Name::from_text("prop_crate_b"), stands).map(|_| ())
        })
        .unwrap();
        edit_types(&maps.join(TYPES), |types| {
            types.archetypes[1].bb_max.z = 4.5;
            types.archetypes.remove(0);
            types.name = HashedString::new("quay_types");
            add_archetype(types, None, "prop_crate_d", None).map(|_| ())
        })
        .unwrap();

        let (text, json, compared) = both(&file);
        assert!(compared);
        let expected = vec![
            (0, "file".to_string(), usize::MAX),
            (0, "added".to_string(), 4),
            (0, "deleted".to_string(), 0),
            (0, "changed".to_string(), 0),
            (0, "ambiguous".to_string(), 4),
        ];
        assert_eq!(listed_of(&json, MAP), expected, "{json:#}");
        assert_eq!(said_of(&text, MAP), expected, "{text}");
        let expected = vec![
            (0, "file".to_string(), usize::MAX),
            (0, "added".to_string(), 1),
            (0, "deleted".to_string(), 0),
            (0, "changed".to_string(), 0),
        ];
        assert_eq!(listed_of(&json, TYPES), expected, "{json:#}");
        assert_eq!(said_of(&text, TYPES), expected, "{text}");

        // What each form says of one change is the same change.
        let entry = &json[0]["sources"][0];
        assert_eq!(entry["kind"], "map");
        assert_eq!(entry["file"], "harbour.ymap");
        assert_eq!(entry["root"][0]["member"], "contentFlags");
        assert_eq!(entry["root"][0]["to"], 66);
        assert!(
            text.contains("    the file  contentFlags 65 to 66\n"),
            "{text}"
        );
        assert_eq!(entry["changed"][0]["base"]["index"], 1);
        let moved = &entry["changed"][0]["changes"][0];
        assert_eq!(moved["what"], "moved");
        assert_eq!(moved["from"], json!([14.0, 20.0, 0.5]));
        assert_eq!(moved["to"], json!([16.0, 20.0, 0.5]));
        assert!(
            text.contains("changed   entity 0 (1 of the base): ")
                && text.contains("moved from (14, 20, 0.5) to (16, 20, 0.5)"),
            "{text}"
        );
        assert_eq!(entry["ambiguous"][0]["now"][0]["index"], 3);
        assert!(
            text.contains("entity 4 of the base and entity 3 now cannot be told apart"),
            "{text}"
        );
        assert!(!text.contains("moved from (34"), "a twin is never moved");
        assert_eq!(json[0]["sources"][1]["kind"], "types");
        assert_eq!(
            json[0]["sources"][1]["changed"][0]["changes"][0]["member"],
            "bbMax"
        );
        assert!(text.contains("bbMax (2, 2, 2) to (2, 2, 4.5)"), "{text}");

        // A map keeps the hash of a name, so the text of one is what a person typed into the pack for it.
        let hash = Name::from_text("prop_crate_b").hash;
        assert_eq!(entry["changed"][0]["base"]["archetype"]["hash"], hash);
        assert_eq!(
            entry["changed"][0]["base"]["archetype"]["text"],
            Value::Null
        );
        assert!(text.contains(&Name::of(hash).shown()), "{text}");
        let mut model = Maps::read(&maps).unwrap();
        assert!(model.remember(&["prop_crate_b", "prop_tree"]));
        model.write(&maps).unwrap();
        let (text, json, _) = both(&file);
        let entry = &json[0]["sources"][0];
        assert_eq!(
            entry["changed"][0]["base"]["archetype"]["text"],
            "prop_crate_b"
        );
        assert_eq!(entry["ambiguous"][0]["archetype"]["text"], "prop_tree");
        assert_eq!(entry["deleted"][0]["archetype"]["text"], Value::Null);
        assert!(
            text.contains(
                "changed   entity 0 (1 of the base): prop_crate_b at (14, 20, 0.5), guid 2: moved"
            ),
            "{text}"
        );
        assert!(text.contains("ambiguous prop_tree: entity 4"), "{text}");
        assert_eq!(
            json[0]["sources"][1]["changed"][0]["base"]["name"]["text"],
            "prop_crate_b"
        );
        assert!(
            text.contains("changed   archetype 0 (1 of the base): prop_crate_b: bbMax"),
            "{text}"
        );

        // The content is named, and one that is not there is refused.
        let path = file.to_str().unwrap().to_string();
        let named = |id: &str| report(&[path.clone(), "--module".to_string(), id.to_string()]);
        assert!(matches!(named("maps"), Ok((said, true)) if said == text));
        assert!(matches!(named("roads"), Err(Run(why)) if why.contains("`roads`")));
    }

    fn entity(archetype: &str, guid: u32, position: [f32; 3]) -> CEntityDefAny {
        let mut entity = CEntityDef::default();
        entity.archetype_name = HashedString::new(archetype);
        entity.guid = guid;
        entity.position.x = position[0];
        entity.position.y = position[1];
        entity.position.z = position[2];
        entity.rotation.w = 1.0;
        entity.parent_index = -1;
        CEntityDefAny::Base(entity)
    }

    /// The text of a comparison of entities and the JSON of it, as the changes each lists.
    fn of_entities(changes: &MapChanges) -> (String, Vec<Said>, Vec<Said>) {
        let mut text = String::new();
        entities_said(&mut text, &Maps::new(), changes, "    ");
        let listed = listed(&serde_json::to_value(changes).unwrap(), 0);
        (text.clone(), said(text.lines()), listed)
    }

    #[test]
    fn each_kind_of_change_of_an_entity_is_a_line_of_the_text_and_an_entry_of_the_json() {
        // A parent taken out, a child found by its place with another guid, one found as the nearest, one that names another entity
        // by the number it stated, and one with a member of its own changed.
        let mut base = vec![
            entity("yard_lod_a", 1, [0.0, 0.0, 0.0]),
            entity("yard_lod_b", 2, [50.0, 0.0, 0.0]),
            entity("yard_hd_a", 3, [0.0, 2.0, 0.0]),
            entity("yard_hd_b", 4, [50.0, 2.0, 0.0]),
            entity("prop_bench", 5, [9.0, 9.0, 0.0]),
            entity("prop_lamp", 6, [20.0, 0.0, 0.0]),
            entity("prop_bin", 7, [30.0, 0.0, 0.0]),
        ];
        base[2].parent_index = 0;
        base[3].parent_index = 1;
        base[6].parent_index = 0;
        let mut now = vec![
            entity("yard_lod_b", 2, [50.0, 0.0, 0.0]),
            entity("yard_hd_a", 3, [0.0, 2.0, 0.0]),
            entity("yard_hd_b", 4, [50.0, 2.0, 0.0]),
            entity("prop_bench", 105, [9.0, 9.0, 0.0]),
            entity("prop_lamp", 106, [21.0, 0.0, 0.0]),
            entity("prop_bin", 7, [30.0, 0.0, 0.0]),
        ];
        now[2].parent_index = 0;
        now[2].tint_value = 3;
        now[5].parent_index = 0;
        let changes = entity_changes(&base, &now);
        let (text, said, listed) = of_entities(&changes);
        let expected = vec![
            (0, "deleted".to_string(), 0),
            (0, "changed".to_string(), 1),
            (0, "changed".to_string(), 2),
            (0, "changed".to_string(), 3),
            (0, "changed".to_string(), 4),
            (0, "changed".to_string(), 5),
        ];
        assert_eq!(listed, expected, "{changes:#?}");
        assert_eq!(said, expected, "{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[1].ends_with(": parent 0 to none"), "{text}");
        assert!(lines[2].ends_with(": tintValue"), "{text}");
        assert!(
            lines[3].ends_with(", found at its place: guid 5 to 105"),
            "{text}"
        );
        assert!(
            lines[4].ends_with(
                ", found as the nearest of its archetype: moved from (20, 0, 0) to (21, 0, 0); guid 6 to 106"
            ),
            "{text}"
        );
        assert!(lines[5].ends_with(": parent 0 is another entity"), "{text}");
    }

    #[test]
    fn the_entities_of_an_interior_are_listed_under_it_in_the_text_as_in_the_json() {
        let mut held = CMloArchetypeDef::default();
        held.base.name = HashedString::new("v_shop");
        held.mlo_flags = 1;
        held.entities = vec![
            entity("prop_chair", 1, [0.0, 0.0, 0.0]),
            entity("prop_table", 2, [1.0, 0.0, 0.0]),
            entity("prop_lamp", 3, [2.0, 0.0, 0.0]),
        ];
        let mut other = held.clone();
        other.mlo_flags = 2;
        other.entities[2].position.z = 1.5;
        other.entities.remove(0);
        let (mut base, mut now) = (CMapTypes::default(), CMapTypes::default());
        // A name the base declares twice, of which one changed, and a file the changed one depends on.
        for (types, top) in [(&mut base, 1.0), (&mut now, 2.0)] {
            for _ in 0..2 {
                types.archetypes.push(CBaseArchetypeDefAny::default());
                let last = types.archetypes.len() - 1;
                types.archetypes[last].name = HashedString::new("prop_twice");
            }
            types.archetypes[1].bb_max.z = top;
        }
        base.archetypes
            .push(CBaseArchetypeDefAny::CMloArchetypeDef(held));
        now.archetypes
            .push(CBaseArchetypeDefAny::CMloArchetypeDef(other));
        now.dependencies.push(HashedString::new("harbour_props"));
        let changes = types_changes(&base, &now);

        let mut text = String::new();
        file_said(&mut text, &Maps::new(), &changes.root, changes.other);
        archetypes_said(&mut text, &Maps::new(), &changes);
        let expected = vec![
            (0, "file".to_string(), usize::MAX),
            (0, "changed".to_string(), 2),
            (1, "deleted".to_string(), 0),
            (1, "changed".to_string(), 1),
            (0, "ambiguous".to_string(), 0),
        ];
        let json = serde_json::to_value(&changes).unwrap();
        assert_eq!(listed(&json, 0), expected, "{json:#}");
        assert_eq!(said(text.lines()), expected, "{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            "    the file  dependencies (none) to harbour_props"
        );
        assert_eq!(
            lines[1],
            "    changed   archetype 2 (2 of the base): v_shop: mloFlags 1 to 2; entities: 0 added, 1 deleted, 1 changed, 0 ambiguous, 1 unchanged"
        );
        assert_eq!(
            lines[2],
            "      deleted   entity 0 of the base: prop_chair at (0, 0, 0), guid 1"
        );
        assert!(
            lines[3].starts_with("      changed   entity 1 (2 of the base): prop_lamp")
                && lines[3].ends_with(": moved from (2, 0, 0) to (2, 0, 1.5)"),
            "{text}"
        );
        assert_eq!(
            lines[4],
            "    ambiguous prop_twice: archetype 0, 1 of the base and archetype 0, 1 now cannot be told apart"
        );

        // What no typed member names has a word, of the file as of an archetype, and a file that states what it stated has no line.
        let mut text = String::new();
        file_said(&mut text, &Maps::new(), &[], false);
        assert_eq!(text, "");
        file_said(&mut text, &Maps::new(), &changes.root, true);
        assert_eq!(
            text,
            "    the file  dependencies (none) to harbour_props; a member no typed record names\n"
        );
    }

    #[test]
    fn a_source_that_cannot_be_compared_is_named_with_the_reason_and_the_command_fails() {
        let dir = tempfile::tempdir().unwrap();
        let file = a_project(dir.path());
        let maps = dir.path().join("maps");
        std::fs::write(maps.join(MAP).join("metadata.xml"), "not a map").unwrap();
        let (text, json, compared) = both(&file);
        assert!(!compared);
        assert!(text.contains("the source does not cook"), "{text}");
        let error = json[0]["sources"][0]["error"].as_str().unwrap();
        assert!(error.starts_with("the source does not cook"), "{error}");
        assert_eq!(
            listed_of(&json, TYPES),
            [],
            "the other source is still compared"
        );

        let path = file.to_str().unwrap().to_string();
        assert_eq!(
            crate::run(&["maps".to_string(), "changes".to_string(), path.clone()]),
            ExitCode::FAILURE
        );
        assert_eq!(
            crate::run(&["maps".to_string(), "changes".to_string()]),
            ExitCode::from(2),
            "no project named is a usage error"
        );
    }

    #[test]
    fn a_source_that_is_not_there_fails_the_command_and_one_that_is_no_map_is_passed_over() {
        let dir = tempfile::tempdir().unwrap();
        let file = a_project(dir.path());
        let maps = dir.path().join("maps");

        // A texture dictionary of the set is neither a map file nor a type file: it is not compared, and not a failure.
        let textures = maps.join("bark.textures.src");
        std::fs::create_dir_all(&textures).unwrap();
        let origin = or_source::asset::Origin {
            how: or_source::asset::How::Authored,
            from_game: false,
            virtual_path: None,
            game_build: None,
            sha256: None,
            cooked_sha256: None,
            extra: Default::default(),
        };
        let manifest =
            or_source::asset::AssetManifest::new("textures", "bark", "gen8", origin).unwrap();
        std::fs::write(textures.join("asset.json"), manifest.to_text()).unwrap();
        // The comparison looks at what a source is and never cooks this one, so its rules are not read.
        std::fs::write(textures.join(or_source::textures::RULES), "{}").unwrap();
        let mut model = Maps::read(&maps).unwrap();
        model.maps[0].sources.push("bark.textures.src".to_string());
        model.write(&maps).unwrap();
        let (text, json, compared) = both(&file);
        assert!(compared, "{text}");
        assert!(!text.contains("bark"), "{text}");
        assert_eq!(json[0]["sources"].as_array().unwrap().len(), 2);

        std::fs::remove_dir_all(maps.join(TYPES)).unwrap();
        let (text, json, compared) = both(&file);
        assert!(!compared);
        let line = format!("  {TYPES}: is not in the folder of its content\n");
        assert!(text.contains(&line), "{text}");
        assert_eq!(json[0]["sources"][1]["source"], TYPES);
        assert_eq!(
            json[0]["sources"][1]["error"],
            "is not in the folder of its content"
        );
        assert_eq!(listed_of(&json, MAP), [], "the map is still compared");
    }
}
