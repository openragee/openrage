//! GTA VI's data XML read through GTA VI's library. Every top-level row of a build that is not a
//! nested archive is read, deciphered where it has a cipher record and walked by its chunk table,
//! and is a data XML document where its text starts with a tag; each is read through the library
//! the user's parser dump builds. Measured on the Xbox build 1.0.2.1 by this suite on 2026-11-26,
//! on the round-36 tree: 491 documents, 406 whose root the library holds (by name or by the hash
//! of the XML's spelling), 405 of them typed and 398 typed with no structure the library lacks.
//! Every document opens; the 7 typed documents that name a structure the library lacks read that
//! element as its base, and the 1 rooted document whose such element carries an attribute its base
//! has no member for opens as its tree, so that the attribute is kept. A save of a typed document
//! writes the XML's own names, never a `hash_` spelling, and states every bit its source names.
//!
//! Install-gated and game-derived; it skips without:
//!   `OPENRAGE_GTA6_KEYS`    the `gtavi_magic.dat` key snapshot with content keys (never committed)
//!   `OPENRAGE_GTA6_BUILD`   the build root holding `*.rpf` and `pack/*/*.rpf`
//!   `OPENRAGE_GTA6_SCHEMA`  GTA VI's schema snapshot, built from the user's parser dump

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_io::meta::instance::parser::{member_counts, parse_typed_or_tree};
use or_io::meta::{MetaInstance, SchemaLibrary, Value};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const SCHEMA_ENV: &str = "OPENRAGE_GTA6_SCHEMA";
/// A body stored larger than this is not read; no data XML comes near it.
const CAP: u64 = 64 << 20;
const DOCUMENTS: usize = 491;
const ROOTED: usize = 406;
/// The rooted documents a lenient read still refuses to type, which open as their tree instead:
/// an element of a structure the library lacks with an attribute its base has no member for.
const KEPT_UNTYPED: usize = 1;
/// The rooted documents that name no structure the library lacks.
const WHOLLY_TYPED: usize = 398;
/// The share of the wholly typed documents' members a structure states, measured by this suite on
/// 2026-11-26: 225,912 of 244,192 (92.5 %); the check allows 2 points less.
const MEMBERS_TYPED: f64 = 0.925;

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// Every `*.rpf` directly under `root`, then every `pack/*/*.rpf`, sorted.
fn build_archives(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut rpfs_in = |dir: &Path| {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rpf") {
                out.push(path);
            }
        }
    };
    rpfs_in(root);
    for pack in std::fs::read_dir(root.join("pack"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if pack.path().is_dir() {
            rpfs_in(&pack.path());
        }
    }
    out.sort();
    out
}

/// Whether `data` is XML: `<` and a name or `?` after an optional byte order mark and white space.
fn is_xml(data: &[u8]) -> bool {
    let text = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data);
    let start = text
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'));
    match start.and_then(|at| text.get(at..at + 2)) {
        Some([b'<', next]) => *next == b'?' || *next == b'_' || next.is_ascii_alphabetic(),
        _ => false,
    }
}

/// The name of the first element of `text`, past a declaration and comments.
fn root_of(text: &str) -> Option<&str> {
    let mut rest = text.trim_start_matches('\u{feff}');
    loop {
        let after = rest.trim_start().strip_prefix('<')?;
        let skipped = match after.as_bytes().first() {
            Some(b'!') if after.starts_with("!--") => after.find("-->").map(|at| at + 3),
            Some(b'?' | b'!') => after.find('>').map(|at| at + 1),
            _ => {
                let end = after.find(|c: char| c.is_whitespace() || c == '>' || c == '/')?;
                return Some(&after[..end]);
            }
        };
        rest = &after[skipped?..];
    }
}

/// Whether an element of `inst` names by `type` a structure `library` lacks, read as its base.
fn lacks_a_structure(library: &SchemaLibrary, inst: &MetaInstance) -> bool {
    fn value(library: &SchemaLibrary, v: &Value) -> bool {
        match v {
            Value::Struct(inner) => lacks_a_structure(library, inner),
            Value::Pointer(Some(inner)) => {
                library.resolve_any(&inner.type_name).is_none() || lacks_a_structure(library, inner)
            }
            Value::Array(items) => items.iter().any(|i| value(library, i)),
            Value::Map(pairs) => pairs.iter().any(|(_, v)| value(library, v)),
            _ => false,
        }
    }
    inst.fields.iter().any(|(_, v)| value(library, v))
}

/// How many members named each tag `inst` holds as a bitset with no bit set.
fn zero_bitsets(inst: &MetaInstance, out: &mut HashMap<String, usize>) {
    fn value(name: &str, v: &Value, out: &mut HashMap<String, usize>) {
        match v {
            Value::Bitset { bits: 0, .. } => *out.entry(name.to_string()).or_default() += 1,
            Value::Struct(inner) | Value::Pointer(Some(inner)) => zero_bitsets(inner, out),
            Value::Array(items) => items.iter().for_each(|i| value("", i, out)),
            Value::Map(pairs) => pairs.iter().for_each(|(_, v)| value("", v, out)),
            _ => {}
        }
    }
    inst.fields.iter().for_each(|(n, v)| value(n, v, out));
}

/// How many `<tag>` elements of `text` are empty or state the number 0.
fn empty_or_zero(text: &str, tag: &str) -> usize {
    let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
    let zero = |v: &str| {
        let hex = v.strip_prefix("0x").or(v.strip_prefix("0X"));
        v.is_empty()
            || v == "0"
            || hex.is_some_and(|h| !h.is_empty() && h.bytes().all(|b| b == b'0'))
    };
    let elements = text.match_indices(&open).filter_map(|(at, _)| {
        let rest = &text[at + open.len()..];
        // The tag ends where its name does: `<Flags` is not `<FlagsOther`.
        rest.starts_with(|c: char| c == '>' || c == '/' || c.is_whitespace())
            .then_some(rest)
    });
    elements
        .filter(|rest| {
            let Some(end) = rest.find('>') else {
                return false;
            };
            if rest[..end].ends_with('/') {
                return true;
            }
            let body = &rest[end + 1..];
            body.find('<')
                .is_some_and(|stop| zero(body[..stop].trim()) && body[stop..].starts_with(&close))
        })
        .count()
}

#[derive(Default)]
struct Tally {
    rows: usize,
    unread: usize,
    documents: usize,
    opened: usize,
    rooted: usize,
    typed: usize,
    wholly_typed: usize,
    members: usize,
    members_untyped: usize,
    saved_hash_names: Vec<String>,
    /// A document whose save states no bit for a bitset its source names bits of, with the tag.
    bits_cleared: Vec<String>,
    saves_at_a_fixed_point: usize,
    /// A document whose root the library holds that reads as no typed tree, with what refused it.
    rooted_untyped: Vec<String>,
}

impl Tally {
    fn document(&mut self, library: &SchemaLibrary, text: &str, at: &str) {
        self.documents += 1;
        let rooted = root_of(text).is_some_and(|root| library.resolve_any(root).is_some());
        self.rooted += usize::from(rooted);
        let Ok((instance, untyped)) = parse_typed_or_tree(library, text.as_bytes()) else {
            return;
        };
        self.opened += 1;
        if let Some(refused) = untyped {
            if rooted {
                self.rooted_untyped.push(format!("{at}: {refused}"));
            }
            return;
        }
        self.typed += 1;
        if !lacks_a_structure(library, &instance) {
            self.wholly_typed += 1;
            let (typed, untyped) = member_counts(&instance);
            self.members += typed + untyped;
            self.members_untyped += untyped;
        }
        let mut zeros = HashMap::new();
        zero_bitsets(&instance, &mut zeros);
        for (tag, count) in zeros.iter().filter(|(tag, _)| !tag.is_empty()) {
            if *count > empty_or_zero(text, tag) {
                self.bits_cleared.push(format!("{at} <{tag}>"));
            }
        }
        let Ok(saved) = or_io::meta::write_meta(&instance) else {
            return;
        };
        if saved.contains("<hash_") && !text.contains("<hash_") {
            self.saved_hash_names.push(at.to_string());
        }
        let again = parse_typed_or_tree(library, saved.as_bytes())
            .ok()
            .and_then(|(i, _)| or_io::meta::write_meta(&i).ok());
        if again.as_deref() == Some(saved.as_str()) {
            self.saves_at_a_fixed_point += 1;
        }
    }
}

#[test]
fn the_builds_data_xml_reads_typed_through_gta_vis_library() {
    let what = "GTA VI's data XML through GTA VI's library";
    let (Some(snapshot), Some(build), Some(schema)) = (
        env_path(GTA6_SNAPSHOT_ENV),
        env_path(BUILD_ENV),
        env_path(SCHEMA_ENV),
    ) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV}, {BUILD_ENV} and {SCHEMA_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    if !keys.has_content_keys() {
        skip_install(
            what,
            format_args!("the snapshot at {GTA6_SNAPSHOT_ENV} carries no content section"),
        );
        return;
    }
    let library = or_io::meta::load_snapshot(&schema).expect("GTA VI's schema snapshot loads");
    let archives = build_archives(&build);
    assert!(!archives.is_empty(), "no .rpf under {}", build.display());

    let mut t = Tally::default();
    for path in &archives {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys)
            .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        for entry in &mount.archive.entries {
            // No 24-byte row, or a nested archive.
            if matches!(entry.class, None | Some(0)) || entry.stored.is_none_or(|s| s > CAP) {
                continue;
            }
            t.rows += 1;
            let Ok(data) = mount.read_entry_decrypted(&mut file, entry, &keys) else {
                t.unread += 1;
                continue;
            };
            if !is_xml(&data) {
                continue;
            }
            let at = format!("{} entry {}", path.display(), entry.index);
            t.document(&library, &String::from_utf8_lossy(&data), &at);
        }
    }

    let share = 1.0 - t.members_untyped as f64 / t.members.max(1) as f64;
    println!(
        "data XML: {} rows read ({} unread), {} documents, {} opened, {} with a root structure, \
         {} typed, {} wholly; {} of {} members of those typed ({:.1} %); {} saves at a fixed point",
        t.rows - t.unread,
        t.unread,
        t.documents,
        t.opened,
        t.rooted,
        t.typed,
        t.wholly_typed,
        t.members - t.members_untyped,
        t.members,
        share * 100.0,
        t.saves_at_a_fixed_point,
    );
    assert!(
        t.saved_hash_names.is_empty(),
        "a save wrote a hash_ spelling for a name the XML spells: {:?}",
        &t.saved_hash_names[..t.saved_hash_names.len().min(5)]
    );
    assert!(
        t.bits_cleared.is_empty(),
        "{} bitsets the source names bits of read as no bit: {:?}",
        t.bits_cleared.len(),
        &t.bits_cleared[..t.bits_cleared.len().min(5)]
    );
    assert_eq!(t.documents, DOCUMENTS, "data XML documents");
    assert_eq!(t.opened, DOCUMENTS, "every data XML document opens");
    assert_eq!(t.rooted, ROOTED, "documents whose root the library holds");
    assert_eq!(
        t.typed,
        ROOTED - KEPT_UNTYPED,
        "rooted documents read typed; untyped: {:?}",
        t.rooted_untyped
    );
    assert_eq!(
        t.wholly_typed, WHOLLY_TYPED,
        "documents naming no structure the library lacks"
    );
    assert_eq!(
        t.saves_at_a_fixed_point, t.typed,
        "typed saves at a fixed point"
    );
    assert!(
        share >= MEMBERS_TYPED - 0.02,
        "{:.1} % of the members typed, measured {:.1} % before",
        share * 100.0,
        MEMBERS_TYPED * 100.0
    );
}

#[test]
fn a_bitset_read_as_no_bit_is_told_from_one_its_source_leaves_empty() {
    let text = "<a><Flags>ZQ_A</Flags><Flags/><Flags >0x00</Flags><FlagsOther/></a>";
    assert_eq!(empty_or_zero(text, "Flags"), 2);
    assert_eq!(
        empty_or_zero("<Flags>0x1</Flags><Flags>0</Flags>", "Flags"),
        1
    );
}

#[test]
fn a_root_is_found_past_a_declaration_and_a_comment() {
    let text = "\u{feff}<?xml version=\"1.0\"?>\n<!-- a > b -->\n<!DOCTYPE z>\n<CZq a=\"1\"/>";
    assert_eq!(root_of(text), Some("CZq"));
    assert_eq!(root_of("<Zq>"), Some("Zq"));
    assert_eq!(root_of("no tag"), None);
    assert!(is_xml(b"\xEF\xBB\xBF\r\n<?xml?>"));
    assert!(!is_xml(b"<!-- first -->"));
}
