//! `rpf9 patch` and `rpf9 fits`: entry bodies changed chunk by chunk into a new archive whose every
//! table is the source's byte for byte, or only whether each change would go in so.
//!
//!   `rpf9 patch <archive> --entry <path> --with <file> [...] --out <new>`
//!   `rpf9 fits <archive> --entry <path> --with <file> [...]`   writes nothing
//!
//! An entry's path walks nested archives, each part named as `rpf9 extract` names an entry (a
//! recovered name, a `0x…`/`hash_…` hash or a decimal index); a part may itself hold slashes, so the
//! longest leading part that names a nested archive is walked into. `--with` is the inflated body
//! `rpf9 extract` writes, deciphered where the row is enciphered: the bytes go in as they are.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::container::rpf::{
    plan_patch, write_patched_copy, BodyPatch, Gta6Entry, Gta6Mount, GtaViKeys, NameResolver,
    PatchPlan, PatchRefusal,
};

use super::{find_entry, load_resolver, take_common, usage, Common};

/// What `patch` and `fits` say of a patched copy.
pub(crate) const PATCHED: &str = "A patched copy keeps every table byte-identical to the \
source's, so its table signature still matches, and no check read in the game's code rejects a \
body changed chunk by chunk; no run of the game was made with one, so whether the game loads it \
is not known.";

/// What `rpf9 replace` adds when its change would have gone in place.
const WOULD_FIT: &str = " The change would have fitted in place: rpf9 patch writes it as a \
patched copy, whose tables stay byte-identical.";

/// One `--entry`/`--with` pair: the row's path and the file holding its new inflated body.
type Pair = (String, PathBuf);

/// A pair's row and new body, with the plan of putting it in place or why it cannot go.
type Planned = (BodyPatch, Result<PatchPlan, PatchRefusal>);

pub(super) fn run(verb: &str, args: &[String]) -> ExitCode {
    let mut common = Common::default();
    let (mut entries, mut withs) = (Vec::new(), Vec::new());
    let (mut out, mut archive): (Option<PathBuf>, Option<PathBuf>) = (None, None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match take_common(&mut common, arg, &mut it) {
            Ok(true) => continue,
            Err(code) => return code,
            Ok(false) => {}
        }
        match arg.as_str() {
            "--entry" | "--with" | "--out" => {
                let Some(value) = it.next() else {
                    return usage(&format!("{arg} needs a value"));
                };
                match arg.as_str() {
                    "--entry" => entries.push(value.clone()),
                    "--with" => withs.push(PathBuf::from(value)),
                    _ => out = Some(PathBuf::from(value)),
                }
            }
            other if other.starts_with("--") => return usage(&format!("unknown option {other}")),
            other => {
                if archive.replace(PathBuf::from(other)).is_some() {
                    return usage("give one archive");
                }
            }
        }
    }
    let Some(archive) = archive else {
        return usage(&format!("give an archive to {verb}"));
    };
    if entries.is_empty() || entries.len() != withs.len() {
        return usage("give each --entry its --with, in pairs");
    }
    if verb == "fits" && out.is_some() {
        return usage("fits writes nothing, so it takes no --out");
    }
    let pairs: Vec<Pair> = entries.into_iter().zip(withs).collect();
    let keys = match crate::gta6_keys::load(common.keys_path.as_deref(), common.exe_path.as_deref())
    {
        Ok(keys) => keys,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let resolver = load_resolver(common.names_dir.as_deref());
    let result = match verb {
        "patch" => patch(&archive, &pairs, out.as_deref(), &keys, &resolver).map(|r| (r, true)),
        _ => fits(&archive, &pairs, &keys, &resolver),
    };
    match result {
        Ok((report, all_fit)) => {
            println!("{report}");
            if all_fit {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `rpf9 patch`: every pair put in place into the new file `out`, or nothing written.
fn patch(
    archive: &Path,
    pairs: &[Pair],
    out: Option<&Path>,
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<String, String> {
    let out = out.ok_or("--out names the new archive to write")?;
    if crate::rpf9_write::same_file(archive, out) {
        return Err("--out must name a new file; the source archive is never written".into());
    }
    if out.exists() {
        return Err(format!(
            "{}: exists; a patch writes a new file",
            out.display()
        ));
    }
    let mut patches = Vec::with_capacity(pairs.len());
    let planned = plan_pairs(archive, pairs, keys, resolver)?;
    for ((path, _), (body_patch, plan)) in pairs.iter().zip(planned) {
        if let Err(refusal) = plan {
            return Err(format!(
                "{path} does not fit in place ({refusal}); nothing was written. rpf9 replace \
                 rebuilds the archive instead, and the game refuses a rebuilt archive"
            ));
        }
        patches.push(body_patch);
    }
    let report = write_patched_copy(archive, out, keys, &patches).map_err(|e| e.to_string())?;
    Ok(format!(
        "{}: {} chunk(s) changed, {} bytes written. {PATCHED}",
        out.display(),
        report.chunks_changed,
        report.bytes_written,
    ))
}

/// `rpf9 fits`: per pair and per chunk, whether the new body goes in place; nothing is written. The
/// flag is whether every pair fits.
fn fits(
    archive: &Path,
    pairs: &[Pair],
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<(String, bool), String> {
    let (mut lines, mut all_fit) = (Vec::new(), true);
    let planned = plan_pairs(archive, pairs, keys, resolver)?;
    for ((path, _), (_, plan)) in pairs.iter().zip(planned) {
        match plan {
            Ok(plan) => {
                lines.push(format!("{path}: fits in place"));
                for (i, chunk) in plan.chunks.iter().enumerate() {
                    lines.push(if chunk.changed {
                        format!(
                            "  chunk {i}: {} bytes inflated, {} stored under a {}-byte ceiling, {} \
                             to spare",
                            chunk.inflated,
                            chunk.new_stored,
                            chunk.ceiling,
                            chunk.ceiling.saturating_sub(chunk.new_stored),
                        )
                    } else {
                        format!("  chunk {i}: unchanged, its stored bytes kept")
                    });
                }
            }
            Err(refusal) => {
                all_fit = false;
                lines.push(format!("{path}: does not fit in place: {refusal}"));
            }
        }
    }
    lines.push(format!("Nothing was written. {PATCHED}"));
    Ok((lines.join("\n"), all_fit))
}

/// Each pair's row found, its body read and its plan made, in the order given.
fn plan_pairs(
    archive: &Path,
    pairs: &[Pair],
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<Vec<Planned>, String> {
    let mut file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let mut planned: Vec<Planned> = Vec::with_capacity(pairs.len());
    for (path, with) in pairs {
        let (mount, entry) = find_row(&mut file, path, keys, resolver)?;
        if planned
            .iter()
            .any(|(p, _)| p.base == mount.base && p.entry == entry.index)
        {
            return Err(format!("{path} names a row another --entry names"));
        }
        let body = std::fs::read(with).map_err(|e| format!("{}: {e}", with.display()))?;
        let patch = BodyPatch {
            base: mount.base,
            entry: entry.index,
            body,
        };
        let plan = plan_patch(&mut file, keys, &patch);
        planned.push((patch, plan));
    }
    Ok(planned)
}

/// The row `path` names and the mount holding it: while the rest of the path names no entry, the
/// longest leading part of it that names a nested archive is walked into.
fn find_row(
    file: &mut File,
    path: &str,
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<(Gta6Mount, Gta6Entry), String> {
    let mut mount = Gta6Mount::open_root(file, keys).map_err(|e| e.to_string())?;
    mount.archive.resolve_names(resolver);
    let mut rest = path.trim_matches(['/', '\\']);
    loop {
        if let Some(entry) = find_entry(&mount, rest) {
            if mount
                .entry_begins_archive(file, &entry)
                .map_err(|e| e.to_string())?
            {
                return Err(format!(
                    "{path:?} is a nested archive: patch the entries inside it"
                ));
            }
            return Ok((mount, entry));
        }
        let mut step = None;
        for (at, _) in rest.rmatch_indices(['/', '\\']) {
            if let Some(entry) = find_entry(&mount, &rest[..at]) {
                if mount
                    .entry_begins_archive(file, &entry)
                    .map_err(|e| e.to_string())?
                {
                    step = Some((at, entry));
                    break;
                }
            }
        }
        let Some((at, nested)) = step else {
            return Err(format!("no entry {path:?} in the archive"));
        };
        mount = mount
            .open_nested(file, &nested, keys)
            .map_err(|e| e.to_string())?;
        mount.archive.resolve_names(resolver);
        rest = &rest[at + 1..];
    }
}

/// The sentence `rpf9 replace` adds when the body it puts at `path` (entry indices from the top
/// table) would have gone in place; empty otherwise.
pub(crate) fn replace_note(file: &mut File, path: &[u32], body: &[u8], keys: &GtaViKeys) -> String {
    match fits_in_place(file, path, body, keys) {
        Some(true) => WOULD_FIT.to_owned(),
        _ => String::new(),
    }
}

/// Whether `body` at `path` goes in place and changes a chunk; `None` where it cannot be planned.
fn fits_in_place(file: &mut File, path: &[u32], body: &[u8], keys: &GtaViKeys) -> Option<bool> {
    let (&last, nested) = path.split_last()?;
    let mut mount = Gta6Mount::open_root(file, keys).ok()?;
    for &index in nested {
        let entry = mount.archive.entries.get(index as usize)?.clone();
        mount = mount.open_nested(file, &entry, keys).ok()?;
    }
    let patch = BodyPatch {
        base: mount.base,
        entry: last,
        body: body.to_vec(),
    };
    let plan = plan_patch(file, keys, &patch).ok()?;
    Some(plan.chunks.iter().any(|chunk| chunk.changed))
}

// Shared with the `rpf9` tests, which use parts of it this file does not.
#[cfg(test)]
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "rpf9_fixture.rs"]
mod fixture;

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::fixture::{self, Body};
    use super::*;
    use or_io::gta6::hash::name_hash64;

    /// Bytes no DEFLATE level shrinks much.
    fn noise(len: usize) -> Vec<u8> {
        let mut x = 0x9E37_79B9u32;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 24) as u8
            })
            .collect()
    }

    fn resolver_of(names: &[&str]) -> NameResolver {
        let mut resolver = NameResolver::new();
        for name in names {
            resolver.insert(name_hash64(name.as_bytes()), *name);
        }
        resolver
    }

    /// A folder holding `a.rpf`, whose entry 1 is `data/body.bin` with `chunks`, and `body.bin`
    /// holding `with`.
    fn staged(chunks: Vec<(Vec<u8>, bool)>, with: &[u8]) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let source = dir.path().join("a.rpf");
        let archive = fixture::archive(&[
            ("first.bin", Body::Raw(b"one".to_vec())),
            ("data/body.bin", Body::Chunked(chunks)),
        ]);
        std::fs::write(&source, archive).unwrap();
        let body = dir.path().join("body.bin");
        std::fs::write(&body, with).unwrap();
        (dir, source, body)
    }

    fn tail() -> Vec<u8> {
        b"kept".repeat(8)
    }

    /// A first chunk of noise that zeros re-deflate far under, and a stored chunk left as it is.
    fn roomy() -> Vec<(Vec<u8>, bool)> {
        vec![(noise(4096), true), (tail(), false)]
    }

    fn zeros_then_tail() -> Vec<u8> {
        let mut body = vec![0u8; 4096];
        body.extend(tail());
        body
    }

    #[test]
    fn a_patch_that_fits_writes_a_copy_that_reads_back_and_leaves_the_source() {
        let new = zeros_then_tail();
        let (dir, source, with) = staged(roomy(), &new);
        let before = std::fs::read(&source).unwrap();
        let out = dir.path().join("b.rpf");
        let keys = fixture::keys();
        let pairs = [("data/body.bin".to_owned(), with)];
        let report = patch(
            &source,
            &pairs,
            Some(&out),
            &keys,
            &resolver_of(&["data/body.bin"]),
        )
        .expect("the body goes in place");
        assert!(report.contains(PATCHED), "{report}");
        assert_eq!(std::fs::read(&source).unwrap(), before);

        let bytes = std::fs::read(&out).unwrap();
        let mount = Gta6Mount::open_root(&mut Cursor::new(&bytes[..]), &keys).expect("it reopens");
        let read = mount
            .read_entry(Cursor::new(&bytes[..]), &mount.archive.entries[1])
            .expect("the body reads");
        assert_eq!(read, new);
        assert_eq!(bytes.len(), before.len());
    }

    #[test]
    fn fits_writes_no_file_and_says_what_a_patched_copy_is() {
        let (dir, source, with) = staged(roomy(), &zeros_then_tail());
        let listed = || {
            let mut names: Vec<_> = std::fs::read_dir(dir.path())
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            names.sort();
            names
        };
        let before = listed();
        let pairs = [("1".to_owned(), with)];
        let (report, all_fit) =
            fits(&source, &pairs, &fixture::keys(), &NameResolver::new()).expect("it plans");
        assert!(all_fit, "{report}");
        assert!(report.contains("1: fits in place"), "{report}");
        assert!(report.contains("chunk 0: "), "{report}");
        assert!(report.contains("Nothing was written."), "{report}");
        assert!(report.contains(PATCHED), "{report}");
        assert_eq!(listed(), before);
    }

    #[test]
    fn a_body_that_cannot_fit_is_reported_by_fits_and_refused_by_patch() {
        let (dir, source, with) = staged(vec![(vec![0u8; 4096], true)], &noise(4096));
        let keys = fixture::keys();
        let pairs = [("1".to_owned(), with)];
        let (report, all_fit) =
            fits(&source, &pairs, &keys, &NameResolver::new()).expect("it plans");
        assert!(!all_fit, "{report}");
        assert!(report.contains("1: does not fit in place: "), "{report}");

        let out = dir.path().join("b.rpf");
        let refused = patch(&source, &pairs, Some(&out), &keys, &NameResolver::new()).unwrap_err();
        assert!(refused.contains("does not fit in place"), "{refused}");
        assert!(!out.exists());
    }

    #[test]
    fn an_out_equal_to_the_archive_is_refused_and_the_archive_is_untouched() {
        let (_dir, source, with) = staged(roomy(), &zeros_then_tail());
        let before = std::fs::read(&source).unwrap();
        let pairs = [("1".to_owned(), with)];
        let refused = patch(
            &source,
            &pairs,
            Some(&source),
            &fixture::keys(),
            &NameResolver::new(),
        )
        .unwrap_err();
        assert!(refused.contains("never written"), "{refused}");
        assert_eq!(std::fs::read(&source).unwrap(), before);
    }

    #[test]
    fn a_nested_archive_is_mounted_by_a_recovered_name_that_holds_slashes() {
        let inner = fixture::archive(&[("a.bin", Body::Raw(b"a".to_vec()))]);
        let outer = fixture::archive(&[
            ("top.bin", Body::Raw(b"top".to_vec())),
            ("models/models", Body::Nested(inner)),
        ]);
        let dir = tempfile::tempdir().expect("a temp dir");
        let source = dir.path().join("a.rpf");
        std::fs::write(&source, outer).unwrap();
        let keys = fixture::keys();
        let named = resolver_of(&["models/models"]);
        let mut file = File::open(&source).unwrap();
        let path = ["models".to_owned(), "models".to_owned()];
        let mount = super::super::mount_path(&mut file, &path, &keys, &named).expect("mounted");
        assert!(mount.base > 0, "the nested archive, not the top one");
        let missing = ["models".to_owned()];
        let refused = super::super::mount_path(&mut file, &missing, &keys, &named).unwrap_err();
        assert!(refused.contains("no entry"), "{refused}");
    }

    #[test]
    fn a_nested_row_is_found_by_its_path_through_the_archives() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let source = dir.path().join("a.rpf");
        let inner = fixture::archive(&[
            ("a.bin", Body::Raw(b"a".to_vec())),
            ("data/deep.bin", Body::Chunked(roomy())),
        ]);
        let outer = fixture::archive(&[
            ("top.bin", Body::Raw(b"top".to_vec())),
            ("maps/inner.rpf", Body::Nested(inner)),
        ]);
        std::fs::write(&source, outer).unwrap();
        let keys = fixture::keys();
        let mut file = File::open(&source).unwrap();

        let named = resolver_of(&["maps/inner.rpf", "data/deep.bin"]);
        let (mount, entry) =
            find_row(&mut file, "maps/inner.rpf/data/deep.bin", &keys, &named).expect("found");
        assert!(mount.base > 0);
        assert_eq!(entry.index, 1);

        let hashed = format!("0x{:016x}/1", name_hash64(b"maps/inner.rpf"));
        let (by_hash, entry) =
            find_row(&mut file, &hashed, &keys, &NameResolver::new()).expect("found");
        assert_eq!((by_hash.base, entry.index), (mount.base, 1));

        let refused = find_row(&mut file, "maps/inner.rpf", &keys, &named).unwrap_err();
        assert!(refused.contains("nested archive"), "{refused}");
        assert!(find_row(&mut file, "maps/inner.rpf/missing", &keys, &named).is_err());

        let new = zeros_then_tail();
        let with = dir.path().join("deep.bin");
        std::fs::write(&with, &new).unwrap();
        let out = dir.path().join("b.rpf");
        let pairs = [("maps/inner.rpf/data/deep.bin".to_owned(), with)];
        patch(&source, &pairs, Some(&out), &keys, &named).expect("the body goes in place");
        let bytes = std::fs::read(&out).unwrap();
        let root = Gta6Mount::open_root(&mut Cursor::new(&bytes[..]), &keys).expect("it reopens");
        let child = root
            .open_nested(
                &mut Cursor::new(&bytes[..]),
                &root.archive.entries[1],
                &keys,
            )
            .expect("the nested table reopens");
        let read = child
            .read_entry(Cursor::new(&bytes[..]), &child.archive.entries[1])
            .expect("the body reads");
        assert_eq!(read, new);
    }

    #[test]
    fn replace_names_patch_only_when_its_change_would_have_fitted() {
        let (_dir, source, _) = staged(roomy(), &[]);
        let keys = fixture::keys();
        let mut file = File::open(&source).unwrap();
        let note = replace_note(&mut file, &[1], &zeros_then_tail(), &keys);
        assert!(note.contains("rpf9 patch"), "{note}");
        let mut longer = zeros_then_tail();
        longer.push(0);
        assert_eq!(replace_note(&mut file, &[1], &longer, &keys), "");
    }

    /// Over the smallest archive at a real build's root, a clear chunk-tabled row given its own body
    /// fits with every chunk unchanged, and patched with it writes a copy equal to the source.
    /// Install-gated: it needs the key snapshot and the build; it reads at most 256 rows.
    #[test]
    fn a_gta6_build_row_given_its_own_body_fits_unchanged_and_patches_to_an_equal_copy() {
        const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
        let what = "rpf9 fits and patch over a build's row";
        let env = |var: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.exists())
        };
        let snapshot_env = or_io::container::rpf::GTA6_SNAPSHOT_ENV;
        let (Some(snapshot), Some(build)) = (env(snapshot_env), env(BUILD_ENV)) else {
            or_test_support::skip_install(what, format_args!("set {snapshot_env} and {BUILD_ENV}"));
            return;
        };
        let exe = env(or_io::container::rpf::GTA6_EXE_ENV);
        let keys = crate::gta6_keys::load(Some(&snapshot), exe.as_deref()).expect("the keys open");
        let source = crate::rpf9::build_archives(&build)
            .into_iter()
            .filter(|p| p.parent() == Some(build.as_path()))
            .min_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX))
            .expect("the build root holds an archive");
        let mut file = File::open(&source).unwrap();
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        let row = mount.archive.entries.iter().take(256).find_map(|entry| {
            let small = entry.real.is_some_and(|n| n <= 4 << 20);
            let chunked = or_io::container::rpf::gta6_chunk_table::entry_chunks(
                &mount.archive,
                entry.index as usize,
            )
            .is_ok_and(|t| t.is_some());
            if !small || !chunked || entry.cipher.is_some() {
                return None;
            }
            let body = mount.read_entry(&mut file, entry).ok()?;
            Some((entry.index, body))
        });
        let Some((index, body)) = row else {
            or_test_support::skip_install(what, "no clear chunk-tabled row in the first 256");
            return;
        };
        let dir = tempfile::tempdir().expect("a temp dir");
        let with = dir.path().join("body.bin");
        std::fs::write(&with, &body).unwrap();
        let pairs = [(index.to_string(), with)];
        let resolver = NameResolver::new();
        let (report, all_fit) = fits(&source, &pairs, &keys, &resolver).expect("it plans");
        assert!(all_fit, "{report}");
        assert!(
            !report.contains(" stored under a "),
            "a chunk changed: {report}"
        );

        if std::fs::metadata(&source).unwrap().len() <= 64 << 20 {
            let out = dir.path().join("copy.rpf");
            patch(&source, &pairs, Some(&out), &keys, &resolver).expect("it patches");
            assert!(std::fs::read(&out).unwrap() == std::fs::read(&source).unwrap());
        }
    }

    #[test]
    fn an_entry_without_its_with_is_a_usage_error() {
        let args: Vec<String> = ["a.rpf", "--entry", "1", "--out", "b.rpf"]
            .map(String::from)
            .to_vec();
        assert_eq!(run("patch", &args), ExitCode::from(2));
        let args: Vec<String> = ["a.rpf", "--entry", "1", "--with", "x", "--out", "b.rpf"]
            .map(String::from)
            .to_vec();
        assert_eq!(run("fits", &args), ExitCode::from(2));
    }
}
