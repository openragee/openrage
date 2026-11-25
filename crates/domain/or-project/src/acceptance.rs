//! The acceptance sheet: what a person looks for in a running game, by the kind of file a build wrote, and what was seen there.
//!
//! Nothing in the tree runs GTA V, so a build says only that its files read back, and `acceptance.json` beside this file is where a
//! run of the game is written down.
//!
//! ⚠ Nothing here writes an observation, and a line with none is not checked in game, which is all that may be said of it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Map, Value};

use crate::build::files_under;
use crate::built::{unpack, BuiltKind};
use crate::container::Target;
use crate::dlc::PACK;
use crate::oiv::EXTENSION;
use crate::plan::MANIFEST;
use crate::readers;

/// The sheet the tree ships, as the text of its file.
pub const SHEET: &str = include_str!("acceptance.json");

/// The `format` a sheet states.
pub const FORMAT: &str = "openrage-acceptance";
/// The newest `schema` of a sheet this reads.
pub const SCHEMA: u64 = 1;

/// Why a text is not a sheet.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SheetError {
    /// The file as a whole is refused.
    #[error("the acceptance sheet {0}")]
    Sheet(String),
    /// A line with no id to name it by, told by its place in the file, counted from one.
    #[error("line {place} of the acceptance sheet {why}")]
    Unnamed { place: usize, why: String },
    /// A line is refused, by its id.
    #[error("line `{id}` of the acceptance sheet {why}")]
    Line { id: String, why: String },
}

/// The lines of a sheet, in the order of its file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sheet {
    pub lines: Vec<Line>,
}

/// One thing to look for in a running game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Stable for as long as the line exists and never reused: lower-case words joined by `-`.
    pub id: String,
    /// The kind of file it is about, as [`kind_of`] says a built file's.
    pub kind: String,
    /// The row of the Roadmap that writes that kind, such as `R32`.
    pub row: String,
    /// What to do in the game, in one sentence.
    pub action: String,
    /// What to look for there, in one sentence.
    pub look_for: String,
    /// Every run a person wrote down, in the order of the file.
    pub results: Vec<Observation>,
}

/// One run of the game by a person, and what it showed of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub edition: Target,
    /// The game's build as the person read it, such as its build number.
    pub build: String,
    pub place: Place,
    /// The day of the run, `YYYY-MM-DD`, so that the text orders as the calendar does.
    pub date: String,
    pub verdict: Verdict,
}

/// Where a build was loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    FivemServer,
    StoryMode,
}

impl Place {
    pub const ALL: [Place; 2] = [Self::FivemServer, Self::StoryMode];

    /// The word a sheet stores.
    pub fn word(self) -> &'static str {
        match self {
            Self::FivemServer => "fivem-server",
            Self::StoryMode => "story-mode",
        }
    }

    /// The place as a sentence names it.
    pub fn label(self) -> &'static str {
        match self {
            Self::FivemServer => "on a FiveM server",
            Self::StoryMode => "in story mode",
        }
    }
}

/// What the game did with a build, as the person saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Loaded,
    NotLoaded,
    /// The game loaded it and something was wrong, said in a sentence.
    LoadedWithFault(String),
}

const LOADED: &str = "loaded";
const NOT_LOADED: &str = "not-loaded";
const LOADED_WITH_FAULT: &str = "loaded-with-fault";

impl Verdict {
    /// The words a sheet stores, in the order the guide lists them.
    pub const WORDS: [&'static str; 3] = [LOADED, NOT_LOADED, LOADED_WITH_FAULT];
}

/// A line a build concerns, and what the sheet holds of it for one edition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Concern<'a> {
    pub line: &'a Line,
    /// The newest run of the line for the edition, none when nobody wrote one down.
    pub newest: Option<&'a Observation>,
}

impl Concern<'_> {
    /// Whether the newest run for the edition says the game loaded it, with no fault.
    pub fn loaded(&self) -> bool {
        self.newest
            .is_some_and(|seen| seen.verdict == Verdict::Loaded)
    }
}

impl Line {
    /// The newest run of the line for `edition`; of two on one day, the later in the file.
    pub fn newest(&self, edition: Target) -> Option<&Observation> {
        (self.results.iter())
            .filter(|seen| seen.edition == edition)
            .max_by(|a, b| a.date.cmp(&b.date))
    }
}

impl Sheet {
    /// The sheet the tree ships.
    pub fn shipped() -> Result<Self, SheetError> {
        Self::parse(SHEET)
    }

    /// The sheet `text` holds, or the first thing that makes it none, with the id of the line it is in.
    pub fn parse(text: &str) -> Result<Self, SheetError> {
        let whole = SheetError::Sheet;
        let top: Value =
            serde_json::from_str(text).map_err(|e| whole(format!("is not JSON: {e}")))?;
        let top = top
            .as_object()
            .ok_or_else(|| whole("is not a JSON object".into()))?;
        match top.get("format").and_then(Value::as_str) {
            Some(FORMAT) => {}
            other => {
                return Err(whole(format!(
                    "states the format {}, not `{FORMAT}`",
                    other.map_or("of nothing".to_string(), |f| format!("`{f}`"))
                )))
            }
        }
        match top.get("schema").and_then(Value::as_u64) {
            Some(schema) if schema <= SCHEMA => {}
            Some(newer) => {
                return Err(whole(format!(
                    "was written by a newer OpenRage (schema {newer}) than this one reads (schema {SCHEMA})"
                )))
            }
            None => return Err(whole("states no schema".into())),
        }
        only_keys(top, &["format", "schema", "lines"]).map_err(whole)?;
        let lines = top
            .get("lines")
            .and_then(Value::as_array)
            .ok_or_else(|| whole("holds no list of `lines`".into()))?;
        let mut sheet = Self::default();
        for (index, entry) in lines.iter().enumerate() {
            let line = read_line(entry, index + 1)?;
            if sheet.lines.iter().any(|held| held.id == line.id) {
                return Err(SheetError::Line {
                    id: line.id,
                    why: "is there twice: an id names one line".into(),
                });
            }
            sheet.lines.push(line);
        }
        Ok(sheet)
    }

    /// The lines about files of `kinds`, in the sheet's order, each with what the sheet holds of it for `edition`.
    pub fn about(&self, kinds: &BTreeSet<String>, edition: Target) -> Vec<Concern<'_>> {
        (self.lines.iter())
            .filter(|line| kinds.contains(&line.kind))
            .map(|line| Concern {
                line,
                newest: line.newest(edition),
            })
            .collect()
    }

    /// The kinds of `kinds` no line is about, which the sheet says nothing of.
    pub fn without_a_line<'a>(&self, kinds: &'a BTreeSet<String>) -> Vec<&'a str> {
        (kinds.iter())
            .filter(|kind| !self.lines.iter().any(|line| line.kind == **kind))
            .map(String::as_str)
            .collect()
    }
}

/// The kind of file `path` is to a build: its extension in lower case, none for a name that has none.
pub fn kind_of(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next()?;
    let (stem, extension) = name.rsplit_once('.')?;
    (!stem.is_empty() && !extension.is_empty()).then(|| extension.to_ascii_lowercase())
}

/// Whether a build can tell a file of `kind` that it wrote: one it opens again through a reader, or the archive a pack is.
pub fn knows(kind: &str) -> bool {
    readers::reads(kind) || kind_of(PACK).as_deref() == Some(kind)
}

/// How many files of each kind the build at `built` holds, a resource folder or a pack; a pack's archive and its package are each one file more.
pub fn kinds_in(built: &Path) -> Result<BTreeMap<String, usize>, String> {
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut count = |path: &str| {
        if let Some(kind) = kind_of(path) {
            *kinds.entry(kind).or_default() += 1;
        }
    };
    if is_packed(built) {
        let mut holds_a_pack = false;
        let kind = unpack(built, &mut |path, of_the_pack, _| {
            holds_a_pack |= of_the_pack;
            count(path);
            Ok(())
        })?;
        if kind == BuiltKind::Oiv {
            count(&format!("package.{EXTENSION}"));
        }
        if kind == BuiltKind::Dlc || holds_a_pack {
            count(PACK);
        }
    } else if built.join(MANIFEST).is_file() {
        for path in files_under(built) {
            count(&path);
        }
    } else {
        // ⚠ A project's folder and the folder it builds into hold files too, and their kinds are not what a build wrote.
        return Err(format!(
            "{} is not a folder a build wrote: it holds no {MANIFEST}, no {PACK} and no .{EXTENSION} package",
            built.display()
        ));
    }
    Ok(kinds)
}

/// Whether `built` is a pack or a package, or the folder a build wrote one into.
fn is_packed(built: &Path) -> bool {
    let has = |extension: &str| {
        built
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case(extension))
    };
    if built.is_file() {
        return has("rpf") || has(EXTENSION);
    }
    built.join(PACK).is_file()
        || std::fs::read_dir(built)
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| {
                let path = entry.path();
                path.is_file()
                    && path
                        .extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case(EXTENSION))
            })
}

/// The first key of `object` that is none of `known`, as the reason to refuse it.
fn only_keys(object: &Map<String, Value>, known: &[&str]) -> Result<(), String> {
    match object.keys().find(|key| !known.contains(&key.as_str())) {
        Some(key) => Err(format!(
            "has the key `{key}`, which is none of {}",
            known.join(", ")
        )),
        None => Ok(()),
    }
}

/// The text `object` holds at `key`, trimmed, when it is one line of text that is not empty.
fn sentence<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    match object.get(key).and_then(Value::as_str).map(str::trim) {
        Some(text) if !text.is_empty() && !text.contains(['\n', '\r']) => Ok(text),
        Some(_) => Err(format!("has a `{key}` that is empty or runs over a line")),
        None => Err(format!("states no `{key}` as text")),
    }
}

/// Whether `text` is lower-case letters and digits in words joined by `-`.
fn is_id(text: &str) -> bool {
    !text.is_empty()
        && text.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Whether `text` has the shape of a row of the Roadmap: `R`, a number, and at most one lower-case letter.
fn is_row(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('R') else {
        return false;
    };
    let digits = rest.trim_end_matches(|c: char| c.is_ascii_lowercase());
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && rest.len() - digits.len() <= 1
}

/// Whether `text` is a day of the calendar written `YYYY-MM-DD`.
fn is_date(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return false;
    };
    let digits = |part: &str, width: usize| {
        (part.len() == width && part.bytes().all(|b| b.is_ascii_digit()))
            .then(|| part.parse::<u32>().ok())
            .flatten()
    };
    let (Some(year), Some(month), Some(day)) = (digits(year, 4), digits(month, 2), digits(day, 2))
    else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

fn read_line(entry: &Value, place: usize) -> Result<Line, SheetError> {
    let unnamed = |why: &str| SheetError::Unnamed {
        place,
        why: why.to_string(),
    };
    let object = entry
        .as_object()
        .ok_or_else(|| unnamed("is not a JSON object"))?;
    let id = match object.get("id").and_then(Value::as_str) {
        Some(id) if is_id(id) => id.to_string(),
        Some(id) => {
            return Err(unnamed(&format!(
                "has the id `{id}`, which is not lower-case words joined by `-`"
            )))
        }
        None => return Err(unnamed("states no `id`")),
    };
    let refused = |why: String| SheetError::Line {
        id: id.clone(),
        why,
    };
    only_keys(object, &["id", "kind", "row", "do", "look_for", "results"]).map_err(refused)?;
    let kind = sentence(object, "kind").map_err(refused)?;
    if !knows(kind) {
        return Err(refused(format!(
            "is about files of the kind `{kind}`, which no build reads back"
        )));
    }
    let row = sentence(object, "row").map_err(refused)?;
    if !is_row(row) {
        return Err(refused(format!(
            "names the row `{row}`, which is not a row of the Roadmap such as R32 or R70a"
        )));
    }
    let said = |key: &str| match sentence(object, key) {
        // ⚠ The guide holds each sentence in a cell of a table, which a `|` would end.
        Ok(text) if text.contains('|') => Err(refused(format!("has a `|` in its `{key}`"))),
        Ok(text) => Ok(text.to_string()),
        Err(why) => Err(refused(why)),
    };
    let (action, look_for) = (said("do")?, said("look_for")?);
    let results = object
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| refused("holds no list of `results`".into()))?;
    let results = results
        .iter()
        .enumerate()
        .map(|(index, result)| {
            read_result(result)
                .map_err(|why| refused(format!("has a result {} that {why}", index + 1)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Line {
        id: id.clone(),
        kind: kind.to_string(),
        row: row.to_string(),
        action,
        look_for,
        results,
    })
}

/// One result of a line, or what is wrong with it, as the end of a sentence that begins "a result that".
fn read_result(entry: &Value) -> Result<Observation, String> {
    let object = entry
        .as_object()
        .ok_or_else(|| "is not a JSON object".to_string())?;
    only_keys(
        object,
        &["edition", "build", "where", "date", "verdict", "fault"],
    )?;
    let edition = sentence(object, "edition")?;
    let edition = Target::from_word(edition).ok_or_else(|| {
        format!("states the edition `{edition}`, which is neither legacy nor enhanced")
    })?;
    let build = sentence(object, "build")?.to_string();
    let place = sentence(object, "where")?;
    let place = (Place::ALL.into_iter())
        .find(|known| known.word() == place)
        .ok_or_else(|| {
            format!(
                "was loaded in `{place}`, which is neither {} nor {}",
                Place::FivemServer.word(),
                Place::StoryMode.word()
            )
        })?;
    let date = sentence(object, "date")?;
    if !is_date(date) {
        return Err(format!(
            "is dated `{date}`, which is not a day written YYYY-MM-DD"
        ));
    }
    let fault = match object.get("fault") {
        Some(_) => Some(sentence(object, "fault")?),
        None => None,
    };
    let verdict = match (sentence(object, "verdict")?, fault) {
        (LOADED, None) => Verdict::Loaded,
        (NOT_LOADED, None) => Verdict::NotLoaded,
        (LOADED_WITH_FAULT, Some(fault)) => Verdict::LoadedWithFault(fault.to_string()),
        (LOADED_WITH_FAULT, None) => {
            return Err(format!("says {LOADED_WITH_FAULT} and states no `fault`"))
        }
        (verdict, Some(_)) if Verdict::WORDS.contains(&verdict) => {
            return Err(format!("says {verdict} and states a `fault`"))
        }
        (verdict, _) => {
            return Err(format!(
                "has the verdict `{verdict}`, which is none of {}",
                Verdict::WORDS.join(", ")
            ))
        }
    };
    Ok(Observation {
        edition,
        build,
        place,
        date: date.to_string(),
        verdict,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `base` with each key of `changed` set to its value, or taken away where the value is null.
    fn laid_over(mut base: Value, changed: Value) -> Value {
        let keys = base.as_object_mut().unwrap();
        for (key, value) in changed.as_object().cloned().unwrap_or_default() {
            match value {
                Value::Null => keys.remove(&key),
                value => keys.insert(key, value),
            };
        }
        base
    }

    /// A sheet of one line about `.ymap` files, with `changed` laid over the line.
    fn sheet_with(changed: Value) -> String {
        let line = serde_json::json!({
            "id": "map-object-moved",
            "kind": "ymap",
            "row": "R32",
            "do": "Go to the object.",
            "look_for": "It is there.",
            "results": [],
        });
        let line = laid_over(line, changed);
        serde_json::json!({ "format": FORMAT, "schema": SCHEMA, "lines": [line] }).to_string()
    }

    /// One result, with `changed` laid over it.
    fn result_with(changed: Value) -> Value {
        let result = serde_json::json!({
            "edition": "legacy",
            "build": "3258",
            "where": "fivem-server",
            "date": "2026-11-25",
            "verdict": "loaded",
        });
        laid_over(result, changed)
    }

    /// Why the line `map-object-moved` of `text` is refused.
    fn refusal(text: &str) -> String {
        match Sheet::parse(text) {
            Err(SheetError::Line { id, why }) => {
                assert_eq!(id, "map-object-moved", "the refusal names the line");
                why
            }
            other => panic!("expected the line to be refused, got {other:?}"),
        }
    }

    #[test]
    fn the_shipped_sheet_parses_and_every_id_is_one_lines() {
        let sheet = Sheet::shipped().expect("the sheet the tree ships is a sheet");
        assert!(!sheet.lines.is_empty(), "a sheet with no line asks nothing");
        for line in &sheet.lines {
            assert!(knows(&line.kind), "{}: {}", line.id, line.kind);
        }
        let ids: BTreeSet<&str> = sheet.lines.iter().map(|line| line.id.as_str()).collect();
        assert_eq!(ids.len(), sheet.lines.len(), "every id is one line's");
    }

    #[test]
    fn a_line_that_is_wrong_is_refused_with_its_id() {
        assert!(Sheet::parse(&sheet_with(serde_json::json!({}))).is_ok());

        let why = refusal(&sheet_with(serde_json::json!({ "kind": "lua" })));
        assert!(why.contains("`lua`"), "{why}");
        for row in ["seventy", "R", "Ra", "r70", "R70ab", "R7a0", "R 70"] {
            let why = refusal(&sheet_with(serde_json::json!({ "row": row })));
            assert!(why.contains(&format!("`{row}`")), "{why}");
        }
        assert!(Sheet::parse(&sheet_with(serde_json::json!({ "row": "R70a" }))).is_ok());
        let why = refusal(&sheet_with(serde_json::json!({ "look_for": null })));
        assert!(why.contains("`look_for`"), "{why}");
        let why = refusal(&sheet_with(serde_json::json!({ "do": "Go | there." })));
        assert!(why.contains("`|`"), "{why}");
        let why = refusal(&sheet_with(
            serde_json::json!({ "do": "Go there.\nThen look." }),
        ));
        assert!(why.contains("`do`"), "{why}");
        let spaced = Sheet::parse(&sheet_with(
            serde_json::json!({ "look_for": "  It is there. " }),
        ))
        .expect("a sentence with space about it reads");
        assert_eq!(
            spaced.lines[0].look_for, "It is there.",
            "the guide's table holds a sentence with no space about it"
        );
        let why = refusal(&sheet_with(serde_json::json!({ "note": "seen" })));
        assert!(why.contains("`note`"), "{why}");

        let mut twice: Value = serde_json::from_str(&sheet_with(serde_json::json!({}))).unwrap();
        let line = twice["lines"][0].clone();
        twice["lines"].as_array_mut().unwrap().push(line);
        let why = refusal(&twice.to_string());
        assert!(why.contains("twice"), "{why}");
    }

    #[test]
    fn a_line_with_no_id_is_refused_by_its_place() {
        for id in [Value::Null, "Map Object".into(), "map--object".into()] {
            let found = Sheet::parse(&sheet_with(serde_json::json!({ "id": id })));
            assert!(
                matches!(found, Err(SheetError::Unnamed { place: 1, .. })),
                "{found:?}"
            );
        }
    }

    #[test]
    fn a_result_that_is_wrong_is_refused_with_the_id_of_its_line() {
        let with = |result: Value| sheet_with(serde_json::json!({ "results": [result] }));
        let kept = Sheet::parse(&with(result_with(serde_json::json!({})))).expect("a result reads");
        assert_eq!(
            kept.lines[0].results,
            vec![Observation {
                edition: Target::Legacy,
                build: "3258".into(),
                place: Place::FivemServer,
                date: "2026-11-25".into(),
                verdict: Verdict::Loaded,
            }]
        );
        let faulty = result_with(
            serde_json::json!({ "verdict": "loaded-with-fault", "fault": "It flickers." }),
        );
        let kept = Sheet::parse(&with(faulty)).expect("a fault that is said reads");
        assert_eq!(
            kept.lines[0].results[0].verdict,
            Verdict::LoadedWithFault("It flickers.".into())
        );

        for (changed, names) in [
            (serde_json::json!({ "verdict": "fine" }), "`fine`"),
            (serde_json::json!({ "verdict": "Loaded" }), "`Loaded`"),
            (
                serde_json::json!({ "verdict": "loaded-with-fault" }),
                "`fault`",
            ),
            (serde_json::json!({ "fault": "It flickers." }), "`fault`"),
            (serde_json::json!({ "date": "2026-02-30" }), "`2026-02-30`"),
            (serde_json::json!({ "date": "2026-13-01" }), "`2026-13-01`"),
            (serde_json::json!({ "date": "2026-00-10" }), "`2026-00-10`"),
            (serde_json::json!({ "date": "2026-01-00" }), "`2026-01-00`"),
            (serde_json::json!({ "date": "25/11/2026" }), "`25/11/2026`"),
            (serde_json::json!({ "date": "2026-1-5" }), "`2026-1-5`"),
            (serde_json::json!({ "date": "yesterday" }), "`yesterday`"),
            (serde_json::json!({ "date": null }), "`date`"),
            (serde_json::json!({ "edition": "gen9" }), "`gen9`"),
            (serde_json::json!({ "where": "my server" }), "`my server`"),
            (serde_json::json!({ "build": "" }), "`build`"),
            (serde_json::json!({ "seen_by": "me" }), "`seen_by`"),
        ] {
            let why = refusal(&with(result_with(changed.clone())));
            assert!(why.contains("result 1"), "{changed}: {why}");
            assert!(why.contains(names), "{changed}: {why}");
        }
        assert!(is_date("2028-02-29") && !is_date("2026-02-29") && !is_date("2100-02-29"));
    }

    #[test]
    fn a_text_that_is_no_sheet_is_refused_whole() {
        for text in [
            "not json",
            "[]",
            r#"{ "format": "openrage-maps", "schema": 1, "lines": [] }"#,
            r#"{ "format": "openrage-acceptance", "schema": 2, "lines": [] }"#,
            r#"{ "format": "openrage-acceptance", "lines": [] }"#,
            r#"{ "format": "openrage-acceptance", "schema": 1 }"#,
            r#"{ "format": "openrage-acceptance", "schema": 1, "lines": [], "results": [] }"#,
        ] {
            let found = Sheet::parse(text);
            assert!(
                matches!(found, Err(SheetError::Sheet(_))),
                "{text}: {found:?}"
            );
        }
    }

    #[test]
    fn the_newest_run_of_an_edition_is_what_a_line_says_of_it() {
        let run = |edition: &str, date: &str, verdict: &str| {
            let fault = (verdict == "loaded-with-fault").then_some("It flickers.");
            result_with(serde_json::json!({
                "edition": edition, "date": date, "verdict": verdict, "fault": fault,
            }))
        };
        let sheet = Sheet::parse(&sheet_with(serde_json::json!({ "results": [
            run("legacy", "2026-12-02", "loaded"),
            run("legacy", "2026-12-01", "not-loaded"),
            run("enhanced", "2026-12-01", "loaded"),
            run("enhanced", "2026-12-03", "loaded-with-fault"),
            run("enhanced", "2026-12-03", "not-loaded"),
        ] })))
        .unwrap();
        let kinds = BTreeSet::from(["ymap".to_string()]);
        let legacy = sheet.about(&kinds, Target::Legacy);
        assert!(
            legacy[0].loaded(),
            "an older refusal does not stand over a newer run"
        );
        let enhanced = sheet.about(&kinds, Target::Enhanced);
        assert_eq!(
            enhanced[0].newest.map(|seen| &seen.verdict),
            Some(&Verdict::NotLoaded),
            "of two runs on one day the later in the file is the newest"
        );
        assert!(
            !enhanced[0].loaded(),
            "an older run that loaded does not stand over a newer one that did not"
        );

        let faulty = Sheet::parse(&sheet_with(serde_json::json!({ "results": [
            run("legacy", "2026-12-01", "loaded"),
            run("legacy", "2026-12-02", "loaded-with-fault"),
        ] })))
        .unwrap();
        let legacy = faulty.about(&kinds, Target::Legacy);
        assert!(
            !legacy[0].loaded(),
            "a run that loaded with a fault is not one that loaded"
        );
    }

    #[test]
    fn a_kind_is_the_extension_of_a_built_file_and_a_build_knows_the_ones_it_reads_back() {
        assert_eq!(kind_of("stream/Harbour.YMAP").as_deref(), Some("ymap"));
        assert_eq!(kind_of("x64/levels/a.rpf/b.ytyp").as_deref(), Some("ytyp"));
        assert_eq!(kind_of("dlc.rpf").as_deref(), Some("rpf"));
        assert_eq!(kind_of("stream\\a.b.ydr").as_deref(), Some("ydr"));
        for none in ["stream/readme", ".gitignore", "stream.d/readme", "a."] {
            assert_eq!(kind_of(none), None, "{none}");
        }
        for known in ["ymap", "ytyp", "ybn", "ydr", "rpf"] {
            assert!(knows(known), "{known}");
        }
        for unknown in ["lua", "xml", "YMAP", "oiv", ""] {
            assert!(!knows(unknown), "{unknown}");
        }
    }
}
