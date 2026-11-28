//! The world verbs any registered title answers through the world contract: `world index`, `at`,
//! `ground`, `find` and `select` with `--neutral`, each by the capabilities the opened world states.
//!
//! ⚠ A weather is the title's own number of it, and an option is asked by the title's number of
//! it: no shared code turns a name a person reads into either.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use glam::Vec3;
use or_job::{Cancel, Progress};
use or_world::caps::Capability;
use or_world::collision::GroundProbe;
use or_world::contract::{Keys, Opening, Worker, WorldInstall, WorldSpec, WorldTitle};
use or_world::driver::{Driver, Why};
use or_world::kept::{self, Report, Verdict};
use or_world::model::{Gate, Index, MapId, Moment, Switched};
use or_world::sentence::Sentence;
use or_world::slots::{Choices, OptionId, OptionKind, OptionValue};
use or_world::streaming::NodeState;

use super::world::flag_value;

/// The flags of these verbs that take a value.
const VALUED_FLAGS: &[&str] = &["--game", "--keys", "--option", "--hour", "--weather"];

/// The most maps a search reads the placement rows of.
const MOST_READ: usize = 20;

const USAGE: &str = "world <index|at|ground|find|select> <install> [<x,y,z> | <text>] --neutral \
                     [--option <id>=<value>]... [--hour <0-23>] [--weather <number>] [--keys <file>]";

/// A world opened through its title, with its index from the kept file.
pub(super) struct Opened {
    pub install: Arc<dyn WorldInstall>,
    pub index: Arc<Index>,
    pub report: Report,
    /// Where the index is kept.
    pub file: PathBuf,
}

/// Open `world` through `title` with `keys`, its index kept under the configuration folder `config`.
pub(super) fn open(
    title: &dyn WorldTitle,
    world: &WorldSpec,
    keys: Keys<'_>,
    config: &Path,
) -> Result<Opened, String> {
    let (cancel, progress) = (Cancel::new(), Progress::new());
    let opening = Opening {
        config,
        keys,
        cancel: &cancel,
        progress: &progress,
    };
    let install = title.open(world, &opening).map_err(|e| e.to_string())?;
    let file = kept::file_for(config, title.id(), world);
    let (index, report) =
        kept::refresh(title.id(), world, &*install, &file).map_err(|e| e.to_string())?;
    Ok(Opened {
        install,
        index: Arc::new(index),
        report,
        file,
    })
}

fn verdict_in_words(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Absent => "none was kept, so every archive was read".to_string(),
        Verdict::Current => "of the world as it stands, so nothing was read".to_string(),
        Verdict::Changed => {
            "of the world as it was, so each archive that changed was read again".to_string()
        }
        Verdict::Foreign => "of another world, so every archive was read".to_string(),
        Verdict::Refused(why) => format!("refused ({why}), so every archive was read"),
    }
}

/// What a capability is called.
fn capability_name(capability: Capability) -> &'static str {
    match capability {
        Capability::Ground => "a height of the ground",
        Capability::Collision => "a collision query",
        Capability::Placements => "placement rows",
        Capability::WeatherGate => "a weather gate",
        Capability::Edits => "an edit store",
        Capability::Base => "a base to compare with",
        Capability::OwnPrograms => "its own programs",
        Capability::Water => "sheets of water",
    }
}

/// What `world index --neutral` prints of an opened world.
pub(super) fn index_lines(
    title: &dyn WorldTitle,
    world: &WorldSpec,
    opened: &Opened,
) -> Vec<String> {
    let (maps, report) = (opened.index.maps(), &opened.report);
    let count = |held: fn(&Gate) -> bool| maps.iter().filter(|row| held(&row.gate)).count();
    let caps = opened.install.caps();
    let stated: Vec<&str> = (Capability::ALL.into_iter())
        .filter(|capability| caps.has(*capability))
        .map(capability_name)
        .collect();
    let mut lines = vec![
        format!(
            "World index of {} ({}, {})",
            world.install.display(),
            title.id(),
            world.game.0
        ),
        format!(
            "  kept in {}: {}",
            opened.file.display(),
            verdict_in_words(&report.verdict)
        ),
        format!(
            "  {} archive(s), {} read",
            opened.install.inputs().archives.len(),
            report.read.len()
        ),
        format!(
            "  {} map(s): {} that state no box, {} only a request lets in, {} by the clock or the weather",
            maps.len(),
            maps.iter().filter(|row| row.boxes.is_none()).count(),
            count(|gate| matches!(gate, Gate::Held { .. })),
            count(|gate| matches!(gate, Gate::When { .. })),
        ),
        format!("  it states: {}", match stated.is_empty() {
            true => "no capability".to_string(),
            false => stated.join(", "),
        }),
    ];
    for option in title.options() {
        let choices = match &option.choices {
            Choices::Toggle => "on or off".to_string(),
            Choices::Named => "a name of the install's".to_string(),
            Choices::OneOf(of) => {
                let each = of
                    .iter()
                    .map(|(id, label)| format!("{id} ({})", label.key()));
                each.collect::<Vec<_>>().join(", ")
            }
        };
        lines.push(format!(
            "  option {}  {}: {choices}",
            option.id.0,
            option.label.key()
        ));
    }
    lines
}

/// What lets a map in, in words.
fn gate_in_words(gate: &Gate) -> String {
    match gate {
        Gate::Always => "always".to_string(),
        Gate::Held { .. } => "only when asked for".to_string(),
        Gate::When { hours, weathers } => format!(
            "during the hours {hours:#08x}, under {}",
            match weathers.len() {
                0 => "any weather".to_string(),
                count => format!("{count} weather(s)"),
            }
        ),
    }
}

/// What `world at --neutral` prints: the maps a camera at `at` streams in, each with whether its gate lets it in at `moment`.
pub(super) fn at_lines(opened: &Opened, at: Vec3, moment: &Moment) -> Vec<String> {
    let policy = opened.install.policy();
    let none = Switched::default();
    let rows = (0u32..).map(MapId).zip(opened.index.maps());
    let here: Vec<String> = rows
        .filter(|(_, row)| {
            row.boxes
                .is_some_and(|boxes| (policy.streams_in)(&boxes, at))
        })
        .map(|(id, row)| {
            let lets = match row.gate.admits(id, moment, &none) {
                true => "in",
                false => "held back",
            };
            format!(
                "  map       {}  {} ({lets})",
                row.label,
                gate_in_words(&row.gate)
            )
        })
        .collect();
    let mut lines = vec![format!(
        "At ({}, {}, {}), {} h: {} map(s) stream by their box",
        at.x,
        at.y,
        at.z,
        moment.hour,
        here.len()
    )];
    lines.extend(here);
    lines
}

/// An error where the world of `opened` does not state `capability`, in the words a person reads.
fn stated(opened: &Opened, capability: Capability) -> Result<(), String> {
    match opened.install.caps().has(capability) {
        true => Ok(()),
        false => Err(format!(
            "this world states no {}",
            capability_name(capability)
                .trim_start_matches("a ")
                .trim_start_matches("an ")
        )),
    }
}

/// What `world ground --neutral` prints: the ground under `at`, looked for from `at` down where it states a height and from far above where it states none.
pub(super) fn ground_lines(
    opened: &Opened,
    x: f32,
    y: f32,
    z: Option<f32>,
) -> Result<Vec<String>, String> {
    stated(opened, Capability::Ground)?;
    let probe = GroundProbe {
        max_rise: 0.0,
        max_drop: f32::INFINITY,
        max_slope: std::f32::consts::FRAC_PI_2,
    };
    let from = Vec3::new(x, y, z.unwrap_or(10_000.0));
    let mut worker = Worker::new();
    let found = (opened.install)
        .ground(&opened.index, from, &probe, &mut worker)
        .map_err(|e| e.to_string())?;
    Ok(vec![match found {
        Some(hit) => format!(
            "Ground under ({x}, {y}): {} (the surface faces {}, {}, {})",
            hit.position.z, hit.normal.x, hit.normal.y, hit.normal.z
        ),
        None => format!("Ground under ({x}, {y}): none"),
    }])
}

/// What `world find --neutral` prints: the maps whose name holds `text`, in any case, or is the number `text` states, each with how many things it places.
pub(super) fn find_lines(opened: &Opened, text: &str) -> Result<Vec<String>, String> {
    stated(opened, Capability::Placements)?;
    let wanted = text.to_lowercase();
    let number = match wanted.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => wanted.parse().ok(),
    };
    let rows = (0u32..).map(MapId).zip(opened.index.maps());
    let found: Vec<_> = rows
        .filter(|(_, row)| row.label.to_lowercase().contains(&wanted) || Some(row.name) == number)
        .collect();
    let mut lines = vec![format!("`{text}`: {} map(s)", found.len())];
    let indexer = opened.install.indexer();
    for (at, (id, row)) in found.iter().enumerate() {
        let placed = (at < MOST_READ).then(|| {
            let mut rows = Vec::new();
            match indexer.placements(&opened.index, *id, &mut rows) {
                Ok(()) => format!("places {}", rows.len()),
                Err(e) => format!("unread: {e}"),
            }
        });
        lines.push(format!(
            "  map       {}  {:#x}  {}",
            row.label,
            row.name,
            placed.unwrap_or_else(|| "not read".to_string())
        ));
    }
    Ok(lines)
}

/// What `world select --neutral` prints: what the core's driver streams and selects for a camera at `eye` at `moment`, every model taken as loaded.
pub(super) fn select_lines(opened: &Opened, eye: Vec3, moment: &Moment) -> Vec<String> {
    let mut driver = Driver::new(Arc::clone(&opened.install), Arc::clone(&opened.index));
    let view = (driver.policy()).view(eye, Vec3::ZERO, std::f32::consts::FRAC_PI_4);
    let streamed = driver.stream(&view, moment, &Switched::default(), None, usize::MAX);
    let selected = driver.select(&view, moment, &|_| NodeState::Ready);
    let label = |map: MapId| {
        let row = opened.index.map(map);
        row.map_or_else(|| format!("{map:?}"), |row| row.label.clone())
    };
    let mut lines = vec![format!(
        "View at ({}, {}, {}), {} h, weather {}: {} map(s) wanted, {} that gave no body; {} interior(s) brought in, {} that gave none",
        eye.x,
        eye.y,
        eye.z,
        moment.hour,
        moment.weather,
        streamed.wanted.len(),
        streamed.unread.len(),
        streamed.interiors.len(),
        streamed.interiors_unread.len()
    )];
    for (map, why) in &streamed.wanted {
        let why = match why {
            Why::Own => "the view is within its box".to_string(),
            Why::ParentOf(child) => format!("the parent of {}", label(*child)),
        };
        lines.push(format!("  map       {}  {why}", label(*map)));
    }
    for (map, why) in &streamed.unread {
        lines.push(format!("  unread    {}: {why}", label(*map)));
    }
    for at in &streamed.interiors {
        lines.push(format!(
            "  interior  {:#x}  node {} of {}",
            at.placed.interior,
            at.placed.node,
            label(at.map)
        ));
    }
    for (at, why) in &streamed.interiors_unread {
        lines.push(format!(
            "  interior  {:#x}  node {} of {}: {why}",
            at.placed.interior,
            at.placed.node,
            label(at.map)
        ));
    }
    let of = &selected.instances;
    lines.push(format!(
        "  {} instance(s) selected, every model taken as loaded; {} model(s)",
        of.len(),
        of.models.len()
    ));
    let mut by_level: BTreeMap<u8, usize> = BTreeMap::new();
    for node in &of.node {
        if let Some(node) = driver.node(*node) {
            *by_level.entry(node.level as u8).or_default() += 1;
        }
    }
    let counts: Vec<String> = (by_level.iter())
        .map(|(level, count)| format!("level {level}: {count}"))
        .collect();
    lines.push(format!("    {}", counts.join(", ")));
    for at in 0..of.len() {
        let (key, node) = of.node[at];
        let words = &of.words[at * of.width()..][..of.width()];
        lines.push(format!(
            "    instance  {key:#x}:{node}  model {:#x}  fade {}  at ({}, {}, {})  words {words:x?}",
            of.model_of(at).map_or(0, |model| model.0),
            of.fade[at],
            of.rows[at][0][3],
            of.rows[at][1][3],
            of.rows[at][2][3],
        ));
    }
    lines
}

/// The options `args` set with `--option <id>=<value>`, each read as its kind among `kinds` takes it: `on` or `off` for a toggle, the number of a choice or its label in any case, and the text as it stands for a name.
fn options_of(
    args: &[String],
    kinds: &[OptionKind],
) -> Result<Vec<(OptionId, OptionValue)>, String> {
    let mut options = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if word != "--option" {
            continue;
        }
        let set = words.next().ok_or("`--option` takes <id>=<value>")?;
        let (id, value) = (set.split_once('='))
            .and_then(|(id, value)| Some((id.parse::<u32>().ok()?, value)))
            .ok_or_else(|| format!("`--option` takes <id>=<value>, not `{set}`"))?;
        let kind = (kinds.iter().find(|kind| kind.id.0 == id))
            .ok_or_else(|| format!("the title has no option {id}"))?;
        // ⚠ By the option's kind, never by what the text looks like: a name may be all digits.
        let value = match (&kind.choices, value) {
            (Choices::Toggle, "on") => OptionValue::On(true),
            (Choices::Toggle, "off") => OptionValue::On(false),
            (Choices::Toggle, other) => {
                return Err(format!("option {id} is `on` or `off`, not `{other}`"))
            }
            (Choices::Named, name) => OptionValue::Named(name.to_string()),
            (Choices::OneOf(of), word) => {
                let labelled =
                    |(_, label): &&(u32, Sentence)| label.key().eq_ignore_ascii_case(word);
                let choice = (word
                    .parse()
                    .ok()
                    .filter(|n| of.iter().any(|(id, _)| id == n)))
                .or_else(|| of.iter().find(labelled).map(|(id, _)| *id))
                .ok_or_else(|| format!("option {id} has no choice `{word}`"))?;
                OptionValue::Choice(choice)
            }
        };
        options.push((kind.id, value));
    }
    Ok(options)
}

/// The moment `args` state with `--hour` and `--weather`: noon under the weather 0 where they state none.
fn moment_of(args: &[String]) -> Result<Moment, String> {
    let hour = match flag_value(args, "--hour") {
        Some(word) => (word.parse::<u8>().ok())
            .filter(|hour| *hour < 24)
            .ok_or_else(|| format!("--hour is an hour of the day, 0 to 23, not `{word}`"))?,
        None => 12,
    };
    let weather = match flag_value(args, "--weather") {
        // In tens, or in sixteens after `0x`, as a hash is written.
        Some(word) => (word.strip_prefix("0x").or_else(|| word.strip_prefix("0X")))
            .map_or_else(
                || word.parse().ok(),
                |hex| u64::from_str_radix(hex, 16).ok(),
            )
            .ok_or_else(|| format!("--weather is the title's number of a weather, not `{word}`"))?,
        None => 0,
    };
    Ok(Moment {
        hour: f32::from(hour),
        weather,
    })
}

/// What the verb `verb` says of `args` through the registry's title for the install they name.
fn said(verb: &str, args: &[String]) -> Result<Vec<String>, String> {
    let known = |arg: &str| VALUED_FLAGS.contains(&arg) || arg == "--neutral";
    if let Some(flag) = (args.iter()).find(|arg| arg.starts_with("--") && !known(arg)) {
        return Err(format!("`{flag}` is no flag of this verb"));
    }
    if let Some(flag) = (args.last()).filter(|arg| VALUED_FLAGS.contains(&arg.as_str())) {
        return Err(format!("`{flag}` takes a value"));
    }
    let (install, rest) = super::world_index::install_of(args, VALUED_FLAGS)?;
    let root = std::fs::canonicalize(&install).map_err(|e| format!("{install}: {e}"))?;
    let (entry, game) = or_worlds::of_install(&root)
        .ok_or("no title of this build opens a world of that install")?;
    let world = WorldSpec {
        options: options_of(args, &entry.title.options())?,
        ..WorldSpec::of(game, &root)
    };
    let config =
        or_title::config::config_dir().ok_or("no configuration directory to keep the index in")?;
    // The keys kept for the game where they are, else those of the install's own executable.
    // As `of_install` above: a folder no profile claims is GTA V's.
    let detected = or_title::Game::detect_from_root(&root).unwrap_or(or_title::Game::GtaVLegacy);
    let cache = super::keys::shared_keys_cache_path_for(detected).filter(|file| file.is_file());
    let executable = detected.find_executable(&root);
    let keys = match (flag_value(args, "--keys"), &cache, &executable) {
        (Some(file), ..) => Keys::Snapshot(Path::new(file)),
        (None, Some(cache), _) => Keys::Snapshot(cache),
        (None, None, Some(executable)) => Keys::Executable(executable),
        (None, None, None) => Keys::None,
    };
    let opened = open(entry.title, &world, keys, &config)?;
    if let Some(e) = &opened.report.not_kept {
        tracing::warn!(error = %e, "the index was not kept");
    }
    let place = || -> Result<Vec3, String> {
        let numbers = super::world_index::numbers_of(&rest, VALUED_FLAGS)?;
        match numbers[..] {
            [x, y, z] => Ok(Vec3::new(x, y, z)),
            _ => Err(format!(
                "a place is three numbers, x,y,z, and {} were given",
                numbers.len()
            )),
        }
    };
    match verb {
        "index" => Ok(index_lines(entry.title, &world, &opened)),
        "at" => Ok(at_lines(&opened, place()?, &moment_of(args)?)),
        "select" => Ok(select_lines(&opened, place()?, &moment_of(args)?)),
        "ground" => {
            let numbers = super::world_index::numbers_of(&rest, VALUED_FLAGS)?;
            match numbers[..] {
                [x, y] => ground_lines(&opened, x, y, None),
                [x, y, z] => ground_lines(&opened, x, y, Some(z)),
                _ => Err("a column is two numbers, x,y, and a point three".to_string()),
            }
        }
        "find" => {
            let mut after_flag = false;
            let text = rest.iter().find(|arg| {
                let value =
                    std::mem::replace(&mut after_flag, VALUED_FLAGS.contains(&arg.as_str()));
                !value && !arg.starts_with("--")
            });
            find_lines(&opened, text.ok_or("what to find comes after the install")?)
        }
        other => Err(format!("`{other}` is no neutral verb")),
    }
}

pub(crate) fn cmd(verb: &str, args: &[String]) -> ExitCode {
    match said(verb, args) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {USAGE}: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
#[path = "world_neutral_tests.rs"]
mod tests;
