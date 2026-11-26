//! `rpf9 verify-patch <source> <copy>`: whether a copy keeps its source's length and every header,
//! table and names span of every archive at any depth, with each differing byte inside a row that
//! reads through its table in the copy. It exits 0 only for such a patched copy.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::container::rpf::gta6_patch::{every_table_span, read_row_by_chunks};
use or_io::container::rpf::RPF9_HEADER_SIZE;
use or_io::container::rpf::{open_at, Gta6Class, Gta6Mount, GtaViKeys, NameResolver};

use super::patch::PATCHED;
use super::{load_resolver, take_common, usage, Common};

/// What the verdict adds when a table span differs.
const REBUILT: &str = " A copy whose table differs is a rebuilt archive, which the game refuses \
because only its maker can sign the table.";

/// The bytes of each file compared at a time.
const BLOCK: usize = 1 << 20;

pub(super) fn run(args: &[String]) -> ExitCode {
    let mut common = Common::default();
    let mut files: Vec<PathBuf> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match take_common(&mut common, arg, &mut it) {
            Ok(true) => continue,
            Err(code) => return code,
            Ok(false) => {}
        }
        if arg.starts_with("--") {
            return usage(&format!("unknown option {arg}"));
        }
        files.push(PathBuf::from(arg));
    }
    let [source, copy] = &files[..] else {
        return usage("give the source archive, then the copy");
    };
    let keys = match crate::gta6_keys::load(common.keys_path.as_deref(), common.exe_path.as_deref())
    {
        Ok(keys) => keys,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let resolver = load_resolver(common.names_dir.as_deref());
    match verify(source, copy, &keys, &resolver) {
        Ok((report, patched)) => {
            println!("{report}");
            if patched {
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

/// A leaf row of the source: its path, its archive (an index into the mounts), its entry, its stored
/// extent, and the bytes of it that differ in the copy.
struct Row {
    path: String,
    mount: usize,
    entry: u32,
    at: u64,
    len: u64,
    differ: u64,
    first: u64,
    end: u64,
}

/// Differing bytes counted over a span or outside every row: how many, and the first.
#[derive(Clone, Copy, Default)]
struct Hits {
    bytes: u64,
    first: u64,
}

impl Hits {
    fn add(&mut self, at: u64, n: u64) {
        if self.bytes == 0 {
            self.first = at;
        }
        self.bytes += n;
    }
}

/// Where the differing bytes fall: in rows, in table spans or in neither.
struct Counts {
    /// The furthest end of the rows up to each one, so rows stating overlapping extents are each
    /// found by walking back while an earlier row still reaches.
    reach: Vec<u64>,
    span_hits: Vec<Hits>,
    stray: Hits,
}

impl Counts {
    fn new(rows: &[Row], spans: usize) -> Self {
        let reach = (rows.iter())
            .scan(0u64, |m, r| {
                *m = (*m).max(r.at + r.len);
                Some(*m)
            })
            .collect();
        Self {
            reach,
            span_hits: vec![Hits::default(); spans],
            stray: Hits::default(),
        }
    }

    /// Count the differing run `a..b` against every row whose extent covers each byte of it, else
    /// against the spans it lies in, else as stray.
    fn run(&mut self, rows: &mut [Row], spans: &[(u64, u64)], a: u64, b: u64) {
        let mut cur = a;
        while cur < b {
            let i = rows.partition_point(|r| r.at <= cur);
            let next = rows.get(i).map_or(b, |r| r.at.min(b));
            let covering: Vec<usize> = (0..i)
                .rev()
                .take_while(|&j| self.reach[j] > cur)
                .filter(|&j| cur < rows[j].at + rows[j].len)
                .collect();
            if !covering.is_empty() {
                let end = (covering.iter())
                    .map(|&j| rows[j].at + rows[j].len)
                    .fold(next, u64::min);
                for &j in &covering {
                    let row = &mut rows[j];
                    if row.differ == 0 {
                        row.first = cur;
                    }
                    row.differ += end - cur;
                    row.end = end;
                }
                cur = end;
                continue;
            }
            let mut in_spans = 0;
            let from = spans.partition_point(|&(_, end)| end <= cur);
            for (k, &(s, e)) in spans.iter().enumerate().skip(from) {
                if s >= next {
                    break;
                }
                let (lo, hi) = (cur.max(s), next.min(e));
                if lo < hi {
                    self.span_hits[k].add(lo, hi - lo);
                    in_spans += hi - lo;
                }
            }
            if in_spans < next - cur {
                self.stray.add(cur, next - cur - in_spans);
            }
            cur = next;
        }
    }
}

/// The report and whether `copy` is a patched copy of `source`.
fn verify(
    source: &Path,
    copy: &Path,
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<(String, bool), String> {
    let open = |p: &Path| File::open(p).map_err(|e| format!("{}: {e}", p.display()));
    let (mut src, mut cpy) = (open(source)?, open(copy)?);
    let length = |f: &File, p: &Path| {
        (f.metadata())
            .map(|m| m.len())
            .map_err(|e| format!("{}: {e}", p.display()))
    };
    let (src_len, cpy_len) = (length(&src, source)?, length(&cpy, copy)?);
    let in_source = |e: or_io::IoError| format!("{}: {e}", source.display());
    let spans = every_table_span(&mut src, keys).map_err(in_source)?;
    let (mounts, mut rows) = rows_of(&mut src, src_len, keys, resolver).map_err(in_source)?;

    let mut counts = Counts::new(&rows, spans.len());
    let mut differ = 0u64;
    diff_runs(&mut src, &mut cpy, src_len.min(cpy_len), |a, b| {
        differ += b - a;
        counts.run(&mut rows, &spans, a, b);
    })
    .map_err(|e| e.to_string())?;
    let Counts {
        span_hits, stray, ..
    } = counts;

    let mut why = Vec::new();
    if src_len != cpy_len {
        why.push(format!(
            "the copy is {cpy_len} bytes and the source {src_len}, and a patched copy keeps its \
             source's length"
        ));
    }
    let mut table_differs = false;
    for (&(a, b), hits) in spans.iter().zip(&span_hits) {
        if hits.bytes > 0 {
            table_differs = true;
            why.push(format!(
                "the table span {a:#x}..{b:#x} differs in {} byte(s), the first at {:#x}",
                hits.bytes, hits.first
            ));
        }
    }
    if stray.bytes > 0 {
        why.push(format!(
            "{} byte(s) differ outside every row's stored extent and every table span, the first \
             at {:#x}",
            stray.bytes, stray.first
        ));
    }

    let mut lines = Vec::new();
    let mut copy_mounts: HashMap<usize, Result<Gta6Mount, String>> = HashMap::new();
    let changed: Vec<&Row> = rows.iter().filter(|r| r.differ > 0).collect();
    for row in &changed {
        let src_mount = &mounts[row.mount];
        let cpy_mount = copy_mounts.entry(row.mount).or_insert_with(|| {
            let base = src_mount.base;
            open_at(&mut cpy, base, keys)
                .map(|archive| Gta6Mount { base, archive })
                .map_err(|e| e.to_string())
        });
        let read = match cpy_mount {
            Ok(mount) => {
                read_row_by_chunks(mount, &mut cpy, keys, row.entry).map_err(|e| e.to_string())
            }
            Err(e) => Err(format!("its archive's table does not open ({e})")),
        };
        let stored = format!(
            "{}: {} stored byte(s) differ in {:#x}..{:#x}",
            row.path, row.differ, row.first, row.end
        );
        match read {
            Ok(new) => {
                let inflated = match read_row_by_chunks(src_mount, &mut src, keys, row.entry) {
                    Ok(old) => format!(
                        "inflated, {} of its {} byte(s) differ",
                        bytes_differing(&old, &new),
                        new.len()
                    ),
                    Err(e) => format!(
                        "it reads through its table in the copy at {} byte(s), and the source's \
                         row does not ({e})",
                        new.len()
                    ),
                };
                lines.push(format!("{stored}; {inflated}"));
            }
            Err(e) => {
                lines.push(format!(
                    "{stored}; it does not read through its table in the copy: {e}"
                ));
                why.push(format!(
                    "{} does not read through its table in the copy",
                    row.path
                ));
            }
        }
    }

    let patched = why.is_empty();
    lines.push(if patched {
        format!(
            "{}: a patched copy of {}: {differ} byte(s) differ, in {} row(s), and every header, \
             table and names span of its {} archive(s) is byte-identical. {PATCHED}",
            copy.display(),
            source.display(),
            changed.len(),
            mounts.len()
        )
    } else {
        format!(
            "{}: not a patched copy of {}: {}.{}",
            copy.display(),
            source.display(),
            why.join("; "),
            if table_differs { REBUILT } else { "" }
        )
    });
    Ok((lines.join("\n"), patched))
}

/// Every archive of `file` (the top one and each nested one at any depth) and every leaf row of them
/// with a stored extent, sorted by where it sits.
fn rows_of(
    file: &mut File,
    file_end: u64,
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<(Vec<Gta6Mount>, Vec<Row>), or_io::IoError> {
    let (mut mounts, mut rows) = (Vec::new(), Vec::new());
    let mut todo = vec![(Gta6Mount::open_root(file, keys)?, String::new())];
    while let Some((mut mount, prefix)) = todo.pop() {
        mount.archive.resolve_names(resolver);
        let extras = mount.archive.entry_extras();
        for e in &mount.archive.entries {
            let Some(start) = mount.absolute_start(e) else {
                continue;
            };
            let path = format!("{prefix}{}", e.display_name());
            // The same archives `every_table_span` walks; a nested one's bytes are its own rows.
            if e.kind() == Some(Gta6Class::NestedArchive) {
                if e.stored.is_some()
                    && start > mount.base
                    && start + RPF9_HEADER_SIZE as u64 <= file_end
                {
                    todo.push((mount.open_nested(file, e, keys)?, format!("{path}/")));
                }
                continue;
            }
            // A 20-byte row's body begins past its 16-byte zero slot, where a patch writes it.
            let nested = extras.get(e.index as usize).and_then(|x| x.nested);
            let extent = match (e.stored, nested) {
                (Some(n), _) => Some((start, n)),
                (None, Some(row)) => Some(row.ciphered_body_extent(start)),
                (None, None) => None,
            };
            if let Some((at, len)) = extent {
                rows.push(Row {
                    path,
                    mount: mounts.len(),
                    entry: e.index,
                    at,
                    len,
                    differ: 0,
                    first: 0,
                    end: 0,
                });
            }
        }
        mounts.push(mount);
    }
    rows.sort_by_key(|r| r.at);
    Ok((mounts, rows))
}

/// Each run of bytes that differ between `a` and `b` over their first `len` bytes, as `[start, end)`.
fn diff_runs(
    a: &mut File,
    b: &mut File,
    len: u64,
    mut on_run: impl FnMut(u64, u64),
) -> std::io::Result<()> {
    a.seek(SeekFrom::Start(0))?;
    b.seek(SeekFrom::Start(0))?;
    let (mut x, mut y) = (vec![0u8; BLOCK], vec![0u8; BLOCK]);
    let (mut pos, mut open) = (0u64, None::<u64>);
    while pos < len {
        let n = (len - pos).min(BLOCK as u64) as usize;
        a.read_exact(&mut x[..n])?;
        b.read_exact(&mut y[..n])?;
        if x[..n] == y[..n] {
            if let Some(s) = open.take() {
                on_run(s, pos);
            }
        } else {
            for (i, (p, q)) in x[..n].iter().zip(&y[..n]).enumerate() {
                let at = pos + i as u64;
                match (p != q, open) {
                    (true, None) => open = Some(at),
                    (false, Some(s)) => {
                        on_run(s, at);
                        open = None;
                    }
                    _ => {}
                }
            }
        }
        pos += n as u64;
    }
    if let Some(s) = open {
        on_run(s, len);
    }
    Ok(())
}

/// The bytes at which `a` and `b` differ, a length difference counted in full.
fn bytes_differing(a: &[u8], b: &[u8]) -> usize {
    let common = a.iter().zip(b).filter(|(p, q)| p != q).count();
    common + a.len().abs_diff(b.len())
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
    use or_io::container::rpf::gta6_patch::plan_patch_with;
    use or_io::container::rpf::{write_patched_copy, BodyPatch};
    use or_io::gta6::hash::name_hash64;

    fn row(at: u64, len: u64) -> Row {
        Row {
            path: String::new(),
            mount: 0,
            entry: 0,
            at,
            len,
            differ: 0,
            first: 0,
            end: 0,
        }
    }

    /// Rows stating one extent twice, and a short row inside a longer one sorted after it, are each
    /// counted for the bytes they cover, and a byte only a span covers is the span's.
    #[test]
    fn a_differing_run_counts_against_every_row_that_covers_it() {
        let mut rows = vec![row(100, 50), row(100, 50), row(200, 100), row(200, 10)];
        let spans = [(0u64, 50u64)];
        let mut counts = Counts::new(&rows, spans.len());
        counts.run(&mut rows, &spans, 110, 120);
        counts.run(&mut rows, &spans, 250, 260);
        counts.run(&mut rows, &spans, 205, 208);
        counts.run(&mut rows, &spans, 40, 60);
        let differ: Vec<u64> = rows.iter().map(|r| r.differ).collect();
        assert_eq!(differ, [10, 10, 13, 3]);
        assert_eq!(counts.span_hits[0].bytes, 10);
        assert_eq!(counts.stray.bytes, 10);
    }

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

    fn tail() -> Vec<u8> {
        b"kept".repeat(8)
    }

    fn named() -> NameResolver {
        let mut resolver = NameResolver::new();
        for name in ["maps/inner.rpf", "data/deep.bin"] {
            resolver.insert(name_hash64(name.as_bytes()), name);
        }
        resolver
    }

    /// An archive whose entry 1 is a nested archive, whose entry 1 is a row of a zlib chunk of noise
    /// and a stored chunk, with `deep` as that row's chunks.
    fn nested_archive(deep: Vec<(Vec<u8>, bool)>) -> Vec<u8> {
        let inner = fixture::archive(&[
            ("a.bin", Body::Raw(b"a".to_vec())),
            ("data/deep.bin", Body::Chunked(deep)),
        ]);
        fixture::archive(&[
            ("top.bin", Body::Raw(b"top".to_vec())),
            ("maps/inner.rpf", Body::Nested(inner)),
        ])
    }

    fn roomy() -> Vec<(Vec<u8>, bool)> {
        vec![(noise(4096), true), (tail(), false)]
    }

    /// The nested archive's mount and its deep row's stored start, over `bytes`.
    fn inner_of(bytes: &[u8]) -> (Gta6Mount, u64) {
        let keys = fixture::keys();
        let root = Gta6Mount::open_root(&mut Cursor::new(bytes), &keys).expect("the root opens");
        let inner = root
            .open_nested(&mut Cursor::new(bytes), &root.archive.entries[1], &keys)
            .expect("the nested table opens");
        let start = inner
            .absolute_start(&inner.archive.entries[1])
            .expect("a start");
        (inner, start)
    }

    /// A folder holding the nested archive as `a.rpf`, and `b.rpf`, a copy of it with `edit` applied.
    fn staged(edit: impl FnOnce(&mut Vec<u8>)) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let (source, copy) = (dir.path().join("a.rpf"), dir.path().join("b.rpf"));
        let bytes = nested_archive(roomy());
        std::fs::write(&source, &bytes).unwrap();
        let mut edited = bytes;
        edit(&mut edited);
        std::fs::write(&copy, edited).unwrap();
        (dir, source, copy)
    }

    #[test]
    fn a_copy_written_by_write_patched_copy_verifies() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let (source, copy) = (dir.path().join("a.rpf"), dir.path().join("b.rpf"));
        let bytes = nested_archive(roomy());
        std::fs::write(&source, &bytes).unwrap();
        let (inner, _) = inner_of(&bytes);
        let mut body = vec![0u8; 4096];
        body.extend(tail());
        let keys = fixture::keys();
        let patch = BodyPatch {
            base: inner.base,
            entry: 1,
            body,
        };
        write_patched_copy(&source, &copy, &keys, &[patch]).expect("the patch goes in place");

        let (report, patched) = verify(&source, &copy, &keys, &named()).expect("it verifies");
        assert!(patched, "{report}");
        assert!(
            report.contains("maps/inner.rpf/data/deep.bin: "),
            "{report}"
        );
        // Only the noise's nonzero bytes differ from the zeros put over them.
        let differ = noise(4096).iter().filter(|&&b| b != 0).count();
        let inflated = format!("inflated, {differ} of its 4128 byte(s) differ");
        assert!(report.contains(&inflated), "{report}");
        assert!(report.contains(": a patched copy of "), "{report}");
        assert!(report.contains("in 1 row(s)"), "{report}");
        assert!(report.contains("of its 2 archive(s)"), "{report}");
        assert!(report.contains(PATCHED), "{report}");
    }

    #[test]
    fn an_unchanged_copy_verifies_with_no_row_changed() {
        let (_dir, source, copy) = staged(|_| {});
        let (report, patched) =
            verify(&source, &copy, &fixture::keys(), &named()).expect("it verifies");
        assert!(patched, "{report}");
        assert!(report.contains("0 byte(s) differ, in 0 row(s)"), "{report}");
    }

    #[test]
    fn a_rebuilt_archive_is_not_a_patched_copy_and_the_line_names_the_table_span() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let (source, copy) = (dir.path().join("a.rpf"), dir.path().join("b.rpf"));
        std::fs::write(&source, nested_archive(roomy())).unwrap();
        // The deep row's first chunk deflates to far fewer bytes, so the tables are laid out again.
        let rebuilt = nested_archive(vec![(vec![0u8; 4096], true), (tail(), false)]);
        std::fs::write(&copy, rebuilt).unwrap();

        let (report, patched) =
            verify(&source, &copy, &fixture::keys(), &named()).expect("it verifies");
        assert!(!patched, "{report}");
        assert!(report.contains(": not a patched copy of "), "{report}");
        assert!(report.contains("the table span 0x0.."), "{report}");
        assert!(
            report.contains("a patched copy keeps its source's length"),
            "{report}"
        );
        assert!(report.contains(REBUILT), "{report}");
        assert!(!report.contains(PATCHED), "{report}");
    }

    #[test]
    fn one_byte_flipped_in_a_nested_table_span_is_named() {
        let bytes = nested_archive(roomy());
        let (inner, _) = inner_of(&bytes);
        let base = inner.base;
        let (_dir, source, copy) = staged(|b| b[base as usize + 0x10] ^= 0x01);
        let (report, patched) =
            verify(&source, &copy, &fixture::keys(), &named()).expect("it verifies");
        assert!(!patched, "{report}");
        let span = format!("the table span {base:#x}..");
        assert!(report.contains(&span), "{report}");
        assert!(
            report.contains(&format!(
                "differs in 1 byte(s), the first at {:#x}",
                base + 0x10
            )),
            "{report}"
        );
        assert!(report.contains(REBUILT), "{report}");
    }

    #[test]
    fn one_byte_flipped_in_a_row_that_then_does_not_inflate_is_named() {
        let bytes = nested_archive(roomy());
        let (_, start) = inner_of(&bytes);
        // Inside the first chunk's zlib stream, whose checksum then fails.
        let at = start as usize + 100;
        let (_dir, source, copy) = staged(|b| b[at] ^= 0x01);
        let (report, patched) =
            verify(&source, &copy, &fixture::keys(), &named()).expect("it verifies");
        assert!(!patched, "{report}");
        assert!(
            report.contains(&format!(
                "maps/inner.rpf/data/deep.bin: 1 stored byte(s) differ in {at:#x}..{:#x}; it does \
                 not read through its table in the copy: ",
                at + 1
            )),
            "{report}"
        );
        assert!(
            report.contains("maps/inner.rpf/data/deep.bin does not read through its table"),
            "{report}"
        );
        assert!(!report.contains(REBUILT), "{report}");
    }

    #[test]
    fn a_byte_flipped_outside_every_row_and_span_is_named() {
        let bytes = nested_archive(roomy());
        let (_, start) = inner_of(&bytes);
        // The gap before the deep row belongs to no row and to no span.
        let at = start - 1;
        let (_dir, source, copy) = staged(|b| b[at as usize] ^= 0x01);
        let (report, patched) =
            verify(&source, &copy, &fixture::keys(), &named()).expect("it verifies");
        assert!(!patched, "{report}");
        assert!(
            report.contains(&format!(
                "1 byte(s) differ outside every row's stored extent and every table span, the \
                 first at {at:#x}"
            )),
            "{report}"
        );
    }

    #[test]
    fn verify_patch_without_two_files_is_a_usage_error() {
        let args: Vec<String> = ["a.rpf"].map(String::from).to_vec();
        assert_eq!(run(&args), ExitCode::from(2));
    }

    /// A row of a real build, one byte of it changed, patched into a copy that then verifies.
    /// Install-gated: it needs the key snapshot and the build, and fails where no root archive of at
    /// most 64 MiB holds a clear chunk-tabled row whose edit fits in place.
    #[test]
    fn a_gta6_build_row_patched_into_a_copy_verifies() {
        const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
        let what = "rpf9 verify-patch over a build's patched row";
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
        let mut archives: Vec<(u64, PathBuf)> = crate::rpf9::build_archives(&build)
            .into_iter()
            .filter(|p| p.parent() == Some(build.as_path()))
            .filter_map(|p| Some((std::fs::metadata(&p).ok()?.len(), p)))
            .filter(|(len, _)| *len <= 64 << 20)
            .collect();
        archives.sort();
        // A byte set to the one before it keeps the body about as compressible as it was; with no
        // optimal parse (a cap of 0), a row whose edit no level fits is passed over.
        let found = archives.into_iter().find_map(|(_, source)| {
            let mut file = File::open(&source).ok()?;
            let mount = Gta6Mount::open_root(&mut file, &keys).ok()?;
            mount.archive.entries.iter().take(256).find_map(|entry| {
                let chunked = or_io::container::rpf::gta6_chunk_table::entry_chunks(
                    &mount.archive,
                    entry.index as usize,
                )
                .is_ok_and(|t| t.is_some());
                if !chunked || entry.cipher.is_some() || entry.real.is_none_or(|n| n > 4 << 20) {
                    return None;
                }
                let mut body = read_row_by_chunks(&mount, &mut file, &keys, entry.index).ok()?;
                let k = (body.len() / 2..body.len()).find(|&k| k > 0 && body[k] != body[k - 1])?;
                body[k] = body[k - 1];
                let patch = BodyPatch {
                    base: 0,
                    entry: entry.index,
                    body,
                };
                let plan = plan_patch_with(&mut file, &keys, &patch, 0, &mut |_, _| true).ok()?;
                (plan.chunks.iter().any(|c| c.changed)).then(|| (source.clone(), patch))
            })
        });
        let Some((source, patch)) = found else {
            panic!("{what}: no root archive of at most 64 MiB holds a row whose edit fits");
        };
        let dir = tempfile::tempdir().expect("a temp dir");
        let copy = dir.path().join("copy.rpf");
        write_patched_copy(&source, &copy, &keys, &[patch]).expect("it patches");
        let (report, patched) =
            verify(&source, &copy, &keys, &NameResolver::new()).expect("it verifies");
        assert!(patched, "{report}");
        assert!(report.contains("in 1 row(s)"), "{report}");
        assert!(report.contains("inflated, 1 of its "), "{report}");
    }
}
