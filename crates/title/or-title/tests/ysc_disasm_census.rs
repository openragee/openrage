//! Whether the disassembler's decode agrees with every script an install ships.
//!
//! Point `OPENRAGE_GAME_ROOT` at one or more install roots, `;`-separated.
//! Nothing here runs without one, and the skip is recorded rather than silent.
//!
//! ⚠ **Every check is against the file, never against an expectation.** A
//! script states its own code length, its own native table and its own page
//! size; a correct decode consumes exactly the one, names only slots of the
//! other and never crosses the third, and every jump, switch case and call it
//! finds lands on an instruction it also found. None of that needs a reference
//! disassembler to agree with.
//!
//! ⚠ **One crawl feeds every test.** A crawl of two installs decrypts 3,022
//! scripts and two gigabytes of bytecode, and one per test multiplies that by
//! the test count. It is a fold — one script's bytes live at a time.
//!
//! Set `OPENRAGE_NATIVES` to `;`-separated natives lists to measure what they
//! name. That measurement is reported and never asserted: the lists are the
//! user's, and OpenRage carries none.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::{Arc, OnceLock};

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_io::gta5::script::disasm::{self, CodeCheck, Operand};
use or_io::gta5::script::natives::{NativeDb, NativeDbSet};
use or_io::gta5::script::opcode::{self, InstructionSet, Shape};
use or_io::gta5::script::ScrProgram;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_title::install::crawl::{self, ControlFlow};

const NATIVES_ENV: &str = "OPENRAGE_NATIVES";

/// Operand widths the falsification gets wrong on purpose: one read a byte
/// too wide, one a byte too narrow.
const PERTURBATIONS: [(u8, Shape, Shape); 2] = [
    (opcode::IADD_U8, Shape::U8, Shape::U16),
    (opcode::PUSH_CONST_S16, Shape::S16, Shape::U8),
];

/// What decoding one script under one instruction set came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Outcome {
    Clean,
    Disagrees,
    Stops,
}

#[derive(Default)]
struct Install {
    root: String,
    scripts: usize,
    clean: usize,
    distinct_hashes: HashSet<u64>,
    named_hashes: HashSet<u64>,
}

#[derive(Default)]
struct Totals {
    code_bytes: u64,
    bytes_decoded: u64,
    instructions: u64,
    functions: u64,
    functions_named: u64,
    straddles: u64,
    jumps: u64,
    jumps_off: u64,
    jumps_to_end: u64,
    cases: u64,
    cases_off: u64,
    calls: u64,
    calls_off: u64,
    calls_through_padding: u64,
    calls_not_to_enter: u64,
    natives: u64,
    natives_out_of_range: u64,
    native_slots: u64,
    native_slots_unreferenced: u64,
    op_counts: Vec<u64>,
}

/// What a rival reading of the same bytes would have found.
#[derive(Default)]
struct Rivals {
    /// Native slot read little-endian, and inside the table.
    native_le_in_range: u64,
    /// Switch offset measured from the end of the whole switch.
    case_from_switch_end_lands: u64,
    /// Jump offset measured from the opcode rather than the end.
    jump_from_opcode_lands: u64,
}

/// What decoding each script under the other instruction set came to.
#[derive(Default)]
struct RivalSet {
    stops: usize,
    disagrees: usize,
    /// Clean by every width check, but reads `CATCH` or `THROW`, which no
    /// release script uses under its own set.
    reads_catch_or_throw: usize,
    /// Clean and unrefuted: the version is the only evidence against it.
    unrefuted: usize,
    /// Of those, how many find the same instruction boundaries as the own set.
    unrefuted_same_boundaries: usize,
    unrefuted_largest: u32,
    first_unrefuted: Vec<String>,
}

struct Falsified {
    op: u8,
    from: Shape,
    to: Shape,
    /// The table no longer holds `from` for `op`, so the perturbation is not the one described.
    table_disagrees: bool,
    scripts_with_op: usize,
    failed_loudly: usize,
    first_failures: Vec<String>,
    scripts_without_op: usize,
    without_op_still_clean: usize,
}

#[derive(Default)]
struct NativeMeasure {
    lists: Vec<(String, usize)>,
    scripts_named: usize,
    scripts_complete: usize,
    slots_named: u64,
    slots_distinct: u64,
    chosen: BTreeMap<String, usize>,
    signatures_by_list: BTreeMap<String, HashSet<u32>>,
    /// Globals signature → (scripts, natives named, natives, a script named completely).
    by_signature: BTreeMap<u32, (usize, usize, usize, Option<String>)>,
}

#[derive(Default)]
struct Census {
    installs: Vec<Install>,
    seen: usize,
    read: usize,
    refused: Vec<String>,
    decoded: usize,
    decode_failures: Vec<String>,
    unknown_opcodes: BTreeMap<u8, usize>,
    clean: usize,
    first_problems: Vec<String>,
    first_is_enter: usize,
    /// `(version, set, outcome)` → scripts.
    by_set: BTreeMap<(u32, &'static str, Outcome), usize>,
    /// Under the set the version does *not* name.
    rival: RivalSet,
    totals: Totals,
    rivals: Rivals,
    falsified: Vec<Falsified>,
    natives: Option<NativeMeasure>,
}

impl Census {
    fn fold(&mut self, program: &ScrProgram, lists: Option<&NativeDbSet>) {
        self.read += 1;
        self.fold_sets(program);
        let install = self.installs.last_mut().expect("an install is open");
        install.scripts += 1;
        let hashes = program.native_hashes();
        install.distinct_hashes.extend(hashes.iter().copied());

        match disasm::check(program) {
            Err(e) => {
                if let disasm::DecodeErrorKind::UnknownOpcode(op) = e.kind {
                    *self.unknown_opcodes.entry(op).or_default() += 1;
                }
                if self.decode_failures.len() < 12 {
                    let at = e.at as usize;
                    let context = &program.code[at..(at + 12).min(program.code.len())];
                    let set = program
                        .instruction_set()
                        .unwrap_or(InstructionSet::U24Locals);
                    let before = disasm::decode(&program.code[..at], set).unwrap_or_default();
                    let lead: Vec<String> = before
                        .iter()
                        .rev()
                        .take(4)
                        .rev()
                        .map(|i| {
                            let b = &program.code
                                [i.at as usize..(i.at + i.len).min(i.at + 10) as usize];
                            format!("{:06X} {} {:02X?}", i.at, i.mnemonic(), b)
                        })
                        .collect();
                    self.decode_failures.push(format!(
                        "{}: {e}; bytes {:02X?}\n      after {}",
                        program.name,
                        context,
                        lead.join("\n            ")
                    ));
                }
            }
            Ok((instructions, report)) => {
                self.decoded += 1;
                if report.is_clean() {
                    self.clean += 1;
                    install.clean += 1;
                } else if self.first_problems.len() < 12 {
                    let by_at: std::collections::HashMap<u32, &disasm::Instruction> =
                        instructions.iter().map(|i| (i.at, i)).collect();
                    let mut detail = Vec::new();
                    for site in report
                        .calls_not_to_enter
                        .first
                        .iter()
                        .chain(&report.jumps_off_boundary.first)
                    {
                        let i = by_at[site];
                        for t in i.targets() {
                            let near: Vec<String> = instructions
                                .iter()
                                .filter(|n| (n.at as i64 - t).abs() <= 12)
                                .map(|n| format!("{:06X}:{}", n.at, n.mnemonic()))
                                .collect();
                            let from = (t.max(0) as usize).min(program.code.len());
                            let bytes = &program.code[from..(from + 8).min(program.code.len())];
                            detail.push(format!(
                                "{} at {:06X} (len {}) -> {:06X} bytes {:02X?} near [{}]",
                                i.mnemonic(),
                                i.at,
                                i.len,
                                t,
                                bytes,
                                near.join(" ")
                            ));
                        }
                    }
                    self.first_problems.push(format!(
                        "{} (v{}): {}\n      {}",
                        program.name,
                        program.resource_version,
                        report.problems().join("; "),
                        detail.join("\n      ")
                    ));
                }
                if instructions.first().map(|i| i.opcode) == Some(opcode::ENTER) {
                    self.first_is_enter += 1;
                }
                self.fold_totals(program, &instructions, &report);
            }
        }

        self.falsify(program);

        if let (Some(lists), Some(measure)) = (lists, self.natives.as_mut()) {
            let install = self.installs.last_mut().expect("an install is open");
            let sig = measure
                .by_signature
                .entry(program.globals_signature)
                .or_default();
            sig.0 += 1;
            sig.2 += hashes.iter().collect::<HashSet<_>>().len();
            if let Some((db, coverage)) = lists.best_for(program) {
                sig.1 += coverage.named;
                if coverage.is_complete() && sig.3.is_none() {
                    sig.3 = Some(format!("{} ({})", program.name, db.label()));
                }
                measure.scripts_named += 1;
                if coverage.is_complete() {
                    measure.scripts_complete += 1;
                }
                measure.slots_named += coverage.named as u64;
                measure.slots_distinct += coverage.distinct as u64;
                *measure.chosen.entry(db.label().to_owned()).or_default() += 1;
                measure
                    .signatures_by_list
                    .entry(db.label().to_owned())
                    .or_default()
                    .insert(program.globals_signature);
                install
                    .named_hashes
                    .extend(hashes.iter().copied().filter(|h| db.name(*h).is_some()));
            } else {
                measure.slots_distinct += hashes.iter().collect::<HashSet<_>>().len() as u64;
            }
        }
    }

    /// Decode under every set, so the one the version names is compared with
    /// its rival rather than only with itself.
    fn fold_sets(&mut self, program: &ScrProgram) {
        let own = program.instruction_set();
        let mut decodes = Vec::new();
        for set in InstructionSet::ALL {
            let result = disasm::check_with(program, set.table());
            let outcome = match &result {
                Err(_) => Outcome::Stops,
                Ok((_, report)) if report.is_clean() => Outcome::Clean,
                Ok(_) => Outcome::Disagrees,
            };
            *self
                .by_set
                .entry((program.resource_version, set.name(), outcome))
                .or_default() += 1;
            decodes.push((set, outcome, result.ok().map(|(i, _)| i)));
        }
        let own_decode = decodes
            .iter()
            .find(|(set, ..)| Some(*set) == own)
            .and_then(|(_, _, i)| i.clone());
        for (set, outcome, decoded) in decodes {
            if Some(set) == own {
                continue;
            }
            let r = &mut self.rival;
            match outcome {
                Outcome::Stops => r.stops += 1,
                Outcome::Disagrees => r.disagrees += 1,
                Outcome::Clean => {
                    let decoded = decoded.expect("a clean decode has instructions");
                    if decoded
                        .iter()
                        .any(|i| i.op == opcode::CATCH || i.op == opcode::THROW)
                    {
                        r.reads_catch_or_throw += 1;
                    } else {
                        r.unrefuted += 1;
                        r.unrefuted_largest = r.unrefuted_largest.max(program.code_length);
                        let same = own_decode.as_ref().is_some_and(|own| {
                            own.len() == decoded.len()
                                && own
                                    .iter()
                                    .zip(&decoded)
                                    .all(|(a, b)| a.at == b.at && a.len == b.len)
                        });
                        if same {
                            r.unrefuted_same_boundaries += 1;
                        }
                        if r.first_unrefuted.len() < 8 {
                            r.first_unrefuted.push(format!(
                                "{} (v{}, {} bytes, same boundaries {same})",
                                program.name, program.resource_version, program.code_length
                            ));
                        }
                    }
                }
            }
        }
    }

    fn fold_totals(
        &mut self,
        program: &ScrProgram,
        instructions: &[disasm::Instruction],
        report: &CodeCheck,
    ) {
        let t = &mut self.totals;
        if t.op_counts.is_empty() {
            t.op_counts = vec![0; 256];
        }
        t.code_bytes += report.code_length as u64;
        t.bytes_decoded += report.bytes_decoded as u64;
        t.instructions += report.instructions as u64;
        t.functions += report.functions as u64;
        t.straddles += report.straddles.count as u64;
        t.jumps += report.jumps_checked as u64;
        t.jumps_off += report.jumps_off_boundary.count as u64;
        t.jumps_to_end += report.jumps_to_code_end as u64;
        t.calls_through_padding += report.calls_through_padding as u64;
        t.cases += report.cases_checked as u64;
        t.cases_off += report.cases_off_boundary.count as u64;
        t.calls += report.calls_checked as u64;
        t.calls_off += report.calls_off_boundary.count as u64;
        t.calls_not_to_enter += report.calls_not_to_enter.count as u64;
        t.natives += report.natives_checked as u64;
        t.natives_out_of_range += report.native_slots_out_of_range.count as u64;
        t.native_slots += program.natives.len() as u64;
        t.native_slots_unreferenced += report.native_slots_unreferenced as u64;
        for (op, n) in report.op_counts.iter().enumerate() {
            t.op_counts[op] += n;
        }

        let code = &program.code;
        let starts: HashSet<u32> = instructions.iter().map(|i| i.at).collect();
        let lands = |target: i64| target >= 0 && starts.contains(&(target as u32));
        for i in instructions {
            let at = i.at as usize;
            match &i.operand {
                Operand::Enter { name, .. } if !name.is_empty() => t.functions_named += 1,
                Operand::Native { .. } => {
                    let le = u16::from_le_bytes([code[at + 2], code[at + 3]]) as usize;
                    if le < program.natives.len() {
                        self.rivals.native_le_in_range += 1;
                    }
                }
                Operand::Jump { .. } => {
                    let offset = i16::from_le_bytes([code[at + 1], code[at + 2]]) as i64;
                    if lands(at as i64 + offset) {
                        self.rivals.jump_from_opcode_lands += 1;
                    }
                }
                Operand::Switch(cases) => {
                    let end = at as i64 + i.len as i64;
                    for (n, _) in cases.iter().enumerate() {
                        let off = at + 2 + n * 6 + 4;
                        let offset = i16::from_le_bytes([code[off], code[off + 1]]) as i64;
                        if lands(end + offset) {
                            self.rivals.case_from_switch_end_lands += 1;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Decode again with one operand a byte off, and see who notices.
    fn falsify(&mut self, program: &ScrProgram) {
        let Some(set) = program.instruction_set() else {
            return;
        };
        let used: HashSet<u8> = disasm::decode(&program.code, set)
            .map(|is| is.iter().map(|i| i.op).collect())
            .unwrap_or_default();
        if self.falsified.is_empty() {
            self.falsified = PERTURBATIONS
                .iter()
                .map(|(op, from, to)| Falsified {
                    op: *op,
                    from: *from,
                    to: *to,
                    scripts_with_op: 0,
                    failed_loudly: 0,
                    first_failures: Vec::new(),
                    scripts_without_op: 0,
                    without_op_still_clean: 0,
                    table_disagrees: false,
                })
                .collect();
        }
        for f in &mut self.falsified {
            let mut table = set.table().to_vec();
            let entry = table
                .iter_mut()
                .find(|e| e.op == f.op)
                .expect("both sets hold it");
            if entry.shape != f.from {
                f.table_disagrees = true;
                continue;
            }
            entry.shape = f.to;

            let outcome = match disasm::check_with(program, &table) {
                Err(e) => Err(e.to_string()),
                Ok((_, report)) if !report.is_clean() => Err(report.problems().join("; ")),
                Ok(_) => Ok(()),
            };
            if used.contains(&f.op) {
                f.scripts_with_op += 1;
                if let Err(why) = outcome {
                    f.failed_loudly += 1;
                    if f.first_failures.len() < 2 {
                        f.first_failures.push(format!("{}: {why}", program.name));
                    }
                }
            } else {
                f.scripts_without_op += 1;
                if outcome.is_ok() {
                    f.without_op_still_clean += 1;
                }
            }
        }
    }

    fn require_exercised(&self, check: &str, count: u64, floor: u64) {
        assert!(
            count >= floor,
            "{check} was exercised {count} time(s) and wants at least {floor}.\n{}\n\
             A check that ran on nothing passes for the wrong reason, so this is a \
             failure rather than a skip.",
            self.report()
        );
    }

    fn report(&self) -> String {
        let t = &self.totals;
        let r = &self.rivals;
        let exercised = t.op_counts.iter().filter(|n| **n > 0).count();
        let unused: Vec<&str> = (0..InstructionSet::U24Locals.table().len())
            .filter(|op| t.op_counts.get(*op).is_none_or(|n| *n == 0))
            .map(|op| opcode::mnemonic(op as u8))
            .collect();
        let mut out = String::new();
        for ((version, set, outcome), n) in &self.by_set {
            out.push_str(&format!("version {version} under {set}: {outcome:?} {n}\n"));
        }
        let rv = &self.rival;
        out.push_str(&format!(
            "under the other set: stops {}, disagrees {}, clean but reads CATCH/THROW {}, \
             unrefuted {} (same boundaries {}, largest {} bytes) {:?}\nnever used: {unused:?}\n",
            rv.stops,
            rv.disagrees,
            rv.reads_catch_or_throw,
            rv.unrefuted,
            rv.unrefuted_same_boundaries,
            rv.unrefuted_largest,
            rv.first_unrefuted
        ));
        for i in &self.installs {
            out.push_str(&format!(
                "install {}: {} scripts, {} decode clean, {} distinct native hashes\n",
                i.root,
                i.scripts,
                i.clean,
                i.distinct_hashes.len()
            ));
        }
        out.push_str(&format!(
            ".ysc seen {}, read {}, refused {}; decoded to the end {}, clean {}\n\
             unknown opcodes {:?}\n\
             code bytes {}, decoded {}, instructions {}, opcodes exercised {} of {}\n\
             functions {} (named {}), first instruction ENTER in {}\n\
             page crossings {}\n\
             jumps {} (to the end of the code {}, off an instruction {}; from the opcode instead: {} land)\n\
             switch cases {} (off {}; from the switch's end instead: {} land)\n\
             calls {} (off {}, through page padding {}, reaching no ENTER {})\n\
             native calls {} (slot out of range {}; read little-endian: {} in range)\n\
             native slots {} (named by no instruction {})\n",
            self.seen,
            self.read,
            self.refused.len(),
            self.decoded,
            self.clean,
            self.unknown_opcodes,
            t.code_bytes,
            t.bytes_decoded,
            t.instructions,
            exercised,
            InstructionSet::U24Locals.table().len(),
            t.functions,
            t.functions_named,
            self.first_is_enter,
            t.straddles,
            t.jumps,
            t.jumps_to_end,
            t.jumps_off,
            r.jump_from_opcode_lands,
            t.cases,
            t.cases_off,
            r.case_from_switch_end_lands,
            t.calls,
            t.calls_off,
            t.calls_through_padding,
            t.calls_not_to_enter,
            t.natives,
            t.natives_out_of_range,
            r.native_le_in_range,
            t.native_slots,
            t.native_slots_unreferenced,
        ));
        for f in &self.falsified {
            out.push_str(&format!(
                "falsified ({} read as {:?} instead of {:?}): {} of {} scripts using it fail \
                 loudly; {} of {} scripts not using it stay clean\n",
                opcode::mnemonic(f.op),
                f.to,
                f.from,
                f.failed_loudly,
                f.scripts_with_op,
                f.without_op_still_clean,
                f.scripts_without_op,
            ));
            for line in &f.first_failures {
                out.push_str(&format!("  {line}\n"));
            }
        }
        for line in self.decode_failures.iter().chain(&self.first_problems) {
            out.push_str(&format!("  {line}\n"));
        }
        if let Some(m) = &self.natives {
            out.push_str(&format!(
                "natives lists {:?}: {} scripts named by some list ({} completely); \
                 {} of {} per-script distinct natives named; chosen {:?}\n",
                m.lists,
                m.scripts_named,
                m.scripts_complete,
                m.slots_named,
                m.slots_distinct,
                m.chosen
            ));
            for (list, sigs) in &m.signatures_by_list {
                out.push_str(&format!(
                    "  {list}: chosen for {} globals signature(s)\n",
                    sigs.len()
                ));
            }
            for (sig, (scripts, named, distinct, complete)) in &m.by_signature {
                out.push_str(&format!(
                    "  signature 0x{sig:08X}: {scripts} scripts, {named} of {distinct} natives named; \
                     complete in {complete:?}\n"
                ));
            }
            for i in &self.installs {
                out.push_str(&format!(
                    "  install {}: {} of {} distinct hashes named\n",
                    i.root,
                    i.named_hashes.len(),
                    i.distinct_hashes.len()
                ));
            }
        }
        out
    }
}

fn crypto_for(root: &Path) -> Option<Arc<dyn RpfCryptoProvider>> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    Some(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))))
}

fn natives_lists() -> Option<NativeDbSet> {
    let raw = std::env::var_os(NATIVES_ENV)?;
    let mut dbs = Vec::new();
    for path in raw
        .to_string_lossy()
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let path = Path::new(path);
        let bytes = std::fs::read(path)
            .unwrap_or_else(|e| panic!("{NATIVES_ENV}: {}: {e}", path.display()));
        let label = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        dbs.push(NativeDb::parse(label, &bytes).unwrap_or_else(|e| panic!("{NATIVES_ENV}: {e}")));
    }
    Some(NativeDbSet::new(dbs))
}

fn build() -> Result<Census, String> {
    let roots = game_roots();
    if roots.is_empty() {
        return Err(format!(
            "no install root; set {GAME_ROOT_ENV} to a ;-separated list of install directories"
        ));
    }
    let lists = natives_lists();
    let mut census = Census {
        natives: lists.as_ref().map(|set| NativeMeasure {
            lists: set
                .dbs()
                .iter()
                .map(|d| (d.label().to_owned(), d.len()))
                .collect(),
            ..NativeMeasure::default()
        }),
        ..Census::default()
    };
    for root in &roots {
        let Some(provider) = crypto_for(root) else {
            continue;
        };
        census.installs.push(Install {
            root: root.display().to_string(),
            ..Install::default()
        });
        let mut warnings = Vec::new();
        let _ = crawl::crawl(root, provider, &mut warnings, &mut |found| {
            if !found.leaf.to_ascii_lowercase().ends_with(".ysc") {
                return ControlFlow::Continue(());
            }
            census.seen += 1;
            let program = found
                .read()
                .map_err(|e| e.to_string())
                .and_then(|bytes| ScrProgram::from_bytes(&bytes).map_err(|e| e.to_string()));
            match program {
                Ok(program) => census.fold(&program, lists.as_ref()),
                Err(e) => census.refused.push(format!("{}: {e}", found.leaf)),
            }
            ControlFlow::Continue(())
        });
    }
    if census.installs.is_empty() {
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

/// Every script decodes from its first byte to exactly its `code_length`.
///
/// ⚠ The floor is on scripts *read*, so a crawl that found nothing fails
/// rather than passing with nothing refused.
#[test]
fn every_script_decodes_to_exactly_its_code_length() {
    let Some(c) = census("every_script_decodes_to_exactly_its_code_length") else {
        return;
    };
    c.require_exercised("reading a script", c.read as u64, 1_000);
    assert!(
        c.refused.is_empty(),
        "scripts refused before decoding:\n{}",
        c.report()
    );
    assert_eq!(
        c.decoded,
        c.read,
        "every script should decode to its end\n{}",
        c.report()
    );
    assert_eq!(
        c.totals.bytes_decoded,
        c.totals.code_bytes,
        "{}",
        c.report()
    );
    assert_eq!(
        c.clean,
        c.read,
        "every decode should agree with its file\n{}",
        c.report()
    );
    assert_eq!(
        c.first_is_enter,
        c.read,
        "every script should open with ENTER\n{}",
        c.report()
    );

    // Nearly the whole instruction set is exercised, so a table entry being
    // wrong is not hidden by nobody using it.
    let exercised = c.totals.op_counts.iter().filter(|n| **n > 0).count();
    c.require_exercised("distinct opcodes", exercised as u64, 120);
    c.require_exercised("decoding an instruction", c.totals.instructions, 10_000_000);
}

/// The compiler pads with `NOP` so no instruction crosses a 16 KiB code page.
#[test]
fn no_instruction_crosses_a_code_page() {
    let Some(c) = census("no_instruction_crosses_a_code_page") else {
        return;
    };
    c.require_exercised("a page-crossing check", c.totals.instructions, 10_000_000);
    // Scripts span more than one page, or there is no boundary to cross.
    c.require_exercised(
        "code bytes past the first page",
        c.totals.code_bytes,
        100 * 0x4000,
    );
    assert_eq!(c.totals.straddles, 0, "{}", c.report());
}

/// Every jump and switch case lands on an instruction the decode found, and
/// the rival reading of each offset does not.
#[test]
fn every_jump_and_switch_case_lands_on_an_instruction() {
    let Some(c) = census("every_jump_and_switch_case_lands_on_an_instruction") else {
        return;
    };
    let (t, r) = (&c.totals, &c.rivals);
    c.require_exercised("a jump", t.jumps, 1_000_000);
    c.require_exercised("a switch case", t.cases, 100_000);
    assert_eq!(t.jumps_off, 0, "{}", c.report());
    assert_eq!(t.cases_off, 0, "{}", c.report());

    // ⚠ Side by side, so a check every reading would pass is visible. Most
    // instructions are short, so a wrong origin still lands often; what
    // refutes it is that it misses at all, and it misses millions of times.
    let jump_rival_misses = t.jumps - r.jump_from_opcode_lands;
    let case_rival_misses = t.cases - r.case_from_switch_end_lands;
    assert!(
        jump_rival_misses * 10 > t.jumps,
        "a jump measured from its opcode misses only {jump_rival_misses} times\n{}",
        c.report()
    );
    assert!(
        case_rival_misses * 10 > t.cases,
        "a case measured from the switch's end misses only {case_rival_misses} times\n{}",
        c.report()
    );
}

/// A call names its function's `ENTER`, or page padding that runs into one.
///
/// ⚠ **The padding case is a rule, not a tolerance.** It passes only when
/// `NOP`s and unconditional jumps lead from the call's target to an `ENTER`
/// at the first byte of a code page — the compiler moved the function and
/// kept the address its callers had.
#[test]
fn a_call_through_page_padding_still_reaches_an_enter() {
    let Some(c) = census("a_call_through_page_padding_still_reaches_an_enter") else {
        return;
    };
    c.require_exercised(
        "a call through page padding",
        c.totals.calls_through_padding,
        1_000,
    );
    assert_eq!(c.totals.calls_not_to_enter, 0, "{}", c.report());
}

/// The RSC7 version names the instruction set, and the other set is refuted
/// by the checks for every script with more than a page of code.
///
/// ⚠ **Below a page the version is the only evidence**, and the suite says
/// so rather than hiding it: the two sets differ only from opcode 94 up, and
/// a stub that uses none of the opcodes whose width moved decodes to the
/// same boundaries under both.
#[test]
fn the_version_names_the_instruction_set() {
    let Some(c) = census("the_version_names_the_instruction_set") else {
        return;
    };
    c.require_exercised("a script decoded under its own set", c.clean as u64, 1_000);
    assert_eq!(c.clean, c.read, "{}", c.report());

    let rv = &c.rival;
    let refuted = rv.stops + rv.disagrees + rv.reads_catch_or_throw;
    assert_eq!(refuted + rv.unrefuted, c.read, "{}", c.report());
    assert!(
        refuted * 10 > c.read * 8,
        "the other set is refuted for only {refuted}\n{}",
        c.report()
    );
    assert!(
        (rv.unrefuted_largest as usize) < or_io::gta5::script::CODE_PAGE_SIZE,
        "a script of {} bytes is decided by its version alone\n{}",
        rv.unrefuted_largest,
        c.report()
    );
}

/// Every call lands on an `ENTER`.
#[test]
fn every_call_lands_on_an_enter() {
    let Some(c) = census("every_call_lands_on_an_enter") else {
        return;
    };
    c.require_exercised("a call", c.totals.calls, 1_000_000);
    c.require_exercised("a function", c.totals.functions, 100_000);
    assert_eq!(c.totals.calls_off, 0, "{}", c.report());
    assert_eq!(c.totals.calls_not_to_enter, 0, "{}", c.report());
}

/// Every native slot an instruction names is inside the script's own table,
/// and reading the slot little-endian would not be.
#[test]
fn every_native_call_names_a_slot_in_the_table() {
    let Some(c) = census("every_native_call_names_a_slot_in_the_table") else {
        return;
    };
    let t = &c.totals;
    c.require_exercised("a native call", t.natives, 1_000_000);
    assert_eq!(t.natives_out_of_range, 0, "{}", c.report());
    assert_eq!(
        t.native_slots_unreferenced,
        0,
        "every slot should be called\n{}",
        c.report()
    );
    assert!(
        c.rivals.native_le_in_range * 2 < t.natives,
        "a little-endian slot is in range too often to be ruled out\n{}",
        c.report()
    );
}

/// ⚠ **The falsification.** One operand a byte off fails the census loudly,
/// in most of the scripts that use it and in none that do not.
///
/// Not in all of them, and the suite holds the figure rather than a claim of
/// completeness: a byte swallowed from a one-byte instruction nothing jumps
/// to leaves a script's boundaries intact, so a width error hides in a script
/// that only ever uses the opcode that way. Measured at 2,270 of 2,545 for
/// `IADD_U8` read two bytes wide.
#[test]
fn an_operand_a_byte_off_fails_loudly() {
    let Some(c) = census("an_operand_a_byte_off_fails_loudly") else {
        return;
    };
    assert_eq!(c.falsified.len(), PERTURBATIONS.len(), "{}", c.report());
    for f in &c.falsified {
        let what = format!("{} read as {:?}", opcode::mnemonic(f.op), f.to);
        assert!(
            !f.table_disagrees,
            "the table does not read {} as {:?}",
            opcode::mnemonic(f.op),
            f.from
        );
        c.require_exercised(
            &format!("a script using {what}"),
            f.scripts_with_op as u64,
            1_000,
        );
        assert!(
            f.failed_loudly * 10 >= f.scripts_with_op * 8,
            "{what} was noticed in only {} of {} scripts\n{}",
            f.failed_loudly,
            f.scripts_with_op,
            c.report()
        );
        assert_eq!(
            f.without_op_still_clean,
            f.scripts_without_op,
            "{what}\n{}",
            c.report()
        );
    }
}

/// What a user-supplied natives list names, per script and per install.
///
/// ⚠ Reported, never asserted: the lists are the user's, and a figure over
/// data OpenRage does not carry is not something this tree can hold.
#[test]
fn what_a_natives_list_names() {
    let Some(c) = census("what_a_natives_list_names") else {
        return;
    };
    let Some(m) = &c.natives else {
        skip_install(
            "what_a_natives_list_names",
            format_args!("no natives list; set {NATIVES_ENV} to measure one"),
        );
        return;
    };
    let chosen: usize = m.chosen.values().sum();
    assert_eq!(chosen, m.scripts_named);
}
