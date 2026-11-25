//! `openrage-cli gta6-mission`: the mission state tables of an inflated body of a GTA VI script
//! archive, as `rpf scan-body` writes one, read by the body's own block table. Nothing is written.

use std::collections::HashMap;
use std::process::ExitCode;

use or_io::gta6::mission_tables::{state_key, Body, Run};
use or_io::gta6::names::Vocabulary;

mod other;

/// ` (name)` where `vocabulary` holds the one name the 32-bit `hash` names, else nothing.
fn said32(vocabulary: &Vocabulary, hash: u32) -> String {
    (vocabulary.name32(hash)).map_or_else(String::new, |name| format!(" ({name})"))
}

const USAGE: &str = "usage: gta6-mission <inflated_body_file> [--name STATE]...";

pub fn run(args: &[String]) -> ExitCode {
    let mut names: HashMap<u32, &str> = HashMap::new();
    let mut file: Option<&str> = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--name" | "-n" => match rest.next() {
                Some(name) => {
                    names.insert(state_key(name), name);
                }
                None => return crate::usage_error("--name needs a value"),
            },
            path if file.is_none() && !path.starts_with('-') => file = Some(path),
            _ => return crate::usage_error(USAGE),
        }
    }
    let Some(file) = file else {
        return crate::usage_error(USAGE);
    };
    let read = std::fs::read(file)
        .map_err(|e| e.to_string())
        .and_then(|bytes| lines(&bytes, &names, crate::gta6_names::vocabulary()));
    match read {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {file}: {e}");
            ExitCode::from(1)
        }
    }
}

/// The lines of the body `bytes`, a state named by `names` where given there, else by `vocabulary`.
fn lines(
    bytes: &[u8],
    names: &HashMap<u32, &str>,
    vocabulary: &Vocabulary,
) -> Result<Vec<String>, String> {
    let body = Body::open(bytes).map_err(|e| e.to_string())?;
    let tables = body.tables(bytes).map_err(|e| e.to_string())?;
    let mut out = vec![format!(
        "{} state tables of {} records in {} blocks, each found by the body's block table and type table",
        tables.len(),
        tables.iter().map(|t| t.records.len()).sum::<usize>(),
        body.blocks.len()
    )];
    for (k, table) in tables.iter().enumerate() {
        out.push(format!(
            "table {k} at {:#X}, block {}: {} states",
            table.offset,
            table.block + 1,
            table.records.len()
        ));
        for r in &table.records {
            let name =
                (names.get(&r.key)).map_or_else(|| said32(vocabulary, r.key), |n| format!(" {n}"));
            let (block, offset) = r.run.map_or((0, 0), |(block, offset)| (block + 1, offset));
            out.push(format!(
                "  {:#010X}{name}  {} elements at {offset:#X} of block {block}",
                r.key, r.count
            ));
            match body.run(bytes, &tables, r) {
                None => {}
                Some(Run::Table(to)) => out.push(format!("    records of table {to}")),
                Some(Run::Other { element }) => {
                    out.push(format!(
                        "    elements of {element:#X} bytes that are not placed in the world"
                    ));
                    out.extend(other::lines(bytes, &body, r, vocabulary));
                }
                Some(Run::Placed(elements)) => {
                    for (at, e) in elements {
                        let [x, y, z] = e.position;
                        let [a, b, c] = e.rotation;
                        out.push(format!(
                            "    element at {at:#X}: archetype {:#010X}{}, position {x:.3} {y:.3} {z:.3}, rotation {a:.3} {b:.3} {c:.3}, link {:#010X}",
                            e.archetype, said32(vocabulary, e.archetype), e.link
                        ));
                    }
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(tag);
        out.extend_from_slice(&(data.len() as u32 + 8).to_be_bytes());
        out.extend_from_slice(data);
    }

    /// A body of two blocks: a table of two states at 0x30, and the element of `archetype` the
    /// first owns at 0x70.
    fn body(archetype: u32) -> Vec<u8> {
        let mut psin = Vec::new();
        for (class, size) in [(0x101u32, 0x40u32), (0x102, 0x40)] {
            psin.extend_from_slice(&class.to_le_bytes());
            psin.extend_from_slice(&size.to_le_bytes());
            psin.extend_from_slice(&[0xAA; 8]);
        }
        // Two rows end at 0x28 of the body, so the first block starts at 0x30.
        psin.extend_from_slice(&[0x70; 8]);
        for (name, pointer, count) in [("Approach", 2u64, 1u16), ("Leave", 0, 0)] {
            let mut record = [0u8; 0x20];
            record[..4].copy_from_slice(&state_key(name).to_le_bytes());
            record[16..24].copy_from_slice(&pointer.to_le_bytes());
            record[24..26].copy_from_slice(&count.to_le_bytes());
            record[26..28].copy_from_slice(&count.to_le_bytes());
            psin.extend_from_slice(&record);
        }
        let mut element = [0u8; 0x40];
        element[..4].copy_from_slice(&archetype.to_le_bytes());
        for at in [0x10, 0x1C] {
            element[at..at + 4].copy_from_slice(&1.5f32.to_le_bytes());
        }
        element[0x30..0x34].copy_from_slice(&archetype.to_le_bytes());
        element[0x34..0x38].copy_from_slice(&0xC0DEu32.to_le_bytes());
        psin.extend_from_slice(&element);
        let mut lsch = Vec::new();
        for (class, size) in [(0x101u32, 0x20u32), (0x102, 0x40)] {
            let mut header = [0u8; 32];
            header[..4].copy_from_slice(&class.to_le_bytes());
            header[8] = 0x0C;
            header[24..28].copy_from_slice(&size.to_le_bytes());
            lsch.extend_from_slice(&header);
        }
        let mut lrsc = vec![0u8; 0x40];
        lrsc[0x38] = 2;
        lrsc[0x3C] = 2;
        let mut out = Vec::new();
        frame(&mut out, b"PSIN", &psin);
        frame(&mut out, b"LSCH", &lsch);
        for tag in [b"LENM", b"LSNS", b"LGID"] {
            frame(&mut out, tag, &[]);
        }
        frame(&mut out, b"LRSC", &lrsc);
        frame(&mut out, b"CHKS", &[0; 12]);
        out
    }

    #[test]
    fn a_body_s_tables_and_elements_are_listed_with_the_names_given() {
        let names = HashMap::from([(state_key("approach"), "approach")]);
        let none = Vocabulary::default();
        let out = lines(&body(0xABCD), &names, &none).expect("a body");
        assert!(
            out[0].starts_with("1 state tables of 2 records in 2 blocks"),
            "{}",
            out[0]
        );
        assert_eq!(out[1], "table 0 at 0x30, block 1: 2 states");
        assert!(
            out[2].ends_with(" approach  1 elements at 0x0 of block 2"),
            "{}",
            out[2]
        );
        assert_eq!(
            out[3],
            "    element at 0x70: archetype 0x0000ABCD, position 1.500 0.000 0.000, rotation 0.000 0.000 0.000, link 0x0000C0DE"
        );
        assert!(
            out[4].ends_with("  0 elements at 0x0 of block 0"),
            "{}",
            out[4]
        );
        assert_eq!(out.len(), 5);
        // Bytes that are no document are refused, where a scan once found tables in anything.
        assert!(lines(&[0u8; 0x100], &names, &none).is_err());
    }

    #[test]
    fn a_state_and_an_archetype_no_name_was_given_for_are_named_by_the_vocabulary() {
        let mut vocabulary = Vocabulary::default();
        for name in ["approach", "leave", "zq_mission_prop"] {
            assert!(vocabulary.add(name, None));
        }
        let archetype = state_key("zq_mission_prop");
        let names = HashMap::from([(state_key("approach"), "Approach_Given")]);
        let out = lines(&body(archetype), &names, &vocabulary).expect("a body");
        // A name given on the command line comes before the vocabulary's.
        assert!(out[2].contains(" Approach_Given  "), "{}", out[2]);
        assert!(
            out[3].contains(&format!("archetype {archetype:#010X} (zq_mission_prop), ")),
            "{}",
            out[3]
        );
        assert!(out[4].contains(" (leave)  0 elements"), "{}", out[4]);
    }
}
