//! `openrage-cli maps put-back`: plan what makes one entity of a map source of a project's maps content state what the base its
//! source keeps states again, a deleted one with the chain of far models its deletion took, and write it with `--apply`
//! (`or_maps::put_back`).

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use or_maps::changes::EntityRef;
use or_maps::operation::{apply_all, inverses, At, Error as Refused, Operations};
use or_maps::put_back::{self, Named, PutBack, Why};
use or_maps::Maps;
use serde_json::{json, Value};

use crate::maps::{counted, maps_of};
use crate::maps_changes::{change, entity, name_hashes, sources_read};
use crate::project::{parse, Failure};
use Failure::{Run, Usage};

pub(crate) fn run(args: &[String]) -> Result<ExitCode, Failure> {
    print!("{}", report(args)?);
    Ok(ExitCode::SUCCESS)
}

/// The entity `text` names: `<source>:<entity>` for one the map file holds, and `<source>:base:<entity>` for one of the base.
fn named(text: &str) -> Option<Named> {
    let (rest, index) = text.rsplit_once(':')?;
    let index = index.parse().ok()?;
    let (source, in_base) = match rest.strip_suffix(":base") {
        Some(source) => (source, true),
        None => (rest, false),
    };
    let at = At {
        source: source.to_string(),
        index,
    };
    (!source.is_empty()).then_some(match in_base {
        true => Named::Base(at),
        false => Named::Now(at),
    })
}

/// Why a plan was not written.
fn refusal(e: Refused) -> Failure {
    match e {
        // The one failure after which a source may hold what the plan wrote: it says which.
        Refused::Write(_) => Run(e.to_string()),
        refused => Run(format!("{refused}; no source was changed")),
    }
}

/// The lines that say a plan: what is put back, what comes back and why, what is left, each count that changes, and each note.
fn lines(model: &Maps, planned: &PutBack) -> Vec<String> {
    let of_base = |source: &str, base: &EntityRef| {
        format!(
            "{source} entity {} of the base, {}",
            base.index,
            entity(model, base)
        )
    };
    let mut lines = Vec::new();
    if let Some(put) = &planned.restored {
        let undone: Vec<String> = put.changes.iter().map(|c| change(model, c)).collect();
        let undone = match undone.is_empty() {
            true => "the entity its link names".to_string(),
            false => undone.join("; "),
        };
        lines.push(format!(
            "  puts back   {} entity {} as {}: it undoes {undone}",
            put.source,
            put.now.index,
            of_base(&put.source, &put.base)
        ));
    }
    for back in &planned.returns {
        let why = match &back.why {
            Why::Chosen => "chosen".to_string(),
            Why::FarModel => "the far model of what comes back".to_string(),
            Why::Under { source, index } => {
                format!("under entity {index} of the base of {source}")
            }
        };
        lines.push(format!(
            "  comes back  {} as entity {}: {why}",
            of_base(&back.source, &back.base),
            back.index
        ));
    }
    for left in &planned.left {
        lines.push(format!(
            "  left        {}: it cannot be told from a twin, so it is not put back by a guess",
            of_base(&left.source, &left.base)
        ));
    }
    for count in &planned.recounted {
        lines.push(format!(
            "  counts      {} entity {}: its count of children goes from {} to {}",
            count.source, count.entity, count.from, count.to
        ));
    }
    for note in &planned.notes {
        lines.push(format!("  note        {note}"));
    }
    lines
}

/// What `maps put-back` prints for `args`, once it has done what they ask.
pub(crate) fn report(args: &[String]) -> Result<String, Failure> {
    let apply = args.iter().any(|a| a == "--apply");
    let json_out = args.iter().any(|a| a == "--json");
    let rest: Vec<String> = (args.iter())
        .filter(|a| !matches!(a.as_str(), "--apply" | "--json"))
        .cloned()
        .collect();
    let (positional, flags) = parse(&rest, &["--module", "--inverse"])?;
    let [path, text] = positional.as_slice() else {
        return Err(Usage(
            "expected <project.openrage-proj> and one entity, as <source>:<entity> or <source>:base:<entity>".into(),
        ));
    };
    let entity = named(text).ok_or_else(|| {
        Usage(format!(
            "`{text}` names no entity: say <source>:<entity> for one the map file holds, such as city.metadata.src:12, \
             or <source>:base:<entity> for one of the base, as `maps changes` lists a deleted one"
        ))
    })?;
    let inverse = flags.get("--inverse");
    if inverse.is_some() && !apply {
        return Err(Usage(
            "--inverse keeps the way back of what --apply writes, and --apply is not given".into(),
        ));
    }
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
    // ⚠ A plan writes nothing, so it may read a linked content, which only the project that holds it may write.
    let folder = if apply {
        or_maps::module_folder_to_edit(Path::new(path), &project.container, module)
    } else {
        or_maps::module_folder(&project.folder(), module)
    }
    .map_err(|e| Run(format!("content `{id}`: {e}")))?;

    let planned = put_back::plan(&folder, &entity)
        .map_err(|e| Run(format!("content `{id}`: {e}; no source was changed")))?;
    if apply {
        // ⚠ The way back is kept before a source is written: a plan whose inverse cannot be kept is not done.
        if let Some(file) = inverse {
            let tried = apply_all(&folder, &planned.operations, true).map_err(refusal)?;
            let list = Operations::new(inverses(&tried))
                .to_json()
                .map_err(|e| Run(e.to_string()))?;
            std::fs::write(file, list)
                .map_err(|e| Run(format!("{file}: {e}; no source was changed")))?;
        }
        put_back::apply(&folder, &planned).map_err(refusal)?;
    }

    let model = Maps::read(&folder).map_err(|e| Run(format!("content `{id}`: {e}")))?;
    let (Named::Now(at) | Named::Base(at)) = &entity;
    // What the source still states otherwise than its base: what the question before a discard of the map counts.
    let left =
        (sources_read(&folder, &model).changes(&at.source).ok()).map(|changes| changes.count());

    if json_out {
        let mut plan = serde_json::to_value(&planned).map_err(|e| Run(e.to_string()))?;
        // The operations are the list `maps apply` takes, and hold records, in which nothing is a name of this report.
        let operations = plan.get_mut("operations").map(Value::take);
        name_hashes(&mut plan, &model);
        let mut report = json!({
            "content": id,
            "applied": apply,
            "plan": plan,
            "changes_left": left,
        });
        report["plan"]["operations"] = json!({
            "format": or_maps::operation::FORMAT,
            "schema": or_maps::operation::SCHEMA,
            "operations": operations,
        });
        let mut text = serde_json::to_string_pretty(&report).map_err(|e| Run(e.to_string()))?;
        text.push('\n');
        return Ok(text);
    }

    let mut text = String::new();
    let _ = writeln!(text, "content {id}:");
    for line in lines(&model, &planned) {
        let _ = writeln!(text, "{line}");
    }
    let verb = if apply { "changed" } else { "would change" };
    for source in &planned.sources {
        let _ = writeln!(text, "{verb} {source}/{}", or_source::metadata::XML);
    }
    let _ = writeln!(
        text,
        "content {id}: {} {verb} {}{}",
        counted(planned.operations.len(), "operation", "operations"),
        counted(planned.sources.len(), "source", "sources"),
        if apply {
            ""
        } else {
            "; nothing was written (write it with --apply)"
        }
    );
    if let Some(file) = inverse {
        let _ = writeln!(
            text,
            "the list that takes it back is {file}: `maps apply` does it"
        );
    }
    let when = if apply { "after this" } else { "now" };
    match left {
        Some(0) => {
            let _ = writeln!(text, "{} states what its base states {when}", at.source);
        }
        Some(left) => {
            let _ = writeln!(
                text,
                "{} holds {} of its base {when}, which is what a discard of the map would lose",
                at.source,
                counted(left, "change", "changes")
            );
        }
        None => {}
    }
    Ok(text)
}
