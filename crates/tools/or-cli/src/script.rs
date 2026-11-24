//! `openrage-cli script`: what a `.ysc` holds, and what its code does.
//!
//! ⚠ **A native is named only from a list the user supplies**, with
//! `--natives-db` or `OPENRAGE_NATIVES`, or downloaded on their request with
//! `script download-natives`. OpenRage ships none, so every output that shows
//! natives says which list named them — or that none did.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::gta5::script::natives::{
    Coverage, DownloadedFile, NativeDb, NativeDbSet, DOWNLOAD_MANIFEST,
};
use or_io::gta5::script::ScrProgram;
use or_io::gta5::script::{decompile, disasm};

#[path = "gta6_lifted.rs"]
mod lifted;

const NATIVES_ENV: &str = "OPENRAGE_NATIVES";

/// What `script download-natives` fetches: `(repository, branch, path, crossmap)`.
///
/// ⚠ The names are alloc8or's, keyed by each native's first hash, and the
/// crossmap carries them to the hashes version 12 scripts call; that pair
/// names the most of what retail scripts call (R63). None of it is bundled.
const NATIVES_UPSTREAM: [(&str, &str, &str, bool); 3] = [
    (
        "alloc8or/gta5-nativedb-data",
        "master",
        "natives.json",
        false,
    ),
    (
        "alloc8or/gta5-nativedb-data",
        "master",
        "natives_gen9.json",
        false,
    ),
    ("CSY0N/GTA-V-Crossmaps", "main", "Enhanced Crossmap.h", true),
];

pub fn run(args: &[String]) -> ExitCode {
    if args.first().map(String::as_str) == Some("download-natives") {
        return download_natives_command();
    }
    if args.first().map(String::as_str) == Some("lifted") {
        return lifted::run(&args[1..]);
    }
    let (mode, rest) = match args.first().map(String::as_str) {
        Some("disasm") => (Mode::Disasm, &args[1..]),
        Some("decompile") => (Mode::Decompile, &args[1..]),
        Some("check") => (Mode::Check, &args[1..]),
        _ => (Mode::Info, args),
    };

    let mut paths: Vec<PathBuf> = Vec::new();
    let mut what = What::Summary;
    let mut lists: Vec<PathBuf> = Vec::new();
    let mut show_bytes = true;

    let mut iter = rest.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" | "help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            "--natives" => what = What::Natives,
            "--strings" => what = What::Strings,
            "--statics" => what = What::Statics,
            "--no-bytes" => show_bytes = false,
            "--natives-db" => match iter.next() {
                Some(path) => lists.push(PathBuf::from(path)),
                None => {
                    eprintln!("Error: --natives-db needs a path");
                    return ExitCode::from(2);
                }
            },
            other if other.starts_with('-') => {
                eprintln!("Error: unknown option: {other}");
                return ExitCode::from(2);
            }
            other => paths.push(PathBuf::from(other)),
        }
    }

    if paths.is_empty() || (mode != Mode::Check && paths.len() > 1) {
        print_usage();
        return ExitCode::from(2);
    }

    let names = match load_lists(lists) {
        Ok(set) => set,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if mode == Mode::Check {
        return check_all(&paths);
    }

    let path = &paths[0];
    let Some(program) = load(path) else {
        return ExitCode::FAILURE;
    };
    match mode {
        Mode::Disasm => disassemble(&program, &names, show_bytes),
        Mode::Decompile => {
            let (db, sentence) = provenance(&program, &names);
            match source(&program, db.as_ref(), &sentence) {
                Ok(text) => {
                    print!("{text}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Mode::Info => {
            match what {
                What::Summary => info(&program, &names),
                What::Natives => natives(&program, &names),
                What::Strings => {
                    for s in &program.strings {
                        println!("{s}");
                    }
                }
                What::Statics => {
                    for (i, value) in program.statics.iter().enumerate() {
                        let tag = if (i as u32) < program.parameter_count {
                            " (argument)"
                        } else {
                            ""
                        };
                        println!("[{i}] 0x{value:016X}{tag}");
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Mode::Check => unreachable!(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Info,
    Disasm,
    Decompile,
    Check,
}

enum What {
    Summary,
    Natives,
    Strings,
    Statics,
}

fn load(path: &Path) -> Option<ScrProgram> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: read {}: {e}", path.display());
            return None;
        }
    };
    if bytes.starts_with(&or_io::container::rpf::RPF9_MAGIC.to_le_bytes()) {
        eprintln!(
            "Error: {}: {}",
            path.display(),
            super::gta6_scripts::NO_PROGRAM
        );
        return None;
    }
    match ScrProgram::from_bytes(&bytes) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("Error: {}: {e}", path.display());
            None
        }
    }
}

/// Where `script download-natives` writes, under the configuration directory the app also uses.
pub fn natives_dir(config_root: &Path) -> PathBuf {
    config_root.join("natives")
}

fn config_root() -> Option<PathBuf> {
    or_title::config::config_dir()
}

/// The lists a download left in `dir`, or none when nothing was downloaded.
pub fn downloaded_natives(dir: &Path) -> Result<NativeDbSet, String> {
    NativeDbSet::from_download(dir)
        .map(Option::unwrap_or_default)
        .map_err(|e| format!("the downloaded natives in {}: {e}", dir.display()))
}

/// Fetch every [`NATIVES_UPSTREAM`] file at its branch's current commit into `dir`, recording the commits.
///
/// ⚠ Only on the user's request: every caller is a command or a button the
/// user chose, and no test calls it. Nothing is written unless every file
/// arrives and reads.
pub fn download_natives(dir: &Path) -> Result<Vec<DownloadedFile>, String> {
    let agent = ureq::Agent::config_builder()
        .user_agent(concat!("openrage/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent();
    let mut fetched = Vec::new();
    for (repo, branch, path, crossmap) in NATIVES_UPSTREAM {
        let url = format!("https://api.github.com/repos/{repo}/commits/{branch}");
        let commit = agent
            .get(&url)
            .header("Accept", "application/vnd.github.sha")
            .call()
            .and_then(|mut r| r.body_mut().read_to_string())
            .map_err(|e| format!("{url}: {e}"))?
            .trim()
            .to_owned();
        if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("{url} answered `{commit}`, which is not a commit"));
        }
        let url = format!(
            "https://raw.githubusercontent.com/{repo}/{commit}/{}",
            path.replace(' ', "%20")
        );
        let bytes = agent
            .get(&url)
            .call()
            .and_then(|mut r| r.body_mut().read_to_vec())
            .map_err(|e| format!("{url}: {e}"))?;
        let file = DownloadedFile {
            file: path.rsplit('/').next().unwrap_or(path).to_owned(),
            repo: repo.to_owned(),
            path: path.to_owned(),
            commit,
            crossmap,
        };
        let reads = if crossmap {
            NativeDb::default()
                .cross_map(&file.label(), &bytes)
                .map(drop)
        } else {
            NativeDb::parse(file.label(), &bytes).map(drop)
        };
        reads.map_err(|e| format!("{url}: {e}"))?;
        fetched.push((file, bytes));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    for (file, bytes) in &fetched {
        let at = dir.join(&file.file);
        std::fs::write(&at, bytes).map_err(|e| format!("write {}: {e}", at.display()))?;
    }
    let files: Vec<DownloadedFile> = fetched.into_iter().map(|(file, _)| file).collect();
    let manifest = serde_json::to_vec_pretty(&files).map_err(|e| e.to_string())?;
    let at = dir.join(DOWNLOAD_MANIFEST);
    std::fs::write(&at, manifest).map_err(|e| format!("write {}: {e}", at.display()))?;
    Ok(files)
}

fn download_natives_command() -> ExitCode {
    let Some(dir) = config_root().map(|root| natives_dir(&root)) else {
        eprintln!("Error: no configuration directory; set OPENRAGE_CONFIG_DIR");
        return ExitCode::FAILURE;
    };
    match download_natives(&dir) {
        Ok(files) => {
            for file in &files {
                println!("{}", file.label());
            }
            println!("Downloaded into {}.", dir.display());
            println!(
                "OpenRage neither bundles nor redistributes these files, and alloc8or's \
                 lists carry no licence; they were fetched because you asked."
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The lists named on the command line, or else in `OPENRAGE_NATIVES`, or else the downloaded ones.
fn load_lists(named: Vec<PathBuf>) -> Result<NativeDbSet, String> {
    let paths = if named.is_empty() {
        std::env::var_os(NATIVES_ENV)
            .map(|raw| {
                raw.to_string_lossy()
                    .split(';')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default()
    } else {
        named
    };
    if paths.is_empty() {
        return match config_root() {
            Some(root) => downloaded_natives(&natives_dir(&root)),
            None => Ok(NativeDbSet::default()),
        };
    }
    let mut dbs = Vec::new();
    for path in paths {
        let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let label = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        dbs.push(NativeDb::parse(label, &bytes).map_err(|e| e.to_string())?);
    }
    Ok(NativeDbSet::new(dbs))
}

/// The list naming most of `program`'s natives, and the sentence saying which it is or why none is.
pub fn provenance(program: &ScrProgram, names: &NativeDbSet) -> (Option<NativeDb>, String) {
    let distinct = program
        .native_hashes()
        .into_iter()
        .collect::<BTreeSet<_>>()
        .len();
    if names.is_empty() {
        return (
            None,
            format!(
                "{} Supply one with --natives-db or {NATIVES_ENV}, or download one with \
                 `openrage-cli script download-natives`.",
                ScrProgram::native_names_are_unavailable()
            ),
        );
    }
    match names.best_for(program) {
        None => (
            None,
            format!(
                "No supplied natives list names any of this script's {distinct} native(s); \
                 they are shown by hash. The lists are per build, and this script's \
                 globals signature is 0x{:08X}.",
                program.globals_signature
            ),
        ),
        Some((db, Coverage { named, distinct })) => (
            Some(db.clone()),
            format!(
                "Native names from `{}`: {named} of {distinct} distinct native(s) named{}.",
                db.label(),
                if named == distinct {
                    String::new()
                } else {
                    format!("; the other {} are shown by hash", distinct - named)
                }
            ),
        ),
    }
}

fn info(program: &ScrProgram, names: &NativeDbSet) {
    println!("name              {}", program.name);
    println!("name hash         0x{:08X}", program.name_hash);
    println!("globals signature 0x{:08X}", program.globals_signature);
    println!(
        "code              {} bytes over {} page(s)",
        program.code_length,
        ScrProgram::pages_for(program.code_length as usize)
    );
    println!(
        "statics           {} ({} of them arguments)",
        program.statics.len(),
        program.parameter_count
    );
    println!("globals           {}", program.globals_count);
    println!(
        "natives           {} call site(s), {} distinct",
        program.natives.len(),
        program
            .native_hashes()
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
    );
    println!(
        "strings           {} over {} byte(s)",
        program.strings.len(),
        program.strings_length
    );
    match disasm::check(program) {
        Ok((_, report)) => println!(
            "instructions      {} in {} function(s){}",
            report.instructions,
            report.functions,
            if report.is_clean() {
                ""
            } else {
                " — the decode disagrees with the file; run `script check`"
            }
        ),
        Err(e) => println!("instructions      does not decode: {e}"),
    }
    println!();
    println!("{}", provenance(program, names).1);
}

fn natives(program: &ScrProgram, names: &NativeDbSet) {
    let (db, sentence) = provenance(program, names);
    let distinct: BTreeSet<u64> = program.native_hashes().into_iter().collect();
    for hash in &distinct {
        match db.as_ref().and_then(|db| db.name(*hash)) {
            Some(name) => println!("0x{hash:016X}  {name}"),
            None => println!("0x{hash:016X}"),
        }
    }
    println!();
    println!(
        "{} distinct native(s) across {} call site(s).",
        distinct.len(),
        program.natives.len()
    );
    println!("{sentence}");
}

fn disassemble(program: &ScrProgram, names: &NativeDbSet, show_bytes: bool) -> ExitCode {
    let (db, sentence) = provenance(program, names);
    match listing(program, db.as_ref(), &sentence, show_bytes) {
        Ok((text, clean)) => {
            print!("{text}");
            if clean {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The listing `script disasm` prints under `sentence`, and whether the decode agrees with the file.
pub fn listing(
    program: &ScrProgram,
    names: Option<&NativeDb>,
    sentence: &str,
    show_bytes: bool,
) -> Result<(String, bool), String> {
    use std::fmt::Write as _;

    let (instructions, report) =
        disasm::check(program).map_err(|e| format!("`{}` does not decode: {e}", program.name))?;
    let write = |out: &mut String| -> std::fmt::Result {
        writeln!(
            out,
            "; {}: {} bytes of code, {} instructions in {} function(s)",
            program.name, report.code_length, report.instructions, report.functions
        )?;
        writeln!(out, "; {sentence}")?;
        for problem in report.problems() {
            writeln!(out, "; ⚠ {problem}")?;
        }
        disasm::write_listing(out, program, &instructions, names, show_bytes)
    };
    let mut out = String::new();
    write(&mut out).map_err(|e| e.to_string())?;
    Ok((out, report.is_clean()))
}

/// The C-like source `script decompile` prints under `sentence`.
pub fn source(
    program: &ScrProgram,
    names: Option<&NativeDb>,
    sentence: &str,
) -> Result<String, String> {
    use std::fmt::Write as _;

    let out = decompile::decompile(program, names)
        .map_err(|e| format!("`{}` does not decode: {e}", program.name))?;
    let s = out.stats;
    let mut text = String::new();
    let _ = writeln!(
        text,
        "// {}: decompiled from {} bytes of code, {} instructions in {} function(s), {} of them with a goto",
        program.name, program.code_length, s.instructions, s.functions, s.functions_with_goto
    );
    let _ = writeln!(text, "// {sentence}");
    if s.stack_mismatches > 0 {
        let _ = writeln!(
            text,
            "// ⚠ the stack model failed {} time(s); `__missing` marks each place",
            s.stack_mismatches
        );
    }
    text.push_str(&out.text);
    Ok(text)
}

fn check_all(paths: &[PathBuf]) -> ExitCode {
    let mut clean = 0usize;
    for path in paths {
        let Some(program) = load(path) else {
            continue;
        };
        match disasm::check(&program) {
            Err(e) => println!("{}: does not decode: {e}", path.display()),
            Ok((_, report)) if report.is_clean() => {
                clean += 1;
                println!(
                    "{}: clean — {} bytes, {} instructions, {} jump(s), {} case(s), {} call(s), \
                     {} native call(s) checked",
                    path.display(),
                    report.bytes_decoded,
                    report.instructions,
                    report.jumps_checked,
                    report.cases_checked,
                    report.calls_checked,
                    report.natives_checked
                );
            }
            Ok((_, report)) => println!("{}: {}", path.display(), report.problems().join("; ")),
        }
    }
    println!(
        "{clean} of {} script(s) decode and agree with their own file.",
        paths.len()
    );
    if clean == paths.len() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn print_usage() {
    eprintln!("Inspect and disassemble a compiled script (.ysc).");
    eprintln!();
    eprintln!("Usage: openrage-cli script <file.ysc> [--natives|--strings|--statics]");
    eprintln!("       openrage-cli script disasm <file.ysc> [--no-bytes]");
    eprintln!("       openrage-cli script decompile <file.ysc>");
    eprintln!("       openrage-cli script check <file.ysc>...");
    eprintln!("       openrage-cli script download-natives");
    eprintln!("       openrage-cli script lifted <listing.ysc.cs> [-o <file>]");
    eprintln!("       openrage-cli script lifted --census <folder> [-o <folder>]");
    eprintln!();
    eprintln!("  disasm      An instruction listing: jump and switch targets resolved,");
    eprintln!("              calls named by function, natives named where a list does.");
    eprintln!("  decompile   C-like source: functions, expressions, if/else, loops and");
    eprintln!("              switches, with a labelled goto only where none fits.");
    eprintln!("  check       Decode each script and check it against its own file: exactly");
    eprintln!("              code_length bytes, no instruction across a code page, every");
    eprintln!("              jump, case and call on an instruction, every native slot in");
    eprintln!("              the table. Exits non-zero if any script disagrees.");
    eprintln!("  lifted      Readable source from a lifted listing of a GTA VI script, the");
    eprintln!("              text another decompiler writes from its bytecode. A compiled");
    eprintln!("              GTA VI script cannot be read yet; `script lifted --help` says");
    eprintln!("              what the command takes.");
    eprintln!("  --natives   The natives this script calls, by hash, named where a list does.");
    eprintln!("  --strings   The script's string heap, one entry per line.");
    eprintln!("  --statics   The initial static values; the leading ones are arguments.");
    eprintln!();
    eprintln!("Native names:");
    eprintln!("  --natives-db <path>   A natives list, JSON or `0xHASH NAME` text. Repeat");
    eprintln!("                        for one list per build; each script takes the list");
    eprintln!("                        naming most of its natives. Without the flag,");
    eprintln!("                        {NATIVES_ENV} is read as ;-separated paths, and");
    eprintln!("                        without either, what download-natives fetched.");
    eprintln!("  download-natives      Fetch alloc8or's natives lists and CSY0N's crossmap from");
    eprintln!("                        GitHub into the configuration directory, recording");
    eprintln!("                        each file's commit. Runs only when you ask.");
    eprintln!("  ⚠ OpenRage ships no natives list. A hash no list names stays a hash.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta5::script::opcode;

    /// ⚠ Never fetches: the download is a directory written by hand, as `script download-natives` would leave it.
    #[test]
    fn a_downloaded_database_names_natives_and_the_sentence_names_its_commit() {
        let dir = tempfile::tempdir().unwrap();
        assert!(downloaded_natives(dir.path()).unwrap().is_empty());
        std::fs::write(
            dir.path().join("natives.json"),
            br#"{"BUILTIN": {"0x00000000000000A1": {"name": "WAIT",
                "params": [{"type": "int", "name": "ms"}], "return_type": "void"}}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("Enhanced Crossmap.h"), b"{0xA1, 0xC1},\n").unwrap();
        let files = [
            DownloadedFile {
                file: "natives.json".into(),
                repo: "alloc8or/gta5-nativedb-data".into(),
                path: "natives.json".into(),
                commit: "424fb51b089049a9fbcebcc641500b1d44d255b4".into(),
                crossmap: false,
            },
            DownloadedFile {
                file: "Enhanced Crossmap.h".into(),
                repo: "CSY0N/GTA-V-Crossmaps".into(),
                path: "Enhanced Crossmap.h".into(),
                commit: "5378f84da53cdc1d633418b1decbddd32b5619d2".into(),
                crossmap: true,
            },
        ];
        std::fs::write(
            dir.path().join(DOWNLOAD_MANIFEST),
            serde_json::to_vec(&files).unwrap(),
        )
        .unwrap();
        let names = downloaded_natives(dir.path()).unwrap();

        let code = vec![
            opcode::ENTER,
            0,
            2,
            0,
            0,
            opcode::NATIVE,
            1 << 2,
            0,
            0,
            opcode::LEAVE,
            0,
            0,
        ];
        let code_length = code.len() as u32;
        let program = ScrProgram {
            code_length,
            code,
            natives: vec![0xC1u64.rotate_right(code_length & 0x3F)],
            name: "main".into(),
            resource_version: 12,
            ..ScrProgram::default()
        };
        let (db, sentence) = provenance(&program, &names);
        assert_eq!(
            sentence,
            "Native names from `alloc8or/gta5-nativedb-data/natives.json@424fb51b0890 through \
             CSY0N/GTA-V-Crossmaps/Enhanced Crossmap.h@5378f84da53c`: 1 of 1 distinct native(s) named."
        );
        let (text, _) = listing(&program, db.as_ref(), &sentence, false).unwrap();
        assert!(
            text.contains("BUILTIN::WAIT(int ms) -> void  ; 0x00000000000000C1"),
            "{text}"
        );
    }

    #[test]
    fn decompile_prints_the_source_under_its_provenance_sentence() {
        let code = vec![
            opcode::ENTER,
            0,
            2,
            0,
            0,
            opcode::PUSH_CONST_7,
            opcode::LEAVE,
            0,
            1,
        ];
        let program = ScrProgram {
            code_length: code.len() as u32,
            code,
            name: "main".into(),
            resource_version: 12,
            ..ScrProgram::default()
        };
        let text = source(&program, None, "no list").expect("decompiles");
        let head = format!("// main: decompiled from {} bytes", program.code_length);
        assert!(text.starts_with(&head), "{text}");
        assert!(text.contains("// no list\n"), "{text}");
        assert!(
            text.contains("var func_0()\n{\n    return 7;\n}\n"),
            "{text}"
        );
    }
}
