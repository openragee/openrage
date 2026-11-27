use crate::corpus;
use crate::support;

use std::collections::BTreeMap;

use corpus::{format_label, Corpus};
use or_io::meta::binary::parse_meta_binary_with;
use or_io::meta::{Diagnostics, GapKind};

#[derive(Default)]
struct Census {
    files: usize,
    parsed: usize,
    hard_errors: Vec<(String, String)>,
    gaps: BTreeMap<(GapKind, String), (usize, usize)>,
    dirty_files: Vec<(String, usize)>,
}

fn run_census(filter: Option<&str>) -> Census {
    let corpus = Corpus::load();
    let lib = support::schema();
    let mut census = Census::default();

    for fixture in corpus.iter() {
        if let Some(f) = filter {
            if format_label(fixture.format) != f {
                continue;
            }
        }
        census.files += 1;

        let mut diags = Diagnostics::lenient();
        match parse_meta_binary_with(lib, &fixture.bytes, &mut diags) {
            Ok(_) => census.parsed += 1,
            Err(e) => {
                census
                    .hard_errors
                    .push((fixture.name.clone(), e.to_string()));
                continue;
            }
        }

        let per_file = diags.census();
        if !per_file.is_empty() {
            let total: usize = per_file.iter().map(|(_, n)| *n).sum();
            census.dirty_files.push((fixture.name.clone(), total));
        }
        for (key, count) in per_file {
            let slot = census.gaps.entry(key).or_insert((0, 0));
            slot.0 += count;
            slot.1 += 1;
        }
    }
    census
}

fn print_census(title: &str, census: &Census) {
    println!("\n=== {title} ===");
    println!(
        "files: {}  parsed: {}  hard errors: {}",
        census.files,
        census.parsed,
        census.hard_errors.len()
    );

    if !census.hard_errors.is_empty() {
        println!("\n-- hard errors --");
        for (name, err) in census.hard_errors.iter().take(20) {
            println!("  {name}: {err}");
        }
        if census.hard_errors.len() > 20 {
            println!("  ... and {} more", census.hard_errors.len() - 20);
        }
    }

    let mut ranked: Vec<_> = census.gaps.iter().collect();
    ranked.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then_with(|| a.0.cmp(b.0)));

    let total: usize = census.gaps.values().map(|(n, _)| *n).sum();
    println!(
        "\n-- reader gaps: {total} occurrences across {} categories --",
        ranked.len()
    );
    println!("{:>10}  {:>6}  {:<26} DETAIL", "COUNT", "FILES", "KIND");
    for ((kind, detail), (count, files)) in ranked.iter().take(40) {
        println!("{count:>10}  {files:>6}  {:<26} {detail}", kind.as_str());
    }

    let mut dirty = census.dirty_files.clone();
    dirty.sort_by_key(|(_, n)| *n);
    if !dirty.is_empty() {
        println!("\n-- smallest dirty fixtures (best debugging targets) --");
        for (name, n) in dirty.iter().take(5) {
            println!("  {n:>6} gaps  {name}");
        }
        println!("-- worst offenders --");
        for (name, n) in dirty.iter().rev().take(5) {
            println!("  {n:>6} gaps  {name}");
        }
    }
}

#[test]
#[ignore = "diagnostic tool; run with --ignored --nocapture"]
fn census_report() {
    let corpus = Corpus::load();
    if corpus.is_empty() {
        eprintln!(
            "(no binary fixtures present — run crates/io/or-io-verify/scripts/stage-meta-corpus.ps1)"
        );
        return;
    }
    for label in ["PSO", "RBM", "RBF"] {
        let census = run_census(Some(label));
        if census.files > 0 {
            print_census(label, &census);
        }
    }
    print_census("ALL FORMATS", &run_census(None));
}

#[test]
fn corpus_parses_without_hard_errors() {
    let corpus = Corpus::load();
    if corpus.is_empty() {
        or_test_support::skip_corpus(
            "corpus_parses_without_hard_errors",
            "no binary fixtures present",
        );
        return;
    }
    let census = run_census(None);
    assert!(
        census.hard_errors.is_empty(),
        "{} of {} fixtures failed to parse; first: {:?}",
        census.hard_errors.len(),
        census.files,
        census.hard_errors.first()
    );
}

#[test]
fn corpus_is_gap_free() {
    let corpus = Corpus::load();
    if corpus.is_empty() {
        or_test_support::skip_corpus("corpus_is_gap_free", "no binary fixtures present");
        return;
    }
    let census = run_census(None);
    if !census.gaps.is_empty() {
        let mut ranked: Vec<_> = census.gaps.iter().collect();
        ranked.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then_with(|| a.0.cmp(b.0)));
        let summary: Vec<String> = ranked
            .iter()
            .take(10)
            .map(|((k, d), (n, f))| format!("{n}x ({f} files) {} {d}", k.as_str()))
            .collect();
        panic!(
            "reader gaps remain across {} fixtures:\n  {}",
            census.files,
            summary.join("\n  ")
        );
    }
}

#[test]
fn strict_mode_agrees_with_lenient() {
    let corpus = Corpus::load();
    if corpus.is_empty() {
        or_test_support::skip_corpus(
            "strict_mode_agrees_with_lenient",
            "no binary fixtures present",
        );
        return;
    }
    let lib = support::schema();
    for fixture in corpus.iter() {
        let mut lenient = Diagnostics::lenient();
        let lenient_ok = parse_meta_binary_with(lib, &fixture.bytes, &mut lenient).is_ok();
        if !lenient_ok || !lenient.is_clean() {
            continue;
        }
        let mut strict = Diagnostics::strict();
        let strict_result = parse_meta_binary_with(lib, &fixture.bytes, &mut strict);
        assert!(
            strict_result.is_ok(),
            "{} parses clean in lenient mode but fails in strict mode: {:?}",
            fixture.name,
            strict_result.err()
        );
    }
}

/// What `world_types::first_class_unstated` rests on: a world root with such a class is saved as reshaped, off the
/// layout it arrived with, so a tree as it was read must never name one.
#[test]
fn every_staged_tree_names_only_classes_its_own_type_table_states() {
    let corpus = Corpus::load();
    require_corpus!(corpus);
    let lib = support::schema();
    let (mut judged, mut named) = (0usize, Vec::new());
    for fixture in corpus.iter() {
        let mut diags = Diagnostics::lenient();
        let Ok((root, layout)) =
            or_io::meta::binary::parse_meta_binary_full(lib, &fixture.bytes, &mut diags)
        else {
            continue;
        };
        judged += 1;
        if let Some(class) = or_io::meta::typed::world_types::first_class_unstated(&layout, &root) {
            named.push(format!("{}: `{class}`", fixture.name));
        }
    }
    assert!(
        judged > 0,
        "no staged document was read, so nothing was judged"
    );
    assert!(
        named.is_empty(),
        "{} of {judged} staged documents name a class their own table does not state:\n  {}",
        named.len(),
        named
            .iter()
            .take(15)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    eprintln!("{judged} staged documents judged, none names a class its own table lacks");
}

/// What `world_types::first_member_unstated` rests on: a save refuses a world root that holds such a member, as a copy
/// that would lose it, so a tree as it was read must never hold one.
#[test]
fn every_staged_tree_holds_only_members_its_own_type_table_states() {
    let corpus = Corpus::load();
    require_corpus!(corpus);
    let lib = support::schema();
    let (mut judged, mut held) = (0usize, Vec::new());
    for fixture in corpus.iter() {
        let mut diags = Diagnostics::lenient();
        let Ok((root, layout)) =
            or_io::meta::binary::parse_meta_binary_full(lib, &fixture.bytes, &mut diags)
        else {
            continue;
        };
        judged += 1;
        let found = or_io::meta::typed::world_types::first_member_unstated(&layout, &root);
        if let Some((class, member)) = found {
            held.push(format!("{}: `{class}::{member}`", fixture.name));
        }
    }
    assert!(
        judged > 0,
        "no staged document was read, so nothing was judged"
    );
    assert!(
        held.is_empty(),
        "{} of {judged} staged documents hold a member their own table does not state:\n  {}",
        held.len(),
        held.iter()
            .take(15)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    eprintln!("{judged} staged documents judged, none holds a member its own table lacks");
}

/// What `world_types::first_member_missing` rests on: a world root whose save fails while a record under it lacks such
/// a member is saved again off the layout it arrived with, as one that holds a copy, so a tree as it was read must
/// never lack one.
#[test]
fn every_staged_tree_lacks_no_member_its_own_type_table_states() {
    let corpus = Corpus::load();
    require_corpus!(corpus);
    let lib = support::schema();
    let (mut judged, mut lacked) = (0usize, Vec::new());
    for fixture in corpus.iter() {
        let mut diags = Diagnostics::lenient();
        let Ok((root, layout)) =
            or_io::meta::binary::parse_meta_binary_full(lib, &fixture.bytes, &mut diags)
        else {
            continue;
        };
        judged += 1;
        let found = or_io::meta::typed::world_types::first_member_missing(&layout, &root);
        if let Some((class, member)) = found {
            lacked.push(format!("{}: `{class}` lacks {member:#010X}", fixture.name));
        }
    }
    assert!(
        judged > 0,
        "no staged document was read, so nothing was judged"
    );
    assert!(
        lacked.is_empty(),
        "{} of {judged} staged documents lack a member their own table states:\n  {}",
        lacked.len(),
        lacked
            .iter()
            .take(15)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    eprintln!("{judged} staged documents judged, none lacks a member its own table states");
}
