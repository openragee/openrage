//! `openrage-cli script lifted`: readable source from a lifted listing of a GTA VI script.
//!
//! ⚠ **A lifted listing is text, not a script.** It is what another decompiler writes from a
//! script's bytecode, one C# method per script function, and this command reads that text and
//! nothing else. A compiled GTA VI script is read by `script decompile`, never by this command.
//!
//! ⚠ **Every name is the user's.** Natives come from `--natives-db` or `OPENRAGE_NATIVES`, type
//! and program names from `--names`, and the source's first lines say which lists named things.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use or_io::gta5::script::natives::NativeDb;
use or_io::gta6::script::listing::Listing;
use or_io::gta6::script::names::declared_natives;
use or_io::gta6::script::project::index;
use or_io::gta6::script::{
    decompile_listing, Context, DecompileError, Level, Names, Options, Pass, Project, Stats,
    CONSTRUCTS,
};

/// What this command reads and what it does not, for the help and for a file that is no text.
const WHAT_IS_READ: &str = "`script lifted` reads a lifted listing, the text another decompiler \
     writes from a script's bytecode; `script decompile` reads a compiled GTA VI script";

/// The listings a census takes from a folder.
const LISTING_SUFFIX: &str = ".ysc.cs";

/// The lists the user supplied.
#[derive(Default)]
pub struct Supplied {
    /// Natives lists; each listing takes the one naming most of the natives it declares.
    pub natives: Vec<NativeDb>,
    /// Type and program names.
    pub names: Names,
    /// The level and the passes asked for; the four first passes where none is.
    pub options: Option<Options>,
    /// What other programs state of the types and functions a listing uses, where a project file was named.
    pub project: Option<Project>,
}

pub fn run(args: &[String]) -> ExitCode {
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut natives: Vec<PathBuf> = Vec::new();
    let mut names: Vec<PathBuf> = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut census = false;
    let mut index_of = false;
    let mut level: Option<Level> = None;
    let mut off: Vec<Pass> = Vec::new();
    let mut project: Option<PathBuf> = None;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" | "help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            "--census" => census = true,
            "--index" => index_of = true,
            "--natives-db" | "--names" | "--project" | "-o" => {
                let Some(path) = iter.next().map(PathBuf::from) else {
                    eprintln!("Error: {arg} needs a path");
                    return ExitCode::from(2);
                };
                match arg.as_str() {
                    "--natives-db" => natives.push(path),
                    "--names" => names.push(path),
                    "--project" => project = Some(path),
                    _ => out = Some(path),
                }
            }
            "--level" | "--no" => {
                let value = iter.next().map(String::as_str).unwrap_or_default();
                let known = match arg.as_str() {
                    "--level" => Level::from_name(value).map(|asked| level = Some(asked)),
                    _ => Pass::from_name(value).map(|pass| off.push(pass)),
                };
                if known.is_none() {
                    eprintln!("Error: {arg} takes a name `script passes` prints, not `{value}`");
                    return ExitCode::from(2);
                }
            }
            other if other.starts_with('-') => {
                eprintln!("Error: unknown option: {other}");
                return ExitCode::from(2);
            }
            other => paths.push(PathBuf::from(other)),
        }
    }
    let [path] = paths.as_slice() else {
        print_usage();
        return ExitCode::from(2);
    };
    if index_of {
        let Some(to) = &out else {
            eprintln!("Error: --index needs -o <file> for the project");
            return ExitCode::from(2);
        };
        return match indexed(path, to) {
            Ok(line) => {
                println!("{line}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if census && out.as_deref().is_some_and(Path::is_file) {
        eprintln!("Error: with --census, -o names a folder for the sources and not a file");
        return ExitCode::from(2);
    }
    let lists = supplied(natives, &names).and_then(|mut supplied| {
        // With no level asked the source is what the four first passes always wrote; `--no` alone takes them from every pass.
        if level.is_some() || !off.is_empty() {
            let all = Options::at(level.unwrap_or(Level::Source));
            let asked = off.iter().fold(all, |options, pass| options.without(*pass));
            supplied.options = Some(asked);
        }
        if let Some(path) = &project {
            let refused = |why: String| format!("{}: {why}", path.display());
            let bytes = std::fs::read(path).map_err(|e| refused(format!("read: {e}")))?;
            supplied.project = Some(index::load(&bytes).map_err(|e| refused(e.to_string()))?);
        }
        Ok(supplied)
    });
    let supplied = match lists {
        Ok(supplied) => supplied,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if census {
        return match census_of(path, &supplied, out.as_deref()) {
            Ok((lines, failed)) => {
                for line in &lines {
                    println!("{line}");
                }
                match failed {
                    0 => ExitCode::SUCCESS,
                    _ => ExitCode::FAILURE,
                }
            }
            Err(e) => {
                eprintln!("Error: {e}");
                ExitCode::FAILURE
            }
        };
    }

    let written = read(path)
        .and_then(|text| source(&text, &supplied).map_err(|e| e.to_string()))
        .and_then(|(text, _)| match &out {
            Some(to) => {
                std::fs::write(to, text).map_err(|e| format!("write {}: {e}", to.display()))
            }
            None => {
                print!("{text}");
                Ok(())
            }
        });
    match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {}: {e}", path.display());
            ExitCode::FAILURE
        }
    }
}

/// Reads the lists named: natives as `script` reads them everywhere, and `0xHASH name` lists of types and programs.
pub fn supplied(natives: Vec<PathBuf>, names: &[PathBuf]) -> Result<Supplied, String> {
    Ok(Supplied {
        natives: super::load_lists(natives)?.dbs().to_vec(),
        names: names_from(names)?,
        ..Supplied::default()
    })
}

/// Reads names lists, each under its file's name.
pub(super) fn names_from(paths: &[PathBuf]) -> Result<Names, String> {
    let mut names = Names::default();
    for path in paths {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        (names.read_list(&name_of(path), &text)).map_err(|e| e.to_string())?;
    }
    Ok(names)
}

/// The readable source of the listing `text`, and what decompiling it counted.
pub fn source(text: &str, supplied: &Supplied) -> Result<(String, Stats), DecompileError> {
    let listing = Listing::open(text)?;
    let declared: BTreeSet<u64> = declared_natives(listing.declarations())
        .into_keys()
        .collect();
    let mut names = supplied.names.clone();
    names.natives = Names::best_natives(&supplied.natives, &declared).map(|(db, _)| db.clone());

    let options = supplied.options.unwrap_or_else(Options::baseline);
    let project = supplied.project.as_ref().unwrap_or(Project::empty());
    let done = decompile_listing(text, &Context::new(&names, project, options))?;
    let text = done.files.into_iter().map(|file| file.text).collect();
    Ok((text, done.stats))
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::InvalidData => format!("the file is not text. {WHAT_IS_READ}"),
        _ => format!("read: {e}"),
    })
}

/// One line per listing of `folder` and a total, and how many listings failed; each source goes to `sources` under the listing's name where a folder is given.
// ⚠ A listing that does not decompile is a line, never the end of the census.
pub fn census_of(
    folder: &Path,
    supplied: &Supplied,
    sources: Option<&Path>,
) -> Result<(Vec<String>, usize), String> {
    if let Some(dir) = sources {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let listings = listings_of(folder)?;

    let outcomes = over(&listings, THREADS, |path| {
        let text = read(path)?;
        // A fault in one listing's decompile is that listing's failure.
        match std::panic::catch_unwind(|| source(&text, supplied)) {
            Ok(Ok((written, stats))) => {
                if let Some(dir) = sources {
                    let name = name_of(path);
                    let stem = name.strip_suffix(LISTING_SUFFIX).unwrap_or(&name);
                    let to = dir.join(format!("{stem}.cs"));
                    std::fs::write(&to, written)
                        .map_err(|e| format!("write {}: {e}", to.display()))?;
                }
                Ok(stats)
            }
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err("the decompiler panicked; its message is on stderr".to_owned()),
        }
    });

    let mut total = Stats::default();
    let mut failed = 0;
    let mut lines = Vec::new();
    for (path, outcome) in listings.iter().zip(&outcomes) {
        match outcome {
            Ok(stats) => {
                total.add(stats);
                lines.push(format!("{}: {}", name_of(path), counts(stats)));
            }
            Err(e) => {
                failed += 1;
                lines.push(format!("{}: failed: {e}", name_of(path)));
            }
        }
    }
    lines.push(format!(
        "total: {} listing(s), {failed} failed: {}",
        listings.len(),
        counts(&total)
    ));
    Ok((lines, failed))
}

/// The listings of `folder`, in the order of their names.
fn listings_of(folder: &Path) -> Result<Vec<PathBuf>, String> {
    let entries =
        std::fs::read_dir(folder).map_err(|e| format!("read {}: {e}", folder.display()))?;
    let mut listings: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| name_of(path).ends_with(LISTING_SUFFIX))
        .collect();
    listings.sort();
    match listings.is_empty() {
        true => Err(format!(
            "{} holds no `*{LISTING_SUFFIX}` listing",
            folder.display()
        )),
        false => Ok(listings),
    }
}

/// Writes to `to` the project of every listing of `folder`, which `--project` reads; the line saying so.
// ⚠ One listing that does not read ends it, named: a project of some of the programs would answer for all.
fn indexed(folder: &Path, to: &Path) -> Result<String, String> {
    let listings = listings_of(folder)?;
    let project = index::build(&listings).map_err(|e| e.to_string())?;
    let bytes = index::save(&project).map_err(|e| e.to_string())?;
    std::fs::write(to, &bytes).map_err(|e| format!("write {}: {e}", to.display()))?;
    Ok(format!(
        "{} listing(s): a project of {} byte(s) in {}",
        listings.len(),
        bytes.len(),
        to.display()
    ))
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn counts(s: &Stats) -> String {
    format!(
        "{} function(s), {} with no body; statements {} -> {}; gotos {} -> {}; temporaries {} -> \
         {}; {} of {} native call(s) named{}",
        s.functions,
        s.incomplete,
        s.statements_in,
        s.statements_out,
        s.gotos_in,
        s.gotos_out,
        s.temporaries_in,
        s.temporaries_out,
        s.natives_named,
        s.native_calls,
        constructs(s)
    )
}

/// The source constructs counted, each kind that was made with its count; nothing where none was.
pub(super) fn constructs(s: &Stats) -> String {
    let made = CONSTRUCTS.iter().zip(s.constructs);
    let made: Vec<String> = made
        .filter(|(_, count)| *count > 0)
        .map(|(kind, count)| format!("{kind} {count}"))
        .collect();
    match made.is_empty() {
        true => String::new(),
        false => format!("; constructs: {}", made.join(", ")),
    }
}

/// The threads a census works on at most.
// ⚠ A thread holds one listing's whole text and its source, and the largest listing is 200 MB.
pub(super) const THREADS: usize = 8;

/// What `each` gives for every item, in the items' order, worked out on up to `threads` threads.
pub(super) fn over<I: Sync, T: Send>(
    items: &[I],
    threads: usize,
    each: impl Fn(&I) -> T + Sync,
) -> Vec<T> {
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<(usize, T)>> = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(threads)
        .min(items.len());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let n = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(n) else {
                    break;
                };
                let result = each(item);
                let mut done = done.lock().unwrap_or_else(PoisonError::into_inner);
                done.push((n, result));
            });
        }
    });
    let mut done = done.into_inner().unwrap_or_else(PoisonError::into_inner);
    done.sort_by_key(|(n, _)| *n);
    done.into_iter().map(|(_, result)| result).collect()
}

fn print_usage() {
    eprintln!("Decompile a lifted listing of a GTA VI script to readable source.");
    eprintln!();
    eprintln!("Usage: openrage-cli script lifted <listing.ysc.cs> [-o <file>]");
    eprintln!("       openrage-cli script lifted --census <folder> [-o <folder>]");
    eprintln!("         [--level <name>] [--no <pass>]... [--project <file>]");
    eprintln!("       openrage-cli script lifted --index <folder> -o <project.json>");
    eprintln!();
    eprintln!("  A lifted listing is the text another decompiler writes from a script's");
    eprintln!("  bytecode, one C# method per script function. This command reads that text");
    eprintln!("  and nothing else: `script decompile` reads a compiled GTA VI script.");
    eprintln!();
    eprintln!("  -o <file>   Write the source to a file instead of printing it.");
    eprintln!(
        "  --census    Decompile every `*{LISTING_SUFFIX}` listing of a folder and print one"
    );
    eprintln!("              line of counts per listing and a total. A listing that fails is");
    eprintln!("              reported and the census goes on; it exits non-zero if any did.");
    eprintln!("              With -o <folder>, each listing's source is written there too.");
    eprintln!("  --index     Write the project of every listing of a folder: what the listings");
    eprintln!("              state of each other's types, fields and functions, for --project.");
    eprintln!("  --level <name>   How far the source is taken: lifted, structured or source,");
    eprintln!("                   with every pass of that level. Without it the source is");
    eprintln!("                   what the four first passes write, as it always was.");
    eprintln!("  --no <pass>      Leave one pass off, by the name `script passes` gives it.");
    eprintln!("                   Repeat for several.");
    eprintln!("  --project <file> The project.json a `script export` wrote.");
    eprintln!();
    eprintln!("Names:");
    eprintln!("  --natives-db <path>   A natives list, JSON or `0xHASH NAME` text. Repeat for");
    eprintln!("                        several; each listing takes the list naming most of the");
    eprintln!("                        natives it declares. Without the flag,");
    eprintln!("                        OPENRAGE_NATIVES is read as ;-separated paths.");
    eprintln!("  --names <path>        Type and program names: one `0xHASH name` per line, or a");
    eprintln!("                        tab-separated table whose leading columns are hashes");
    eprintln!("                        and whose next column is the name. Repeat for several.");
    eprintln!("  ⚠ OpenRage ships no names. The first lines of the source say which lists");
    eprintln!("    named things, and a hash no list names stays a hash.");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A listing written by hand, under names and hashes that are no game's.
    const LISTING: &str = r#"// import 1: program_0x0A0B0C0D, types [2, 100)
[VmProgram(0x00000001U)]
public static class Program_00000001
{
    [VmFunction(0x000010)]
    public static void main() // 0x000010
    {
        VmValue[] parameters = new VmValue[] {  };
        VmValue[] locals = new VmValue[1];
        ref VmValue local_0 = ref locals[0];
        local_0 = Native.DEMO.GET_DEMO_THING(7); // RVA: 0x01020304
        Native._0x00000000000000BB(local_0); // RVA: 0x01020308
        return;
        throw new global::System.InvalidOperationException("Unexpected bytecode fallthrough.");
    }

    public static class Native
    {
        public static class DEMO
        {
            public static VmValue GET_DEMO_THING(VmValue p0) => Vm.Native(0x00000000000000A1UL, 0x01020304U, 1, p0)[0];
        }
        public static void _0x00000000000000BB(VmValue p0) => Vm.Native(0x00000000000000BBUL, 0x01020308U, 0, p0);
    }
}
"#;

    #[test]
    fn a_listing_becomes_source_under_a_header_that_says_no_list_named_anything() {
        let (text, stats) = source(LISTING, &Supplied::default()).expect("the listing decompiles");
        let head = "\
// program 0x00000001
// imports: 0x0A0B0C0D
// natives: 2 declared, no natives list supplied: 1 named by the listing itself, 1 shown by hash
// names: no names list supplied: a type or a program the listing does not name is shown by its hash
// 1 function(s), 0 of them with no body in the listing;";
        // The first line says a lifted listing was read, and no compiled script.
        assert!(
            text.starts_with("// Readable source of a lifted listing: "),
            "{text}"
        );
        assert!(text.contains(head), "{text}");
        assert!(text.contains("\nvoid main()\n{\n"), "{text}");
        assert!(text.contains("DEMO.GET_DEMO_THING(7)"), "{text}");
        assert!(text.contains("_0x00000000000000BB("), "{text}");
        assert_eq!((stats.functions, stats.native_calls), (1, 2));
        assert_eq!(stats.natives_named, 1);
    }

    #[test]
    fn the_lists_the_user_names_name_natives_and_programs() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("other.txt");
        let natives = dir.path().join("natives.txt");
        let names = dir.path().join("names.tsv");
        std::fs::write(
            &other,
            "# build: another build\n0x00000000000000C1 MISC::NOT_CALLED\n",
        )
        .unwrap();
        std::fs::write(
            &natives,
            "# build: a test's\n0x00000000000000BB DEMO::USE_DEMO_THING\n",
        )
        .unwrap();
        std::fs::write(
            &names,
            "hash\tname\n0x00000001\tdemo_program\n0x0A0B0C0D\tshared\n",
        )
        .unwrap();

        let supplied = supplied(vec![other, natives.clone()], &[names]).expect("the lists read");
        let (text, stats) = source(LISTING, &supplied).expect("the listing decompiles");
        let head = "\
// program 0x00000001 (demo_program)
// imports: 0x0A0B0C0D (shared)
// natives: 2 declared: 1 named by `a test's`, 1 by the listing itself, 0 shown by hash
// names: types and programs from `names.tsv`
";
        assert!(text.contains(head), "{text}");
        assert!(text.contains("DEMO.USE_DEMO_THING("), "{text}");
        assert_eq!(stats.natives_named, 2);

        // A natives list is named again so that no list of this machine's is looked for.
        let missing = dir.path().join("missing.tsv");
        let unread = super::supplied(vec![natives], &[missing]);
        assert!(unread.is_err_and(|e| e.starts_with("read ")));
    }

    #[test]
    fn a_census_reports_a_listing_that_fails_and_goes_on() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a_good.ysc.cs"), LISTING).unwrap();
        std::fs::write(dir.path().join("b_bad.ysc.cs"), "not a listing\n").unwrap();
        std::fs::write(dir.path().join("c_good.ysc.cs"), LISTING).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not counted\n").unwrap();

        let kept = tempfile::tempdir().unwrap();
        let sources = kept.path().join("sources");
        let (lines, failed) =
            census_of(dir.path(), &Supplied::default(), Some(&sources)).expect("the folder reads");
        assert_eq!(failed, 1);
        // Each listing that decompiled left its source, and the one that did not left none.
        let written = std::fs::read_to_string(sources.join("a_good.cs")).unwrap();
        assert!(written.contains("\nvoid main()\n{\n"), "{written}");
        assert!(sources.join("c_good.cs").is_file());
        assert!(!sources.join("b_bad.cs").exists());
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(
            lines[0].starts_with("a_good.ysc.cs: 1 function(s), 0 with no body; "),
            "{lines:?}"
        );
        assert!(
            lines[1].starts_with("b_bad.ysc.cs: failed: line 1: "),
            "{lines:?}"
        );
        assert_eq!(lines[2].replace("c_good", "a_good"), lines[0]);
        assert!(
            lines[3].starts_with("total: 3 listing(s), 1 failed: 2 function(s), "),
            "{lines:?}"
        );

        let empty = tempfile::tempdir().unwrap();
        let none = census_of(empty.path(), &Supplied::default(), None);
        assert!(none.is_err_and(|e| e.ends_with("holds no `*.ysc.cs` listing")));
    }

    #[test]
    fn the_project_of_a_folder_of_listings_is_written_as_the_project_flag_reads_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a_good.ysc.cs"), LISTING).unwrap();
        let to = dir.path().join("project.json");
        let line = indexed(dir.path(), &to).expect("the folder indexes");
        assert!(line.starts_with("1 listing(s): a project of "), "{line}");
        let project = index::load(&std::fs::read(&to).unwrap()).expect("the project loads");
        assert!(project.programs.contains_key(&1));

        // One listing that does not read ends it, named, and no project is written.
        std::fs::write(dir.path().join("b_bad.ysc.cs"), "not a listing\n").unwrap();
        let other = dir.path().join("other.json");
        let refused = indexed(dir.path(), &other).expect_err("a listing is refused");
        assert!(refused.contains("b_bad.ysc.cs"), "{refused}");
        assert!(!other.exists());

        let empty = tempfile::tempdir().unwrap();
        let none = indexed(empty.path(), &to);
        assert!(none.is_err_and(|e| e.ends_with("holds no `*.ysc.cs` listing")));
    }

    #[test]
    fn a_file_that_is_no_text_is_told_from_a_listing() {
        let dir = tempfile::tempdir().unwrap();
        let compiled = dir.path().join("compiled.ysc");
        std::fs::write(&compiled, [0xFFu8, 0xFE, 0x00, 0x80]).unwrap();
        let refused = read(&compiled).expect_err("bytes that are no text");
        assert!(
            refused.contains("`script decompile` reads a compiled GTA VI script"),
            "{refused}"
        );
    }
}
