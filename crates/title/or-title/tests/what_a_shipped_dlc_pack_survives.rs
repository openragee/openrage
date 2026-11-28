//! Every DLC document a real install ships, written back and compared.
//!
//! Point `OPENRAGE_GAME_ROOT` at one or more install roots, `;`-separated.
//! ⚠ The install is opened read-only and nothing here writes into one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, ExtractionProgress, Gta5RetailCryptoProvider};
use or_io::container::rpf::RpfArchive;
use or_io::container::Gta5Keys;
use or_title::dlc::{ContentXml, DlcList, SetupData};
use or_title::Game;

const WHAT: &str = "what_a_shipped_dlc_pack_survives";

// ⚠ **This walks the XML itself rather than reusing the reader under test.**
// A census built on `or-title`'s own tree would agree with it by construction:
// an element the reader drops would be missing from both sides and the
// comparison would report nothing lost.

/// Every element in a document as `path | attrs | text`, counted.
fn elements(xml: &[u8]) -> BTreeMap<String, usize> {
    use quick_xml::events::Event;

    let body = xml.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(xml);
    let mut reader = quick_xml::Reader::from_reader(body);
    reader.config_mut().trim_text(false);

    let mut out: BTreeMap<String, usize> = BTreeMap::new();
    let mut buf = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut text: Vec<String> = Vec::new();
    let mut attrs: Vec<String> = Vec::new();

    let key = |path: &[String], attrs: &str, text: &str| {
        format!("{} |{}| {}", path.join("/"), attrs, normalise(text))
    };

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => break,
            Ok(Event::Start(e)) => {
                path.push(local(e.name().as_ref()));
                attrs.push(attr_key(&e));
                text.push(String::new());
            }
            Ok(Event::Empty(e)) => {
                path.push(local(e.name().as_ref()));
                *out.entry(key(&path, &attr_key(&e), "")).or_default() += 1;
                path.pop();
            }
            Ok(Event::Text(t)) => {
                if let Some(top) = text.last_mut() {
                    top.push_str(&String::from_utf8_lossy(t.as_ref()));
                }
            }
            Ok(Event::CData(t)) => {
                if let Some(top) = text.last_mut() {
                    top.push_str(&String::from_utf8_lossy(t.as_ref()));
                }
            }
            Ok(Event::GeneralRef(r)) => {
                if let Some(top) = text.last_mut() {
                    let name = r.decode().unwrap_or_default().to_string();
                    let resolved = match r.resolve_char_ref() {
                        Ok(Some(ch)) => ch.to_string(),
                        _ => quick_xml::escape::resolve_predefined_entity(&name)
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("&{name};")),
                    };
                    top.push_str(&resolved);
                }
            }
            Ok(Event::End(_)) => {
                let body = text.pop().unwrap_or_default();
                let a = attrs.pop().unwrap_or_default();
                *out.entry(key(&path, &a, &body)).or_default() += 1;
                path.pop();
            }
            _ => {}
        }
        buf.clear();
    }
    out
}

fn local(name: &[u8]) -> String {
    let local = match name.iter().rposition(|&b| b == b':') {
        Some(pos) => &name[pos + 1..],
        None => name,
    };
    String::from_utf8_lossy(local).to_ascii_lowercase()
}

fn attr_key(e: &quick_xml::events::BytesStart<'_>) -> String {
    let mut pairs: Vec<String> = e
        .attributes()
        .flatten()
        .map(|a| {
            let v = String::from_utf8_lossy(&a.value);
            let v = quick_xml::escape::unescape(&v)
                .map(|s| s.into_owned())
                .unwrap_or_else(|_| v.into_owned());
            format!("{}={}", local(a.key.as_ref()), v.trim())
        })
        .collect();
    pairs.sort();
    pairs.join(",")
}

/// ⚠ **Text is compared with its whitespace collapsed**, because indentation
/// is the one difference a writer is entitled to and every shipped document
/// disagrees with every other about it — `content.xml` mixes tabs and spaces
/// inside a single list.
fn normalise(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Default)]
struct Tally {
    documents: usize,
    model_round_trips: usize,
    elements: usize,
    lost: BTreeMap<String, usize>,
    added: BTreeMap<String, usize>,
    examples: BTreeMap<String, Vec<String>>,
    /// Elements the written document has and the shipped one does not, that
    /// carry text — the writer inventing a value rather than an empty shape.
    added_text: Vec<String>,
    /// Documents where at least one element went missing.
    documents_with_loss: usize,
}

impl Tally {
    fn compare(&mut self, what: &str, shipped: &[u8], written: &str) {
        self.documents += 1;
        let before = elements(shipped);
        let after = elements(written.as_bytes());
        self.elements += before.values().sum::<usize>();

        let mut lost_here = 0;
        for (k, n) in &before {
            let got = after.get(k).copied().unwrap_or(0);
            if got < *n {
                lost_here += n - got;
                *self.lost.entry(shape(k)).or_default() += n - got;
                let slot = self.examples.entry(shape(k)).or_default();
                if slot.len() < 2 {
                    slot.push(format!("{what} LOST {:?}", k));
                }
            }
        }
        for (k, n) in &after {
            let had = before.get(k).copied().unwrap_or(0);
            if had < *n {
                *self.added.entry(shape(k)).or_default() += n - had;
                if !k.rsplit("| ").next().unwrap_or("").is_empty() && self.added_text.len() < 10 {
                    self.added_text.push(format!("{what}: {k}"));
                }
            }
        }
        if lost_here > 0 {
            self.documents_with_loss += 1;
            if self.documents_with_loss <= 3 {
                eprintln!("  first losses in {what}: {lost_here} element(s)");
            }
        }
    }
}

/// The element path without its value, so a report names a kind of loss
/// rather than one of ten thousand filenames.
fn shape(key: &str) -> String {
    key.split(" |").next().unwrap_or(key).to_string()
}

fn open(path: &Path, keys: &Arc<Gta5Keys>) -> Option<RpfArchive> {
    let crypto = Arc::new(Gta5RetailCryptoProvider::new(Arc::clone(keys)));
    RpfArchive::open_with(path, crypto).ok()
}

fn ending_in(archive: &RpfArchive, suffix: &str) -> Vec<(String, Vec<u8>)> {
    archive
        .nodes
        .iter()
        .filter(|n| !n.is_dir())
        .filter(|n| {
            n.path()
                .to_ascii_lowercase()
                .replace('\\', "/")
                .ends_with(suffix)
        })
        .filter_map(|n| {
            archive
                .read_entry(n.entry_index())
                .ok()
                .map(|b| (n.path().to_string(), b))
        })
        .collect()
}

fn dlc_rpfs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root.join("update/x64/dlcpacks")) {
        for e in entries.flatten() {
            let p = e.path().join("dlc.rpf");
            if p.is_file() {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_dlc_document_an_install_ships_is_the_same_document_after_a_write() {
    let roots = or_test_support::game_roots();
    if roots.is_empty() {
        or_test_support::skip_install(
            WHAT,
            "no install named; set OPENRAGE_GAME_ROOT to one or more roots, `;`-separated",
        );
        return;
    }

    let mut tally = Tally::default();
    // extension -> fileType -> how many shipped entries spell it that way.
    let mut by_extension: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut packs_found = 0usize;
    let mut packs_unreadable: Vec<String> = Vec::new();
    let mut change_sets = 0usize;
    let mut map_change_sets = 0usize;
    let mut contents_elements = 0usize;
    let mut change_set_groups = 0usize;
    let mut installs = 0usize;

    for root in &roots {
        // ⚠ Asking the title rather than naming an executable: Enhanced ships
        // `GTA5_Enhanced.exe`, so a hard-coded `GTA5.exe` skips it and the
        // report says "no install" about an install that is right there.
        let Some(exe) = Game::detect_from_root(root).and_then(|game| game.find_executable(root))
        else {
            or_test_support::skip_install(
                WHAT,
                format_args!("{} has no executable to take keys from", root.display()),
            );
            continue;
        };
        let progress = |_: ExtractionProgress| {};
        let Ok(keys) = extract_keys(&exe, &progress) else {
            or_test_support::skip_install(
                WHAT,
                format_args!("key extraction failed for {}", exe.display()),
            );
            continue;
        };
        let keys = Arc::new(keys);
        installs += 1;

        for rpf in dlc_rpfs(root) {
            packs_found += 1;
            let label = rpf
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();

            let Some(archive) = open(&rpf, &keys) else {
                packs_unreadable.push(format!("{label}: archive did not open"));
                continue;
            };
            let Ok(setup_bytes) = archive.read("setup2.xml") else {
                packs_unreadable.push(format!("{label}: no setup2.xml"));
                continue;
            };

            let setup = SetupData::parse(&setup_bytes)
                .unwrap_or_else(|e| panic!("{label}/setup2.xml did not parse: {e}"));
            change_set_groups += setup.content_change_set_groups.len();
            let written = setup.to_xml();
            let reparsed = SetupData::parse(written.as_bytes())
                .unwrap_or_else(|e| panic!("{label}/setup2.xml did not re-parse: {e}"));
            assert_eq!(setup, reparsed, "{label}/setup2.xml lost a field");
            assert_eq!(
                written,
                reparsed.to_xml(),
                "{label}/setup2.xml does not survive a second write, so the \
                 writer drifts rather than converging"
            );
            tally.model_round_trips += 1;
            tally.compare(&format!("{label}/setup2.xml"), &setup_bytes, &written);

            let dat = if setup.dat_file.is_empty() {
                "content.xml".to_string()
            } else {
                setup.dat_file.clone()
            };
            if let Ok(content_bytes) = archive.read(&dat) {
                let content = ContentXml::parse(&content_bytes)
                    .unwrap_or_else(|e| panic!("{label}/{dat} did not parse: {e}"));
                change_sets += content.content_change_sets.len();
                map_change_sets += content
                    .content_change_sets
                    .iter()
                    .map(|c| c.map_change_set_data.len())
                    .sum::<usize>();
                contents_elements += content
                    .data_files
                    .iter()
                    .filter(|d| !d.contents.is_empty())
                    .count();
                for df in &content.data_files {
                    let ext = df
                        .filename
                        .rsplit('.')
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    *by_extension
                        .entry(ext)
                        .or_default()
                        .entry(df.file_type.clone())
                        .or_default() += 1;
                }

                let written = content.to_xml();
                let reparsed = ContentXml::parse(written.as_bytes())
                    .unwrap_or_else(|e| panic!("{label}/{dat} did not re-parse: {e}"));
                assert_eq!(content, reparsed, "{label}/{dat} lost a field");
                assert_eq!(
                    written,
                    reparsed.to_xml(),
                    "{label}/{dat} does not survive a second write"
                );
                tally.model_round_trips += 1;
                tally.compare(&format!("{label}/{dat}"), &content_bytes, &written);
            }
        }

        for archive_name in ["common.rpf", "update/update.rpf"] {
            let path = root.join(archive_name);
            if !path.is_file() {
                continue;
            }
            let Some(archive) = open(&path, &keys) else {
                continue;
            };
            for (inner, bytes) in ending_in(&archive, "dlclist.xml") {
                let list = DlcList::parse(&bytes)
                    .unwrap_or_else(|e| panic!("{archive_name}/{inner} did not parse: {e}"));
                assert!(
                    !list.paths.is_empty(),
                    "{archive_name}/{inner} listed no packs, which is not a shipped dlclist"
                );
                let written = list.to_xml();
                let reparsed = DlcList::parse(written.as_bytes()).expect("dlclist re-parses");
                assert_eq!(list, reparsed, "{archive_name}/{inner} lost a path");
                assert_eq!(
                    written,
                    reparsed.to_xml(),
                    "{archive_name}/{inner} does not survive a second write"
                );
                tally.model_round_trips += 1;
                tally.compare(&format!("{archive_name}/{inner}"), &bytes, &written);
            }
        }
    }

    eprintln!(
        "parsed {} of {packs_found} shipped dlc.rpf across {installs} install(s)",
        packs_found - packs_unreadable.len()
    );
    eprintln!(
        "round-tripped {} documents, {} elements compared, {} lost across {} document(s)",
        tally.documents,
        tally.elements,
        tally.lost.values().sum::<usize>(),
        tally.documents_with_loss
    );
    eprintln!(
        "exercised {change_sets} change sets, {map_change_sets} map change sets, \
         {change_set_groups} change-set groups, {contents_elements} `contents` elements"
    );
    if !tally.lost.is_empty() {
        eprintln!("lost by element:");
        for (shape, n) in &tally.lost {
            eprintln!("  {n:>6}  {shape}");
            for ex in tally.examples.get(shape).into_iter().flatten() {
                eprintln!("            e.g. {ex}");
            }
        }
    }
    if !tally.added.is_empty() {
        eprintln!("added by element:");
        for (shape, n) in &tally.added {
            eprintln!("  {n:>6}  {shape}");
        }
    }

    assert!(
        packs_unreadable.is_empty(),
        "{} of {packs_found} packs could not be read, so this suite measured only the rest:\n  {}",
        packs_unreadable.len(),
        packs_unreadable.join("\n  ")
    );

    // ⚠ A check that compares only what it reached reports green by reaching
    // less: no install, an install whose archives do not open, or a parse that
    // returns an empty document. The counts are asserted so each of those fails.
    assert!(
        packs_found >= 20,
        "{packs_found} packs is not a retail install; the enumeration is wrong, not the game"
    );
    assert_eq!(
        tally.model_round_trips, tally.documents,
        "every document compared must also have round-tripped through the model"
    );
    assert!(
        tally.elements > 10_000,
        "only {} elements compared over {} documents — the census walked almost nothing",
        tally.elements,
        tally.documents
    );
    assert!(
        change_sets > 0 && map_change_sets > 0 && change_set_groups > 0,
        "the constructs the model used to drop were never exercised: \
         {change_sets} change sets, {map_change_sets} map change sets, \
         {change_set_groups} groups — a pass here would be about nothing"
    );
    assert!(
        contents_elements > 0,
        "no `<contents>` element was exercised, and it is one of the fields this work added"
    );

    // ⚠ **`file_type_for` is a claim about the game, so it is checked against
    // the game.** Every extension it names must be spelled that way by a
    // majority of the entries a retail install ships.
    for (ext, spellings) in &by_extension {
        let total: usize = spellings.values().sum();
        let mut top: Vec<(&String, &usize)> = spellings.iter().collect();
        top.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        let shown: Vec<String> = top
            .iter()
            .take(3)
            .map(|(k, n)| format!("{k}={n}"))
            .collect();
        eprintln!("  .{ext:<6} {total:>6}  {}", shown.join(" "));
    }

    let majority = |ext: &str| -> Option<(String, usize, usize)> {
        let spellings = by_extension.get(ext)?;
        let (name, n) = spellings.iter().max_by_key(|(_, n)| **n)?;
        Some((name.clone(), *n, spellings.values().sum()))
    };

    let mut ruled_on = 0;
    for ext in ["rpf", "ityp", "meta", "xml"] {
        let Some((most_common, n, total)) = majority(ext) else {
            panic!("no shipped pack registers a `.{ext}`, so this row measures nothing");
        };
        match or_title::dlc::file_type_for(&format!("x.{ext}")) {
            Some(claimed) => {
                assert_eq!(
                    claimed, most_common,
                    "`.{ext}`: shipped packs spell it `{most_common}` {n} of {total} times"
                );
                assert!(
                    n * 2 > total,
                    "`file_type_for` settles `.{ext}` as `{claimed}`, and shipped packs \
                     only agree {n} of {total} times — that is not a majority and the \
                     claim should be withdrawn"
                );
            }
            None => assert!(
                n * 2 <= total,
                "`file_type_for` says `.{ext}` settles nothing, and shipped packs spell it \
                 `{most_common}` {n} of {total} times — which does settle it"
            ),
        }
        ruled_on += 1;
    }
    assert_eq!(ruled_on, 4, "every extension with a ruling must be checked");
    eprintln!("file_type_for: {ruled_on} extension rulings corroborated against shipped packs");

    // ⚠ **A value is never allowed to go missing; a container may be merged.**
    // Every value in these documents lives in an `<Item>`, so refusing a lost
    // `/item` outright keeps `MERGED` from hiding data loss as a merge.
    let lost_values: Vec<(&String, &usize)> = tally
        .lost
        .iter()
        .filter(|(shape, _)| shape.ends_with("/item"))
        .collect();
    assert!(
        lost_values.is_empty(),
        "values went missing, which no normalisation explains: {lost_values:?}"
    );

    for (shape, n) in &tally.lost {
        let Some((_, per_install, why)) = MERGED.iter().find(|(s, _, _)| s == shape) else {
            panic!(
                "{n} `{shape}` element(s) did not survive the round trip and nothing \
                 explains it; every other shape in these documents round-trips"
            );
        };
        // ⚠ Per install, because every install ships the same pack: an
        // absolute cap here passes on one machine and fails on a machine with
        // two installs, which reads as a regression in the writer.
        let cap = per_install * installs;
        assert!(
            n <= &cap,
            "`{shape}` lost {n} element(s) over {installs} install(s), and only \
             {cap} is accounted for by: {why}"
        );
    }

    assert!(
        tally.added_text.is_empty(),
        "the writer invented text a document did not carry: {:?}",
        tally.added_text
    );
}

/// The only elements a shipped document has that a written one does not, with
/// a per-install cap and the reason each is a shape rather than a value.
///
/// ⚠ **`/item` paths may never appear here**; the assertion above refuses
/// them.
const MERGED: &[(&str, usize, &str)] = &[(
    "cdatafilemgr__contentsofdatafilexml/datafiles",
    1,
    "mplowrider's content.xml carries two <dataFiles> sections. The model holds \
     one list of entries and the writer emits one section, so the count of \
     container elements drops by one while every <Item> inside both survives.",
)];
