//! What a `.ysc` holds, over every one a real install ships.
//!
//! Point `OPENRAGE_GAME_ROOT` at one or more install roots, `;`-separated.
//! Nothing here runs without one, and the skip is recorded rather than
//! silent. A staged corpus is the wrong denominator for this format: the
//! questions below are about how many distinct native hashes an *install*
//! holds, and a dozen hand-picked scripts cannot answer that.
//!
//! ⚠ **Every assertion here is over an exercised count, and the counts are
//! asserted too.** A suite that only checked "no script disagreed" would
//! pass having read nothing, so each check ends with
//! `Census::require_exercised`.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use or_io::container::resource::{YSC_FILE_VERSION, YSC_FILE_VERSION_GEN8_ALT};
use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::{ResourceEnvelope, RpfCryptoProvider, Rsc7};
use or_io::gta5::script::{ScrProgram, CODE_PAGE_SIZE};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_title::install::crawl::{self, ControlFlow};

const WHAT: &str = "what_a_script_container_holds";

/// One fold over every `.ysc` in every install offered.
///
/// A fold rather than a collection: a retail install holds a third of a
/// gigabyte of bytecode, and an unbounded install sweep has run a machine out
/// of memory.
#[derive(Default)]
struct Census {
    installs: Vec<String>,
    /// Files whose extension matched, whether or not they read.
    seen: usize,
    /// Files a reader opened.
    read: usize,
    /// Files a reader refused, with the reasons.
    refused: Vec<String>,
    versions: std::collections::BTreeMap<u32, usize>,
    /// Scripts whose `name_hash` is the hash of their own `name`.
    name_hash_agrees: usize,
    /// Scripts whose joined code pages are exactly `code_length` bytes.
    code_length_agrees: usize,
    native_slots: usize,
    stored: HashSet<u64>,
    hashes: HashSet<u64>,
    statics: usize,
    strings: usize,
    code_bytes: u64,
    globals_signatures: HashSet<u32>,
    /// Scripts whose `parameter_count` is non-zero.
    with_parameters: usize,
}

impl Census {
    fn fold(&mut self, program: &ScrProgram) {
        self.read += 1;

        // Computed, never transcribed: this is the hash function the engine
        // uses, called on the name the file carries.
        if or_io::hash::string_hash(program.name.as_bytes()) == program.name_hash {
            self.name_hash_agrees += 1;
        }
        if program.code.len() == program.code_length as usize {
            self.code_length_agrees += 1;
        }

        self.native_slots += program.natives.len();
        self.stored.extend(program.natives.iter().copied());
        self.hashes.extend(program.native_hashes());
        self.statics += program.statics.len();
        self.strings += program.strings.len();
        self.code_bytes += program.code.len() as u64;
        self.globals_signatures.insert(program.globals_signature);
        if program.parameter_count > 0 {
            self.with_parameters += 1;
        }
    }

    /// Fail unless `count` scripts actually reached the check.
    ///
    /// ⚠ A perturbation that stops the walk finding files leaves every
    /// *exercised* comparison passing.
    fn require_exercised(&self, check: &str, count: usize, floor: usize) {
        assert!(
            count >= floor,
            "{check} was exercised on {count} and wants at least {floor}.\n{}\n\
             A check that ran on nothing passes for the wrong reason, so this is a \
             failure rather than a skip.",
            self.report()
        );
    }

    fn report(&self) -> String {
        format!(
            "installs: {}\n\
             .ysc seen {}, read {}, refused {}\n\
             versions: {:?}\n\
             native slots {}, distinct as stored {}, distinct de-rotated {}\n\
             statics {}, strings {}, code bytes {}\n\
             globals signatures {}, scripts with parameters {}",
            self.installs.join(", "),
            self.seen,
            self.read,
            self.refused.len(),
            self.versions,
            self.native_slots,
            self.stored.len(),
            self.hashes.len(),
            self.statics,
            self.strings,
            self.code_bytes,
            self.globals_signatures.len(),
            self.with_parameters,
        )
    }
}

fn crypto_for(root: &Path) -> Option<Arc<dyn RpfCryptoProvider>> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    Some(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))))
}

fn census() -> Option<Census> {
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(
            WHAT,
            format_args!(
                "no install root; set {GAME_ROOT_ENV} to a ;-separated list of install \
                 directories. A .ysc lives inside an encrypted archive, so there is no \
                 loose-file fallback."
            ),
        );
        return None;
    }

    let mut census = Census::default();
    for root in &roots {
        let Some(provider) = crypto_for(root) else {
            continue;
        };
        census.installs.push(root.display().to_string());
        let mut warnings = Vec::new();
        let _ = crawl::crawl(root, provider, &mut warnings, &mut |found| {
            if !found.leaf.to_ascii_lowercase().ends_with(".ysc") {
                return ControlFlow::Continue(());
            }
            census.seen += 1;
            let Ok(bytes) = found.read() else {
                census
                    .refused
                    .push(format!("{}: could not extract", found.leaf));
                return ControlFlow::Continue(());
            };
            if let Ok(seal) = Rsc7::peek(&bytes) {
                *census.versions.entry(seal.version()).or_default() += 1;
            }
            match ScrProgram::from_bytes(&bytes) {
                Ok(program) => census.fold(&program),
                Err(e) => census.refused.push(format!("{}: {e}", found.leaf)),
            }
            ControlFlow::Continue(())
        });
    }

    if census.installs.is_empty() {
        skip_install(
            WHAT,
            "no install root yielded an executable to take keys from",
        );
        return None;
    }
    Some(census)
}

/// Every `.ysc` an install ships opens, and the counts say how many.
#[test]
fn what_a_script_container_holds() {
    let Some(census) = census() else {
        return;
    };
    eprintln!("{}", census.report());

    // ⚠ A refusal is a failure rather than a statistic. A corpus that is
    // present and unreadable is not an absent corpus, and the reader claims
    // to read every version the registry accepts.
    assert!(
        census.refused.is_empty(),
        "{} of {} .ysc refused:\n  {}\n{}",
        census.refused.len(),
        census.seen,
        census
            .refused
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  "),
        census.report()
    );
    assert_eq!(census.read, census.seen);

    // A retail Legacy install ships 1,158 and an Enhanced one 1,864. The
    // floor is well under either, so one edition alone still runs and zero
    // cannot pass.
    census.require_exercised("opening a script", census.read, 1_000);
}

/// The name a script carries hashes to the number beside it.
///
/// ⚠ **This is what pins the header's field offsets.** `name_hash` and
/// `name` are eight bytes apart, and a header misread by any amount puts a
/// number beside a string that does not hash to it. Computed by calling the
/// hash function, never by comparing against a converted literal.
#[test]
fn a_scripts_name_hashes_to_the_number_beside_it() {
    let Some(census) = census() else {
        return;
    };
    census.require_exercised("hashing a script name", census.read, 1_000);
    assert_eq!(
        census.name_hash_agrees,
        census.read,
        "every script's name should hash to its own name_hash\n{}",
        census.report()
    );
}

/// Every page of bytecode a script states is reachable, and they add up.
///
/// ⚠ No field states the page count — it is `code_length / 0x4000`, rounded
/// up — so this is the only thing standing behind that rule.
#[test]
fn the_code_pages_add_up_to_the_length_the_header_states() {
    let Some(census) = census() else {
        return;
    };
    census.require_exercised("joining code pages", census.read, 1_000);
    assert_eq!(
        census.code_length_agrees,
        census.read,
        "every script's joined pages should be exactly code_length bytes\n{}",
        census.report()
    );
    assert!(
        census.code_bytes > CODE_PAGE_SIZE as u64,
        "a census that read no bytecode has not exercised the page walk\n{}",
        census.report()
    );
}

/// The per-slot rotation is real, and undoing it collapses the population.
///
/// ⚠ The stored values do not collapse: the key includes the script's own
/// code length, so one native is a different number in every script that
/// calls it. Measured over a retail Legacy install — 653,093 slots, 161,254
/// distinct stored, 7,402 distinct de-rotated.
///
/// A de-rotation that did nothing, or that rotated by a constant, leaves the
/// two counts within a whisker of each other and fails the first assertion.
/// One that destroyed the hash instead of recovering it collapses far too far
/// and fails the second.
#[test]
fn de_rotating_a_native_slot_collapses_it_onto_a_real_hash() {
    let Some(census) = census() else {
        return;
    };
    census.require_exercised("de-rotating a native slot", census.native_slots, 100_000);

    assert!(
        census.hashes.len() * 4 < census.stored.len(),
        "de-rotation should collapse the population by an order of magnitude; \
         {} distinct stored became {} distinct hashes, which is no collapse and \
         means the rotation key is wrong\n{}",
        census.stored.len(),
        census.hashes.len(),
        census.report()
    );

    // GTA V ships on the order of five thousand natives, and an install holds
    // scripts from several builds, so the recovered set is thousands rather
    // than a handful.
    assert!(
        census.hashes.len() > 1_000,
        "{} distinct hashes is too few to be the game's native set\n{}",
        census.hashes.len(),
        census.report()
    );
}

/// Both RSC7 versions a retail install ships read, and the registry accepts
/// exactly the numbers the installs hold.
///
/// ⚠ **Neither number names a generation**, which is the trap: both editions
/// ship both, so a reader dispatching on the version would be dispatching on
/// nothing.
#[test]
fn both_shipped_versions_read_and_neither_names_an_edition() {
    let Some(census) = census() else {
        return;
    };
    census.require_exercised("reading a shipped version", census.read, 1_000);

    let shipped: Vec<u32> = census.versions.keys().copied().collect();
    assert_eq!(
        shipped,
        vec![YSC_FILE_VERSION_GEN8_ALT, YSC_FILE_VERSION],
        "an install should ship exactly the two versions the registry accepts\n{}",
        census.report()
    );
}
