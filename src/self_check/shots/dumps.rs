//! The `comparedump` step: a `world dump` held to a reference dump, field by field over a stated
//! list, with what a run took left out.
//!
//! ⚠ A field of a dump that neither list names fails the comparison, so that a field added to
//! the dump is compared or left out on purpose.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::script::{file_stem, split_word};

/// The fields two runs of one build hold alike at a settled camera: where the camera is, what is switched on, what streams, each node that draws with how opaque, the far cards, how many instances the frame was given and saw, and whether the game's passes lit it.
const COMPARED: [&str; 24] = [
    "camera",
    "selected_eye",
    "hour",
    "ground",
    "lod_scale",
    "modes",
    "groups_on",
    "streaming.maps_wanted",
    "streaming.maps_loaded",
    "streaming.maps_waiting",
    "streaming.nodes",
    "streaming.drawn",
    "wanted_map_names",
    "loaded_map_names",
    "nodes_by_level",
    "drawn_by_level",
    "list_instances",
    "drawn_nodes",
    "swaps",
    "jumps",
    "cards",
    "frame.lit_by_the_game",
    "frame.selected",
    "frame.seen",
];

/// The fields left out, which two runs of one build do not hold alike: how long a selection or a read took and how many reads a run made on its way, and what the device holds, which follows the order the reads arrived in.
// ⚠ Measured on two runs of one build: the textures on the device differed in one key in a hundred, the stand-ins' slots in most, and the draws of a frame by four in 4,775.
const LEFT_OUT: [&str; 18] = [
    "selection_ms",
    "asked_ms",
    "handed_ms",
    "streaming.maps_read",
    "streaming.map_waits_s",
    "streaming.map_wait_most_s",
    "streaming.map_oldest_s",
    "models_whole",
    "models_taken",
    "textures_taken",
    "device_textures",
    "device_stand_ins",
    "frame.draws",
    "frame.models",
    "frame.textures",
    "frame.textures_unread",
    "frame.pages",
    "frame.bytes",
];

/// How many entries of a list that differs are said.
const SAID: usize = 4;

/// A dump to hold to a reference.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CompareDump {
    /// The name an earlier `world dump` wrote it under.
    pub dump: String,
    pub reference: PathBuf,
}

/// The comparison the words after `comparedump` name: a dump's name and a reference dump.
pub(super) fn parse(rest: &str) -> Result<CompareDump, String> {
    let (dump, reference) = split_word(rest);
    if reference.is_empty() {
        return Err("needs a dump's name and a reference dump".into());
    }
    Ok(CompareDump {
        dump: file_stem(dump)?,
        reference: PathBuf::from(reference),
    })
}

/// A dump's fields by the names the lists use: a member of an object a list looks into is `OBJECT.MEMBER`.
fn fields(dump: &Map<String, Value>) -> Vec<(String, &Value)> {
    let looked_into = |name: &str| {
        (COMPARED.iter().chain(&LEFT_OUT)).any(|path| {
            path.split_once('.')
                .is_some_and(|(object, _)| object == name)
        })
    };
    let mut out = Vec::new();
    for (name, value) in dump {
        match value.as_object().filter(|_| looked_into(name)) {
            Some(members) => out.extend(
                (members.iter()).map(|(member, value)| (format!("{name}.{member}"), value)),
            ),
            None => out.push((name.clone(), value)),
        }
    }
    out
}

/// How `ours` differs from `theirs`, in a line: for two lists of names, how many entries each holds alone and the first of them.
fn apart(ours: &Value, theirs: &Value) -> String {
    // A list of names is said by what each holds alone; a place or a count is said whole.
    fn names(value: &Value) -> Option<&Vec<Value>> {
        (value.as_array()).filter(|list| list.iter().all(Value::is_string))
    }
    let short = |value: &Value| {
        let text = value.to_string();
        match text.char_indices().nth(120) {
            Some((at, _)) => format!("{}…", &text[..at]),
            None => text,
        }
    };
    let (Some(ours), Some(theirs)) = (names(ours), names(theirs)) else {
        return format!(
            "{} where the reference holds {}",
            short(ours),
            short(theirs)
        );
    };
    let set = |list: &[Value]| -> BTreeSet<String> { list.iter().map(Value::to_string).collect() };
    let (ours, theirs) = (set(ours), set(theirs));
    let alone = |a: &BTreeSet<String>, b: &BTreeSet<String>| {
        let alone: Vec<&String> = a.difference(b).collect();
        let said: Vec<&str> = alone.iter().take(SAID).map(|s| s.as_str()).collect();
        (alone.len(), said.join(" "))
    };
    let ((mine, first), (other, theirs_first)) = (alone(&ours, &theirs), alone(&theirs, &ours));
    if mine + other == 0 {
        return "the same entries in another order or number".into();
    }
    format!("{mine} entries the reference lacks ({first}), {other} it holds alone ({theirs_first})")
}

/// Every way the dump `ours` differs from the reference `theirs` over the compared fields, and every field no list names.
pub(super) fn differences(ours: &Value, theirs: &Value) -> Result<Vec<String>, String> {
    let object = |value: &'_ Value, what: &str| {
        (value.as_object().cloned()).ok_or(format!("{what} is not a dump: no JSON object"))
    };
    let (ours, theirs) = (object(ours, "the dump")?, object(theirs, "the reference")?);
    let (ours, theirs) = (fields(&ours), fields(&theirs));
    let mut found = Vec::new();
    for (name, _) in ours.iter().chain(&theirs) {
        let named = COMPARED.contains(&name.as_str()) || LEFT_OUT.contains(&name.as_str());
        let said = found.iter().any(|line: &String| line.starts_with(name));
        if !named && !said {
            found.push(format!(
                "{name} is a field neither compared nor left out: name it in one list"
            ));
        }
    }
    let of = |fields: &[(String, &Value)], name: &str| {
        (fields.iter().find(|(field, _)| field == name)).map(|(_, value)| (*value).clone())
    };
    for name in COMPARED {
        match (of(&ours, name), of(&theirs, name)) {
            (Some(ours), Some(theirs)) if ours == theirs => {}
            (Some(ours), Some(theirs)) => found.push(format!("{name}: {}", apart(&ours, &theirs))),
            (None, None) => found.push(format!("{name}: neither dump holds it")),
            (None, _) => found.push(format!("{name}: the dump does not hold it")),
            (_, None) => found.push(format!("{name}: the reference does not hold it")),
        }
    }
    Ok(found)
}

/// Hold the dump `compare` names in `out` to its reference: an error where a compared field differs, a field is named by no list, or either file is missing.
pub(super) fn run(compare: &CompareDump, out: &Path) -> Result<(), String> {
    let dump = out.join(&compare.dump);
    if !dump.is_file() {
        return Err(format!(
            "no dump `{}` was written before its comparison",
            compare.dump
        ));
    }
    if !compare.reference.is_file() {
        return Err(format!(
            "{}: no reference at {}",
            compare.dump,
            compare.reference.display()
        ));
    }
    let open = |path: &Path| -> Result<Value, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("{} is not JSON: {e}", path.display()))
    };
    let found = differences(&open(&dump)?, &open(&compare.reference)?)
        .map_err(|why| format!("{}: {why}", compare.dump))?;
    if found.is_empty() {
        println!(
            "comparedump  {}: {} fields alike",
            compare.dump,
            COMPARED.len()
        );
        return Ok(());
    }
    Err(format!(
        "{}: {} of {} fields differ\n    {}",
        compare.dump,
        found.len(),
        COMPARED.len().max(found.len()),
        found.join("\n    ")
    ))
}

#[cfg(test)]
#[path = "dumps_tests.rs"]
mod tests;
