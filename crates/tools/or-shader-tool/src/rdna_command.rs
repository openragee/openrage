//! `rdna-devices`: GTA VI's native programs on every adapter of this machine, and the merge of several machines' reports.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_shader_tool::rdna_report::{merge, table, Report};

pub fn print_usage(program: &str) {
    eprintln!(
        "  {program} rdna-devices <ARCHIVE.rpf> <OFFSET> [--out <REPORT.json>] [--cache-dir <DIR>]"
    );
    eprintln!("               [--adapter <TEXT>] [--jobs <N>] [--chunk <N>] [--timeout <SECONDS>] [--units <FROM>..<TO>]");
    eprintln!("               [--stack-mib <N>] [--technique <NAME>[,<NAME>...]]");
    eprintln!("                                translate every native program of the GTA VI effect container at OFFSET");
    eprintln!("                                (or of a container saved as a file, with no OFFSET) and build a pipeline");
    eprintln!("                                of each on every adapter of this machine, of every backend, the software");
    eprintln!("                                ones too: each program alone, then each vertex and pixel pair a pass draws");
    eprintln!("                                with. Counts per adapter what was created, what was refused and why, and");
    eprintln!("                                what needs a feature the device lacks. The work runs in child processes,");
    eprintln!("                                so a crash of a driver or of the translator is counted and passed over.");
    eprintln!("                                --out writes the report: the adapter, its driver, features and limits,");
    eprintln!("                                and the units behind each count. --cache-dir saves each adapter's");
    eprintln!("                                pipeline cache where its backend has one, and times what it saves.");
    eprintln!("                                --technique checks only the pairs the passes of those techniques draw");
    eprintln!("                                with, such as the ones that shade a model.");
    eprintln!("  {program} rdna-devices --merge <REPORT.json>... [--out <MERGED.json>]");
    eprintln!("                                several machines' reports as one table; a merged file merges again");
    eprintln!("  {program} rdna-devices --help");
    eprintln!("                                how to run the check on another machine and send its report back");
}

/// How a person with another machine adds its adapters to the report.
pub const ANOTHER_MACHINE: &str = "\
Adding a machine to the report of GTA VI's native programs

You need this repository built with its pinned toolchain, the game's shaders.rpf, and from
whoever asked you: the offset of each effect container in it, and the names of the techniques
that shade a model. Nothing opens a window; each run prints a table and writes one JSON file.

1. Build:
     cargo build -p or-shader-tool --features device-check

2. Run, from the folder the binary is in (target/debug). The small container takes a few
   minutes whole, the large one hours:
     or-shader-tool rdna-devices <shaders.rpf> <SMALL OFFSET> --out small.json
     or-shader-tool rdna-devices <shaders.rpf> <LARGE OFFSET> --technique <NAME>,<NAME> --out shading.json
   The second is the passes that shade a model. For more of the large container, a range of
   units a run, each to a file of its own:
     or-shader-tool rdna-devices <shaders.rpf> <LARGE OFFSET> --units 0..3000 --out large-00000.json
   A worker process holds a device on every adapter and may take 3 GB: --jobs sets how many
   run at once (half the cores unless said). --adapter <TEXT> keeps the adapters whose name or
   backend holds the text, such as vulkan, dx12, metal or part of the card's name.

3. On Windows, Direct3D 12 compiles with FXC unless told otherwise. To add the DXC rows, put
   dxcompiler.dll on PATH and run again with the variable set, to other files:
     set WGPU_DX12_COMPILER=dxc
     or-shader-tool rdna-devices <shaders.rpf> <SMALL OFFSET> --adapter dx12 --out small-dxc.json

4. Send the JSON files. A file holds each adapter's name, driver, features and limits, the
   numbers of the units behind every count and the devices' own refusals. It holds no game
   data and no path of your machine.

5. Whoever collects them merges any number, merged files too:
     or-shader-tool rdna-devices --merge mine.json yours-small.json yours-shading.json --out all.json
   Two runs of the same container on the same adapter, backend, compiler and driver are added
   where they checked different units; where they overlap the later file stands.

Reading the table: created is a pipeline the device built; skipped is a unit that needs a
device feature the adapter lacks, by the feature's name; rejected is a refusal in the
device's words; crashed and timed out are a driver that ended or held a worker twice. The
lines marked noted say what the units ask of any device, and which pairs read a parameter
nothing writes.
";

fn fail(why: impl std::fmt::Display) -> ExitCode {
    eprintln!("Error: {why}");
    ExitCode::from(1)
}

fn read_report(path: &str) -> Result<Report, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("{path} is not a report: {e}"))
}

fn write_report(path: &Path, report: &Report) -> Result<(), String> {
    let text = serde_json::to_string_pretty(report).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The value after `flag`, taken out of `args`.
fn take(args: &mut Vec<String>, flag: &str) -> Result<Option<String>, String> {
    let Some(at) = args.iter().position(|a| a == flag) else {
        return Ok(None);
    };
    if at + 1 >= args.len() {
        return Err(format!("{flag} needs a value"));
    }
    args.remove(at);
    Ok(Some(args.remove(at)))
}

pub fn run(args: &[String]) -> ExitCode {
    let mut args = args.to_vec();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{ANOTHER_MACHINE}");
        return ExitCode::SUCCESS;
    }
    let out = match take(&mut args, "--out") {
        Ok(out) => out.map(PathBuf::from),
        Err(why) => return fail(why),
    };
    if args.first().map(String::as_str) == Some("--merge") {
        let reports: Result<Vec<Report>, String> =
            args[1..].iter().map(|path| read_report(path)).collect();
        let merged = match reports.and_then(|reports| merge(reports).map_err(|e| e.to_string())) {
            Ok(merged) => merged,
            Err(why) => return fail(why),
        };
        print!("{}", table(&merged));
        return match out.map(|out| write_report(&out, &merged)) {
            Some(Err(why)) => fail(why),
            _ => ExitCode::SUCCESS,
        };
    }
    match check(args, out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => fail(why),
    }
}

#[cfg(not(feature = "device-check"))]
fn check(_args: Vec<String>, _out: Option<PathBuf>) -> Result<(), String> {
    Err("this build has no device check. Build with `--features device-check`.".into())
}

#[cfg(feature = "device-check")]
fn check(mut args: Vec<String>, out: Option<PathBuf>) -> Result<(), String> {
    use std::io::Write as _;
    use std::time::Duration;

    use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind};
    use or_shader_tool::rdna_devices::{self, Options, Programs, Resume};

    /// The most a container inflates to.
    const BODY_LIMIT: u64 = 1 << 30;
    let number = |word: &str| -> Result<usize, String> {
        match word.strip_prefix("0x") {
            Some(hex) => usize::from_str_radix(hex, 16),
            None => word.parse(),
        }
        .map_err(|e| format!("{word} is not a number: {e}"))
    };
    let adapter = take(&mut args, "--adapter")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;

    // The two forms a parent starts its children with; neither is for a person to type.
    match args.first().map(String::as_str) {
        Some("--worker") => {
            let [_, programs, from, to, block, adapter_from] = args.as_slice() else {
                return Err("a worker takes its programs, a range and where to resume".into());
            };
            let programs = Programs::open(Path::new(programs))?;
            let resume = Resume {
                block: number(block)?,
                adapter: number(adapter_from)?,
            };
            let range = number(from)?..number(to)?;
            return rdna_devices::on_worker_stack(|| {
                let mut stdout = std::io::stdout().lock();
                rdna_devices::worker(&programs, range, resume, adapter.as_deref(), &mut stdout)
            })?;
        }
        Some("--cache-worker") => {
            let [_, programs, index, list, file, mode] = args.as_slice() else {
                return Err(
                    "a cache worker takes its programs, an adapter, a list, a file and a mode"
                        .into(),
                );
            };
            let programs = Programs::open(Path::new(programs))?;
            let list = std::fs::read_to_string(list).map_err(|e| format!("{list}: {e}"))?;
            let list: Vec<(usize, usize)> =
                serde_json::from_str(&list).map_err(|e| e.to_string())?;
            let index = number(index)?;
            let result = rdna_devices::on_worker_stack(|| {
                rdna_devices::cache_worker(
                    &programs,
                    index,
                    adapter.as_deref(),
                    &list,
                    Path::new(file),
                    mode == "load",
                )
            })??;
            let text = serde_json::to_string(&result).map_err(|e| e.to_string())?;
            return writeln!(std::io::stdout(), "@@{text}").map_err(|e| e.to_string());
        }
        _ => {}
    }

    let mut options = Options {
        adapter,
        cache_dir: take(&mut args, "--cache-dir")?.map(PathBuf::from),
        ..Default::default()
    };
    if let Some(jobs) = take(&mut args, "--jobs")? {
        options.jobs = number(&jobs)?;
    }
    if let Some(chunk) = take(&mut args, "--chunk")? {
        options.chunk = number(&chunk)?;
    }
    if let Some(seconds) = take(&mut args, "--timeout")? {
        options.timeout = Duration::from_secs(number(&seconds)? as u64);
    }
    if let Some(mib) = take(&mut args, "--stack-mib")? {
        options.stack_mib = Some(number(&mib)?);
    }
    if let Some(names) = take(&mut args, "--technique")? {
        options.techniques = names.split(',').map(str::to_owned).collect();
    }
    if let Some(units) = take(&mut args, "--units")? {
        let (from, to) = units.split_once("..").ok_or("--units takes FROM..TO")?;
        options.units = Some(number(from)?..number(to)?);
    }
    let body = match args.as_slice() {
        [path] => std::fs::read(path).map_err(|e| format!("{path}: {e}"))?,
        [path, offset] => {
            let offset = number(offset)? as u64;
            let file = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
            let (find, bytes) =
                read_rpf9_body(&file, offset, BODY_LIMIT).map_err(|e| e.to_string())?;
            if !matches!(find.kind, Rpf9BodyKind::Sgd5 { .. }) {
                return Err(format!("the body at {offset:#X} is not an SGD5 effect container"));
            }
            bytes
        }
        _ => return Err("usage: rdna-devices <ARCHIVE.rpf> <OFFSET> [--out <REPORT.json>], or --merge <REPORT.json>...".into()),
    };
    let progress = |done: usize, of: usize| {
        eprint!("\r  {done} of {of} units");
        let _ = std::io::stderr().flush();
    };
    let report = rdna_devices::run(&exe, &body, &options, &progress)?;
    eprintln!();
    print!("{}", table(&report));
    match out {
        Some(out) => write_report(&out, &report),
        None => Ok(()),
    }
}
