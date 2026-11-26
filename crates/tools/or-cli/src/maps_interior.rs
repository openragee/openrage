//! `openrage-cli maps interior`: the interiors the type files of a maps content declare, each with its rooms, portals and entity
//! sets and what the rules say of it (`or_maps::interior`), and a list of operations applied to them as `maps apply` applies one.

use std::fmt::Write as _;
use std::process::ExitCode;

use or_maps::interior::{interiors_of, InFile};
use serde_json::json;

use crate::maps::{applied, maps_of};
use crate::project::{parse, Failure};
use Failure::{Run, Usage};

pub(crate) fn run(args: &[String]) -> Result<ExitCode, Failure> {
    let (said, clean) = report(args)?;
    print!("{said}");
    Ok(if clean {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// What `maps interior` prints for `args`, and whether no interior it lists holds what a save refuses.
pub(crate) fn report(args: &[String]) -> Result<(String, bool), Failure> {
    let json_out = args.iter().any(|a| a == "--json");
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let rest: Vec<String> = (args.iter())
        .filter(|a| *a != "--json" && *a != "--dry-run")
        .cloned()
        .collect();
    let (positional, flags) = parse(&rest, &["--module", "--apply"])?;
    let Some((path, named)) = positional.split_first() else {
        return Err(Usage("expected a <project.openrage-proj>".into()));
    };
    match flags.get("--apply") {
        Some(_) if json_out => Err(Usage(
            "--json is the form of the listing, and --apply prints what each operation changed"
                .into(),
        )),
        Some(_) if !named.is_empty() => Err(Usage(
            "--apply takes a list, whose operations each name their source and their interior"
                .into(),
        )),
        Some(list) => applied(path, list, &flags, dry_run).map(|said| (said, true)),
        None if dry_run => Err(Usage("--dry-run goes with --apply".into())),
        None => listed(path, named, &flags, json_out),
    }
}

/// An interior a person names: a type file source of the content, and the index of one archetype of it when one is given.
struct Named {
    source: String,
    archetype: Option<usize>,
}

/// `text` as `<source>` or `<source>:<archetype>`, as `maps delete` names an entity.
fn named(text: &str) -> Result<Named, Failure> {
    let Some((source, index)) = text.rsplit_once(':') else {
        return Ok(Named {
            source: text.to_string(),
            archetype: None,
        });
    };
    let archetype = index.parse().map_err(|_| {
        Usage(format!(
            "`{text}` names no interior: after the `:` comes the index of an archetype of the type file"
        ))
    })?;
    Ok(Named {
        source: source.to_string(),
        archetype: Some(archetype),
    })
}

/// The interiors of the maps content of the project `path`: those `named`, or every one when none is.
fn listed(
    path: &str,
    named_as: &[String],
    flags: &std::collections::HashMap<String, String>,
    json_out: bool,
) -> Result<(String, bool), Failure> {
    let (project, modules) = maps_of(&[path.to_string()], flags)?;
    let asked: Vec<Named> = (named_as.iter())
        .map(|text| named(text))
        .collect::<Result<_, _>>()?;
    let mut found = vec![false; asked.len()];
    let mut text = String::new();
    let mut reports = Vec::new();
    let mut clean = true;
    for id in modules {
        let module = (project.container.module(&id))
            .ok_or_else(|| Run(format!("the project has no maps content `{id}`")))?;
        let folder = or_maps::module_folder(&project.folder(), module)
            .map_err(|e| Run(format!("content `{id}`: {e}")))?;
        let model =
            or_maps::Maps::read(&folder).map_err(|e| Run(format!("content `{id}`: {e}")))?;
        // The names the content remembers: a type file keeps the hash of a name alone.
        let names = |hash: u32| model.name_of(hash);
        let mut interiors: Vec<(String, InFile)> = Vec::new();
        let mut sources: Vec<&String> = model.maps.iter().flat_map(|set| &set.sources).collect();
        sources.sort();
        sources.dedup();
        for source in sources
            .into_iter()
            .filter(|s| or_maps::facts::stays_inside(s))
        {
            let dir = folder.join(source.replace('/', std::path::MAIN_SEPARATOR_STR));
            let wanted: Vec<usize> = (asked.iter().enumerate())
                .filter(|(_, named)| named.source == *source)
                .map(|(at, _)| at)
                .collect();
            if !asked.is_empty() && wanted.is_empty() {
                continue;
            }
            let is_types = or_source::asset::Source::open(&dir)
                .ok()
                .and_then(|opened| or_source::metadata::Inputs::read(&opened).ok())
                .is_some_and(|inputs| inputs.extension() == "ytyp");
            if !is_types {
                if !wanted.is_empty() {
                    return Err(Run(format!(
                        "content `{id}`: `{source}` is no type file source, and an interior is an archetype of a type file"
                    )));
                }
                continue;
            }
            let cooked = or_maps::facts::cooked(&dir)
                .map_err(|e| Run(format!("content `{id}`: `{source}`: {e}")))?;
            let held = interiors_of(&cooked, Some(&names))
                .map_err(|e| Run(format!("content `{id}`: `{source}` {e}")))?;
            for interior in held {
                // ⚠ Every name is asked, and not only up to the first that fits: two names may name one interior.
                let mut chosen = asked.is_empty();
                for at in &wanted {
                    let fits = asked[*at].archetype.is_none_or(|a| a == interior.archetype);
                    found[*at] |= fits;
                    chosen |= fits;
                }
                if chosen {
                    interiors.push((source.clone(), interior));
                }
            }
        }
        clean &= interiors.iter().all(|(_, i)| i.listed.broken.is_empty());
        if json_out {
            let interiors: Vec<serde_json::Value> = (interiors.iter())
                .map(|(source, interior)| {
                    let mut data =
                        serde_json::to_value(interior).map_err(|e| Run(e.to_string()))?;
                    data["source"] = json!(source);
                    Ok(data)
                })
                .collect::<Result<_, Failure>>()?;
            reports.push(json!({ "module": id, "interiors": interiors }));
            continue;
        }
        let _ = writeln!(text, "content {id}: {} interior(s)", interiors.len());
        for (source, interior) in &interiors {
            let mut lines = interior.listed.lines().into_iter();
            let _ = writeln!(
                text,
                "  {source}:{} `{}`: {}",
                interior.archetype,
                interior.name,
                lines.next().unwrap_or_default()
            );
            for line in lines {
                let _ = writeln!(text, "    {line}");
            }
        }
    }
    if let Some(missing) = found.iter().position(|found| !found) {
        return Err(Run(format!(
            "`{}` names no interior of the maps content: `maps interior` with no name lists every one",
            named_as[missing]
        )));
    }
    if json_out {
        text = serde_json::to_string_pretty(&reports).map_err(|e| Run(e.to_string()))?;
        text.push('\n');
    }
    Ok((text, clean))
}
