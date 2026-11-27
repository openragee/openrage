//! The catalogs and the code hold each other to account: a text the code asks for is in every built-in catalog, an entry the code does not ask for is not, and a translation of any built-in catalog keeps the arguments of its text and draws with no box.

use std::collections::{BTreeMap, BTreeSet};

use super::catalog::Plural;
use super::po::{self, Entry};
use super::used::{self, Use};

/// Every built-in catalog: its code and its file as read.
fn built_in() -> Vec<(&'static str, po::Parsed)> {
    super::BUILT_IN
        .iter()
        .map(|(code, text)| {
            let read = po::parse(text).unwrap_or_else(|e| panic!("the {code} catalog: {e}"));
            (*code, read)
        })
        .collect()
}

/// The names between braces in `text`, or the first thing in it that is not a name or a doubled brace.
fn arguments(text: &str) -> Result<BTreeSet<String>, String> {
    let mut names = BTreeSet::new();
    let mut rest = text;
    while let Some(at) = rest.find(['{', '}']) {
        let tail = &rest[at..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            rest = &tail[2..];
            continue;
        }
        let Some(inner) = tail.strip_prefix('{').and_then(|t| t.split_once('}')) else {
            return Err(format!("a brace that is not a name or doubled: {text:?}"));
        };
        let name = inner.0;
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("`{{{name}}}` is not a plain name in {text:?}"));
        }
        names.insert(name.to_string());
        rest = inner.1;
    }
    Ok(names)
}

/// What the code asks for: each text, and the text of its other number when it has one.
fn asked(uses: &[Use]) -> BTreeMap<String, Option<String>> {
    uses.iter()
        .map(|u| (u.text.clone(), u.other.clone()))
        .collect()
}

fn where_is(uses: &[Use], text: &str) -> String {
    uses.iter()
        .find(|u| u.text == text)
        .map(|u| format!("{}:{}", u.file.display(), u.line))
        .unwrap_or_default()
}

/// Where the built-in catalogs and `uses` are out of step: a text asked for that a catalog lacks, and an entry nothing asks for.
fn out_of_step(uses: &[Use]) -> Vec<String> {
    let asked = asked(uses);
    let mut faults = Vec::new();
    for (code, catalog) in built_in() {
        let entries = &catalog.entries;
        let have: BTreeMap<&str, &Entry> = entries.iter().map(|e| (e.msgid.as_str(), e)).collect();
        for (text, other) in &asked {
            match have.get(text.as_str()) {
                None => faults.push(format!(
                    "the code asks for {text:?} ({}) and the {code} catalog lacks it",
                    where_is(uses, text)
                )),
                Some(entry) if entry.plural != *other => faults.push(format!(
                    "{text:?} is {} in the code and {} in the {code} catalog",
                    if other.is_some() { "a plural" } else { "plain" },
                    if entry.plural.is_some() {
                        "a plural"
                    } else {
                        "plain"
                    },
                )),
                Some(entry) if entry.forms.iter().any(String::is_empty) => {
                    faults.push(format!("{text:?} has no translation in {code} yet"));
                }
                Some(_) => {}
            }
        }
        for entry in entries {
            if !asked.contains_key(&entry.msgid) {
                faults.push(format!(
                    "the {code} catalog has {:?} and nothing in the code asks for it",
                    entry.msgid
                ));
            }
        }
        let once: BTreeSet<&String> = entries.iter().map(|e| &e.msgid).collect();
        if once.len() != entries.len() {
            faults.push(format!("an entry is in the {code} catalog twice"));
        }
    }
    faults
}

#[test]
fn every_text_the_code_asks_for_is_in_every_built_in_catalog_and_every_entry_is_asked_for() {
    let uses = used::in_the_source();
    assert!(
        uses.len() > 20,
        "only {} calls found: the scan is wrong",
        uses.len()
    );
    let faults = out_of_step(&uses);
    assert!(
        faults.is_empty(),
        "{} faults:\n{}",
        faults.len(),
        faults.join("\n")
    );
}

#[test]
fn a_sentence_a_world_adapter_names_that_no_catalog_holds_fails_the_gate() {
    const TEXT: &str = "A sentence of a world adapter that no catalog holds";
    let root = tempfile::tempdir().expect("a scratch folder");
    let adapter = root.path().join("crates/domain/or-world-testopolis/src");
    std::fs::create_dir_all(&adapter).expect("a folder is made");
    for empty in ["src", "crates/domain/or-world/src"] {
        std::fs::create_dir_all(root.path().join(empty)).expect("a folder is made");
    }
    std::fs::write(
        adapter.join("lib.rs"),
        format!(
            "pub const SECTION: or_world::sentence::Sentence = or_world::sentence!(\"{TEXT}\");\n"
        ),
    )
    .expect("a file is written");
    let uses = used::in_the_folders(&used::source_folders(root.path()));
    assert_eq!(uses.len(), 1, "{uses:?}");
    assert_eq!(
        (uses[0].macro_name.as_str(), uses[0].text.as_str()),
        ("sentence", TEXT)
    );
    let faults = out_of_step(&uses);
    for (code, _) in built_in() {
        let lacks = format!("and the {code} catalog lacks it");
        assert!(
            faults
                .iter()
                .any(|fault| fault.contains(TEXT) && fault.ends_with(&lacks)),
            "the gate does not say the {code} catalog lacks the adapter's sentence"
        );
    }
    // The walk the gate itself makes reads the world core, where the fake title's sentences are.
    let core = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/domain/or-world/src");
    let read = used::in_the_source();
    assert!(
        read.iter()
            .any(|u| u.macro_name == "sentence" && u.file.starts_with(&core)),
        "no sentence of the world core was read"
    );
}

#[test]
fn a_translation_keeps_every_argument_of_its_text_under_its_own_name() {
    let mut faults = Vec::new();
    for (code, catalog) in built_in() {
        for entry in &catalog.entries {
            // A form of a plural entry answers either English text: both are given the number, and the first form of a language is not always for one alone.
            let mut allowed = Vec::new();
            for english in [Some(entry.msgid.as_str()), entry.plural.as_deref()]
                .into_iter()
                .flatten()
            {
                match arguments(english) {
                    Ok(names) => allowed.push(names),
                    Err(why) => faults.push(format!("{code}: {why}")),
                }
            }
            // An empty form is a text nobody translated yet, which is shown in English.
            for form in entry.forms.iter().filter(|f| !f.is_empty()) {
                match arguments(form) {
                    Ok(names) if allowed.contains(&names) => {}
                    Ok(names) => faults.push(format!(
                        "{code}: {:?} takes {allowed:?} and its translation {form:?} takes {names:?}",
                        entry.msgid
                    )),
                    Err(why) => faults.push(format!("{code}: {why}")),
                }
            }
        }
    }
    assert!(
        faults.is_empty(),
        "{} faults:\n{}",
        faults.len(),
        faults.join("\n")
    );
}

#[test]
fn every_built_in_catalog_names_itself_and_counts_in_a_rule_this_reader_knows() {
    let mut faults = Vec::new();
    for (code, catalog) in built_in() {
        if !catalog.header.contains_key("Language-Name") {
            faults.push(format!("{code} has no Language-Name"));
        }
        let rule = catalog.header.get("Plural-Forms");
        let Some(rule) = rule.and_then(|r| Plural::from_header(r)) else {
            faults.push(format!(
                "{code} counts in {rule:?}, which this reader does not know"
            ));
            continue;
        };
        let forms = (0..200).map(|n| rule.form(n)).max().unwrap_or(0) + 1;
        let mut seen = BTreeSet::new();
        for entry in &catalog.entries {
            if !seen.insert(&entry.msgid) {
                faults.push(format!("{code} has {:?} twice", entry.msgid));
            }
            if entry.plural.is_some() && entry.forms.len() != forms {
                faults.push(format!(
                    "{code}: {:?} has {} forms and its language counts in {forms}",
                    entry.msgid,
                    entry.forms.len()
                ));
            }
        }
    }
    assert!(
        faults.is_empty(),
        "{} faults:\n{}",
        faults.len(),
        faults.join("\n")
    );
}

/// ⚠ Machine-dependent for Chinese, Japanese and Korean, which draw from fonts of the system: only a Windows install is sure to have them.
#[test]
fn every_character_a_built_in_catalog_writes_is_in_the_fonts() {
    let (definitions, _) = crate::app::gpu::font_definitions(&crate::state::FontChoice::Default);
    let mut fonts = egui::epaint::text::Fonts::new(Default::default(), definitions);
    let mut faults = Vec::new();
    for (code, catalog) in built_in() {
        if cfg!(not(target_os = "windows")) && ["ja", "ko", "zh"].contains(&code) {
            continue;
        }
        let written: BTreeSet<char> = catalog
            .entries
            .iter()
            .flat_map(|e| e.forms.iter())
            .chain(catalog.header.get("Language-Name"))
            .flat_map(|text| text.chars())
            .filter(|c| !c.is_ascii() && !c.is_whitespace())
            .collect();
        let boxes: String = written
            .into_iter()
            .filter(|c| {
                [egui::FontFamily::Proportional, egui::FontFamily::Monospace]
                    .into_iter()
                    .any(|family| !fonts.has_glyph(&egui::FontId::new(14.0, family), *c))
            })
            .collect();
        if !boxes.is_empty() {
            faults.push(format!("{code} draws these as a box: {boxes}"));
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}

#[test]
fn a_text_with_a_name_in_braces_is_asked_for_with_the_value_of_the_name() {
    let faults: Vec<String> = used::in_the_source()
        .iter()
        .filter(|u| u.macro_name == "tr" && !u.has_args)
        .filter(|u| arguments(&u.text).map_or(true, |names| !names.is_empty()))
        .map(|u| {
            format!(
                "{}:{}: {:?} has braces and no arguments",
                u.file.display(),
                u.line,
                u.text
            )
        })
        .collect();
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}

#[test]
fn a_text_no_catalog_has_is_shown_in_english_and_the_log_says_so_once() {
    const TEXT: &str = "A sentence that no catalog has been given";
    let _french = super::pin("fr");
    let mut shown = Vec::new();
    let log = crate::diagnostics::captured(|| {
        shown.push(super::tr(TEXT));
        shown.push(super::tr(TEXT));
    })
    .snapshot()
    .records;
    assert_eq!(shown, [TEXT, TEXT]);
    let said: Vec<_> = log
        .iter()
        .filter(|r| r.message.contains("no translation"))
        .collect();
    assert_eq!(
        said.len(),
        1,
        "once, not once for each time it is drawn: {log:?}"
    );
    assert!(
        said[0].message.contains(TEXT) || format!("{:?}", said[0]).contains(TEXT),
        "the line names the text: {:?}",
        said[0]
    );
}
