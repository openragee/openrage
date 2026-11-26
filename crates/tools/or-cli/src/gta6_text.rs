//! `rpf scan-open` of a GTA VI text table and of audio game data: their structure and counts, and
//! with `--texts` a table's texts; `rpf scan-body --gxt2` and `--rewrite` of a text table.

use std::collections::BTreeMap;

use or_io::container::rpf::{
    read_rpf9_body, write_rpf9_resource, Rpf9BodyKind, Rpf9Content, Rpf9Form, Rpf9ScanFind,
    Rpf9Sections, Rpf9ZlibRun,
};
use or_io::gta5::text::Gxt2;
use or_io::gta5::File as _;
use or_io::gta6::audio_data::{AudioData, AudioFields};
use or_io::gta6::text::{read, TextItem, TextTable};

/// A text table's header, map, blocks, probe distances and text counts.
pub fn print_table(virtual_section: &[u8]) -> Result<(), String> {
    let (table, layout) = read(virtual_section).map_err(|e| e.to_string())?;
    let sets = (table.entries.iter())
        .filter(|e| matches!(e.item, TextItem::Variants(_)))
        .count();
    println!(
        "  text table    {}, {} bytes, {} virtual block(s)",
        if sets > 0 {
            "with grammatical variants"
        } else {
            "plain"
        },
        layout.length,
        layout.virtual_blocks
    );
    println!(
        "  header        0x4E u16 {}; root block {}",
        layout.word_4e, layout.root
    );
    println!(
        "  map           {} groups, {} entries",
        table.groups,
        table.entries.len()
    );
    for (i, b) in layout.blocks.iter().enumerate() {
        println!(
            "  block {:<2}      type {:08x} at {:#x} size {}",
            i + 1,
            b.kind,
            b.offset,
            b.size
        );
    }
    let mut probe = BTreeMap::new();
    for e in &table.entries {
        let home = (e.key >> 7) & (table.groups - 1);
        *probe
            .entry(e.group.wrapping_sub(home) & (table.groups - 1))
            .or_insert(0) += 1;
    }
    println!("  probe         group distance -> keys {probe:?}");
    let texts: Vec<usize> = table.texts().map(|(_, t)| t.len() + 1).collect();
    let chars = (layout.blocks.iter()).filter(|b| b.kind == 0x10).count();
    println!(
        "  texts         {} ({} of them variants, in {sets} variant entries) in {} bytes, {chars} block(s)",
        texts.len(),
        texts.len() + sets - table.entries.len(),
        texts.iter().sum::<usize>(),
    );
    let keys: Vec<String> = (table.entries.iter().take(8))
        .map(|e| format!("{:08x}", e.key))
        .collect();
    println!("  keys          first 8 in slot order: {}", keys.join(" "));
    Ok(())
}

/// Every text of the text table at `offset` of `path`, a line each: key, variant form, text.
pub fn print_texts(path: &str, offset: u64, limit: u64) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let (find, bytes) = read_rpf9_body(&file, offset, limit).map_err(|e| e.to_string())?;
    let table = table_at(&find, &bytes)?;
    for e in &table.entries {
        match &e.item {
            TextItem::Text(t) => println!("{:#010X}      {}", e.key, t.escape_debug()),
            TextItem::Variants(v) => {
                for (i, form) in v.forms.iter().enumerate() {
                    println!("{:#010X} [{i}]  {}", e.key, form.text.escape_debug());
                }
            }
        }
    }
    Ok(())
}

/// A text table's plain texts as GTA V's text table with `gxt2`, else the table written again.
pub fn converted(find: &Rpf9ScanFind, bytes: &[u8], gxt2: bool) -> Result<Vec<u8>, String> {
    let (original, physical) = sections_at(find, bytes)?;
    let (table, _) = read(original).map_err(|e| e.to_string())?;
    if gxt2 {
        return to_gxt2(&table);
    }
    let (virtual_section, rebuilt) = written(&table, original)?;
    match rebuilt {
        None => println!(
            "  the table is written over the section it was read from, every block at its size, so the body keeps its length"
        ),
        Some(why) => println!("  {why}"),
    }
    let sections = Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section,
        physical: physical.to_vec(),
    };
    write_rpf9_resource(&sections).map_err(|e| e.to_string())
}

/// `table` over `original`, the section it was read from, at its length; else laid out again, and why.
fn written(table: &TextTable, original: &[u8]) -> Result<(Vec<u8>, Option<String>), String> {
    let why = match table.to_section_over(original) {
        Ok(section) => return Ok((section, None)),
        Err(why) => why,
    };
    let section = table.to_section().map_err(|e| e.to_string())?;
    Ok((
        section,
        Some(format!(
            "the table is laid out again at another length ({why}), so an archive that takes it is rebuilt, which the game refuses because only its maker can sign the table"
        )),
    ))
}

/// The virtual and the physical section of the text table `find` reads in `bytes`.
fn sections_at<'a>(find: &Rpf9ScanFind, bytes: &'a [u8]) -> Result<(&'a [u8], &'a [u8]), String> {
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        content: Rpf9Content::TextTable,
        resource: Some(resource),
        ..
    }) = find.kind
    else {
        return Err(format!("the body at {:#X} is no text table", find.offset));
    };
    Ok(resource.sections(bytes))
}

fn table_at(find: &Rpf9ScanFind, bytes: &[u8]) -> Result<TextTable, String> {
    let (table, _) = read(sections_at(find, bytes)?.0).map_err(|e| e.to_string())?;
    Ok(table)
}

/// The plain texts of `table` as GTA V's text table, which holds no grammatical variants.
fn to_gxt2(table: &TextTable) -> Result<Vec<u8>, String> {
    let mut gxt2 = Gxt2::new();
    let mut variants = 0;
    for e in &table.entries {
        match &e.item {
            TextItem::Text(t) => {
                gxt2.insert(e.key, t.clone());
            }
            TextItem::Variants(_) => variants += 1,
        }
    }
    if variants > 0 {
        return Err(format!(
            "GTA V's text table holds one text per key, and {variants} key(s) of this table hold grammatical variants"
        ));
    }
    gxt2.to_bytes().map_err(|e| e.to_string())
}

/// Audio game data's tables and its items' classes, laid out or not.
pub fn print_audio(body: &[u8]) -> Result<(), String> {
    let data = AudioData::read(body).map_err(|e| e.to_string())?;
    println!(
        "  audio data    schema {} ({}), a data block of {} bytes",
        data.schema,
        data.kind.label(),
        data.data.len()
    );
    println!(
        "  groups        {} groups of {} hashes",
        data.groups.len(),
        data.group_hashes.len()
    );
    println!(
        "  names         {} names in {} bytes, encrypted and not read",
        data.names, data.name_bytes
    );
    println!(
        "  index         {} items; a second index of {} rows",
        data.index.len(),
        data.index_y.len()
    );
    println!(
        "  fields        hash {}, wave {}, group {}, reference rows {}, reference fields {}",
        data.hash_fields.len(),
        data.wave_fields.len(),
        data.group_fields.len(),
        data.references.len(),
        data.reference_fields.len()
    );
    let mut classes: BTreeMap<Option<u8>, (usize, usize)> = BTreeMap::new();
    for item in data.items() {
        let row = classes.entry(item.class).or_default();
        row.0 += 1;
        row.1 += usize::from(item.fields != AudioFields::Opaque);
    }
    for (class, (items, laid_out)) in classes {
        let class = class.map_or("none".to_string(), |c| c.to_string());
        println!("  class {class:<4}    {items} items, {laid_out} laid out");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta6::text::{TextEntry, Variant, Variants};

    fn table(with_variants: bool) -> TextTable {
        let mut entries = vec![
            TextEntry {
                key: 0x0000_0101,
                group: 0,
                slot: 0,
                item: TextItem::Text("une ligne".into()),
            },
            TextEntry {
                key: 0x0000_0182,
                group: 1,
                slot: 0,
                item: TextItem::Text("another line".into()),
            },
        ];
        if with_variants {
            let form = |text: &str| Variant {
                text: text.into(),
                gender: 1,
                plurality: 2,
                case: 3,
                hash_34a643c5: [[4, 5]; 9],
            };
            entries.push(TextEntry {
                key: 0x0000_0183,
                group: 1,
                slot: 1,
                item: TextItem::Variants(Variants {
                    gender: 0,
                    plurality: 1,
                    hash_27d7357f: 2,
                    hash_e0237fcf: 0,
                    forms: vec![form("one form"), form("another form")],
                }),
            });
        }
        TextTable { groups: 2, entries }
    }

    /// What a read at offset 0 makes of `file`, a loose resource.
    fn read_loose(file: Vec<u8>) -> (Rpf9ScanFind, Vec<u8>) {
        read_rpf9_body(std::io::Cursor::new(file), 0, u64::MAX).expect("it reads")
    }

    /// `table` as the loose resource a text table saves as, read at offset 0.
    fn loose(table: &TextTable) -> (Rpf9ScanFind, Vec<u8>) {
        let sections = Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section: table.to_section().expect("written"),
            physical: Vec::new(),
        };
        read_loose(write_rpf9_resource(&sections).expect("a loose resource"))
    }

    fn texts(table: &TextTable) -> Vec<(u32, String)> {
        table.texts().map(|(k, t)| (k, t.to_string())).collect()
    }

    #[test]
    fn a_text_table_converts_to_gta_v_s_and_rewrites_as_a_table_that_reads_back() {
        let (find, bytes) = loose(&table(false));
        let gxt2 = converted(&find, &bytes, true).expect("converted");
        let gxt2 = Gxt2::from_bytes(&gxt2).expect("a GXT2");
        assert_eq!(gxt2.get(0x101), Some("une ligne"));
        assert_eq!(gxt2.get(0x182), Some("another line"));
        assert_eq!(gxt2.len(), 2);

        let (find, bytes) = loose(&table(true));
        let refused = converted(&find, &bytes, true).expect_err("refused");
        assert!(refused.contains("1 key(s)"), "{refused}");
        let (find, bytes) = read_loose(converted(&find, &bytes, false).expect("rewritten"));
        let again = table_at(&find, &bytes).expect("a text table");
        assert_eq!(texts(&again), texts(&table(true)));
        assert_eq!(again.entries[..2], table(true).entries[..2]);
    }

    #[test]
    fn a_same_length_edit_keeps_the_section_s_length_and_a_longer_one_is_laid_out_again() {
        let (find, bytes) = loose(&table(true));
        let original = sections_at(&find, &bytes).expect("a text table").0.to_vec();
        let (find, bytes) = read_loose(converted(&find, &bytes, false).expect("rewritten"));
        assert_eq!(
            sections_at(&find, &bytes).expect("a text table").0,
            original,
            "an unedited table is written back as it was"
        );

        let mut edited = table(true);
        edited.entries[1].item = TextItem::Text("anOther line".into());
        let (same, said) = written(&edited, &original).expect("written");
        assert_eq!(said, None);
        assert_eq!(same.len(), original.len());
        assert_eq!(texts(&read(&same).expect("it reads").0), texts(&edited));

        edited.entries[1].item = TextItem::Text("a line longer than the one it was".into());
        let (longer, said) = written(&edited, &original).expect("written");
        assert_ne!(longer.len(), original.len());
        let said = said.expect("it says why");
        assert!(said.contains("rebuilt"), "{said}");
        assert_eq!(texts(&read(&longer).expect("it reads").0), texts(&edited));
    }

    /// A same-length edit of a text table of the build, written over its section, plans as a
    /// patched copy of its archive. Install-gated: `OPENRAGE_GTA6_KEYS` and `OPENRAGE_GTA6_BUILD`.
    #[test]
    fn a_same_length_edit_of_a_build_table_plans_as_a_patched_copy() {
        use std::fs::File;
        use std::io::{Cursor, Read, Seek, SeekFrom};
        use std::path::{Path, PathBuf};
        use std::time::{Duration, Instant};

        use or_io::container::rpf::gta6_chunk_table::entry_chunks;
        use or_io::container::rpf::{
            plan_patch, BodyPatch, Gta6Class, Gta6Entry, Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV,
        };

        const TEXT_TABLE: u32 = 0x6F;
        /// The zero slot a 20-byte row's body begins with.
        const SLOT: u64 = 16;
        const MOST_STORED: u64 = 64 << 10;
        const TRIES: usize = 4;
        const DEPTH: usize = 3;
        const DEADLINE: Duration = Duration::from_secs(120);

        struct Walk<'a> {
            file: File,
            keys: &'a GtaViKeys,
            refused: Vec<String>,
            patched: bool,
            started: Instant,
        }

        fn rpfs_under(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    rpfs_under(&path, out);
                } else if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("rpf"))
                {
                    out.push(path);
                }
            }
        }

        /// One plain text of `original`'s table changed by a letter, written over it and planned.
        fn tried(
            w: &mut Walk,
            mount: &Gta6Mount,
            e: &Gta6Entry,
            original: &[u8],
        ) -> Result<(), String> {
            let (mut table, _) = read(original).map_err(|e| format!("unread: {e}"))?;
            let letter = table
                .entries
                .iter_mut()
                .find_map(|entry| match &mut entry.item {
                    TextItem::Text(t) => {
                        Some((t.bytes().position(|b| b.is_ascii_alphabetic())?, t))
                    }
                    TextItem::Variants(_) => None,
                });
            let Some((at, text)) = letter else {
                return Err("no plain text holds a letter".into());
            };
            let swapped = if text.as_bytes()[at] == b'a' {
                "b"
            } else {
                "a"
            };
            text.replace_range(at..at + 1, swapped);
            let (section, rebuilt) =
                written(&table, original).map_err(|e| format!("unwritten: {e}"))?;
            if let Some(why) = rebuilt {
                return Err(format!("laid out again: {why}"));
            }
            let mut body = (mount.read_entry_decrypted(&mut w.file, e, w.keys))
                .map_err(|e| format!("the inflated body is unread: {e}"))?;
            if !body.starts_with(original) {
                return Err(
                    "laid out otherwise: the inflated body does not begin with its virtual section"
                        .into(),
                );
            }
            body[..section.len()].copy_from_slice(&section);
            let patch = BodyPatch {
                base: mount.base,
                entry: e.index,
                body,
            };
            plan_patch(&mut w.file, w.keys, &patch)
                .map(|_| ())
                .map_err(|why| why.to_string())
        }

        fn walk(w: &mut Walk, mount: &Gta6Mount, depth: usize) {
            let extras = mount.archive.entry_extras();
            for (e, extra) in mount.archive.entries.iter().zip(&extras) {
                if w.patched || w.refused.len() >= TRIES || w.started.elapsed() > DEADLINE {
                    return;
                }
                if e.kind() == Some(Gta6Class::NestedArchive) {
                    if depth < DEPTH {
                        if let Ok(child) = mount.open_nested(&mut w.file, e, w.keys) {
                            walk(w, &child, depth + 1);
                        }
                    }
                    continue;
                }
                let row = (extra.nested)
                    .filter(|r| r.resource_type == TEXT_TABLE && r.stored_size() <= MOST_STORED);
                let Some(row) = row else {
                    continue;
                };
                let Ok(Some(chunks)) = entry_chunks(&mount.archive, e.index as usize) else {
                    continue;
                };
                // A body whose content key the snapshot lacks is not tried.
                let stored = match (e.cipher, mount.absolute_start(e)) {
                    (Some(_), _) => mount
                        .read_entry_deciphered_stored(&mut w.file, e, w.keys)
                        .ok(),
                    (None, Some(start)) => {
                        let mut body = vec![0u8; row.stored_size().saturating_sub(SLOT) as usize];
                        let read = (w.file.seek(SeekFrom::Start(start + SLOT)))
                            .and_then(|_| w.file.read_exact(&mut body));
                        read.ok().map(|()| body)
                    }
                    (None, None) => None,
                };
                let Some(stored) = stored else {
                    continue;
                };
                let limit = chunks.iter().map(|c| u64::from(c.inflated)).sum();
                let sections = (read_rpf9_body(Cursor::new(&stored), 0, limit).ok())
                    .and_then(|(find, bytes)| Rpf9Sections::of(&find, &bytes));
                let Some(sections) = sections else {
                    continue;
                };
                match tried(w, mount, e, &sections.virtual_section) {
                    Ok(()) => w.patched = true,
                    Err(why) => w
                        .refused
                        .push(format!("entry {} at {:#x}: {why}", e.index, mount.base)),
                }
            }
        }

        let what = "a same-length text edit of a GTA VI build planned in place";
        let present = |var: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.exists())
        };
        let (Some(snapshot), Some(build)) =
            (present(GTA6_SNAPSHOT_ENV), present("OPENRAGE_GTA6_BUILD"))
        else {
            or_test_support::skip_install(
                what,
                format_args!("set {GTA6_SNAPSHOT_ENV} and OPENRAGE_GTA6_BUILD"),
            );
            return;
        };
        let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
        let mut archives = Vec::new();
        rpfs_under(&build, &mut archives);
        archives.sort();
        let started = Instant::now();
        let mut refused = Vec::new();
        let mut patched = false;
        for path in archives {
            if patched || refused.len() >= TRIES || started.elapsed() > DEADLINE {
                break;
            }
            let mut file = File::open(&path).expect("the archive opens");
            let Ok(root) = Gta6Mount::open_root(&mut file, &keys) else {
                continue;
            };
            let mut w = Walk {
                file,
                keys: &keys,
                refused: std::mem::take(&mut refused),
                patched,
                started,
            };
            walk(&mut w, &root, 0);
            (refused, patched) = (w.refused, w.patched);
        }
        if !patched && refused.is_empty() {
            or_test_support::skip_install(
                what,
                format_args!("no text table was met in its bounds"),
            );
            return;
        }
        assert!(
            !refused.iter().any(|why| why.contains("laid out")),
            "a same-length edit changed the table's layout: {refused:?}"
        );
        assert!(
            patched,
            "no same-length text edit planned as a patched copy: {refused:?}"
        );
    }
}
