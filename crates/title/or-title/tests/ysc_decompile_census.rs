//! Whether the decompiler turns every script an install ships into structured source.
//!
//! Point `OPENRAGE_GAME_ROOT` at one or more install roots, `;`-separated.
//! Nothing here runs without one, and the skip is recorded rather than silent.
//!
//! ⚠ **Every check is against the script, never against a reference
//! decompiler.** A script emits each of its instructions exactly once, its
//! source balances its braces, and the stack model never pops what was not
//! pushed; none of that needs another tool to agree with. The share of
//! functions that needed a `goto` is measured and reported, not asserted to
//! a figure.
//!
//! ⚠ **One crawl feeds every test**, as in `ysc_disasm_census`: one script's
//! bytes live at a time.
//!
//! Natives are named from the database `openrage-cli script download-natives`
//! left under `OPENRAGE_CONFIG_DIR`, else `%APPDATA%\openrage`, when one is
//! there; this suite never downloads one.

use std::collections::{BTreeMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::{Arc, OnceLock};

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_io::gta5::script::decompile::{self, DecompileStats};
use or_io::gta5::script::natives::NativeDbSet;
use or_io::gta5::script::ScrProgram;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_title::install::crawl::{self, ControlFlow};

const KEPT: usize = 8;

#[derive(Default)]
struct Census {
    roots: Vec<String>,
    scripts: usize,
    refused: Vec<String>,
    does_not_decode: Vec<String>,
    panicked: Vec<String>,
    decompiled: usize,
    stats: DecompileStats,
    /// Scripts whose instructions were not each emitted exactly once.
    unconsumed: Vec<String>,
    unbalanced: Vec<String>,
    /// `(mismatches, script)` for every script the stack model failed in.
    mismatched: Vec<(usize, String)>,
    scripts_mismatched: usize,
    scripts_with_goto: usize,
    /// RSC7 version → (scripts, functions, functions with a goto).
    by_version: BTreeMap<u32, (usize, usize, usize)>,
    source_bytes: u64,
    names: Option<NativeDbSet>,
    /// Per root: the distinct natives its scripts call, and those the list each script took names.
    natives: Vec<(HashSet<u64>, HashSet<u64>)>,
}

/// The natives database a download left on this machine, if any.
fn downloaded_natives() -> Option<NativeDbSet> {
    let root = std::env::var_os("OPENRAGE_CONFIG_DIR")
        .filter(|d| !d.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(|d| Path::new(&d).join("openrage")))?;
    NativeDbSet::from_download(&root.join("natives")).expect("a downloaded natives database reads")
}

fn keep(list: &mut Vec<String>, line: String) {
    if list.len() < KEPT {
        list.push(line);
    }
}

/// Whether every `{` closes, ignoring braces inside string literals and comments.
fn braces_balance(text: &str) -> Result<(), String> {
    let mut depth = 0i64;
    for (n, line) in text.lines().enumerate() {
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' => {
                    while let Some(s) = chars.next() {
                        match s {
                            '\\' => {
                                chars.next();
                            }
                            '"' => break,
                            _ => {}
                        }
                    }
                }
                '/' if chars.peek() == Some(&'/') => break,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth < 0 {
                        return Err(format!("line {}: a `}}` closes nothing", n + 1));
                    }
                }
                _ => {}
            }
        }
    }
    match depth {
        0 => Ok(()),
        d => Err(format!("{d} brace(s) left open")),
    }
}

impl Census {
    fn fold(&mut self, program: &ScrProgram) {
        let db = self
            .names
            .as_ref()
            .and_then(|n| n.best_for(program))
            .map(|(db, _)| db);
        if let Some((called, named)) = self.natives.last_mut() {
            for hash in program.native_hashes() {
                called.insert(hash);
                if db.is_some_and(|db| db.name(hash).is_some()) {
                    named.insert(hash);
                }
            }
        }
        let result = catch_unwind(AssertUnwindSafe(|| decompile::decompile(program, db)));
        let out = match result {
            Err(_) => {
                self.panicked.push(program.name.clone());
                return;
            }
            Ok(Err(e)) => {
                keep(&mut self.does_not_decode, format!("{}: {e}", program.name));
                return;
            }
            Ok(Ok(out)) => out,
        };
        self.decompiled += 1;
        self.source_bytes += out.text.len() as u64;
        let s = out.stats;
        self.stats.add(&s);
        let version = self.by_version.entry(program.resource_version).or_default();
        version.0 += 1;
        version.1 += s.functions;
        version.2 += s.functions_with_goto;
        if s.instructions_consumed != s.instructions || s.instructions_repeated != 0 {
            keep(
                &mut self.unconsumed,
                format!(
                    "{}: {} of {} instructions emitted, {} again",
                    program.name, s.instructions_consumed, s.instructions, s.instructions_repeated
                ),
            );
        }
        if let Err(why) = braces_balance(&out.text) {
            keep(&mut self.unbalanced, format!("{}: {why}", program.name));
        }
        if s.stack_mismatches > 0 {
            self.scripts_mismatched += 1;
            self.mismatched.push((
                s.stack_mismatches,
                format!("{} (v{})", program.name, program.resource_version),
            ));
        }
        if s.functions_with_goto > 0 {
            self.scripts_with_goto += 1;
        }
    }

    fn report(&self) -> String {
        let s = &self.stats;
        let share = |n: usize, d: usize| {
            if d == 0 {
                0.0
            } else {
                100.0 * n as f64 / d as f64
            }
        };
        let mut out = format!(
            "ysc decompile census over {}\n  scripts {} read, {} decompiled, {} refused, {} do not decode, {} panicked\n  \
             functions {}; {} needed a goto ({:.3}%), {} goto(s) in all; scripts with a goto {}\n  \
             instructions {} emitted of {}, {} again\n  stack mismatches {} across {} script(s); {} read(s) below an operand stack\n  \
             source {} bytes",
            self.roots.join("; "),
            self.scripts,
            self.decompiled,
            self.refused.len(),
            self.does_not_decode.len(),
            self.panicked.len(),
            s.functions,
            s.functions_with_goto,
            share(s.functions_with_goto, s.functions),
            s.gotos,
            self.scripts_with_goto,
            s.instructions_consumed,
            s.instructions,
            s.instructions_repeated,
            s.stack_mismatches,
            self.scripts_mismatched,
            s.reads_below_stack,
            self.source_bytes,
        );
        match &self.names {
            None => out.push_str("\n  natives: no downloaded database, so every native is a hash"),
            Some(names) => {
                let labels: Vec<&str> = names.dbs().iter().map(|db| db.label()).collect();
                out.push_str(&format!(
                    "\n  natives from {}\n  native calls {} named of {} ({:.2}%)",
                    labels.join("; "),
                    s.natives_named,
                    s.native_calls,
                    share(s.natives_named, s.native_calls)
                ));
                for (root, (called, named)) in self.roots.iter().zip(&self.natives) {
                    out.push_str(&format!(
                        "\n  {root}: {} of {} distinct natives named ({:.2}%)",
                        named.len(),
                        called.len(),
                        share(named.len(), called.len())
                    ));
                }
            }
        }
        for (version, (scripts, functions, gotos)) in &self.by_version {
            out.push_str(&format!(
                "\n  version {version}: {scripts} script(s), {functions} function(s), {gotos} with a goto ({:.3}%)",
                share(*gotos, *functions)
            ));
        }
        for (what, list) in [
            ("refused", &self.refused),
            ("does not decode", &self.does_not_decode),
            ("panicked", &self.panicked),
            ("not consumed once", &self.unconsumed),
            ("unbalanced", &self.unbalanced),
        ] {
            for line in list.iter().take(KEPT) {
                out.push_str(&format!("\n  {what}: {line}"));
            }
        }
        let mut worst = self.mismatched.clone();
        worst.sort_unstable_by(|a, b| b.cmp(a));
        for (count, script) in worst.iter().take(KEPT) {
            out.push_str(&format!("\n  stack mismatch: {script}: {count}"));
        }
        out
    }
}

fn crypto_for(root: &Path) -> Option<Arc<dyn RpfCryptoProvider>> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    Some(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))))
}

fn build() -> Result<Census, String> {
    let roots = game_roots();
    if roots.is_empty() {
        return Err(format!(
            "no install root; set {GAME_ROOT_ENV} to a ;-separated list of install directories"
        ));
    }
    let mut census = Census {
        names: downloaded_natives(),
        ..Census::default()
    };
    for root in &roots {
        let Some(provider) = crypto_for(root) else {
            continue;
        };
        census.roots.push(root.display().to_string());
        census.natives.push(Default::default());
        let mut warnings = Vec::new();
        let _ = crawl::crawl(root, provider, &mut warnings, &mut |found| {
            if !found.leaf.to_ascii_lowercase().ends_with(".ysc") {
                return ControlFlow::Continue(());
            }
            census.scripts += 1;
            let program = found
                .read()
                .map_err(|e| e.to_string())
                .and_then(|bytes| ScrProgram::from_bytes(&bytes).map_err(|e| e.to_string()));
            match program {
                Ok(program) => census.fold(&program),
                Err(e) => census.refused.push(format!("{}: {e}", found.leaf)),
            }
            ControlFlow::Continue(())
        });
    }
    if census.roots.is_empty() {
        return Err("no install root yielded an executable to take keys from".into());
    }
    eprintln!("{}", census.report());
    Ok(census)
}

fn census(what: &str) -> Option<&'static Census> {
    static CENSUS: OnceLock<Result<Census, String>> = OnceLock::new();
    match CENSUS.get_or_init(build) {
        Ok(c) => Some(c),
        Err(why) => {
            skip_install(what, why);
            None
        }
    }
}

/// Every script decompiles without a panic, and a crawl that found none fails.
#[test]
fn every_script_decompiles_without_a_panic() {
    let Some(c) = census("every_script_decompiles_without_a_panic") else {
        return;
    };
    assert!(c.scripts > 0, "the crawl found no script\n{}", c.report());
    assert!(c.panicked.is_empty(), "{}", c.report());
    assert!(
        c.refused.is_empty() && c.does_not_decode.is_empty(),
        "{}",
        c.report()
    );
    assert_eq!(c.decompiled, c.scripts, "{}", c.report());
}

/// Every instruction of every script is emitted exactly once: none dropped, none repeated.
#[test]
fn every_instruction_is_emitted_exactly_once() {
    let Some(c) = census("every_instruction_is_emitted_exactly_once") else {
        return;
    };
    assert!(c.stats.instructions > 0, "{}", c.report());
    assert!(c.unconsumed.is_empty(), "{}", c.report());
    assert_eq!(c.stats.instructions_consumed, c.stats.instructions);
    assert_eq!(c.stats.instructions_repeated, 0);
}

#[test]
fn every_source_balances_its_braces() {
    let Some(c) = census("every_source_balances_its_braces") else {
        return;
    };
    assert!(c.decompiled > 0, "{}", c.report());
    assert!(c.unbalanced.is_empty(), "{}", c.report());
}

/// The stack model never pops what no instruction pushed, never meets a
/// count it cannot read, and hands every block one depth.
#[test]
fn the_stack_model_holds_in_every_script() {
    let Some(c) = census("the_stack_model_holds_in_every_script") else {
        return;
    };
    assert!(c.decompiled > 0, "{}", c.report());
    assert_eq!(c.stats.stack_mismatches, 0, "{}", c.report());
}

/// Measured, not asserted to a figure: the census prints it, and R63 records it.
#[test]
fn what_share_of_functions_needs_a_goto() {
    let Some(c) = census("what_share_of_functions_needs_a_goto") else {
        return;
    };
    assert!(c.stats.functions > 0, "{}", c.report());
    assert!(c.stats.functions_with_goto <= c.stats.functions);
}

/// Measured: with a downloaded database, nearly every native a retail script calls has a name.
#[test]
fn a_downloaded_natives_database_names_what_retail_scripts_call() {
    const WHAT: &str = "a_downloaded_natives_database_names_what_retail_scripts_call";
    let Some(c) = census(WHAT) else {
        return;
    };
    if c.names.is_none() {
        skip_install(
            WHAT,
            "no natives database downloaded; run `openrage-cli script download-natives`",
        );
        return;
    }
    for (root, (called, named)) in c.roots.iter().zip(&c.natives) {
        assert!(
            !called.is_empty(),
            "{root} called no native\n{}",
            c.report()
        );
        assert!(
            named.len() * 100 >= called.len() * 95,
            "{root}: {} of {} named\n{}",
            named.len(),
            called.len(),
            c.report()
        );
    }
}

#[test]
fn the_brace_check_sees_a_brace_left_open_and_ignores_one_in_a_string() {
    assert!(braces_balance("f() {\n    x = \"}\";\n}\n").is_ok());
    assert!(braces_balance("f() {\n    // }\n").is_err());
    assert!(braces_balance("}\n{").is_err());
}
