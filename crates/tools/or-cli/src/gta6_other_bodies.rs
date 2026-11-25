//! What `rpf scan-open` prints of a GTA VI body its head names: the facts its reader reads.

use or_io::gta6::move_network::MoveOutline;
use or_io::gta6::other_bodies::{
    Gta6HeightGrid, Gta6HeightMap, MoveNetworkHead, OtherBody, SpanGrid,
};

pub(crate) fn print(body: OtherBody, bytes: &[u8]) -> Result<(), String> {
    let refused = |e: or_io::IoError| {
        format!(
            "the reader of a {} does not read this body: {e}",
            body.label()
        )
    };
    match body {
        OtherBody::HeightMap => {
            let map = Gta6HeightMap::open(bytes).map_err(refused)?;
            let grid = &map.grid;
            println!(
                "  height map    GTA V's grid stored little-endian: {} x {} cells over {:?} .. {:?}",
                grid.num_cols, grid.num_rows, grid.bounds_min, grid.bounds_max
            );
            println!(
                "  bytes 5..11   {} (carried: they hold neither GTA V's cell size, pad nor flags)",
                hex(&map.carried)
            );
            match map.tile() {
                Some((side, [column, row])) => {
                    println!("  tile          column {column}, row {row} from the origin, of {side} m tiles")
                }
                None => println!("  tile          none: its bounds are not one square tile"),
            }
        }
        OtherBody::HeightGrid => {
            let grid = Gta6HeightGrid::open(bytes).map_err(refused)?;
            let cells: usize = grid.rows.iter().map(|r| r.cells.len()).sum();
            let empty = grid.rows.iter().filter(|r| r.cells.is_empty()).count();
            println!(
                "  height grid   {} x {} cells of one byte over {:?} .. {:?}",
                grid.num_cols,
                grid.rows.len(),
                grid.bounds_min,
                grid.bounds_max
            );
            println!("  spans         {cells} cells in its row spans; {empty} rows empty");
            let [fill, carried @ ..] = grid.carried.to_le_bytes();
            println!(
                "  byte 0x28     {fill}, which a cell outside every span reads as; bytes 0x29..0x2B {} carried",
                hex(&carried)
            );
        }
        OtherBody::SpanGrid => {
            let grid = SpanGrid::open(bytes).map_err(refused)?;
            println!(
                "  span grid     128 x 128: rows {}..{} with two spans each over {} cells of two byte planes",
                grid.first_row,
                usize::from(grid.first_row) + grid.spans.len(),
                grid.planes[0].len()
            );
            println!(
                "  word          kind {}, word {:#010X} at {} (its low half is its high half's low byte | 0x80); the planes hold data over land, not heights, and what they hold is not known",
                grid.kind,
                grid.word(),
                if grid.kind >= 2 { "0x14" } else { "0x18" }
            );
        }
        OtherBody::MoveNetwork => {
            let outline = MoveOutline::open(bytes).map_err(refused)?;
            let head = &outline.head;
            println!(
                "  move network  version 0x13, name hash_{:08X}; lists of {}, {} and {} entries, word {}",
                head.name_hash,
                head.list_a.len(),
                head.list_b.len(),
                head.list_d.len(),
                head.word_after_b
            );
            let said = |h: u32| format!("hash_{h:08X}{}", super::gta6_names::said32(h));
            for line in list_lines(head, said) {
                println!("{line}");
            }
            let named = (super::gta6_names::vocabulary().name32(head.root_id))
                .map_or_else(String::new, |name| format!(" ({name}, from the text)"));
            println!(
                "  root node     type {}, index {}, id {:#010X}{named} at {:#X}",
                head.root_type, head.root_index, head.root_id, outline.root
            );
            match outline.stops.first() {
                None => println!(
                    "  nodes         {} read, which tile the body from the root to its end with {} tails no offset names",
                    outline.nodes.len(),
                    outline.loose_tails
                ),
                Some(stop) => println!(
                    "  nodes         {} read; {} reached whose layout is not known, the first of type {} at {:#X} (word {:#010X}): the rest is not read",
                    outline.nodes.len(),
                    outline.stops.len(),
                    stop.kind,
                    stop.at,
                    stop.word
                ),
            }
        }
        OtherBody::Gfx => {
            let movie = or_io::gfx::GfxMovie::open(bytes).map_err(refused)?;
            print!("{}", super::gfx::report(&movie));
        }
        OtherBody::Edup => {
            return Err(
                "EDUP: the bytes after its head are of high entropy with no structure found, so they are not read"
                    .to_string(),
            )
        }
        OtherBody::ObjectTrees => super::gta6_scripts::print_object_trees(bytes)?,
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The head's lists A, B and D, each hash as `hash` names it and a list D entry's byte after it.
fn list_lines(head: &MoveNetworkHead, hash: impl Fn(u32) -> String) -> Vec<String> {
    let line = |list: &str, entries: Vec<String>| {
        let entries = if entries.is_empty() {
            "-".to_string()
        } else {
            entries.join(", ")
        };
        format!("  list {list}        {entries}")
    };
    let d = (head.list_d.iter())
        .map(|(h, byte)| format!("{} [{byte:02X}]", hash(*h)))
        .collect();
    vec![
        line("A", head.list_a.iter().map(|h| hash(*h)).collect()),
        line("B", head.list_b.iter().map(|h| hash(*h)).collect()),
        line("D", d),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_head_s_lists_print_their_hashes_named_where_the_names_hold_them() {
        let head = MoveNetworkHead {
            name_hash: 0,
            list_a: vec![0xA1, 0xA2],
            list_b: Vec::new(),
            word_after_b: 0,
            list_d: vec![(0xD1, 0), (0xA1, 2)],
            before_root: [0; 10],
            root_type: 2,
            root_index: 0,
            root_id: 7,
            rest: Vec::new(),
        };
        let said = |h: u32| match h {
            0xA1 => format!("hash_{h:08X} (walk)"),
            h => format!("hash_{h:08X}"),
        };
        assert_eq!(
            list_lines(&head, said),
            [
                "  list A        hash_000000A1 (walk), hash_000000A2",
                "  list B        -",
                "  list D        hash_000000D1 [00], hash_000000A1 (walk) [02]",
            ]
        );
    }
}
