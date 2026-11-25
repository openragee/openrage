//! `script export`: every program of a build folder, an archive or a folder decompiled with the
//! project of them all, counted a line each, and written into one folder where one is named.
//!
//! ⚠ The same input, lists and options write the same bytes: no file holds a time, an absolute
//! path or a machine's name.

use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use or_io::gta6::script::program::Program;
use or_io::gta6::script::project::index;
use or_io::gta6::script::{Names, Options, Project, Stats};
use serde::Serialize;

use super::super::lifted::{over, THREADS};
use super::source::{self, Body};
use super::Asked;

const PROJECT_FILE: &str = "project.json";
const SCRIPTS_FILE: &str = "scripts.json";

/// What `scripts.json` holds: what was read, with what, and where each program is.
#[derive(Serialize)]
struct Scripts<'a> {
    format: u32,
    read: &'a str,
    level: &'a str,
    /// The passes left off.
    off: Vec<&'a str>,
    project: &'a str,
    lists: &'a [String],
    programs: Vec<Row>,
}

#[derive(Serialize)]
struct Row {
    /// The source written; none for a body that failed.
    file: Option<String>,
    program: Option<String>,
    name: Option<String>,
    read: String,
    archive: Option<String>,
    entry: Option<String>,
    functions: usize,
    imports: Vec<String>,
    status: String,
}

/// What the first pass keeps of one program.
struct Stated {
    /// The program's tables; its code is dropped.
    program: Program,
    /// Why the program gave no part of the project, where it gave none.
    left_out: Option<String>,
}

pub(super) fn run(asked: &Asked) -> Result<(String, bool), String> {
    let [root] = asked.paths.as_slice() else {
        return Err(
            "name one build folder, one archive or one folder of deciphered programs".to_owned(),
        );
    };
    let (read, found) = source::gathered(Path::new(root), asked.keys.as_deref())?;
    let unread = format!(
        "{} enciphered entr(ies) or archive(s) could not be read",
        found.unread
    );
    if found.bodies.is_empty() {
        return Err(format!("{root} holds no GTA VI program; {unread}"));
    }
    let names = super::supplied_names(asked)?;
    let (options, threads) = (super::options(asked), asked.threads.unwrap_or(THREADS));
    let out = asked.out.as_deref();
    let (mut lines, failed) = exported(&found.bodies, read, names, options, threads, out)?;
    if found.unread > 0 {
        lines.push(unread);
    }
    lines.push(String::new());
    Ok((lines.join("\n"), failed == 0))
}

/// Decompiles every body with the project of them all, and writes that project, a source each and what was read into `out` where one is given; a line a body and a total, and how many bodies failed.
// ⚠ A body that does not read or does not decompile is a line and a row, never the end.
pub fn exported(
    bodies: &[Body],
    read: &str,
    mut names: Names,
    options: Options,
    threads: usize,
    out: Option<&Path>,
) -> Result<(Vec<String>, usize), String> {
    let write = |file: &str, bytes: &[u8]| match out {
        Some(dir) => {
            let to = dir.join(file);
            std::fs::write(&to, bytes).map_err(|e| format!("write {}: {e}", to.display()))
        }
        None => Ok(()),
    };
    if let Some(dir) = out {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }

    // The first pass: what the programs state of each other, then what the code of each shows.
    let tables = index::tables(bodies.iter().map(|body| body.bytes.as_slice()));
    let firsts = over(bodies, threads, |body| {
        guarded(|| first(&body.bytes, &tables))
    });
    let mut parts = Vec::new();
    let mut stated = Vec::with_capacity(bodies.len());
    for one in firsts {
        stated.push(one.map(|(kept, part)| {
            // A type's members are named in every program that uses the type, so all are bound before any is written.
            super::bind(&mut names, &kept.program);
            parts.extend(part);
            kept
        }));
    }
    let project = index::merge(parts);
    write(
        PROJECT_FILE,
        &index::save(&project).map_err(|e| e.to_string())?,
    )?;

    let mut used = BTreeSet::new();
    let files: Vec<Option<String>> = (stated.iter())
        .map(|stated| {
            let stem = stem_of(&names, stated.as_ref().ok()?.program.program);
            // A second program of one name takes `_2`, in the order the programs were found.
            let (mut file, mut n) = (format!("{stem}.cs"), 1);
            while !used.insert(file.to_ascii_lowercase()) {
                n += 1;
                file = format!("{stem}_{n}.cs");
            }
            Some(file)
        })
        .collect();

    // The second pass: each program decompiled with the project.
    let phrase = match out {
        Some(_) => format!("`{PROJECT_FILE}`"),
        None => "that of the programs read with it, kept in no file".to_owned(),
    };
    let jobs: Vec<_> = bodies.iter().zip(&stated).zip(&files).collect();
    let outcomes = over(&jobs, threads, |((body, stated), file)| {
        if let Err(why) = stated {
            return Err(why.clone());
        }
        guarded(|| {
            let project = Some((phrase.as_str(), &project));
            let (text, stats) = super::decompiled(&body.bytes, &names, project, options, &[])?;
            if let Some(file) = file {
                write(file, text.as_bytes())?;
            }
            Ok(stats)
        })
    });

    let (mut lines, mut total) = (Vec::new(), Stats::default());
    let (mut failed, mut alone) = (0, 0);
    let mut rows = Vec::with_capacity(bodies.len());
    for (((body, stated), file), outcome) in jobs.iter().zip(&outcomes) {
        let stated: Option<&Stated> = Result::as_ref(stated).ok();
        let note = match stated.and_then(|stated| stated.left_out.as_ref()) {
            Some(why) => {
                alone += 1;
                format!("; it gave the project no part: {why}")
            }
            None => String::new(),
        };
        let status = match outcome {
            Ok(stats) => {
                total.add(stats);
                lines.push(format!("{}: {}{note}", body.label, counts(stats)));
                "ok".to_owned()
            }
            Err(why) => {
                failed += 1;
                lines.push(format!("{}: failed: {why}", body.label));
                format!("failed: {why}")
            }
        };
        let program = stated.map(|stated| &stated.program);
        let imports = |p: &Program| -> Vec<String> {
            let imports = p.imports.iter();
            imports.map(|import| super::hex(import.program)).collect()
        };
        rows.push(Row {
            file: (*file).clone().filter(|_| outcome.is_ok()),
            program: program.map(|p| super::hex(p.program)),
            name: program
                .and_then(|p| names.program(u64::from(p.program)))
                .map(str::to_owned),
            read: body.label.clone(),
            archive: body.archive.clone(),
            entry: (body.entry.as_ref()).map(|entry| format!("0x{:016x}", entry.name64)),
            functions: program.map_or(0, |p| p.functions.len()),
            imports: program.map_or_else(Vec::new, imports),
            status,
        });
    }
    lines.push(format!(
        "total: {} program(s), {failed} failed, {alone} that gave the project no part: {}",
        bodies.len(),
        counts(&total)
    ));

    let scripts = Scripts {
        format: 1,
        read,
        level: options.level.name(),
        off: super::left_off(options),
        project: PROJECT_FILE,
        lists: &names.lists,
        programs: rows,
    };
    let mut text = serde_json::to_vec_pretty(&scripts).map_err(|e| e.to_string())?;
    text.push(b'\n');
    write(SCRIPTS_FILE, &text)?;
    Ok((lines, failed))
}

/// Reads one program, and its part of the project where it gives one.
// A program whose part is refused is still decompiled, with what the other programs state.
fn first(bytes: &[u8], tables: &index::Tables) -> Result<(Stated, Option<Project>), String> {
    let mut program = Program::read(bytes).map_err(|e| e.to_string())?;
    // Only its tables are kept: the code of every program at once is the size of the build's scripts.
    program.code = Vec::new();
    let (part, left_out) = match index::of_program_in(bytes, tables) {
        Ok(part) => (Some(part), None),
        Err(why) => (None, Some(why.to_string())),
    };
    Ok((Stated { program, left_out }, part))
}

/// What `work` gives, a fault in it turned into its body's failure.
fn guarded<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(done) => done,
        Err(_) => Err("the decompiler panicked; its message is on stderr".to_owned()),
    }
}

/// The name a program's source is written under: its name where a list names it, else its hash.
fn stem_of(names: &Names, program: u32) -> String {
    // ⚠ A name is the user's text: nothing of it may leave the folder or open a stream of a file.
    let kept = |c: char| match c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') {
        true => c,
        false => '_',
    };
    match names.program(u64::from(program)) {
        Some(name) => name.chars().map(kept).collect(),
        None => super::hex(program),
    }
}

fn counts(s: &Stats) -> String {
    format!(
        "{} function(s), {} not lifted; statements {} -> {}; {} goto(s) left; temporaries {} -> \
         {}; {} of {} native call(s) named{}",
        s.functions,
        s.incomplete,
        s.statements_in,
        s.statements_out,
        s.gotos_out,
        s.temporaries_in,
        s.temporaries_out,
        s.natives_named,
        s.native_calls,
        super::super::lifted::constructs(s)
    )
}

#[cfg(test)]
#[path = "gta6_script_export_tests.rs"]
mod tests;
