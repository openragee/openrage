//! `openrage-cli script`: what a `.ysc` holds, and what its code does.
//!
//! ⚠ **A native is named only from a list the user supplies**, with
//! `--natives-db` or `OPENRAGE_NATIVES`, or downloaded on their request with
//! `script download-natives`. OpenRage ships none, so every output that shows
//! natives says which list named them — or that none did.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::gta5::script::natives::{
    Coverage, DownloadedFile, NativeDb, NativeDbSet, DOWNLOAD_MANIFEST,
};
use or_io::gta5::script::ScrProgram;
use or_io::gta5::script::{decompile, disasm};

#[path = "gta6_compiled.rs"]
mod compiled;
#[path = "gta6_lifted.rs"]
mod lifted;

#[path = "gta6_script.rs"]
mod gta6;

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
    if args.first().map(String::as_str) == Some("compiled") {
        return compiled::run(&args[1..]);
    }
    // A compiled GTA VI script is told from GTA V's by the command that reads it.
    if let Some(code) = gta6::run(args) {
        return code;
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
            "-h" | "--help" | "help" => return printed(&gta6::usage(), ExitCode::SUCCESS),
            "--natives" => what = What::Natives,
            "--strings" => what = What::Strings,
            "--statics" => what = What::Statics,
            "--no-bytes" => show_bytes = false,
            "--natives-db" => match iter.next() {
                Some(path) => lists.push(PathBuf::from(path)),
                None => return misused("--natives-db needs a path"),
            },
            other if other.starts_with('-') => {
                return misused(&format!("unknown option: {other}"));
            }
            other => paths.push(PathBuf::from(other)),
        }
    }

    // Only `check` takes more than one script.
    let one = mode == Mode::Check || paths.len() == 1;
    let Some(first) = paths.first().filter(|_| one) else {
        note(gta6::usage().trim_end());
        return ExitCode::from(2);
    };

    let names = match load_lists(lists) {
        Ok(set) => set,
        Err(e) => return failed(&e),
    };

    if mode == Mode::Check {
        return check_all(&paths);
    }

    let program = match load(first) {
        Ok(program) => program,
        Err(e) => return failed(&e),
    };
    match mode {
        Mode::Disasm => disassemble(&program, &names, show_bytes),
        Mode::Decompile => {
            let (db, sentence) = provenance(&program, &names);
            match source(&program, db.as_ref(), &sentence) {
                Ok(text) => printed(&text, ExitCode::SUCCESS),
                Err(e) => failed(&e),
            }
        }
        // `check` has returned above.
        Mode::Info | Mode::Check => {
            let text = match what {
                What::Summary => info(&program, &names),
                What::Natives => natives(&program, &names),
                What::Strings => program.strings.iter().map(|s| format!("{s}\n")).collect(),
                What::Statics => statics(&program),
            };
            printed(&text, ExitCode::SUCCESS)
        }
    }
}

/// Writes `text` to stdout; a reader that closed the pipe has what it wanted, and that is no error.
fn put(text: &str) -> std::io::Result<()> {
    put_to(&mut std::io::stdout().lock(), text)
}

/// Writes `text` to `out` and flushes it, a closed pipe counting as written.
fn put_to(out: &mut impl std::io::Write, text: &str) -> std::io::Result<()> {
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Err(e) if e.kind() == ErrorKind::BrokenPipe => Ok(()),
        done => done,
    }
}

/// Prints `text` and gives `code`; a stdout that takes nothing for another reason than a closed pipe is a failure.
fn printed(text: &str, code: ExitCode) -> ExitCode {
    match put(text) {
        Ok(()) => code,
        Err(e) => failed(&format!("write to stdout: {e}")),
    }
}

/// Writes one line to stderr.
// ⚠ Never `eprintln!`: with both streams in one pipe a reader that left would turn the line into a panic.
fn note(line: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{line}");
}

/// Says why the command failed, and gives the exit code of a failure.
fn failed(why: &str) -> ExitCode {
    note(&format!("Error: {why}"));
    ExitCode::FAILURE
}

/// Says what is wrong with the arguments, and gives the exit code of a usage error.
fn misused(what: &str) -> ExitCode {
    note(&format!("Error: {what}"));
    ExitCode::from(2)
}

/// `text` with `lines` after its leading comment lines, which are its header.
fn after_header(text: &str, lines: &str) -> String {
    let header = text
        .split_inclusive('\n')
        .take_while(|line| line.starts_with("//"));
    let end: usize = header.map(str::len).sum();
    let (header, rest) = text.split_at(end);
    // A header that is the whole text may end on no line break.
    let gap = match header.is_empty() || header.ends_with('\n') {
        true => "",
        false => "\n",
    };
    format!("{header}{gap}{lines}{rest}")
}

/// A GTA V script's initial static values, one per line.
fn statics(program: &ScrProgram) -> String {
    let mut out = String::new();
    for (i, value) in program.statics.iter().enumerate() {
        let tag = if (i as u32) < program.parameter_count {
            " (argument)"
        } else {
            ""
        };
        let _ = writeln!(out, "[{i}] 0x{value:016X}{tag}");
    }
    out
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

/// The GTA V script at `path`, or why it is none, with the path.
fn load(path: &Path) -> Result<ScrProgram, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    ScrProgram::from_bytes(&bytes).map_err(|e| format!("{}: {e}", path.display()))
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
        return failed("no configuration directory; set OPENRAGE_CONFIG_DIR");
    };
    match download_natives(&dir) {
        Ok(files) => {
            let mut out = String::new();
            for file in &files {
                let _ = writeln!(out, "{}", file.label());
            }
            let _ = writeln!(out, "Downloaded into {}.", dir.display());
            let _ = writeln!(
                out,
                "OpenRage neither bundles nor redistributes these files, and alloc8or's \
                 lists carry no licence; they were fetched because you asked."
            );
            printed(&out, ExitCode::SUCCESS)
        }
        Err(e) => failed(&e),
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

fn info(program: &ScrProgram, names: &NativeDbSet) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "name              {}", program.name);
    let _ = writeln!(out, "name hash         0x{:08X}", program.name_hash);
    let _ = writeln!(out, "globals signature 0x{:08X}", program.globals_signature);
    let _ = writeln!(
        out,
        "code              {} bytes over {} page(s)",
        program.code_length,
        ScrProgram::pages_for(program.code_length as usize)
    );
    let _ = writeln!(
        out,
        "statics           {} ({} of them arguments)",
        program.statics.len(),
        program.parameter_count
    );
    let _ = writeln!(out, "globals           {}", program.globals_count);
    let _ = writeln!(
        out,
        "natives           {} call site(s), {} distinct",
        program.natives.len(),
        program
            .native_hashes()
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
    );
    let _ = writeln!(
        out,
        "strings           {} over {} byte(s)",
        program.strings.len(),
        program.strings_length
    );
    let _ = match disasm::check(program) {
        Ok((_, report)) => writeln!(
            out,
            "instructions      {} in {} function(s){}",
            report.instructions,
            report.functions,
            if report.is_clean() {
                ""
            } else {
                " — the decode disagrees with the file; run `script check`"
            }
        ),
        Err(e) => writeln!(out, "instructions      does not decode: {e}"),
    };
    let _ = writeln!(out, "\n{}", provenance(program, names).1);
    out
}

fn natives(program: &ScrProgram, names: &NativeDbSet) -> String {
    let (db, sentence) = provenance(program, names);
    let distinct: BTreeSet<u64> = program.native_hashes().into_iter().collect();
    let mut out = String::new();
    for hash in &distinct {
        let _ = match db.as_ref().and_then(|db| db.name(*hash)) {
            Some(name) => writeln!(out, "0x{hash:016X}  {name}"),
            None => writeln!(out, "0x{hash:016X}"),
        };
    }
    let _ = writeln!(
        out,
        "\n{} distinct native(s) across {} call site(s).",
        distinct.len(),
        program.natives.len()
    );
    let _ = writeln!(out, "{sentence}");
    out
}

fn disassemble(program: &ScrProgram, names: &NativeDbSet, show_bytes: bool) -> ExitCode {
    let (db, sentence) = provenance(program, names);
    match listing(program, db.as_ref(), &sentence, show_bytes) {
        Ok((text, true)) => printed(&text, ExitCode::SUCCESS),
        Ok((text, false)) => printed(&text, ExitCode::FAILURE),
        Err(e) => failed(&e),
    }
}

/// The listing `script disasm` prints under `sentence`, and whether the decode agrees with the file.
pub fn listing(
    program: &ScrProgram,
    names: Option<&NativeDb>,
    sentence: &str,
    show_bytes: bool,
) -> Result<(String, bool), String> {
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
        let line = match checked(path) {
            Ok((line, agrees)) => {
                clean += usize::from(agrees);
                line
            }
            Err(e) => {
                note(&format!("Error: {e}"));
                continue;
            }
        };
        if let Err(e) = put(&format!("{line}\n")) {
            return failed(&format!("write to stdout: {e}"));
        }
    }
    let total = format!(
        "{clean} of {} script(s) decode and agree with their own file.\n",
        paths.len()
    );
    if clean == paths.len() {
        printed(&total, ExitCode::SUCCESS)
    } else {
        printed(&total, ExitCode::FAILURE)
    }
}

/// The line `check` prints of the GTA V script at `path`, and whether its decode agrees with its file; why the file is no script, with the path.
fn checked(path: &Path) -> Result<(String, bool), String> {
    let program = load(path)?;
    let at = path.display();
    Ok(match disasm::check(&program) {
        Err(e) => (format!("{at}: does not decode: {e}"), false),
        Ok((_, report)) if report.is_clean() => (
            format!(
                "{at}: clean — {} bytes, {} instructions, {} jump(s), {} case(s), {} call(s), \
                 {} native call(s) checked",
                report.bytes_decoded,
                report.instructions,
                report.jumps_checked,
                report.cases_checked,
                report.calls_checked,
                report.natives_checked
            ),
            true,
        ),
        Ok((_, report)) => (format!("{at}: {}", report.problems().join("; ")), false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta5::script::opcode;

    /// A stdout that answers every write with `kind`.
    struct Refusing(ErrorKind);

    impl std::io::Write for Refusing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(self.0.into())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_reader_that_closed_the_pipe_is_no_error_and_any_other_refusal_is_one() {
        assert!(put_to(&mut Refusing(ErrorKind::BrokenPipe), "a line\n").is_ok());
        let full = put_to(&mut Refusing(ErrorKind::StorageFull), "a line\n");
        assert_eq!(full.err().map(|e| e.kind()), Some(ErrorKind::StorageFull));
        let mut taken = Vec::new();
        assert!(put_to(&mut taken, "a line\n").is_ok());
        assert_eq!(taken, b"a line\n");
    }

    #[test]
    fn lines_go_after_the_comment_lines_a_text_opens_with() {
        let text = "// one\n// two\n\nvoid main()\n{\n    // inside\n}\n";
        assert_eq!(
            after_header(text, "// own\n"),
            "// one\n// two\n// own\n\nvoid main()\n{\n    // inside\n}\n"
        );
        // A text that opens with no comment takes them first, and one that is all header takes them last.
        assert_eq!(
            after_header("void main()\n", "// own\n"),
            "// own\nvoid main()\n"
        );
        assert_eq!(after_header("// one", "// own\n"), "// one\n// own\n");
        assert_eq!(after_header("", "// own\n"), "// own\n");
    }

    #[test]
    fn a_file_that_is_no_gta_v_script_is_refused_by_its_path_and_never_checked() {
        let dir = tempfile::tempdir().unwrap();
        let junk = dir.path().join("junk.ysc");
        std::fs::write(&junk, b"no resource").unwrap();
        let refused = checked(&junk).expect_err("no script");
        assert!(
            refused.starts_with(&junk.display().to_string()),
            "{refused}"
        );
        let missing = checked(&dir.path().join("missing.ysc")).expect_err("no file");
        assert!(missing.starts_with("read "), "{missing}");
    }

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
