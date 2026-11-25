use super::*;
use crate::container::rpf::gta6_strings::tests::clean_keys;
use crate::meta::binary::hash_index::default_hash_index;
use crate::meta::binary::pso::string_blob::CATEGORY_1_RUN;
use crate::meta::binary::pso::string_names::registered_strings;
use crate::meta::binary::pso::table_form::tests::{body, member, open, Class};
use crate::meta::binary::{write_meta_binary, BinaryFormat};
use crate::meta::instance::value::{HashedString, Value};

const ROOT: u32 = 0x2000_0008;
const PLAIN: u32 = 0x6666_0000;
const NS: u32 = 0x6666_0001;
const WIDE: u32 = 0x6666_0002;
/// The run of category 0x1E, which an `NsStringHash` of byte 9 = 30 names.
const NS_RUN: u32 = 0x11F;

type Runs<'a> = &'a [(u32, &'a [&'a str])];

fn name(hash: u32) -> String {
    default_hash_index().name_or_literal(hash)
}

/// An `LSNS` blob of the runs `(id, strings)`, each laid out by the measured rule and enciphered
/// with `keys` where its id is.
fn blob(runs: Runs, keys: &GtaViKeys) -> Vec<u8> {
    let datas: Vec<Vec<u8>> = (runs.iter())
        .map(|(id, strings)| {
            let mut d: Vec<u8> = strings
                .iter()
                .flat_map(|s| [s.as_bytes(), b"\0"].concat())
                .collect();
            d.resize((d.len() + 1).next_multiple_of(16), 0);
            if string_blob::is_enciphered(*id) {
                keys.string_encrypt(&mut d)
                    .expect("the string key enciphers");
            }
            d
        })
        .collect();
    let mut b = (runs.len() as u32).to_le_bytes().to_vec();
    for ((id, _), d) in runs.iter().zip(&datas) {
        b.extend(id.to_le_bytes());
        b.extend((d.len() as u32).to_le_bytes());
    }
    datas.iter().for_each(|d| b.extend_from_slice(d));
    b
}

/// `bytes` with its `LSNS` body replaced by `blob`, the `CHKS` length and checksum made good.
fn with_lsns(bytes: &[u8], blob: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[p + 4..p + 8].try_into().unwrap()) as usize;
        if &bytes[p..p + 4] == b"LSNS" {
            out.extend_from_slice(b"LSNS");
            out.extend_from_slice(&(blob.len() as u32 + 8).to_be_bytes());
            out.extend_from_slice(blob);
        } else {
            out.extend_from_slice(&bytes[p..p + len]);
        }
        p += len;
    }
    let n = out.len();
    out[n - 12..n - 8].copy_from_slice(&(n as u32).to_be_bytes());
    super::super::chks::write_checksum(&mut out);
    out
}

/// A document of a `StringHash` holding `zq_held` and an `NsStringHash` of byte 9 = 30 holding
/// `zq_ns_held`, with the `LSNS` runs `runs`.
fn document(runs: Runs, keys: &GtaViKeys) -> Vec<u8> {
    let mut ns = member(NS, 4, 0x4D, 0, 0);
    ns[9] = 30;
    let root = Class {
        name: ROOT,
        size: 8,
        members: vec![member(PLAIN, 0, 0x4A, 0, 0), ns],
    };
    let held = [
        crate::string_hash(b"zq_held"),
        crate::literal_string_hash(b"zq_ns_held"),
    ];
    let data = held.iter().flat_map(|h| h.to_le_bytes()).collect();
    with_lsns(&body(&[root], &[(ROOT, data)], 1), &blob(runs, keys))
}

fn hash_of(value: Option<&Value>) -> Option<u32> {
    match value? {
        Value::Hash(h) => Some(h.hash),
        Value::String(s) => Some(HashedString::from_xml_text(s).hash),
        _ => None,
    }
}

fn set(instance: &mut MetaInstance, member: u32, value: Value) {
    *instance.get_mut(&name(member)).expect("the member") = value;
}

#[test]
fn a_new_text_goes_to_the_run_its_member_names_and_reads_back() {
    let keys = clean_keys();
    let runs: Runs = &[
        (0x101, &["zq_clear"][..]),
        (CATEGORY_1_RUN, &["zq_held"][..]),
        (NS_RUN, &["zq_ns_held"][..]),
    ];
    let doc = document(runs, &keys);
    let (mut instance, layout) = open(&doc).expect("it reads");
    let unedited = write_adding_strings(&layout, &instance, None).expect("a no-op needs no keys");
    assert_eq!(unedited, doc);

    set(&mut instance, PLAIN, Value::String("zq_new".into()));
    set(&mut instance, NS, Value::String("zq_ns_new".into()));
    let err = (write_adding_strings(&layout, &instance, None).expect_err("no keys")).to_string();
    assert!(
        err.contains("no key snapshot") && err.contains("zq_new"),
        "{err}"
    );
    assert!(
        write_meta_binary(BinaryFormat::Pso, Some(&layout), &instance).is_err(),
        "the save with no keys refuses it too"
    );

    let saved = write_adding_strings(&layout, &instance, Some(&keys)).expect("it saves");
    let strings: Vec<(u32, String)> = (registered_strings(&saved, Some(&keys)).expect("it reads"))
        .into_iter()
        .map(|s| (s.category, s.text))
        .collect();
    assert_eq!(
        strings,
        [
            (1, "zq_held".to_string()),
            (1, "zq_new".into()),
            (0x1E, "zq_ns_held".into()),
            (0x1E, "zq_ns_new".into())
        ]
    );
    let (again, layout) = open(&saved).expect("it reopens");
    assert_eq!(
        hash_of(again.get(&name(PLAIN))),
        Some(crate::string_hash(b"zq_new"))
    );
    assert_eq!(
        hash_of(again.get(&name(NS))),
        Some(crate::literal_string_hash(b"zq_ns_new"))
    );
    let n = saved.len();
    assert_eq!(&saved[n - 12..n - 8], &(n as u32).to_be_bytes());
    let stored = u32::from_be_bytes(saved[n - 8..n - 4].try_into().unwrap());
    assert_eq!(super::super::chks::checksum(&saved), Some(stored));
    // Saved again, the texts are the document's own: a fixed point with keys or without.
    assert_eq!(
        write_adding_strings(&layout, &again, Some(&keys)).expect("it saves"),
        saved
    );
    assert_eq!(
        write_adding_strings(&layout, &again, None).expect("it saves"),
        saved
    );
}

#[test]
fn a_text_the_rules_do_not_cover_is_refused_by_name() {
    let keys = clean_keys();
    // The namespaced member's run is not in the document.
    let doc = document(&[(CATEGORY_1_RUN, &["zq_held"][..])], &keys);
    let (mut instance, layout) = open(&doc).expect("it reads");
    set(&mut instance, NS, Value::String("zq_ns_new".into()));
    let err =
        (write_adding_strings(&layout, &instance, Some(&keys)).expect_err("refused")).to_string();
    assert!(err.contains("which the document did not hold"), "{err}");

    // A text its run registers under another hash than the member holds.
    let doc = document(&[(NS_RUN, &["zq_ns_held"][..])], &keys);
    let (mut instance, layout) = open(&doc).expect("it reads");
    set(&mut instance, NS, Value::String("Zq_Ns_New".into()));
    let err =
        (write_adding_strings(&layout, &instance, Some(&keys)).expect_err("refused")).to_string();
    assert!(err.contains("registers under another hash"), "{err}");
    // A new hash with no text to give its run.
    set(
        &mut instance,
        NS,
        Value::Hash(HashedString::from_hash(0x1234_5678)),
    );
    let err =
        (write_adding_strings(&layout, &instance, Some(&keys)).expect_err("refused")).to_string();
    assert!(err.contains("no text that hashes to it"), "{err}");

    // A `StringHash` with no category-1 run is written as its hash alone, as it was before.
    let (mut instance, layout) = open(&doc).expect("it reads");
    set(&mut instance, PLAIN, Value::String("zq_new".into()));
    let saved = write_adding_strings(&layout, &instance, None).expect("no run takes the text");
    assert_eq!(
        registered_strings(&saved, Some(&keys))
            .expect("it reads")
            .len(),
        1
    );
}

/// A document of one 0x4F member of namespace 5, with the `LSNS` runs `runs`.
fn wide_document(runs: Runs) -> Vec<u8> {
    let mut wide = member(WIDE, 0, 0x4F, 0, 0);
    wide[9] = 5;
    let root = Class {
        name: ROOT,
        size: 8,
        members: vec![wide],
    };
    let blob = blob(runs, &clean_keys());
    with_lsns(
        &body(&[root], &[(ROOT, 7u64.to_le_bytes().to_vec())], 1),
        &blob,
    )
}

fn takes_a_new_wide_hash(runs: Runs) -> bool {
    let (mut instance, layout) = open(&wide_document(runs)).expect("it reads");
    set(&mut instance, WIDE, Value::UInt(0x1234_5678_9ABC_DEF0));
    write_adding_strings(&layout, &instance, None).is_ok()
}

#[test]
fn a_namespace_with_no_run_takes_a_new_wide_hash() {
    assert!(takes_a_new_wide_hash(&[]), "no run names namespace 5");
    assert!(
        takes_a_new_wide_hash(&[(0x107, &["zq_other"][..])]),
        "a run of another namespace"
    );
    assert!(
        !takes_a_new_wide_hash(&[(0x106, &["zq_text"][..])]),
        "namespace 5's run"
    );
    assert!(
        !takes_a_new_wide_hash(&[(0x206, &["zq_text"][..])]),
        "a run of namespace 5 of another kind"
    );
}

/// Install-gated: read at run time from the build and the key snapshot, skipped without them.
mod build {
    use std::fs::File;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::container::rpf::{Gta6Mount, GTA6_SNAPSHOT_ENV};
    use crate::meta::binary::parse_meta_binary_full;
    use crate::meta::binary::pso::sections::split_sections;
    use crate::meta::diagnostics::Diagnostics;
    use crate::meta::schema::SchemaLibrary;

    const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
    const CONTENT_CIPHER_TYPE: u16 = 17;
    const MAX_DEPTH: usize = 8;
    /// String sections are small; a larger entry is not inflated for them.
    const MAX_ENTRY: u64 = 4 << 20;
    /// How long a walk reads before it judges what it met, inside the slow tier's limit.
    const WALK_TIME: Duration = Duration::from_secs(600);
    /// The enciphered runs the build holds: 64 in `LSNS`, one in `STNS`, one `STFN`.
    const RUNS: usize = 66;
    /// The name hashes of the file names of the two archives that hold those runs.
    const PSO_ARCHIVES: [u32; 2] = [0xBC35_2010, 0x61CA_8E15];
    /// The hash values of one document a save tries as the one to take a new text.
    const TRIES: usize = 64;
    const ADDED: &str = "zq_openrage_added_text";

    fn keys_and_build(what: &str) -> Option<(GtaViKeys, PathBuf)> {
        let env_path = |var: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.exists())
        };
        let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV))
        else {
            or_test_support::skip_install(
                what,
                format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}"),
            );
            return None;
        };
        let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
        if !keys.has_string_keys() {
            or_test_support::skip_install(
                what,
                format_args!("the snapshot carries no string section; rebuild it"),
            );
            return None;
        }
        Some((keys, build))
    }

    fn archives(root: &Path) -> Vec<PathBuf> {
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
        // The archives that hold the PSO documents with string runs come first, so the walk meets
        // them before it times out in the largest archives a sorted walk reads first.
        out.sort_by_key(|path| {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            !name.is_some_and(|n| {
                PSO_ARCHIVES.contains(&crate::gta6::hash::name_hash32(n.as_bytes()))
            })
        });
        out
    }

    /// Hands `visit` each PSO document of the build, clear or content-ciphered, until it returns
    /// `true` or the walk's time runs out.
    fn each_document(build: &Path, keys: &GtaViKeys, visit: &mut dyn FnMut(&[u8]) -> bool) {
        let until = Instant::now() + WALK_TIME;
        for path in archives(build) {
            let mut file = File::open(&path).expect("open the archive");
            let Ok(mount) = Gta6Mount::open_root(&mut file, keys) else {
                continue;
            };
            if walk(&mut file, &mount, keys, 0, until, visit) {
                return;
            }
        }
    }

    fn walk(
        file: &mut File,
        mount: &Gta6Mount,
        keys: &GtaViKeys,
        depth: usize,
        until: Instant,
        visit: &mut dyn FnMut(&[u8]) -> bool,
    ) -> bool {
        if depth > MAX_DEPTH {
            return false;
        }
        for entry in &mount.archive.entries {
            if Instant::now() >= until {
                return true;
            }
            let clear = entry.cipher.is_none();
            if clear
                && entry.start.is_some()
                && (mount.entry_begins_archive(&mut *file, entry)).unwrap_or(false)
            {
                if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                    if walk(file, &child, keys, depth + 1, until, visit) {
                        return true;
                    }
                }
                continue;
            }
            let content =
                (entry.cipher.as_ref()).is_some_and(|c| c.cipher_type == CONTENT_CIPHER_TYPE);
            if (clear || content) && !entry.real.is_some_and(|r| r > MAX_ENTRY) {
                if let Ok(doc) = mount.read_entry_decrypted(&mut *file, entry, keys) {
                    if doc.starts_with(b"PSIN") && visit(&doc) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// The enciphered extents of a document's string sections, as stored: each enciphered run of an
    /// `LSNS`/`STNS` blob and an `STFN` buffer whole (`STDN` repeats `STNS`).
    fn enciphered_runs(doc: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for section in split_sections(doc).unwrap_or_default() {
            let tag = section.ident.ident().to_be_bytes();
            let body = &section.bytes[8..];
            if &tag == b"STFN" && body.len() >= 16 {
                out.push(body.to_vec());
            }
            if ![b"LSNS", b"STNS"].contains(&&tag) {
                continue;
            }
            for run in string_blob::parse(body).unwrap_or_default() {
                if run.enciphered && run.len >= 16 {
                    out.push(body[run.off..run.off + run.len as usize].to_vec());
                }
            }
        }
        out
    }

    #[test]
    fn every_enciphered_run_met_re_enciphers_byte_identical() {
        let Some((keys, build)) = keys_and_build("GTA VI PSO string cipher encrypt over a build")
        else {
            return;
        };
        let (mut met, mut identical) = (0usize, 0usize);
        each_document(&build, &keys, &mut |doc| {
            for stored in enciphered_runs(doc) {
                let mut run = stored.clone();
                keys.string_decrypt(&mut run)
                    .expect("the string key deciphers");
                keys.string_encrypt(&mut run)
                    .expect("the string key enciphers");
                met += 1;
                identical += usize::from(run == stored);
            }
            met >= RUNS
        });
        println!("string cipher encrypt: {identical} of {met} enciphered runs re-enciphered byte-identical");
        assert!(met > 0, "the walk met no enciphered string run");
        assert_eq!(
            identical, met,
            "every run deciphered and enciphered again is its stored bytes"
        );
    }

    /// The `n`th hash value of `value`'s tree, hash-table keys aside.
    fn nth_hash<'v>(value: &'v mut Value, n: &mut usize) -> Option<&'v mut Value> {
        if matches!(value, Value::Hash(_)) {
            if *n == 0 {
                return Some(value);
            }
            *n -= 1;
            return None;
        }
        match value {
            Value::Struct(inst) | Value::Pointer(Some(inst)) => {
                inst.fields.iter_mut().find_map(|(_, v)| nth_hash(v, n))
            }
            Value::Array(items) => items.iter_mut().find_map(|v| nth_hash(v, n)),
            _ => None,
        }
    }

    fn nth_hash_of(instance: &mut MetaInstance, mut n: usize) -> Option<&mut Value> {
        (instance.fields.iter_mut()).find_map(|(_, v)| nth_hash(v, &mut n))
    }

    fn holds_category_1(doc: &[u8]) -> bool {
        split_sections(doc).unwrap_or_default().iter().any(|s| {
            s.ident.ident().to_be_bytes() == *b"LSNS"
                && (string_blob::parse(&s.bytes[8..]).unwrap_or_default())
                    .iter()
                    .any(|r| r.id == CATEGORY_1_RUN)
        })
    }

    /// The document saved with one value of `doc` holding [`ADDED`], and which value; `None` when
    /// it does not save unedited or no tried value takes the text into the category-1 run.
    fn saved_with_a_text(doc: &[u8], keys: &GtaViKeys) -> Option<(Vec<u8>, usize)> {
        let library = SchemaLibrary::default();
        let (instance, layout) =
            parse_meta_binary_full(&library, doc, &mut Diagnostics::lenient()).ok()?;
        if write_adding_strings(&layout, &instance, Some(keys)).ok()? != doc {
            return None;
        }
        let before = registered_strings(doc, Some(keys)).ok()?.len();
        (0..TRIES).find_map(|n| {
            let mut edited = instance.clone();
            *nth_hash_of(&mut edited, n)? = Value::String(ADDED.into());
            let saved = write_adding_strings(&layout, &edited, Some(keys)).ok()?;
            let strings = registered_strings(&saved, Some(keys)).ok()?;
            let added = strings.iter().any(|s| s.category == 1 && s.text == ADDED);
            (added && strings.len() == before + 1).then_some((saved, n))
        })
    }

    #[test]
    fn a_build_document_saved_with_an_added_text_reopens_with_it() {
        let Some((keys, build)) = keys_and_build("GTA VI PSO save with an added text over a build")
        else {
            return;
        };
        let mut found = None;
        each_document(&build, &keys, &mut |doc| {
            if holds_category_1(doc) {
                found = saved_with_a_text(doc, &keys).map(|(saved, n)| (doc.to_vec(), saved, n));
            }
            found.is_some()
        });
        let Some((doc, saved, n)) = found else {
            or_test_support::skip_install(
                "GTA VI PSO save with an added text over a build",
                format_args!(
                    "the walk met no document with a category-1 run that saves with a new text"
                ),
            );
            return;
        };
        assert_ne!(saved, doc);
        let library = SchemaLibrary::default();
        let (mut again, _) = parse_meta_binary_full(&library, &saved, &mut Diagnostics::lenient())
            .expect("the saved document reopens");
        assert_eq!(
            hash_of(nth_hash_of(&mut again, n).as_deref()),
            Some(crate::string_hash(ADDED.as_bytes())),
            "the edited value holds the new text's hash"
        );
        let end = saved.len();
        let stored = u32::from_be_bytes(saved[end - 8..end - 4].try_into().unwrap());
        assert_eq!(
            super::super::super::chks::checksum(&saved),
            Some(stored),
            "the saved checksum satisfies the loader's rule"
        );
    }
}
