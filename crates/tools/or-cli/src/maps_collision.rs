//! `openrage-cli maps collision`: what the bounds sources of a maps content hold of the collision of an object, as `or-world`
//! proposes it, and the operations that take it out or move it (`or_maps::collision`).

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use glam::{Quat, Vec3};
use or_maps::collision::{
    apply_all, carried, object_box, propose, Applied, Error, Operation, Operations, Planned,
    Proposal,
};
use or_world::collision::{AssociationOptions, NotJudged, Reason};
use or_world_gta5::catalogue::area_of;
use serde_json::{json, Value};

use crate::maps::maps_of;
use crate::project::{parse, Failure};
use Failure::{Run, Usage};

pub(crate) fn run(args: &[String]) -> Result<ExitCode, Failure> {
    print!("{}", report(args)?);
    Ok(ExitCode::SUCCESS)
}

/// The numbers `text` holds between commas, which are `count` of them.
fn numbers(text: &str, count: usize, flag: &str) -> Result<Vec<f32>, Failure> {
    let read: Vec<f32> = text
        .split(',')
        .map(|n| n.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .map_err(|_| Usage(format!("{flag} takes {count} numbers between commas")))?;
    if read.len() != count || read.iter().any(|n| !n.is_finite()) {
        return Err(Usage(format!(
            "{flag} takes {count} numbers between commas"
        )));
    }
    Ok(read)
}

/// Places as a person reads them: runs of neighbours as `first-last`.
fn ranges(places: &[u32]) -> String {
    let mut out = String::new();
    let mut at = 0;
    while at < places.len() {
        let mut end = at;
        while end + 1 < places.len() && places[end + 1] == places[end] + 1 {
            end += 1;
        }
        if !out.is_empty() {
            out.push_str(", ");
        }
        match end - at {
            0 => write!(out, "{}", places[at]),
            _ => write!(out, "{}-{}", places[at], places[end]),
        }
        .ok();
        at = end + 1;
    }
    out
}

fn percent(share: f32) -> String {
    format!("{:.0}%", share * 100.0)
}

/// What a proposal says, source by source and soup by soup.
fn proposal_said(text: &mut String, proposal: &Proposal) {
    let association = &proposal.association;
    match association.not_judged {
        Some(NotJudged::TooLow { height, limit }) => {
            let _ = writeln!(
                text,
                "  the object stands {height} m tall, under the {limit} m from which collision is looked for: nothing is proposed"
            );
        }
        Some(NotJudged::NoBox) => {
            let _ = writeln!(
                text,
                "  the box of the object is no finite size or place: nothing is proposed"
            );
        }
        None if association.bounds.is_empty() => {
            let _ = writeln!(
                text,
                "  no bounds source of the content holds a polygon in the box of the object"
            );
        }
        None => {}
    }
    for bound in &association.bounds {
        let file = proposal.names.get(&bound.key).map_or("?", String::as_str);
        let _ = writeln!(text, "  {} ({file}.ybn)", bound.key);
        for leaf in &bound.leaves {
            let (address, kind) = proposal
                .leaf(&bound.key, leaf.leaf)
                .map_or(("?", "bound"), |name| (name.address.as_str(), name.kind));
            let _ = writeln!(
                text,
                "    {address}, a {kind}: {} proposed, {} left",
                leaf.proposed.len(),
                leaf.left.len()
            );
            let whole: Vec<u32> = (leaf.proposed.iter())
                .filter(|p| p.share.is_none())
                .filter_map(|p| p.polygon)
                .collect();
            if !whole.is_empty() {
                let _ = writeln!(text, "      proposed: {}", ranges(&whole));
            }
            if leaf.proposed.iter().any(|p| p.polygon.is_none()) {
                let _ = writeln!(text, "      proposed: the bound, which is one shape");
            }
            let mostly: Vec<String> = (leaf.proposed.iter())
                .filter_map(|p| Some(format!("{} ({})", p.polygon?, percent(p.share?))))
                .collect();
            if !mostly.is_empty() {
                let _ = writeln!(
                    text,
                    "      proposed, cut with most of it in the box: {}",
                    mostly.join(", ")
                );
            }
            let ground: Vec<u32> = (leaf.left.iter())
                .filter(|left| left.reason == Reason::Ground)
                .filter_map(|left| left.polygon)
                .collect();
            if !ground.is_empty() {
                let _ = writeln!(text, "      left as ground: {}", ranges(&ground));
            }
            let outside: Vec<String> = (leaf.left.iter())
                .filter_map(|left| match left.reason {
                    Reason::MostlyOutside { share } => {
                        Some(format!("{} ({} in the box)", left.polygon?, percent(share)))
                    }
                    _ => None,
                })
                .collect();
            if !outside.is_empty() {
                let _ = writeln!(
                    text,
                    "      left, cut with too little of it in the box: {}",
                    outside.join(", ")
                );
            }
            let shapes: Vec<u32> = (leaf.left.iter())
                .filter(|left| left.reason == Reason::CutShape)
                .filter_map(|left| left.polygon)
                .collect();
            if !shapes.is_empty() {
                let _ = writeln!(
                    text,
                    "      left, a shape the box cuts: {}",
                    ranges(&shapes)
                );
            }
            if (leaf.left.iter()).any(|l| l.polygon.is_none() && l.reason == Reason::CutShape) {
                let _ = writeln!(
                    text,
                    "      left: the bound, which is one shape the box cuts"
                );
            }
        }
    }
}

/// The proposal as data: the association, with the file and the area of each source and the address of each leaf.
fn proposal_json(proposal: &Proposal) -> Result<Value, Failure> {
    let mut value = serde_json::to_value(&proposal.association).map_err(|e| Run(e.to_string()))?;
    for bound in value["bounds"].as_array_mut().into_iter().flatten() {
        let source = bound["key"].as_str().unwrap_or_default().to_string();
        let name = proposal.names.get(&source).cloned().unwrap_or_default();
        bound["source"] = json!(source);
        bound["file"] = json!(format!("{name}.ybn"));
        bound["area"] = json!(area_of(&name));
        if let Some(held) = bound.as_object_mut() {
            held.remove("key");
        }
        for leaf in bound["leaves"].as_array_mut().into_iter().flatten() {
            let number = leaf["leaf"].as_u64().unwrap_or_default() as u32;
            if let Some(name) = proposal.leaf(&source, number) {
                leaf["address"] = json!(name.address);
                leaf["kind"] = json!(name.kind);
            }
        }
    }
    Ok(value)
}

fn operation_said(operation: &Operation) -> String {
    // An operation that is not for its source alone is done to every version of the area.
    let reach = |alone: &bool| match alone {
        true => "",
        false => ", and the same of every version of its area",
    };
    match operation {
        Operation::RemovePolygons {
            source,
            soup,
            polygons,
            alone,
            ..
        } => format!(
            "remove_polygons of {source}: {} polygon(s) of `{soup}`{}",
            polygons.len(),
            reach(alone)
        ),
        Operation::MovePolygons {
            source,
            soup,
            polygons,
            alone,
            ..
        } => format!(
            "move_polygons of {source}: {} polygon(s) of `{soup}`{}",
            polygons.len(),
            reach(alone)
        ),
        Operation::SetMaterial {
            source,
            soup,
            polygons,
            material,
            alone,
            ..
        } => format!(
            "set_material of {source}: {material} on {} polygon(s) of `{soup}`{}",
            polygons.len(),
            reach(alone)
        ),
        Operation::RestoreSources { sources } => format!(
            "restore_sources: the document and the glb of {} bounds source(s) as they were",
            sources.len()
        ),
    }
}

fn refusal(e: Error) -> Failure {
    match e {
        // The one failure after which a source may hold what the list wrote: it says which.
        Error::Write(_) => Run(e.to_string()),
        refused => Run(format!("{refused}; no source was changed")),
    }
}

/// Applies `operations` to the sources in `folder`, once the list that takes them back is written to `inverse`.
fn applied(
    folder: &Path,
    operations: &[Operation],
    dry_run: bool,
    inverse: Option<&String>,
) -> Result<Vec<Applied>, Failure> {
    // ⚠ The way back is kept before a source is written: an edit whose inverse cannot be kept is not made.
    if let (Some(file), false) = (inverse, dry_run) {
        let tried = apply_all(folder, operations, true).map_err(refusal)?;
        // Taken back in the order opposite to the one they were done in.
        let back = Operations::new(tried.iter().rev().map(|a| a.inverse.clone()).collect());
        let list = back.to_json().map_err(|e| Run(e.to_string()))?;
        std::fs::write(file, list)
            .map_err(|e| Run(format!("{file}: {e}; no source was changed")))?;
    }
    apply_all(folder, operations, dry_run).map_err(refusal)
}

/// What applying a list did and the sources it changed, and where the list of what takes it back is when one was asked for.
fn applied_said(
    text: &mut String,
    id: &str,
    operations: &[Operation],
    applied: &[Applied],
    dry_run: bool,
    inverse: Option<&String>,
) -> Vec<String> {
    let mut changed: Vec<String> = Vec::new();
    for (at, (operation, done)) in operations.iter().zip(applied).enumerate() {
        let _ = writeln!(text, "  {}. {}", at + 1, operation_said(operation));
        for line in &done.report {
            let _ = writeln!(text, "     {line}");
        }
        for source in &done.changed {
            if !changed.contains(source) {
                changed.push(source.clone());
            }
        }
    }
    changed.sort();
    let verb = if dry_run { "would change" } else { "changed" };
    for source in &changed {
        let _ = writeln!(text, "{verb} {source}");
    }
    let _ = writeln!(
        text,
        "content {id}: {} operation(s) {verb} {} bounds source(s){}",
        operations.len(),
        changed.len(),
        if dry_run { "; nothing was written" } else { "" }
    );
    if let (Some(file), false) = (inverse, dry_run) {
        let _ = writeln!(
            text,
            "the list that takes this back was written to {file}: `maps collision --operations` applies it"
        );
    }
    changed
}

/// What `maps collision` prints for `args`.
pub(crate) fn report(args: &[String]) -> Result<String, Failure> {
    const SWITCHES: [&str; 4] = ["--json", "--apply", "--take-ground", "--dry-run"];
    let on = |switch: &str| args.iter().any(|a| a == switch);
    let rest: Vec<String> = (args.iter())
        .filter(|a| !SWITCHES.contains(&a.as_str()))
        .cloned()
        .collect();
    let (positional, flags) = parse(
        &rest,
        &[
            "--module",
            "--object",
            "--box",
            "--move",
            "--turn",
            "--operations",
            "--inverse",
        ],
    )?;
    let [path] = positional.as_slice() else {
        return Err(Usage("expected one <project.openrage-proj>".into()));
    };
    let (json_out, dry_run) = (on("--json"), on("--dry-run"));
    let moving = flags.contains_key("--move") || flags.contains_key("--turn");
    if on("--apply") && moving {
        return Err(Usage(
            "--apply takes the proposal out and --move or --turn moves it: ask for one".into(),
        ));
    }
    let listed = flags.get("--operations");
    if listed.is_some() == flags.contains_key("--object") {
        return Err(Usage(
            "name the object with --object <map source>:<entity>, or a list with --operations <file>, and not both".into(),
        ));
    }
    let writes = (on("--apply") || moving || listed.is_some()) && !dry_run;

    let (project, modules) = maps_of(std::slice::from_ref(path), &flags)?;
    let [id] = modules.as_slice() else {
        return Err(Run(format!(
            "{path} holds {} maps contents ({}): say which with --module",
            modules.len(),
            modules.join(", ")
        )));
    };
    let module = (project.container.module(id))
        .ok_or_else(|| Run(format!("{path} has no maps content `{id}`")))?;
    // ⚠ Only a call that writes asks for the folder to edit, which a linked content refuses.
    let folder = if writes {
        or_maps::module_folder_to_edit(Path::new(path), &project.container, module)
    } else {
        or_maps::module_folder(&project.folder(), module)
    }
    .map_err(|e| Run(format!("content `{id}`: {e}")))?;
    let mut text = String::new();

    if let Some(list) = listed {
        let bytes = std::fs::read(list).map_err(|e| Run(format!("{list}: {e}")))?;
        let operations = Operations::from_json(&bytes, list)
            .map_err(|e| Run(e.to_string()))?
            .operations;
        let applied = applied(&folder, &operations, dry_run, flags.get("--inverse"))?;
        let changed = applied_said(
            &mut text,
            id,
            &operations,
            &applied,
            dry_run,
            flags.get("--inverse"),
        );
        return Ok(if json_out {
            let said = json!({ "module": id, "changed": changed, "written": !dry_run });
            format!(
                "{}\n",
                serde_json::to_string_pretty(&said).map_err(|e| Run(e.to_string()))?
            )
        } else {
            text
        });
    }

    let named = flags
        .get("--object")
        .map(String::as_str)
        .unwrap_or_default();
    let (source, entity) = named
        .rsplit_once(':')
        .and_then(|(source, index)| Some((source, index.parse::<usize>().ok()?)))
        .ok_or_else(|| {
            Usage(format!(
                "--object names an entity as <map source>:<its index>, and `{named}` is not one"
            ))
        })?;
    let stated = match flags.get("--box") {
        Some(lanes) => {
            let n = numbers(lanes, 6, "--box")?;
            Some((Vec3::new(n[0], n[1], n[2]), Vec3::new(n[3], n[4], n[5])))
        }
        None => None,
    };
    let object = object_box(&folder, source, entity, stated).map_err(|e| Run(e.to_string()))?;
    let options = AssociationOptions {
        take_ground: on("--take-ground"),
    };
    let proposal = propose(&folder, &object, &options).map_err(|e| Run(e.to_string()))?;
    let planned: Planned = if moving {
        let shift = match flags.get("--move") {
            Some(lanes) => Vec3::from_slice(&numbers(lanes, 3, "--move")?),
            None => Vec3::ZERO,
        };
        let turn = match flags.get("--turn") {
            Some(degrees) => numbers(degrees, 1, "--turn")?[0],
            None => 0.0,
        };
        // The object turns about where it stands, which is the position its entity states and not the middle of its box.
        let dir = folder.join(source.replace('/', std::path::MAIN_SEPARATOR_STR));
        let pose = or_maps::edit::read_entity(&dir, entity)
            .map(|entity| or_maps::edit::pose_of(&entity))
            .map_err(Run)?;
        let (rotation, translation) = carried(
            (pose.position, pose.rotation),
            (
                pose.position + shift,
                Quat::from_rotation_z(turn.to_radians()) * pose.rotation,
            ),
        );
        proposal.moving(rotation, translation)
    } else {
        proposal.removal()
    };

    let size = object.half_extents * 2.0;
    let _ = writeln!(
        text,
        "content {id}: the object is entity {entity} of {source}, a box {} by {} by {} m about {}, {}, {}",
        size.x, size.y, size.z, object.centre.x, object.centre.y, object.centre.z
    );
    proposal_said(&mut text, &proposal);
    for note in &planned.notes {
        let _ = writeln!(text, "  note: {note}");
    }
    let what = if moving {
        "move the proposal"
    } else {
        "take the proposal out"
    };
    let mut applied_json = Value::Null;
    if on("--apply") || moving {
        let applied = applied(
            &folder,
            &planned.operations,
            dry_run,
            flags.get("--inverse"),
        )?;
        let changed = applied_said(
            &mut text,
            id,
            &planned.operations,
            &applied,
            dry_run,
            flags.get("--inverse"),
        );
        applied_json = json!({ "changed": changed, "written": !dry_run });
    } else {
        let _ = writeln!(
            text,
            "{} operation(s) would {what}:",
            planned.operations.len()
        );
        for operation in &planned.operations {
            let _ = writeln!(text, "  {}", operation_said(operation));
        }
        let _ = writeln!(
            text,
            "Nothing was changed. --apply takes the proposal out, --move and --turn move it, and --json gives the operations as a list to edit."
        );
    }
    if !json_out {
        return Ok(text);
    }
    let mut said = proposal_json(&proposal)?;
    said["module"] = json!(id);
    said["object"] = json!({
        "source": source,
        "entity": entity,
        "centre": object.centre.to_array(),
        "rotation": object.rotation.to_array(),
        "half_extents": object.half_extents.to_array(),
    });
    said["notes"] = json!(planned.notes);
    said["operations"] = serde_json::to_value(Operations::new(planned.operations))
        .map_err(|e| Run(e.to_string()))?;
    said["applied"] = applied_json;
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&said).map_err(|e| Run(e.to_string()))?
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use or_io::decl::File;
    use or_maps::collision::sample::{area_bound, Triangle};
    use or_maps::import::{add_files, Incoming};
    use or_maps::sample::{map_file, types_file, Archetype, Entity};
    use or_source::textures::Extraction;

    use super::*;

    const HOUSE: ([f32; 3], [f32; 3]) = ([0.0; 3], [10.0, 8.0, 4.0]);
    const POST: ([f32; 3], [f32; 3]) = ([-0.1, -0.1, 0.0], [0.1, 0.1, 1.2]);
    const VERSIONS: [&str; 3] = ["street_1", "hi@street_1", "ma@street_1"];

    /// A level floor of two triangles and the four walls and the roof of the house, which stands on the origin.
    fn street() -> Vec<Triangle> {
        let mut triangles = Vec::new();
        let mut quad = |[a, b, c, d]: [[f32; 3]; 4], slot: u8| {
            triangles.push(([a, b, c], slot));
            triangles.push(([a, c, d], slot));
        };
        // The floor's diagonal passes under the middle of the house, so the box of the house cuts both of its triangles.
        quad(
            [
                [-20.0, -20.0, 0.0],
                [30.0, -20.0, 0.0],
                [30.0, 28.0, 0.0],
                [-20.0, 28.0, 0.0],
            ],
            0,
        );
        let (lo, hi) = HOUSE;
        for (from, to) in [
            ([lo[0], lo[1]], [hi[0], lo[1]]),
            ([hi[0], lo[1]], [hi[0], hi[1]]),
            ([hi[0], hi[1]], [lo[0], hi[1]]),
            ([lo[0], hi[1]], [lo[0], lo[1]]),
        ] {
            quad(
                [
                    [from[0], from[1], 0.0],
                    [to[0], to[1], 0.0],
                    [to[0], to[1], hi[2]],
                    [from[0], from[1], hi[2]],
                ],
                1,
            );
        }
        quad(
            [
                [lo[0], lo[1], hi[2]],
                [hi[0], lo[1], hi[2]],
                [hi[0], hi[1], hi[2]],
                [lo[0], hi[1], hi[2]],
            ],
            1,
        );
        triangles
    }

    /// A project whose maps content places a house and a fence post and holds the three versions of the collision of their street.
    fn a_project(dir: &Path) -> PathBuf {
        std::fs::write(
            dir.join("street.openrage-proj"),
            r#"{
  "documents": [],
  "game": "both",
  "modules": [{ "id": "maps", "path": "maps", "type": "maps" }],
  "open": [],
  "outputs": [{ "id": "fivem", "kind": "fivem-resource", "name": "street", "take": ["maps"] }],
  "schema": 4
}"#,
        )
        .unwrap();
        let map = map_file(
            "street",
            &[
                Entity::at("street_house", [0.0; 3], 1),
                Entity::at("street_post", [3.0, 3.0, 0.0], 2),
            ],
            ([-100.0; 3], [100.0; 3]),
        );
        let types = types_file(
            "street_types",
            &[
                Archetype::new("street_house", HOUSE.0, HOUSE.1),
                Archetype::new("street_post", POST.0, POST.1),
            ],
        );
        let incoming = |name: &'static str, bytes: &'static [u8]| Incoming {
            name,
            bytes,
            from_game: false,
            virtual_path: None,
        };
        let folder = dir.join("maps");
        let added = add_files(
            &folder,
            &[
                incoming("street.ymap", map.leak()),
                incoming("street_types.ytyp", types.leak()),
            ],
            None,
        )
        .unwrap();
        assert_eq!(added.refused, Vec::<(String, String)>::new());
        let how = Extraction {
            from_game: false,
            virtual_path: None,
        };
        for name in VERSIONS {
            let file = area_bound(&street(), &[0x11, 0x22], [0.0; 3])
                .unwrap()
                .to_bytes()
                .unwrap();
            or_source::bounds::extract(&file, name, &folder, &how).unwrap();
        }
        dir.join("street.openrage-proj")
    }

    fn run(project: &Path, more: &[&str]) -> Result<String, String> {
        let mut args = vec![project.to_str().unwrap().to_string()];
        args.extend(more.iter().map(|a| a.to_string()));
        report(&args).map_err(|e| match e {
            Usage(why) | Run(why) => why,
        })
    }

    /// The files of every bounds source of the content, by path.
    fn bounds_files(project: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let folder = project.parent().unwrap().join("maps");
        let mut out = BTreeMap::new();
        for name in VERSIONS {
            let dir = folder.join(format!("{name}.bounds.src"));
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                out.insert(entry.path(), std::fs::read(entry.path()).unwrap());
            }
        }
        out
    }

    const HOUSE_IS: &str = "street.metadata.src:0";

    #[test]
    fn the_proposal_for_an_object_is_printed_source_by_source_and_nothing_is_written() {
        let work = tempfile::tempdir().unwrap();
        let project = a_project(work.path());
        let before = bounds_files(&project);
        let text = run(&project, &["--object", HOUSE_IS]).unwrap();
        assert!(
            text.starts_with(
                "content maps: the object is entity 0 of street.metadata.src, a box 10 by 8 by 4 m about 5, 4, 2\n"
            ),
            "{text}"
        );
        for name in VERSIONS {
            assert!(
                text.contains(&format!("  {name}.bounds.src ({name}.ybn)\n")),
                "{text}"
            );
        }
        // Ten triangles of the house in each version, and the two of the floor left as ground.
        assert_eq!(
            text.matches("    bound_child0, a PhBoundBVH: 10 proposed, 2 left\n")
                .count(),
            3,
            "{text}"
        );
        assert_eq!(text.matches("      left as ground: ").count(), 3, "{text}");
        assert!(
            text.contains("3 operation(s) would take the proposal out:\n"),
            "{text}"
        );
        assert!(
            text.contains(
                "  remove_polygons of street_1.bounds.src: 10 polygon(s) of `bound_child0`\n"
            ),
            "{text}"
        );
        assert!(text.contains("Nothing was changed."), "{text}");
        assert!(before == bounds_files(&project));

        // Asked for, the ground is judged like any polygon: the box cuts both triangles of the floor and holds little of either.
        let with_ground = run(&project, &["--object", HOUSE_IS, "--take-ground"]).unwrap();
        assert_eq!(
            with_ground.matches("      left as ground: ").count(),
            0,
            "{with_ground}"
        );
        assert_eq!(
            with_ground
                .matches("      left, cut with too little of it in the box: ")
                .count(),
            3,
            "{with_ground}"
        );

        // The box holds 40.905 m² of each triangle of the floor, which is 1,200 m².
        assert_eq!(
            with_ground.matches(" (3% in the box)").count(),
            6,
            "{with_ground}"
        );

        let post = run(&project, &["--object", "street.metadata.src:1"]).unwrap();
        assert!(
            post.contains("the object stands 1.2 m tall, under the 2 m from which collision is looked for: nothing is proposed"),
            "{post}"
        );
        assert!(
            post.contains("0 operation(s) would take the proposal out:"),
            "{post}"
        );
    }

    #[test]
    fn the_proposal_as_json_holds_a_list_that_applies_and_the_inverse_it_writes_takes_it_back() {
        let work = tempfile::tempdir().unwrap();
        let project = a_project(work.path());
        let before = bounds_files(&project);
        let said: Value =
            serde_json::from_str(&run(&project, &["--object", HOUSE_IS, "--json"]).unwrap())
                .unwrap();
        assert_eq!(said["module"], "maps");
        assert_eq!(said["height"], 4.0);
        assert_eq!(said["not_judged"], Value::Null);
        assert_eq!(said["applied"], Value::Null);
        let bounds = said["bounds"].as_array().unwrap();
        assert_eq!(bounds.len(), 3);
        for bound in bounds {
            assert_eq!(bound["area"], "street_1");
            let leaf = &bound["leaves"][0];
            assert_eq!(leaf["address"], "bound_child0");
            assert_eq!(leaf["kind"], "PhBoundBVH");
            assert_eq!(leaf["proposed"].as_array().unwrap().len(), 10);
            let left = leaf["left"].as_array().unwrap();
            assert_eq!(left.len(), 2);
            assert!(left.iter().all(|l| l["reason"] == "ground"), "{left:?}");
        }
        assert!(
            before == bounds_files(&project),
            "a proposal writes nothing"
        );

        // The list is what a person edits: taken as it is, it changes the three versions, and a dry run of it changes none.
        let list = work.path().join("take.json");
        let inverse = work.path().join("back.json");
        std::fs::write(
            &list,
            serde_json::to_string_pretty(&said["operations"]).unwrap(),
        )
        .unwrap();
        let (list, inverse) = (list.to_str().unwrap(), inverse.to_str().unwrap());
        // An operation that does not say it is for its source alone is done to every version, and is said so.
        let mut of_every_version = said["operations"].clone();
        of_every_version["operations"]
            .as_array_mut()
            .unwrap()
            .retain(|operation| operation["source"] == "street_1.bounds.src");
        assert_eq!(of_every_version["operations"][0]["alone"], true);
        of_every_version["operations"][0]
            .as_object_mut()
            .unwrap()
            .remove("alone");
        let through_one = work.path().join("through_one.json");
        std::fs::write(
            &through_one,
            serde_json::to_string_pretty(&of_every_version).unwrap(),
        )
        .unwrap();
        let dry = run(
            &project,
            &["--operations", through_one.to_str().unwrap(), "--dry-run"],
        )
        .unwrap();
        assert!(
            dry.contains("  1. remove_polygons of street_1.bounds.src: 10 polygon(s) of `bound_child0`, and the same of every version of its area\n")
                && dry.contains("content maps: 1 operation(s) would change 3 bounds source(s); nothing was written"),
            "{dry}"
        );
        let dry = run(&project, &["--operations", list, "--dry-run"]).unwrap();
        assert!(
            dry.contains(
                "content maps: 3 operation(s) would change 3 bounds source(s); nothing was written"
            ),
            "{dry}"
        );
        assert!(before == bounds_files(&project));
        let done = run(&project, &["--operations", list, "--inverse", inverse]).unwrap();
        assert!(
            done.contains("content maps: 3 operation(s) changed 3 bounds source(s)\n"),
            "{done}"
        );
        for name in VERSIONS {
            assert!(
                done.contains(&format!("changed {name}.bounds.src\n")),
                "{done}"
            );
            assert!(
                done.contains(&format!(
                    "{name}.bounds.src: 10 polygon(s) taken out of `bound_child0`"
                )),
                "{done}"
            );
        }
        let changed = bounds_files(&project);
        assert!(before != changed);
        // The list names places in the sources as they were, so it is refused now that they are others.
        let stale = run(&project, &["--operations", list]).expect_err("a stale list");
        assert!(
            stale.contains("is not the source the polygons were chosen in")
                && stale.ends_with("; no source was changed"),
            "{stale}"
        );
        assert!(changed == bounds_files(&project));
        // With the house gone nothing is proposed for its box, and the floor is still left as ground.
        let after = run(&project, &["--object", HOUSE_IS]).unwrap();
        assert_eq!(
            after
                .matches("    bound_child0, a PhBoundBVH: 0 proposed, 2 left\n")
                .count(),
            3,
            "{after}"
        );

        let back = run(&project, &["--operations", inverse]).unwrap();
        assert!(
            back.contains(
                "  3. restore_sources: the document and the glb of 1 bounds source(s) as they were"
            ),
            "{back}"
        );
        assert!(
            before == bounds_files(&project),
            "every file is back to the byte"
        );
    }

    #[test]
    fn the_flag_applies_the_proposal_and_a_move_carries_it_and_what_is_asked_wrongly_is_said() {
        let work = tempfile::tempdir().unwrap();
        let project = a_project(work.path());
        let before = bounds_files(&project);
        let dry = run(&project, &["--object", HOUSE_IS, "--apply", "--dry-run"]).unwrap();
        assert!(
            dry.contains("would change 3 bounds source(s); nothing was written"),
            "{dry}"
        );
        assert!(before == bounds_files(&project));

        let inverse = work.path().join("back.json");
        let inverse = inverse.to_str().unwrap();
        let moved = run(
            &project,
            &[
                "--object",
                HOUSE_IS,
                "--move",
                "-12,-9,0",
                "--turn",
                "90",
                "--inverse",
                inverse,
            ],
        )
        .unwrap();
        assert!(
            moved.contains(
                "  3. move_polygons of street_1.bounds.src: 10 polygon(s) of `bound_child0`\n"
            ),
            "{moved}"
        );
        assert!(moved.contains("changed 3 bounds source(s)"), "{moved}");
        // Where the house stood nothing of it is left, and it is found whole where it was carried: turned a quarter about its
        // origin, its box is eight by ten there.
        let stood = run(&project, &["--object", HOUSE_IS]).unwrap();
        assert_eq!(stood.matches(": 0 proposed, ").count(), 3, "{stood}");
        let carried = run(
            &project,
            &["--object", HOUSE_IS, "--box", "-20,-9,0,-12,1,4"],
        )
        .unwrap();
        assert_eq!(carried.matches(": 10 proposed, ").count(), 3, "{carried}");
        run(&project, &["--operations", inverse]).unwrap();
        assert!(before == bounds_files(&project));

        let applied = run(&project, &["--object", HOUSE_IS, "--apply"]).unwrap();
        assert!(
            applied.contains("content maps: 3 operation(s) changed 3 bounds source(s)\n"),
            "{applied}"
        );
        assert!(before != bounds_files(&project));

        for (args, why) in [
            (
                vec!["--object", HOUSE_IS, "--apply", "--move", "1,0,0"],
                "ask for one",
            ),
            (vec![], "name the object with --object"),
            (vec!["--object", "street.metadata.src"], "is not one"),
            (
                vec!["--object", HOUSE_IS, "--box", "1,2,3"],
                "--box takes 6 numbers",
            ),
            (
                vec!["--object", "street.metadata.src:7"],
                "entity 7 is past the 2",
            ),
            (
                vec!["--object", HOUSE_IS, "--operations", "x.json"],
                "and not both",
            ),
        ] {
            let said = run(&project, &args).expect_err(why);
            assert!(said.contains(why), "{args:?}: {said}");
        }
    }

    #[test]
    fn the_list_that_takes_an_edit_back_is_kept_before_a_source_is_written() {
        let work = tempfile::tempdir().unwrap();
        let project = a_project(work.path());
        let before = bounds_files(&project);
        // Nowhere to keep the way back: the edit is not made.
        let nowhere = work.path().join("no such folder").join("back.json");
        let said = run(
            &project,
            &[
                "--object",
                HOUSE_IS,
                "--apply",
                "--inverse",
                nowhere.to_str().unwrap(),
            ],
        )
        .expect_err("the inverse cannot be written");
        assert!(said.ends_with("; no source was changed"), "{said}");
        assert!(before == bounds_files(&project));

        // Two operations on one source, each on the source as the one before left it: the list that takes them back undoes the
        // second first.
        let twice = Operations::new(
            (0..2)
                .map(|_| Operation::RemovePolygons {
                    source: "street_1.bounds.src".to_string(),
                    soup: "bound_child0".to_string(),
                    polygons: vec![0],
                    expects: None,
                    alone: true,
                })
                .collect(),
        );
        let (list, back) = (
            work.path().join("twice.json"),
            work.path().join("back.json"),
        );
        std::fs::write(&list, twice.to_json().unwrap()).unwrap();
        let (list, back) = (list.to_str().unwrap(), back.to_str().unwrap());
        let done: Value = serde_json::from_str(
            &run(
                &project,
                &["--operations", list, "--inverse", back, "--json"],
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            done,
            json!({ "module": "maps", "changed": ["street_1.bounds.src"], "written": true })
        );
        assert!(before != bounds_files(&project));
        run(&project, &["--operations", back]).unwrap();
        assert!(before == bounds_files(&project));

        // As data, an applied proposal says what it changed and whether it wrote.
        for (more, written) in [(vec!["--dry-run"], false), (vec![], true)] {
            let mut args = vec!["--object", HOUSE_IS, "--apply", "--json"];
            args.extend(more);
            let said: Value = serde_json::from_str(&run(&project, &args).unwrap()).unwrap();
            assert_eq!(said["applied"]["written"], written);
            assert_eq!(said["applied"]["changed"].as_array().unwrap().len(), 3);
        }
    }

    #[test]
    fn places_are_printed_as_runs() {
        assert_eq!(ranges(&[]), "");
        assert_eq!(ranges(&[4]), "4");
        assert_eq!(ranges(&[1, 2, 3, 7, 9, 10]), "1-3, 7, 9-10");
    }
}
