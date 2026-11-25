//! What the resources of a server hold under one name, and what of that a build is about to write: rules as data, and each place one applies.
//!
//! Names the same resources share are one finding, as mods that write the same paths are one conflict: who starts last is a
//! function of who holds them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use or_io::meta::instance::HashedString;
use serde::Serialize;

use super::config::{Config, NotRead};
use super::{is_category, Declared, Resource, Scan, Streamed};
use crate::container::OutputKind;
use crate::content::rule::{Rule, Severity};
use crate::paths::{is_under, relative_inside, resolved};
use crate::plan::Plan;
use crate::resource::{word, STREAM_DIR};

/// The kind of claim an archetype's name is: a content that declares archetypes states it, and [`concerning`] reads it.
pub const ARCHETYPE_CLAIM: &str = "archetype name";

/// How many names a finding about several lists before it counts the rest.
const NAMED: usize = 8;

const fn rule(code: &'static str, severity: Severity, summary: &'static str) -> Rule {
    Rule::new(code, severity, false, summary)
}

use Severity::{Error, Info, Warning};

/// Every rule, in the order a report lists them.
pub const RULES: &[Rule] = &[
    rule(
        "folder-unread",
        Warning,
        "a folder of the server cannot be listed, so the resources in it are not known",
    ),
    rule(
        "resource-unread",
        Info,
        "part of a resource was not read, so what it states, streams or declares is not all known",
    ),
    rule(
        "resource-name-twice",
        Warning,
        "two folders have one resource name, and a server knows a resource by its name",
    ),
    rule(
        "file-in-two-resources",
        Warning,
        "two resources stream a file of one name with different bytes, and a server loads one copy",
    ),
    rule(
        "file-in-two-resources-same",
        Info,
        "two resources stream a file of one name with the same bytes",
    ),
    rule(
        "archetype-in-two-resources",
        Warning,
        "two resources declare an archetype of one name, and the game resolves a name once",
    ),
    rule(
        "game-file-overridden",
        Info,
        "a resource streams a file of the name of a file of the game, which it overrides",
    ),
    rule(
        "configuration-path-escapes",
        Warning,
        "the configuration includes a file by a path that leaves the server's folder, which is not followed",
    ),
    rule(
        "configuration-file-unread",
        Warning,
        "a file the configuration includes was not read, so what it starts is not known",
    ),
    rule(
        "configuration-line-unread",
        Info,
        "lines of the configuration neither start a resource nor include a file, and are not read",
    ),
    rule(
        "build-file-streamed-elsewhere",
        Warning,
        "another resource of the server streams a file this build writes, and a server loads one copy",
    ),
    rule(
        "build-archetype-declared-elsewhere",
        Warning,
        "another resource of the server declares an archetype this build declares, and the game resolves a name once",
    ),
    rule(
        "build-folder-outside-category",
        Warning,
        "the build writes its resource under a folder of the server's resources that is no category in brackets, where a server does not look",
    ),
    rule(
        "build-resource-elsewhere",
        Warning,
        "another folder of the server has the name of the resource this build writes, and a server knows a resource by its name",
    ),
];

pub fn rule_of(code: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.code == code)
}

/// What an install is asked about the name of a streamed file.
pub trait GameFiles {
    /// Whether the game holds a file of `name`, which is in lower case with its extension.
    fn holds_file(&self, name: &str) -> bool;
}

/// One place a rule applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub severity: Severity,
    /// The resources it is about, each by its path under the server's folder, in the order of the scan.
    pub resources: Vec<String>,
    /// The names it is about, sorted and each once: of files or of archetypes, or the path a configuration states.
    pub names: Vec<String>,
    /// The one of `resources` the configuration starts last; none where no configuration says so.
    pub starts_last: Option<String>,
    pub message: String,
}

impl Finding {
    fn new(code: &'static str, message: String) -> Self {
        Self {
            code,
            severity: rule_of(code).map_or(Error, |r| r.severity),
            resources: Vec::new(),
            names: Vec::new(),
            starts_last: None,
            message,
        }
    }

    /// The line of a text report that states it.
    pub fn line(&self) -> String {
        format!(
            "  {:<7} {:<34} {}",
            word(self.severity),
            self.code,
            self.message
        )
    }
}

/// `names` in backticks, the first of them listed and the rest counted.
fn listed(names: &[String]) -> String {
    let shown: Vec<&str> = names.iter().take(NAMED).map(String::as_str).collect();
    match names.len() - shown.len() {
        0 => format!("`{}`", shown.join("`, `")),
        more => format!("`{}` and {more} more", shown.join("`, `")),
    }
}

/// `a`, `b` and `c`, each in backticks; past [`NAMED`] of them the rest are counted.
fn joined(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [first @ .., _] if first.len() >= NAMED => format!(
            "`{}` and {} more",
            first[..NAMED].join("`, `"),
            names.len() - NAMED
        ),
        [only] => format!("`{only}`"),
        [first @ .., last] => format!("`{}` and `{last}`", first.join("`, `")),
    }
}

/// Where a configuration first starts each resource of a scan.
struct Order<'a> {
    config: Option<&'a Config>,
    /// For each resource of the scan, the index of the first line that starts it or a category it is in.
    places: Vec<Option<usize>>,
}

impl<'a> Order<'a> {
    fn of(scan: &Scan, config: Option<&'a Config>) -> Self {
        let starts = config.map_or(&[][..], |c| c.starts.as_slice());
        let place = |resource: &Resource| {
            let categories = resource.path.split('/').rev().skip(1);
            starts.iter().position(|start| {
                start.resource == resource.name
                    || (is_category(&start.resource)
                        && categories.clone().any(|part| part == start.resource))
            })
        };
        Self {
            config,
            places: scan.resources.iter().map(place).collect(),
        }
    }

    /// Which of the resources `of` starts last, and the words that say it or say why it is not known.
    fn among(&self, scan: &Scan, of: &[usize]) -> (Option<String>, String) {
        let Some(config) = self.config else {
            let why = "the order they start in is not known: no configuration was given";
            return (None, why.to_string());
        };
        if !config.is_whole() {
            let why = "the order they start in is not known: a file the configuration includes was not read";
            return (None, why.to_string());
        }
        let path = |index: &usize| scan.resources[*index].path.as_str();
        let started: Vec<(usize, usize)> = (of.iter())
            .filter_map(|&index| Some((self.places[index]?, index)))
            .collect();
        let idle: Vec<&str> = (of.iter())
            .filter(|&&index| self.places[index].is_none())
            .map(path)
            .collect();
        let Some(&(last, _)) = started.iter().max() else {
            return (None, "the configuration starts none of them".to_string());
        };
        if started.len() == 1 {
            let only = path(&started[0].1);
            return (None, format!("the configuration starts only `{only}`"));
        }
        let at = &config.starts[last];
        let together: Vec<&str> = (started.iter())
            .filter(|(place, _)| *place == last)
            .map(|(_, index)| path(index))
            .collect();
        let line = format!("line {} of `{}`", at.line, at.file);
        if let [only] = together.as_slice() {
            let rest = match idle.as_slice() {
                [] => String::new(),
                idle => format!(", and the configuration does not start {}", joined(idle)),
            };
            let said = format!("`{only}` starts last, on {line}{rest}");
            return (Some((*only).to_string()), said);
        }
        // ⚠ A line that names no category starts several only where folders share its name, which is no order between them.
        let said = match is_category(&at.resource) {
            true => format!(
                "{} start together with the category `{}` on {line}, and the order inside a category is not known",
                joined(&together),
                at.resource
            ),
            false => format!(
                "{} have one name, which {line} starts, so which of them starts last is not known",
                joined(&together)
            ),
        };
        (None, said)
    }
}

/// What the resources of `scan` hold under one name, what was not read of them, and what `game` holds of the names they stream.
pub fn findings(
    scan: &Scan,
    config: Option<&Config>,
    game: Option<&dyn GameFiles>,
) -> Vec<Finding> {
    let mut found = Vec::new();
    for folder in &scan.unlisted {
        let shown = match folder.path.as_str() {
            "" => "the folder".to_string(),
            path => format!("`{path}`"),
        };
        let message = format!(
            "{shown} cannot be listed, so the resources in it are not known: {}",
            folder.why
        );
        found.push(Finding {
            names: vec![folder.path.clone()],
            ..Finding::new("folder-unread", message)
        });
    }
    for resource in &scan.resources {
        for note in &resource.notes {
            found.push(Finding {
                resources: vec![resource.path.clone()],
                ..Finding::new("resource-unread", format!("`{}`: {note}", resource.path))
            });
        }
    }
    names_twice(scan, &mut found);
    let order = Order::of(scan, config);
    files_twice(scan, &order, &mut found);
    archetypes_twice(scan, &order, &mut found);
    if let Some(game) = game {
        overrides(scan, game, &mut found);
    }
    if let Some(config) = config {
        of_configuration(config, &mut found);
    }
    let place = |code: &str| RULES.iter().position(|r| r.code == code);
    found.sort_by_key(|f| place(f.code));
    found
}

/// A finding for each resource name that two folders have.
fn names_twice(scan: &Scan, found: &mut Vec<Finding>) {
    let mut by_name: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for resource in &scan.resources {
        (by_name.entry(&resource.name).or_default()).push(&resource.path);
    }
    for (name, folders) in by_name {
        if folders.len() < 2 {
            continue;
        }
        let message = format!(
            "{} are {} folders of one resource name, `{name}`, and a server knows a resource by its name",
            joined(&folders),
            folders.len()
        );
        found.push(Finding {
            resources: folders.iter().map(|path| (*path).to_string()).collect(),
            names: vec![name.to_string()],
            ..Finding::new("resource-name-twice", message)
        });
    }
}

/// A finding about names the resources `holders` share.
fn shared(
    code: &'static str,
    scan: &Scan,
    order: &Order<'_>,
    holders: &[usize],
    names: Vec<String>,
    what: impl Fn(&str, &[String]) -> String,
) -> Finding {
    let paths: Vec<&str> = (holders.iter())
        .map(|&index| scan.resources[index].path.as_str())
        .collect();
    let (starts_last, said) = order.among(scan, holders);
    Finding {
        resources: paths.iter().map(|path| (*path).to_string()).collect(),
        starts_last,
        message: format!("{}; {said}", what(&joined(&paths), &names)),
        names,
        ..Finding::new(code, String::new())
    }
}

/// What the digests say of the copies of one file name.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Copies {
    Differ,
    /// A copy could not be read to its end, so it has no digest to compare.
    Unread,
    Same,
}

/// A finding for each set of resources that stream files of one name: one for the names whose copies are the same bytes, one for
/// those that differ and one for those with a copy that was not read.
fn files_twice(scan: &Scan, order: &Order<'_>, found: &mut Vec<Finding>) {
    let mut by_name: BTreeMap<&str, Vec<(usize, &Streamed)>> = BTreeMap::new();
    for (index, resource) in scan.resources.iter().enumerate() {
        for file in &resource.streamed {
            by_name.entry(&file.name).or_default().push((index, file));
        }
    }
    let mut groups: BTreeMap<(Vec<usize>, Copies), Vec<String>> = BTreeMap::new();
    for (name, copies) in by_name {
        let holders: BTreeSet<usize> = copies.iter().map(|copy| copy.0).collect();
        if holders.len() < 2 {
            continue;
        }
        // ⚠ A copy that could not be read to its end has no digest, and is said neither to be the same nor to differ.
        let first = &copies[0].1.digest;
        let said = if copies.iter().any(|copy| copy.1.digest.is_none()) {
            Copies::Unread
        } else if copies.iter().all(|copy| copy.1.digest == *first) {
            Copies::Same
        } else {
            Copies::Differ
        };
        let key = (holders.into_iter().collect(), said);
        groups.entry(key).or_default().push(name.to_string());
    }
    for ((holders, said), names) in groups {
        let code = match said {
            Copies::Same => "file-in-two-resources-same",
            Copies::Differ | Copies::Unread => "file-in-two-resources",
        };
        let bytes = match said {
            Copies::Same => "with the same bytes",
            Copies::Differ => "with different bytes, and a server loads one copy",
            Copies::Unread => {
                "of which a copy was not read, so whether they are the same bytes is not known"
            }
        };
        let what = |who: &str, names: &[String]| match names {
            [name] => format!("{who} stream `{name}` {bytes}"),
            names => format!(
                "{who} stream {} files of one name {bytes}: {}",
                names.len(),
                listed(names)
            ),
        };
        found.push(shared(code, scan, order, &holders, names, what));
    }
}

/// A finding for each set of resources that declare archetypes of one name.
fn archetypes_twice(scan: &Scan, order: &Order<'_>, found: &mut Vec<Finding>) {
    let mut by_hash: BTreeMap<u32, Vec<(usize, &Declared)>> = BTreeMap::new();
    for (index, resource) in scan.resources.iter().enumerate() {
        for archetype in &resource.archetypes {
            (by_hash.entry(archetype.hash).or_default()).push((index, archetype));
        }
    }
    // ⚠ Looked up by path: a resource repeated in two folders shares every archetype, and a search of its files for each is quadratic.
    let streamed: Vec<BTreeMap<&str, &Streamed>> = (scan.resources.iter())
        .map(|r| r.streamed.iter().map(|f| (f.path.as_str(), f)).collect())
        .collect();
    let type_file = |&(index, declared): &(usize, &Declared)| {
        let file = streamed[index].get(declared.file.as_str())?;
        Some((file.name.as_str(), file.digest.as_deref()?))
    };
    let mut groups: BTreeMap<Vec<usize>, BTreeSet<String>> = BTreeMap::new();
    for declarations in by_hash.values() {
        let holders: BTreeSet<usize> = declarations.iter().map(|d| d.0).collect();
        if holders.len() < 2 {
            continue;
        }
        // One type file under one name in each resource is said by the finding about the file, once and not once for each archetype.
        let first = type_file(&declarations[0]);
        if first.is_some() && declarations.iter().all(|d| type_file(d) == first) {
            continue;
        }
        // The text of the name where any of the files keeps it, else the hash.
        let name = (declarations.iter())
            .map(|d| d.1.name.as_str())
            .min_by_key(|name| (name.starts_with("hash_"), *name))
            .unwrap_or_default();
        let holders: Vec<usize> = holders.into_iter().collect();
        groups.entry(holders).or_default().insert(name.to_string());
    }
    for (holders, names) in groups {
        let what = |who: &str, names: &[String]| match names {
            [name] => {
                format!("{who} declare the archetype `{name}`, and the game resolves a name once")
            }
            names => format!(
                "{who} declare {} archetypes of one name, and the game resolves a name once: {}",
                names.len(),
                listed(names)
            ),
        };
        let names = names.into_iter().collect();
        let code = "archetype-in-two-resources";
        found.push(shared(code, scan, order, &holders, names, what));
    }
}

/// A finding for each resource that streams files of the names of files of the game.
fn overrides(scan: &Scan, game: &dyn GameFiles, found: &mut Vec<Finding>) {
    for resource in &scan.resources {
        let names: BTreeSet<&str> = (resource.streamed.iter())
            .map(|file| file.name.as_str())
            .filter(|name| game.holds_file(name))
            .collect();
        let names: Vec<String> = names.into_iter().map(str::to_string).collect();
        let message = match names.as_slice() {
            [] => continue,
            [name] => format!(
                "`{}` streams `{name}`, which is the name of a file of the game, and overrides it",
                resource.path
            ),
            names => format!(
                "`{}` streams {} files of the names of files of the game, and overrides them: {}",
                resource.path,
                names.len(),
                listed(names)
            ),
        };
        found.push(Finding {
            resources: vec![resource.path.clone()],
            names,
            ..Finding::new("game-file-overridden", message)
        });
    }
}

/// A finding for each included file that was not read, and one for the lines of each file that are no command the reader knows.
fn of_configuration(config: &Config, found: &mut Vec<Finding>) {
    for include in &config.includes_unread {
        let at = format!(
            "line {} of `{}` includes `{}`",
            include.line, include.file, include.stated
        );
        let (code, message) = match &include.why {
            NotRead::Escapes => (
                "configuration-path-escapes",
                format!("{at}, which leaves the server's folder and is not followed"),
            ),
            NotRead::Unreadable(why) => (
                "configuration-file-unread",
                format!("{at}, which was not read, so what it starts is not known: {why}"),
            ),
        };
        found.push(Finding {
            names: vec![include.stated.clone()],
            ..Finding::new(code, message)
        });
    }
    for file in &config.read {
        let runs: Vec<String> = (config.unread.iter())
            .filter(|run| run.file == *file)
            .map(|run| match run.first == run.last {
                true => run.first.to_string(),
                false => format!("{} to {}", run.first, run.last),
            })
            .collect();
        if runs.is_empty() {
            continue;
        }
        let more = match runs.len().saturating_sub(NAMED) {
            0 => String::new(),
            more => format!(" and {more} more run(s)"),
        };
        let shown: Vec<&str> = runs.iter().take(NAMED).map(String::as_str).collect();
        let message = format!(
            "`{file}`: line(s) {}{more} neither start a resource nor include a file, and are not read",
            shown.join(", ")
        );
        found.push(Finding::new("configuration-line-unread", message));
    }
}

/// What of `scan` concerns the build of `plan`: each other resource that streams a file the build writes under its stream folder, or declares an archetype it claims.
pub fn concerning(plan: &Plan, scan: &Scan) -> Vec<Finding> {
    // ⚠ A pack and a package are found by no server under a name, so their plans are asked nothing.
    if plan.kind != OutputKind::FivemResource {
        return Vec::new();
    }
    let writes: BTreeSet<String> = (plan.files.iter())
        .filter(|file| is_under(&file.path, STREAM_DIR))
        .filter_map(|file| file.path.rsplit('/').next())
        .map(str::to_lowercase)
        .collect();
    let claims: BTreeMap<u32, &str> = (plan.claims.iter())
        .filter(|claim| claim.kind == ARCHETYPE_CLAIM)
        .map(|claim| {
            let hash = HashedString::from_xml_text(&claim.name).hash;
            (hash, claim.name.as_str())
        })
        .collect();
    let mut found = Vec::new();
    for resource in &scan.resources {
        // ⚠ A resource of the build's own name is the build's earlier copy, which the build replaces: it is not another resource.
        if resource.name == plan.resource {
            continue;
        }
        let streamed: BTreeSet<&str> = (resource.streamed.iter())
            .map(|file| file.name.as_str())
            .filter(|name| writes.contains(*name))
            .collect();
        let streamed: Vec<String> = streamed.into_iter().map(str::to_string).collect();
        let message = match streamed.as_slice() {
            [] => None,
            [name] => Some(format!(
                "`{}` already streams `{name}`, which this build writes, and a server loads one copy",
                resource.path
            )),
            names => Some(format!(
                "`{}` already streams {} files this build writes, and a server loads one copy of each: {}",
                resource.path,
                names.len(),
                listed(names)
            )),
        };
        if let Some(message) = message {
            found.push(Finding {
                resources: vec![resource.path.clone()],
                names: streamed,
                ..Finding::new("build-file-streamed-elsewhere", message)
            });
        }
        let declared: BTreeSet<&str> = (resource.archetypes.iter())
            .filter_map(|archetype| claims.get(&archetype.hash).copied())
            .collect();
        let declared: Vec<String> = declared.into_iter().map(str::to_string).collect();
        let message = match declared.as_slice() {
            [] => continue,
            [name] => format!(
                "`{}` already declares the archetype `{name}`, which this build declares, and the game resolves a name once",
                resource.path
            ),
            names => format!(
                "`{}` already declares {} archetypes this build declares, and the game resolves a name once: {}",
                resource.path,
                names.len(),
                listed(names)
            ),
        };
        found.push(Finding {
            resources: vec![resource.path.clone()],
            names: declared,
            ..Finding::new("build-archetype-declared-elsewhere", message)
        });
    }
    let place = |code: &str| RULES.iter().position(|r| r.code == code);
    found.sort_by_key(|f| place(f.code));
    found
}

/// What of `scan` concerns the folder `at` the build of `plan` writes its resource as: a folder above it that is no category, and another folder of its name.
pub fn placement(plan: &Plan, at: &Path, scan: &Scan) -> Vec<Finding> {
    if plan.kind != OutputKind::FivemResource {
        return Vec::new();
    }
    // ⚠ A build written outside the server's folder is placed nowhere in it: a resource of its name there is the copy it is made for.
    let Some(inside) = relative_inside(&resolved(at), &resolved(&scan.folder)) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let parts: Vec<&str> = inside.split('/').collect();
    let above = parts.split_last().map_or(&[][..], |(_, above)| above);
    // The outermost folder that is no category is where a server stops walking.
    if let Some(depth) = above.iter().position(|part| !is_category(part)) {
        let plain = above[..=depth].join("/");
        let message = format!(
            "the build writes `{inside}`, and `{plain}` is no category in brackets, so a server does not look in it"
        );
        found.push(Finding {
            names: vec![plain],
            ..Finding::new("build-folder-outside-category", message)
        });
    }
    let elsewhere: Vec<&str> = (scan.resources.iter())
        .filter(|resource| resource.name == plan.resource && resource.path != inside)
        .map(|resource| resource.path.as_str())
        .collect();
    if !elsewhere.is_empty() {
        let are = match elsewhere.len() {
            1 => "is another folder",
            _ => "are other folders",
        };
        let message = format!(
            "{} {are} of the resource `{}`, which this build writes at `{inside}`, and a server knows a resource by its name",
            joined(&elsewhere),
            plan.resource
        );
        found.push(Finding {
            resources: elsewhere.iter().map(|path| (*path).to_string()).collect(),
            names: vec![plan.resource.clone()],
            ..Finding::new("build-resource-elsewhere", message)
        });
    }
    found
}

/// One resource of a report: what was read of it, as counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub path: String,
    pub name: String,
    /// The files under its stream folder.
    pub streamed: usize,
    /// The streamed files locked by an asset escrow, which no reader was handed.
    pub locked: usize,
    /// The archetypes its type files declare.
    pub archetypes: usize,
    /// The maps that state a name.
    pub maps: usize,
    pub notes: Vec<String>,
}

/// What was read of a server's resources folder, and what the rules found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub folder: PathBuf,
    /// The configuration read; none when none was given.
    pub configuration: Option<PathBuf>,
    /// How many lines of the configuration start a resource or a category.
    pub starts: usize,
    pub resources: Vec<Summary>,
    /// The folders that are neither a category nor a resource, which were not read.
    pub ignored: Vec<String>,
    pub findings: Vec<Finding>,
}

impl Report {
    /// The report of `scan`, with the findings [`findings`] gives.
    pub fn of(scan: &Scan, config: Option<&Config>, game: Option<&dyn GameFiles>) -> Self {
        let summary = |resource: &Resource| Summary {
            path: resource.path.clone(),
            name: resource.name.clone(),
            streamed: resource.streamed.len(),
            locked: resource.locked(),
            archetypes: resource.archetypes.len(),
            maps: resource.maps.len(),
            notes: resource.notes.clone(),
        };
        Self {
            folder: scan.folder.clone(),
            configuration: config.map(|config| config.file.clone()),
            starts: config.map_or(0, |config| config.starts.len()),
            resources: scan.resources.iter().map(summary).collect(),
            ignored: scan.ignored.clone(),
            findings: findings(scan, config, game),
        }
    }

    pub fn count(&self, severity: Severity) -> usize {
        (self.findings.iter())
            .filter(|f| f.severity == severity)
            .count()
    }

    /// Whether a rule found a warning or an error, which a command answers with a failing exit code.
    pub fn fails(&self) -> bool {
        self.count(Error) + self.count(Warning) > 0
    }

    /// The text form: what was read, one line for each finding in the order of [`Report::findings`], the counts, and what the check cannot see.
    pub fn lines(&self) -> Vec<String> {
        let sum = |of: fn(&Summary) -> usize| self.resources.iter().map(of).sum::<usize>();
        let mut lines = vec![format!("server {}", self.folder.display())];
        lines.push(format!(
            "  {} resource(s): {} streamed file(s), {} locked by an asset escrow and not opened, {} archetype(s) declared, {} map(s) that state a name",
            self.resources.len(),
            sum(|r| r.streamed),
            sum(|r| r.locked),
            sum(|r| r.archetypes),
            sum(|r| r.maps)
        ));
        if !self.ignored.is_empty() {
            lines.push(format!(
                "  {} folder(s) are neither a category in brackets nor a resource with a manifest, and were not read: {}",
                self.ignored.len(),
                listed(&self.ignored)
            ));
        }
        lines.push(match &self.configuration {
            Some(file) => format!(
                "  configuration {}: {} line(s) start a resource or a category",
                file.display(),
                self.starts
            ),
            None => "  configuration: none given".to_string(),
        });
        lines.extend(self.findings.iter().map(Finding::line));
        lines.push(format!(
            "{} error(s), {} warning(s), {} note(s)",
            self.count(Error),
            self.count(Warning),
            self.count(Info)
        ));
        lines.push(
            "This reads the files as a server finds them. It cannot see which copy of a file a server loads, or whether a server starts a resource."
                .to_string(),
        );
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_has_one_code_and_a_sentence() {
        for (index, rule) in RULES.iter().enumerate() {
            assert!(!rule.summary.is_empty(), "{}", rule.code);
            assert!(
                RULES[..index].iter().all(|r| r.code != rule.code),
                "{} is a rule twice",
                rule.code
            );
            assert!(
                crate::resource::rule_of(rule.code).is_none(),
                "{} is a rule of one resource too",
                rule.code
            );
        }
    }

    /// A resource that streams `count` type files, each declaring the one archetype of its number.
    fn of_type_files(path: &str, count: usize) -> Resource {
        let file = |n: usize| format!("stream/types/file_{n:06}.ytyp");
        Resource {
            path: path.to_string(),
            name: path.to_string(),
            streamed: (0..count)
                .map(|n| Streamed {
                    path: file(n),
                    name: format!("file_{n:06}.ytyp"),
                    bytes: 1,
                    digest: Some(format!("{path}{n}")),
                    locked: false,
                })
                .collect(),
            archetypes: (0..count)
                .map(|n| Declared {
                    name: format!("hash_{n:08X}"),
                    hash: n as u32,
                    file: file(n),
                })
                .collect(),
            ..Resource::default()
        }
    }

    #[test]
    fn a_resource_copied_into_a_second_folder_is_compared_in_time_that_grows_with_its_files() {
        let scan_of = |count| Scan {
            resources: vec![of_type_files("a", count), of_type_files("b", count)],
            ..Scan::default()
        };
        let timed = |count: usize| {
            let scan = scan_of(count);
            let began = std::time::Instant::now();
            let found = findings(&scan, None, None);
            let took = began.elapsed();
            let archetypes = (found.iter()).find(|f| f.code == "archetype-in-two-resources");
            assert_eq!(archetypes.map(|f| f.names.len()), Some(count));
            took
        };
        // ⚠ Sixteen times the files in sixteen times the time and some, with a second for a machine that is busy; a search of the
        // files for each archetype takes 256 times.
        let (small, large) = (timed(4_000), timed(64_000));
        assert!(
            large < small * 80 + std::time::Duration::from_secs(1),
            "{small:?} for 4,000 type files and {large:?} for 64,000"
        );
    }

    #[test]
    fn names_are_listed_to_a_count_and_joined_as_a_sentence_names_them() {
        let names: Vec<String> = (0..NAMED + 2).map(|n| format!("n{n}")).collect();
        assert_eq!(listed(&names[..2]), "`n0`, `n1`");
        assert!(listed(&names).ends_with(&format!("`n{}` and 2 more", NAMED - 1)));
        assert_eq!(joined(&["a"]), "`a`");
        assert_eq!(joined(&["a", "b"]), "`a` and `b`");
        assert_eq!(joined(&["a", "b", "c"]), "`a`, `b` and `c`");
        let many: Vec<&str> = names.iter().map(String::as_str).collect();
        assert!(!joined(&many[..NAMED]).contains("more"));
        assert!(joined(&many).ends_with(&format!("`n{}` and 2 more", NAMED - 1)));
        assert!(joined(&many[..NAMED + 1]).ends_with(" and 1 more"));
    }
}
