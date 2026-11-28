//! The default test tier: the cheapest tests that fit a time budget, with the
//! rest named in `.config/nextest.toml` as the slow tier only `just test` runs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use regex::Regex;

use crate::git;
use crate::tiers::target_directory;

/// What the default tier may cost: its tests' times summed, in seconds, as a
/// whole-workspace run records them.
///
/// ⚠ A whole-workspace run loads every core, so a test takes about twice there
/// what it takes in a run of the default tier alone.
const BUDGET_SECONDS: f64 = 1700.0;

/// The app's test binary, as nextest names it.
const APP: &str = "openrage::bin/openrage";

/// Modules of the app that stay in the default tier whatever they cost: the
/// gates over the documents, the sentences and the source, which most changes
/// can fail, and the self-check, which the gate runs nowhere else.
const ALWAYS_DEFAULT: &[&str] = &[
    "doc_",
    "default_view_strings",
    "edition_strings",
    "i18n::",
    "shell_budget",
    "suppressions",
    "self_check::judged_without_an_install",
];

/// Binaries that are slow whatever a run timed them at.
///
/// ⚠ Each walks an install, so a run on a machine with none times it at a skip, and on one with an install it passes the
/// 30 s a default-tier test is ended at.
const ALWAYS_SLOW: &[&str] = &[
    "or-maps::what_the_rules_say_of_the_install",
    "or-maps::deleting_an_entity_of_the_install_keeps_every_link",
    "or-maps::the_doors_bounds_and_path_regions_of_the_install",
    "or-world-gta5::the_world_index_of_a_real_install",
    "or-world-gta5::what_a_map_holds_besides_entities",
    "or-cli::how_many_texture_dictionaries_the_type_files_of_an_install_name_and_no_file_holds",
    "or-maps::how_the_packs_of_an_install_carry_their_maps",
    "or-title::which_groups_the_change_sets_of_an_install_belong_to",
    "or-world-gta5::what_the_flags_of_a_manifest_entry_follow",
    "or-io::rel_interiors_of_an_install",
    "or-maps::what_the_interior_rules_say_of_the_install",
    "or-world-gta5::the_models_of_a_real_install",
    "or-io-verify::rdr2_world_census",
    "or-world-gta5::gta5_over_a_real_install",
];

/// Modules of the app that are slow whatever a run timed them at, for the same reason.
const ALWAYS_SLOW_OF_THE_APP: &[&str] =
    &["state::gta6_world::worldview::reference::of_the_install"];

const BEGIN: &str = "# slow-tier:begin";
const END: &str = "# slow-tier:end";

/// One test of a recorded run: its binary, its name and the seconds it took.
struct Timed {
    binary: String,
    name: String,
    seconds: f64,
}

fn timings(junit: &str) -> Vec<Timed> {
    let case = Regex::new(r#"<testcase name="([^"]*)" classname="([^"]*)"[^>]*? time="([^"]*)""#)
        .expect("a valid pattern");
    case.captures_iter(junit)
        .filter_map(|c| {
            Some(Timed {
                name: c[1].to_string(),
                binary: c[2].to_string(),
                seconds: c[3].parse().ok()?,
            })
        })
        .collect()
}

/// The module a test is in: its name up to the last `::`, empty at the root of its binary.
fn module_of(name: &str) -> &str {
    name.rsplit_once("::").map_or("", |(module, _)| module)
}

/// The modules to leave out so the rest cost `budget` seconds or less, as
/// `(binary, module)`: the costliest per test go first, so the most tests stay.
///
/// ⚠ A module and not a test: a test added to a slow module is slow without a
/// new record, and a module's tests cost about the same.
fn slow_modules(tests: &[Timed], budget: f64) -> BTreeSet<(String, String)> {
    let mut modules: BTreeMap<(&str, &str), (usize, f64)> = BTreeMap::new();
    for test in tests {
        let entry = modules
            .entry((&test.binary, module_of(&test.name)))
            .or_default();
        entry.0 += 1;
        entry.1 += test.seconds;
    }
    let mut left: f64 = tests.iter().map(|t| t.seconds).sum();
    let mut costliest: Vec<_> = modules.into_iter().collect();
    costliest.sort_by(|(_, a), (_, b)| (b.1 / b.0 as f64).total_cmp(&(a.1 / a.0 as f64)));
    let mut slow = BTreeSet::new();
    for ((binary, module), (_, seconds)) in costliest {
        if left <= budget {
            break;
        }
        if binary == APP && ALWAYS_DEFAULT.iter().any(|kept| module.starts_with(kept)) {
            continue;
        }
        left -= seconds;
        slow.insert((binary.to_string(), module.to_string()));
    }
    for test in tests {
        let module = module_of(&test.name);
        let of_the_app = test.binary == APP && ALWAYS_SLOW_OF_THE_APP.contains(&module);
        if ALWAYS_SLOW.contains(&test.binary.as_str()) || of_the_app {
            slow.insert((test.binary.clone(), module.to_string()));
        }
    }
    slow
}

/// The nextest `default-filter` that leaves `slow` out, which [`SlowTier::read`] reads back.
///
/// ⚠ A slow module takes the modules inside it, and a binary whose tests at
/// its root are slow goes whole: the gate hands a binary libtest's `--skip`,
/// which matches a part of a name and cannot say "the tests of no module".
fn filter(tests: &[Timed], slow: &BTreeSet<(String, String)>) -> String {
    let mut all: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for test in tests {
        all.entry(&test.binary)
            .or_default()
            .insert(module_of(&test.name));
    }
    let mut terms = Vec::new();
    for (binary, modules) in all {
        let out: Vec<&str> = modules
            .iter()
            .copied()
            .filter(|m| slow.contains(&(binary.to_string(), (*m).to_string())))
            .collect();
        if out.is_empty() {
            continue;
        }
        if out.len() == modules.len() || out.contains(&"") {
            terms.push(format!("binary_id(={binary})"));
            continue;
        }
        let tests: Vec<String> = out
            .iter()
            .map(|module| format!("test(/^{module}::/)"))
            .collect();
        terms.push(format!(
            "(binary_id(={binary}) & (\n      {}\n    ))",
            tests.join("\n      | ")
        ));
    }
    if terms.is_empty() {
        return "default-filter = 'all()'".to_string();
    }
    format!(
        "default-filter = '''\nnot (\n  {}\n)\n'''",
        terms.join("\n  | ")
    )
}

/// The slow tier as `.config/nextest.toml` states it: the binaries left out whole, and the modules left out of the others.
#[derive(Default)]
pub struct SlowTier {
    whole: BTreeSet<String>,
    modules: BTreeMap<String, Vec<String>>,
}

impl SlowTier {
    pub fn read(root: &Path) -> Result<Self> {
        let config = root.join(".config/nextest.toml");
        Ok(Self::parse(
            &std::fs::read_to_string(&config)
                .with_context(|| format!("could not read {}", config.display()))?,
        ))
    }

    fn parse(config: &str) -> Self {
        let block = config
            .split_once(BEGIN)
            .and_then(|(_, rest)| rest.split_once(END))
            .map_or("", |(block, _)| block);
        let mut tier = Self::default();
        let mut mixed: Option<&str> = None;
        for line in block.lines() {
            let term = line.trim().trim_start_matches("| ");
            if let Some(binary) = term
                .strip_prefix("(binary_id(=")
                .and_then(|rest| rest.strip_suffix(") & ("))
            {
                mixed = Some(binary);
            } else if let Some(binary) = term
                .strip_prefix("binary_id(=")
                .and_then(|rest| rest.strip_suffix(')'))
            {
                tier.whole.insert(binary.to_string());
            } else if let (Some(binary), Some(module)) = (
                mixed,
                term.strip_prefix("test(/^")
                    .and_then(|rest| rest.strip_suffix("/)")),
            ) {
                tier.modules
                    .entry(binary.to_string())
                    .or_default()
                    .push(module.to_string());
            }
        }
        tier
    }

    /// Whether every test of `binary` is in the slow tier.
    pub fn whole(&self, binary: &str) -> bool {
        self.whole.contains(binary)
    }

    /// The slow modules of `binary`, each as the `module::` every one of its tests' names holds.
    pub fn modules(&self, binary: &str) -> &[String] {
        self.modules.get(binary).map_or(&[], Vec::as_slice)
    }
}

fn replace_block(text: &str, block: &str) -> Result<String> {
    let begin = text
        .find(BEGIN)
        .with_context(|| format!("no `{BEGIN}` in .config/nextest.toml"))?;
    let end = text[begin..]
        .find(END)
        .with_context(|| format!("no `{END}` in .config/nextest.toml"))?
        + begin
        + END.len();
    Ok(format!("{}{block}{}", &text[..begin], &text[end..]))
}

/// Rewrite the slow tier in `.config/nextest.toml` from the timings the last `just test` left.
pub fn record(args: &[String]) -> Result<ExitCode> {
    let budget = match args {
        [] => BUDGET_SECONDS,
        [flag, seconds] if flag == "--budget" => {
            seconds.parse().context("--budget takes seconds")?
        }
        _ => bail!("slow-tier takes `--budget SECONDS` or nothing"),
    };
    let junit = target_directory()?.join("nextest/all/junit.xml");
    let text = std::fs::read_to_string(&junit)
        .with_context(|| format!("no {}: run `just test` first", junit.display()))?;
    let tests = timings(&text);
    // ⚠ A run of a few packages would name every other package's tests fast.
    if tests.len() < 1000 {
        bail!(
            "{} holds {} tests, which is not a whole-workspace run: run `just test` first",
            junit.display(),
            tests.len()
        );
    }
    let slow = slow_modules(&tests, budget);
    let is_slow = |t: &&Timed| slow.contains(&(t.binary.clone(), module_of(&t.name).to_string()));
    let (count, seconds) = tests
        .iter()
        .filter(is_slow)
        .fold((0, 0.0), |(n, s), t| (n + 1, s + t.seconds));
    let total: f64 = tests.iter().map(|t| t.seconds).sum();

    let config = git::here()?.join(".config/nextest.toml");
    let old = std::fs::read_to_string(&config)?;
    let block = format!(
        "{BEGIN} (written by `just slow-tier` from a `just test`: {count} of {} tests in {} modules, \
         leaving {:.0} s of {total:.0} s)\n{}\n{END}",
        tests.len(),
        slow.len(),
        total - seconds,
        filter(&tests, &slow)
    );
    std::fs::write(&config, replace_block(&old, &block)?)?;
    println!(
        "default tier: {} tests, {:.0} s summed; slow tier: {count} tests in {} modules, {seconds:.0} s summed",
        tests.len() - count,
        total - seconds,
        slow.len()
    );
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timed(binary: &str, name: &str, seconds: f64) -> Timed {
        Timed {
            binary: binary.to_string(),
            name: name.to_string(),
            seconds,
        }
    }

    fn run() -> Vec<Timed> {
        vec![
            timed("app", "state::walks::every_file", 30.0),
            timed("app", "state::walks::every_other_file", 20.0),
            timed("app", "state::units::one", 1.0),
            timed("app", "state::units::two", 1.0),
            timed("app", "at_the_root", 4.0),
            timed("lib::tests/corpus", "every_file_comes_back", 40.0),
        ]
    }

    #[test]
    fn the_costliest_modules_leave_until_the_rest_fit_the_budget() {
        let slow = slow_modules(&run(), 10.0);
        let named: Vec<(&str, &str)> = slow.iter().map(|(b, m)| (b.as_str(), m.as_str())).collect();
        assert_eq!(
            named,
            [("app", "state::walks"), ("lib::tests/corpus", "")],
            "96 s less the two costliest modules is 6 s, which fits 10"
        );
        assert!(
            slow_modules(&run(), 96.0).is_empty(),
            "a run that fits loses nothing"
        );
    }

    #[test]
    fn a_gate_over_the_documents_stays_in_the_default_tier_whatever_it_costs() {
        let tests = vec![
            timed(APP, "doc_counts::every_figure_is_computed", 60.0),
            timed(APP, "state::walks::every_file", 30.0),
            timed("lib", "doc_counts::a_module_of_another_binary", 50.0),
        ];
        let slow = slow_modules(&tests, 0.0);
        assert!(!slow.contains(&(APP.to_string(), "doc_counts".to_string())));
        assert!(slow.contains(&(APP.to_string(), "state::walks".to_string())));
        assert!(slow.contains(&("lib".to_string(), "doc_counts".to_string())));
    }

    #[test]
    fn a_binary_that_walks_an_install_is_slow_though_a_run_timed_it_at_a_skip() {
        let mut tests: Vec<Timed> = (ALWAYS_SLOW.iter())
            .map(|binary| timed(binary, "a_walk_of_the_install", 0.01))
            .collect();
        tests.push(timed("lib", "units::one", 1.0));
        let slow = slow_modules(&tests, 100.0);
        let mut walks: Vec<(String, String)> = (ALWAYS_SLOW.iter())
            .map(|binary| (binary.to_string(), String::new()))
            .collect();
        walks.sort();
        assert_eq!(
            slow.into_iter().collect::<Vec<_>>(),
            walks,
            "the run fits its budget, and the walk of an install is slow all the same"
        );
        // ⚠ The block of `.config/nextest.toml` in the tree states it, so a tier written by hand and one written from a run agree.
        let config = include_str!("../../../../.config/nextest.toml");
        for binary in ALWAYS_SLOW {
            assert!(SlowTier::parse(config).whole(binary), "{binary}");
        }
    }

    #[test]
    fn a_module_of_the_app_that_walks_an_install_is_slow_though_a_run_timed_it_at_a_skip() {
        let mut tests: Vec<Timed> = (ALWAYS_SLOW_OF_THE_APP.iter())
            .map(|module| timed(APP, &format!("{module}::a_walk_of_the_install"), 0.01))
            .collect();
        tests.push(timed(APP, "state::units::one", 1.0));
        let slow = slow_modules(&tests, 100.0);
        let walks: Vec<(String, String)> = (ALWAYS_SLOW_OF_THE_APP.iter())
            .map(|module| (APP.to_string(), module.to_string()))
            .collect();
        assert_eq!(
            slow.into_iter().collect::<Vec<_>>(),
            walks,
            "the app's other modules stay, and the walk of an install is slow all the same"
        );
        // ⚠ The block of `.config/nextest.toml` in the tree states it, as it states the binaries.
        let config = include_str!("../../../../.config/nextest.toml");
        let stated = SlowTier::parse(config);
        for module in ALWAYS_SLOW_OF_THE_APP {
            let skipped = format!("{module}::");
            assert!(stated.modules(APP).contains(&skipped), "{module}");
        }
    }

    #[test]
    fn a_recorded_run_is_read_by_binary_name_and_time() {
        let junit = r#"<testsuite name="or-io"><testcase name="a::b" classname="or-io" timestamp="2026-11-20T04:32:13.361+01:00" time="0.250"/>
<testcase name="c" classname="or-io::x" timestamp="t" time="12.5"><system-out>time="99"</system-out></testcase>"#;
        let read = timings(junit);
        assert_eq!(read.len(), 2);
        assert_eq!(
            (
                read[0].binary.as_str(),
                read[0].name.as_str(),
                read[0].seconds
            ),
            ("or-io", "a::b", 0.25)
        );
        assert_eq!(
            (read[1].binary.as_str(), read[1].seconds),
            ("or-io::x", 12.5)
        );
    }

    #[test]
    fn the_filter_names_a_wholly_slow_binary_alone_and_a_module_by_its_path() {
        let tests = run();
        let written = filter(&tests, &slow_modules(&tests, 10.0));
        assert_eq!(
            written,
            "default-filter = '''\nnot (\n  (binary_id(=app) & (\n      test(/^state::walks::/)\n    ))\n  | binary_id(=lib::tests/corpus)\n)\n'''"
        );
        assert_eq!(filter(&tests, &BTreeSet::new()), "default-filter = 'all()'");

        let read = SlowTier::parse(&format!("{BEGIN} note\n{written}\n{END}\n"));
        assert!(read.whole("lib::tests/corpus") && !read.whole("app"));
        assert_eq!(read.modules("app"), ["state::walks::"]);
        assert!(read.modules("lib::tests/corpus").is_empty());
    }

    #[test]
    fn a_binary_whose_root_tests_are_slow_goes_whole() {
        let tests = run();
        let written = filter(&tests, &slow_modules(&tests, 5.0));
        assert_eq!(
            written,
            "default-filter = '''\nnot (\n  binary_id(=app)\n  | binary_id(=lib::tests/corpus)\n)\n'''"
        );
    }

    #[test]
    fn only_the_marked_block_of_the_configuration_is_replaced() {
        let config = format!(
            "[profile.default]\n{BEGIN} old\ndefault-filter = 'all()'\n{END}\nslow-timeout = \"10s\"\n"
        );
        let replaced = replace_block(&config, &format!("{BEGIN}\nnew\n{END}")).expect("a block");
        assert_eq!(
            replaced,
            format!("[profile.default]\n{BEGIN}\nnew\n{END}\nslow-timeout = \"10s\"\n")
        );
        assert!(replace_block("[profile.default]\n", "x").is_err());
    }
}
