//! The merge gate: clippy, then the default test tier, over the packages a
//! change touches. Each test binary runs once, its tests as threads.
//!
//! ⚠ Not nextest, which starts a process for every test: the app's default
//! tier is 1,318 tests, 3.5 s in one process and 14 s as 1,318 of them. The
//! price is that a failing test on Windows ends its binary's run, because
//! Cranelift cannot unwind, and that two tests can share state. So a binary
//! that fails is run again by nextest, each test apart, and that verdict stands.

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::affected::{self, Scope, Workspace};
use crate::git;
use crate::slow_tier::SlowTier;
use crate::tiers;

/// A binary of the default tier still running after this is ended and named.
const BINARY_DEADLINE: Duration = Duration::from_secs(120);

/// What Windows allows a command line; a binary's `--skip` arguments must fit it.
const COMMAND_LINE_LIMIT: usize = 32_000;

/// The modules that run as a process of their own beside the rest of their
/// binary, and whether their tests then run one at a time.
const APART: &[(&str, &str, bool)] = &[
    // The app's self-check test takes ten seconds and the app's other 1,500
    // take four, so apart they overlap, and a second run of the others does not
    // pay for the self-check again.
    (
        "openrage::bin/openrage",
        "self_check::judged_without_an_install::",
        false,
    ),
    // ⚠ Its tests count the files of one staging folder, so two of them at once
    // miscount: it failed both runs of the binary in one gate in six. It leaves
    // this list when each of its tests has a folder of its own.
    (
        "openrage::bin/openrage",
        "state::a_drag_copies_and_a_change_of_nothing_does_nothing::",
        true,
    ),
    // ⚠ Its maps are the samples of `or-maps`, which state few of the members
    // a class has, and a map that gains or loses a record is laid out from the
    // schema snapshot once anything in the process has installed one: beside
    // the tests that open metadata, two of its tests failed every run of the
    // binary on a machine that has a snapshot. It leaves this list when such a
    // map is laid out from what its file states, a snapshot installed or not.
    (
        "openrage::bin/openrage",
        "state::the_map_screen_holds_a_session::",
        false,
    ),
];

/// A test binary cargo built, as nextest names it.
#[derive(Clone)]
struct Binary {
    id: String,
    package: String,
    package_dir: PathBuf,
    executable: PathBuf,
    /// The one module of it this run takes, and whether its tests run one at a time.
    only: Option<&'static str>,
    one_at_a_time: bool,
    /// The modules this run leaves to runs of their own.
    without: Vec<&'static str>,
}

struct Ran {
    id: String,
    package: String,
    only: Option<&'static str>,
    seconds: f64,
    /// The tests libtest counted as passed; `None` when the run did not end in a passing summary.
    passed: Option<usize>,
    /// What a first run said when it failed and a second run of the same binary passed.
    failed_once: Option<String>,
    log: PathBuf,
}

pub fn run(args: &[String]) -> Result<ExitCode> {
    let (ours, filters) = match args.iter().position(|a| a == "--") {
        Some(at) => (&args[..at], &args[at + 1..]),
        None => (args, &[][..]),
    };
    let since = match ours.iter().position(|a| a == "--since") {
        Some(at) => ours.get(at + 1).context("--since takes a ref")?.as_str(),
        None => "origin/main",
    };
    if let Some(unknown) = ours.iter().find(|a| {
        !matches!(a.as_str(), "--workspace" | "--tests-only" | "--since") && a.as_str() != since
    }) {
        bail!("gate takes `--workspace`, `--tests-only`, `--since REF` and `-- <parts of test names>`, not `{unknown}`");
    }
    let root = git::here()?;
    let scope = if ours.iter().any(|a| a == "--workspace") {
        Scope::Workspace
    } else {
        affected::scope(&root, since)?
    };
    let selection: Vec<String> = match &scope {
        Scope::Nothing => {
            println!("nothing differs from {since}, so there is nothing to check");
            return Ok(ExitCode::SUCCESS);
        }
        Scope::Workspace => vec!["--workspace".to_string()],
        Scope::Packages(packages) => {
            let names: Vec<&str> = packages.iter().map(String::as_str).collect();
            println!("checking what the change touches: {}", names.join(", "));
            packages
                .iter()
                .flat_map(|p| ["-p".to_string(), p.clone()])
                .collect()
        }
    };

    let started = Instant::now();
    let target = tiers::target_directory()?;
    let logs = target.join("xtask").join("gate");
    std::fs::create_dir_all(&logs)?;
    // ⚠ Clippy runs beside the build and the tests, in a build directory of its
    // own. Cargo locks a build directory, so in one it would wait for the build
    // and the gate would be their sum: 39 s and 31 s after an edit to the app.
    let clippy_log = logs.join("clippy.log");
    let mut clippy = if ours.iter().any(|a| a == "--tests-only") {
        None
    } else {
        let out = File::create(&clippy_log)?;
        Some(
            Command::new("cargo")
                .arg("clippy")
                .args(&selection)
                .args(["--all-targets", "--target-dir"])
                .arg(target.join("lint"))
                .args(["--", "-D", "warnings"])
                .stdin(Stdio::null())
                .stdout(out.try_clone()?)
                .stderr(out)
                .spawn()
                .context("could not run cargo")?,
        )
    };

    // ⚠ Every test binary of the selection is built, the ones wholly in the slow
    // tier too. Naming targets (`--lib --bins --test NAME`) would spare two links
    // in five, and it changes which crates are built for the host alone:
    // `or-io-macros`' own tests then fail to link (`undefined symbol:
    // core::unicode::unicode_data::white_space::WHITESPACE_MAP`).
    let Some((binaries, ledger)) = build(&root, &selection)? else {
        if let Some(clippy) = &mut clippy {
            clippy.kill()?;
            clippy.wait()?;
        }
        return Ok(ExitCode::FAILURE);
    };
    let built = started.elapsed();

    let tier = SlowTier::read(&root)?;
    let (mut in_tier, left_out): (Vec<Binary>, Vec<Binary>) =
        binaries.into_iter().partition(|b| !tier.whole(&b.id));
    for (binary, module, one_at_a_time) in APART {
        if let Some(at) = in_tier
            .iter()
            .position(|b| b.id == *binary && b.only.is_none())
        {
            let mut alone = in_tier[at].clone();
            alone.only = Some(module);
            alone.one_at_a_time = *one_at_a_time;
            alone.without.clear();
            in_tier[at].without.push(module);
            in_tier.insert(0, alone);
        }
    }
    let ran = run_all(in_tier, &tier, filters, &logs);
    let tested = started.elapsed();

    let linted = match &mut clippy {
        Some(clippy) => clippy.wait()?.success(),
        None => true,
    };
    if !linted {
        println!("\nclippy:");
        print!("{}", std::fs::read_to_string(&clippy_log)?);
    }

    for (run, said) in ran
        .iter()
        .filter_map(|r| Some((r, r.failed_once.as_ref()?)))
    {
        println!(
            "\n{} failed once and passed when run again, so tests of it share state:\n{said}",
            run.id
        );
    }
    let mut failed = 0;
    for run in ran.iter().filter(|r| r.passed.is_none()) {
        println!(
            "\n{} did not pass as one process in two runs ({:.1} s, {}):",
            run.id,
            run.seconds,
            run.log.display()
        );
        print!("{}", what_failed(&run.log));
        println!("its tests again, each apart:");
        let apart = Command::new("cargo")
            .args(["nextest", "run", "--no-tests", "pass", "-p", &run.package])
            .args(["-E", &format!("binary_id(={})", run.id)])
            .args(run.only)
            .args(filters)
            .status()
            .context("could not run cargo")?;
        if apart.success() {
            println!(
                "{}: every test passes apart, so what failed above is tests sharing state, not this change",
                run.id
            );
        } else {
            failed += 1;
        }
    }
    if matches!(scope, Scope::Workspace) {
        // ⚠ Drained whatever the tests returned, or the next run reports these skips as its own.
        match ledger {
            Some(ledger) => drop(Command::new(ledger).status()),
            None => drop(tiers::skip_ledger(&[])),
        }
    }
    let passed: usize = ran.iter().filter_map(|r| r.passed).sum();
    println!(
        "gate: {passed} tests passed in {} binaries, {failed} failed; {} binaries are in the slow tier, which `just test` runs",
        ran.iter().filter(|r| r.passed.is_some()).count(),
        left_out.len()
    );
    println!(
        "gate: build {:.0} s, tests {:.0} s, {:.0} s in all{}",
        built.as_secs_f64(),
        (tested - built).as_secs_f64(),
        started.elapsed().as_secs_f64(),
        match (&clippy, linted) {
            (None, _) => "",
            (Some(_), true) => " with clippy beside them",
            (Some(_), false) => "; clippy failed",
        }
    );
    Ok(if failed == 0 && linted {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Build the selection's tests and name each test binary, and the skip ledger's program when it was built; `None` when the build failed.
fn build(root: &Path, selection: &[String]) -> Result<Option<(Vec<Binary>, Option<PathBuf>)>> {
    let out = Command::new("cargo")
        .args([
            "test",
            "--no-run",
            "--message-format",
            "json-render-diagnostics",
        ])
        .args(selection)
        .stderr(Stdio::inherit())
        .output()
        .context("could not run cargo")?;
    if !out.status.success() {
        return Ok(None);
    }
    let workspace = Workspace::read(root)?;
    let root_text = root.to_string_lossy().replace('\\', "/");
    let mut binaries = Vec::new();
    let mut ledger = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(executable) = message["executable"].as_str() else {
            continue;
        };
        let name = message["target"]["name"].as_str().unwrap_or_default();
        let kind = message["target"]["kind"][0].as_str().unwrap_or_default();
        if message["profile"]["test"] != true {
            if kind == "bin" && name == "skip-ledger" {
                ledger = Some(PathBuf::from(executable));
            }
            continue;
        }
        let manifest = message["manifest_path"]
            .as_str()
            .unwrap_or_default()
            .replace('\\', "/");
        let relative = manifest
            .strip_prefix(root_text.as_str())
            .unwrap_or(&manifest)
            .trim_start_matches('/');
        let package = workspace.package_of(relative);
        binaries.push(Binary {
            id: match kind {
                "test" => format!("{package}::{name}"),
                "bin" | "bench" | "example" => format!("{package}::{kind}/{name}"),
                _ => package.to_string(),
            },
            package: package.to_string(),
            package_dir: Path::new(&manifest)
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
            executable: PathBuf::from(executable),
            only: None,
            one_at_a_time: false,
            without: Vec::new(),
        });
    }
    Ok(Some((binaries, ledger)))
}

/// Run every binary, a quarter of the cores' worth at once, each with its slow modules skipped.
fn run_all(binaries: Vec<Binary>, tier: &SlowTier, filters: &[String], logs: &Path) -> Vec<Ran> {
    let workers = std::thread::available_parallelism().map_or(2, |n| (n.get() / 4).max(1));
    let queue = Mutex::new(VecDeque::from(binaries));
    let ran = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let Some(binary) = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front()
                else {
                    return;
                };
                let started = Instant::now();
                let name = format!("{}{}", binary.id, binary.only.unwrap_or_default());
                let name = name.replace([':', '/'], "_");
                let attempt = |log: &Path| {
                    run_one(&binary, tier, filters, log).unwrap_or_else(|error| {
                        let _ = std::fs::write(log, format!("could not run it: {error:#}\n"));
                        None
                    })
                };
                // ⚠ One more run before the verdict: two tests that share state fail one run in
                // five or so, and a second run costs seconds where nextest costs twenty.
                let mut log = logs.join(format!("{name}.log"));
                let mut passed = attempt(&log);
                let mut failed_once = None;
                if passed.is_none() {
                    let said = what_failed(&log);
                    log = logs.join(format!("{name}.again.log"));
                    passed = attempt(&log);
                    failed_once = passed.is_some().then_some(said);
                }
                ran.lock().unwrap_or_else(|e| e.into_inner()).push(Ran {
                    id: binary.id.clone(),
                    package: binary.package.clone(),
                    only: binary.only,
                    seconds: started.elapsed().as_secs_f64(),
                    passed,
                    failed_once,
                    log,
                });
            });
        }
    });
    let mut ran = ran.into_inner().unwrap_or_else(|e| e.into_inner());
    ran.sort_by(|a, b| a.id.cmp(&b.id));
    ran
}

/// Run one binary into `log`; the tests it passed, or `None` when it failed or outlived its deadline.
fn run_one(
    binary: &Binary,
    tier: &SlowTier,
    filters: &[String],
    log: &Path,
) -> Result<Option<usize>> {
    let skips = tier.modules(&binary.id);
    // ponytail: a command line is the only way libtest takes a skip; past this limit, name fewer and wider modules in the slow tier.
    if skips.iter().map(|m| m.len() + 8).sum::<usize>() > COMMAND_LINE_LIMIT {
        bail!(
            "{} has more slow modules than a command line holds",
            binary.id
        );
    }
    let out = File::create(log)?;
    let started = Instant::now();
    let mut child = Command::new(&binary.executable)
        // ⚠ Uncaptured, into the log: a panic then says what it is before the process ends, and
        // the app's tests, which failed 2 runs of 14 under libtest's capture, passed 24 of 24.
        .arg("--nocapture")
        .args(binary.only)
        .args(binary.one_at_a_time.then_some("--test-threads=1"))
        .args(filters)
        .args(skips.iter().flat_map(|m| ["--skip", m.as_str()]))
        .args(binary.without.iter().flat_map(|m| ["--skip", m]))
        // ⚠ What `cargo test` gives a test: its package's directory, as the working directory and by name.
        .current_dir(&binary.package_dir)
        .env("CARGO_MANIFEST_DIR", &binary.package_dir)
        .env("CARGO_PKG_NAME", &binary.package)
        .stdin(Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out.try_clone()?)
        .spawn()?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > BINARY_DEADLINE {
            child.kill()?;
            child.wait()?;
            writeln!(
                &out,
                "\nended by the gate: still running after {} s",
                BINARY_DEADLINE.as_secs()
            )?;
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Ok(status
        .success()
        .then(|| passed_count(&std::fs::read_to_string(log).unwrap_or_default())))
}

/// The tests libtest's summary lines count as passed.
fn passed_count(output: &str) -> usize {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("test result: ok. "))
        .filter_map(|rest| rest.split(' ').next()?.parse::<usize>().ok())
        .sum()
}

/// The lines of a failed binary's output that say what failed: a panic and what follows it, and libtest's verdicts.
fn what_failed(log: &Path) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::new();
    let mut keep = 0;
    for line in &lines {
        if line.contains("panicked at") || line.contains("ended by the gate") {
            keep = 12;
        } else if line.ends_with("... FAILED") || line.starts_with("test result:") {
            keep = keep.max(1);
        }
        if keep > 0 {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
            keep -= 1;
        }
    }
    if out.is_empty() {
        for line in lines.iter().rev().take(15).rev() {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passing_run_is_counted_from_its_summary_lines() {
        let output = "test a ... ok\n\ntest result: ok. 1318 passed; 0 failed; 11 ignored; 0 measured; 1331 filtered out; finished in 3.53s\n";
        assert_eq!(passed_count(output), 1318);
        assert_eq!(
            passed_count("thread 'a' panicked at src/x.rs:1:1:\nboom\n"),
            0
        );
    }

    #[test]
    fn a_failure_is_reported_by_its_panic_and_not_by_the_tests_that_passed() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let log = dir.path().join("x.log");
        std::fs::write(
            &log,
            "test a ... ok\ntest b ... ok\nthread 'c' panicked at src/x.rs:9:5:\nthe count was 3\n",
        )
        .expect("a log");
        let said = what_failed(&log);
        assert!(
            said.contains("panicked at src/x.rs:9:5") && said.contains("the count was 3"),
            "{said}"
        );
        assert!(!said.contains("test a ... ok"), "{said}");
    }
}
