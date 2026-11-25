//! The lines a scanned keyed-blob descriptor, asset index or neighbour table opens to: its fields,
//! then what of it is not known.

use or_io::gta6::headerless::{AssetIndex, KeyedBlobDescriptor, NeighbourTable};

/// A 64-bit key by the name the harvested vocabulary holds for it, else `hash_` and its digits.
fn key_name(key: u64) -> String {
    super::gta6_names::said64(key)
}

/// A descriptor's key, stream length, pair and words.
pub(super) fn descriptor_lines(d: &KeyedBlobDescriptor, lines: &mut Vec<String>) {
    let words: Vec<String> = d.words.iter().map(|w| format!("{w:#X}")).collect();
    lines.push(tr!(
        "A descriptor of the keyed blob of key {key}, the blob after it that states the same key: a stream of {size} and the pair {first}, {second} at 0x50.",
        key = format!("{:016X}", d.key),
        size = or_job::human_bytes(d.stream.len() as u64),
        first = d.pair[0],
        second = d.pair[1],
    ));
    lines.push(tr!(
        "Its words at 0x30 to 0x44 are {words}. Their units and the stream's grammar are not known, so the stream is kept as it is.",
        words = words.join(", "),
    ));
}

/// An asset index's assets with their value records, then each key of tables 1 to 4 with its assets.
pub(super) fn index_lines(index: &AssetIndex, lines: &mut Vec<String>) {
    lines.push(tr!(
        "An asset index of {assets} assets, each keyed by the 64-bit hash of its name. What an asset's value record holds is not known, so it is kept as it is.",
        assets = index.assets.len(),
    ));
    for (key, at) in &index.assets {
        lines.push(tr!(
            "Asset {key}: its value record at {at}.",
            key = key_name(*key),
            at = format!("{at:#X}"),
        ));
    }
    for (t, group) in index.groups.iter().enumerate() {
        for (key, listed) in group {
            let listed: Vec<String> = listed.iter().map(|a| key_name(*a)).collect();
            lines.push(tr!(
                "Table {table}, key {key}: the assets {assets}.",
                table = t + 1,
                key = key_name(*key),
                assets = listed.join(", "),
            ));
        }
    }
}

/// A neighbour table's element count, then each element's four neighbours.
pub(super) fn neighbour_lines(table: &NeighbourTable, lines: &mut Vec<String>) {
    lines.push(tr!(
        "A table of {elements} elements of four neighbours each, every one another element's index. Which mesh it belongs to is not known.",
        elements = table.elements.len(),
    ));
    for (i, [a, b, c, d]) in table.elements.iter().enumerate() {
        lines.push(tr!(
            "Element {index}: {a}, {b}, {c} and {d}.",
            index = i,
            a = a,
            b = b,
            c = c,
            d = d,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_asset_index_lists_each_asset_s_record_and_each_group_s_assets_by_key() {
        let index = AssetIndex {
            assets: vec![
                (0x0A0B_0C0D_0E0F_1011, 0x7B0),
                (0x1213_1415_1617_1819, 0x7C0),
            ],
            groups: [
                Vec::new(),
                vec![(0x2021_2223_2425_2627, vec![0x1213_1415_1617_1819])],
                Vec::new(),
                Vec::new(),
            ],
        };
        let mut lines = Vec::new();
        index_lines(&index, &mut lines);
        assert!(
            lines[0].starts_with("An asset index of 2 assets"),
            "{lines:?}"
        );
        assert_eq!(
            lines[1],
            "Asset hash_0A0B0C0D0E0F1011: its value record at 0x7B0."
        );
        assert_eq!(
            lines[3],
            "Table 2, key hash_2021222324252627: the assets hash_1213141516171819."
        );
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn a_descriptor_and_a_neighbour_table_say_their_fields() {
        let d = KeyedBlobDescriptor {
            key: 0x0123_4567_89AB_CDEF,
            words: [0, 0x21, 0x43, 0x50, 0x65, 0x70],
            pair: [7, 8],
            stream: vec![9; 76],
        };
        let mut lines = Vec::new();
        descriptor_lines(&d, &mut lines);
        assert!(lines[0].contains("key 0123456789ABCDEF"), "{lines:?}");
        assert!(lines[0].contains("the pair 7, 8 at 0x50"), "{lines:?}");
        assert!(
            lines[1]
                .starts_with("Its words at 0x30 to 0x44 are 0x0, 0x21, 0x43, 0x50, 0x65, 0x70."),
            "{lines:?}"
        );
        let table = NeighbourTable {
            elements: vec![[1, 2, 1, 2], [0, 2, 2, 0], [0, 1, 1, 0]],
        };
        let mut lines = Vec::new();
        neighbour_lines(&table, &mut lines);
        assert!(lines[0].starts_with("A table of 3 elements"), "{lines:?}");
        assert_eq!(lines[2], "Element 1: 0, 2, 2 and 0.");
    }
}
