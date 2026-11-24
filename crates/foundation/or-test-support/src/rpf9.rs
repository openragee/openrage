//! Synthetic cipher material and hand-built RPF9 tables, containing no game data.

/// Synthetic invertible white-box tables with a bootstrap key and one index key.
pub fn keys() -> Vec<u8> {
    let mut bytes = b"ORGTAVIK".to_vec();
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(0x4342u32.to_le_bytes());
    bytes.extend([0; 32 + 16]);
    bytes.extend((16u32 * 256).to_le_bytes());
    for _ in 0..16 {
        bytes.extend(0..=255u8);
    }
    bytes.extend((16u32 * 256 * 4).to_le_bytes());
    let lanes = [[0, 13, 10, 7], [1, 14, 4, 11], [5, 2, 8, 15], [9, 6, 12, 3]];
    for table in 0..16 {
        let shift = lanes
            .iter()
            .find_map(|lane| lane.iter().position(|&t| t == table))
            .expect("table lane")
            * 8;
        for byte in 0..256u32 {
            bytes.extend((byte << shift).to_le_bytes());
        }
    }
    bytes.extend(2u32.to_le_bytes());
    for id in [0x1000_0001_0200_0000u64, 0x1234_5678_0200_0000] {
        let blob_size = 4 + 60 * 4 + 16 * 256;
        bytes.extend(id.to_le_bytes());
        bytes.extend((blob_size as u32).to_le_bytes());
        bytes.extend([0x8e, 0x9b, 0x3b, 0x4e]);
        bytes.extend([0; 60 * 4]);
        for _ in 0..16 {
            bytes.extend(0..=255u8);
        }
    }
    bytes
}

/// A plaintext table of `(name64, start, stored length, class)` rows, with an external name column.
pub fn table(entries: &[(u64, u64, u64, u8)]) -> Vec<u8> {
    let count = entries.len();
    assert!((1..=u16::MAX as usize).contains(&count));
    let width = if count <= u8::MAX as usize { 1 } else { 2 };
    let metadata_start = 64 + 4 * 8;
    let group_start = metadata_start + 0x150;
    let index_start = group_start + 3 * 24;
    let main_start = (index_start + 2 * count * width).next_multiple_of(8);
    let split = (main_start + count * (24 + 4)).next_multiple_of(16);
    let external_start = (split + count * width).next_multiple_of(8);
    let end = split + (count * (8 + width)).next_multiple_of(16);
    let mut bytes = vec![0; end];
    bytes[..4].copy_from_slice(b"9FPR");
    let put32 = |bytes: &mut [u8], at: usize, value: usize| {
        bytes[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes())
    };
    put32(&mut bytes, 4, end);
    put32(&mut bytes, 8, split);
    put32(&mut bytes, 0x0c, 0x40);
    bytes[0x19] = 0x7a;
    bytes[0x1a] = 4;
    put32(&mut bytes, 0x1c, 0x150);
    bytes[0x20] = 3;
    bytes[0x22] = 1;
    put32(&mut bytes, 0x24, count);
    bytes[0x2c] = width as u8;
    bytes[0x2d] = 0x0f;
    bytes[0x2e] = 0x09;
    put32(&mut bytes, 0x30, 0x10);
    bytes[0x40] = 1;
    put32(&mut bytes, metadata_start, 0x1234_5678);
    bytes[metadata_start + 4] = 0x10;
    for (group, (hash, size, offset, external)) in [
        (0x33F7_0DFA_F2C5_0989u64, 24usize, 0, false),
        (0xB707_6B62_45D3_5FB5, 4, count * 24, false),
        (0x73BC_B72D_CE74_F11F, 8, 0, true),
    ]
    .into_iter()
    .enumerate()
    {
        let at = group_start + group * 24;
        bytes[at..at + 8].copy_from_slice(&hash.to_le_bytes());
        put32(&mut bytes, at + 8, count);
        put32(&mut bytes, at + 12, offset);
        put32(&mut bytes, at + 16, size | ((external as usize) << 16));
    }
    for (i, &(name, start, stored, class)) in entries.iter().enumerate() {
        assert_eq!((start % 16, stored % 16), (0, 0));
        for at in [
            index_start + i * 2 * width,
            index_start + (i * 2 + 1) * width,
            split + i * width,
        ] {
            bytes[at..at + width].copy_from_slice(&(i as u16).to_le_bytes()[..width]);
        }
        let at = main_start + i * 24;
        for (j, value) in [(start / 16) | ((class as u64) << 40), stored / 16, stored]
            .into_iter()
            .enumerate()
        {
            bytes[at + j * 8..at + (j + 1) * 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes[external_start + i * 8..external_start + (i + 1) * 8]
            .copy_from_slice(&name.to_le_bytes());
    }
    bytes
}

/// A plaintext 20-byte entry at 0x1000 with mode-2 `(decoded, stored, compressed)` chunk descriptors.
pub fn resource_table(chunks: &[(u32, u32, bool)]) -> Vec<u8> {
    let metadata_start = 64 + 4 * 8;
    let groups = metadata_start + 0x150;
    let indices = groups + 3 * 24;
    let data = (indices + 2usize).next_multiple_of(8);
    let cell = data + 20;
    let heap = cell + 8;
    let split = (heap + chunks.len() * 8).next_multiple_of(16);
    let end = split + 16;
    let mut bytes = vec![0; end];
    let put32 = |bytes: &mut [u8], at: usize, value: usize| {
        bytes[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes())
    };
    bytes[..4].copy_from_slice(b"9FPR");
    for (at, value) in [
        (4, end),
        (8, split),
        (0x0c, 0x40),
        (0x1c, 0x150),
        (0x24, 1),
        (0x30, 0x10),
        (metadata_start, 0x1234_5678),
    ] {
        put32(&mut bytes, at, value);
    }
    bytes[0x19] = 0x7a;
    bytes[0x1a] = 4;
    bytes[0x20] = 3;
    bytes[0x22] = 1;
    bytes[0x2c] = 1;
    bytes[0x2d] = 0x0f;
    bytes[0x2e] = 0x09;
    bytes[0x40] = 1;
    bytes[metadata_start + 4] = 0x10;
    for (i, (hash, width, offset)) in [
        (0xBA18_0AFB_232D_1B4Bu64, 20, 0),
        (0xE0C1_2D7A_5D48_84C5, 8, 20),
        (0x73BC_B72D_CE74_F11F, 8 | (1 << 16), 0),
    ]
    .into_iter()
    .enumerate()
    {
        let at = groups + i * 24;
        bytes[at..at + 8].copy_from_slice(&hash.to_le_bytes());
        put32(&mut bytes, at + 8, 1);
        put32(&mut bytes, at + 12, offset);
        put32(&mut bytes, at + 16, width);
    }
    let stored: usize = chunks
        .iter()
        .map(|c| (c.1 as usize).next_multiple_of(16))
        .sum();
    put32(&mut bytes, data, u32::MAX as usize);
    put32(&mut bytes, data + 8, 1 + stored / 16);
    put32(&mut bytes, data + 12, 0x1000 / 16);
    put32(&mut bytes, cell, heap - cell);
    put32(&mut bytes, cell + 4, 0x0208_0000 | chunks.len());
    bytes[split + 8..split + 16].copy_from_slice(&1u64.to_le_bytes());
    for (i, &(decoded, stored, compressed)) in chunks.iter().enumerate() {
        assert_eq!(decoded % 16, 0);
        let aligned = u64::from(stored).next_multiple_of(16);
        let word = u64::from(decoded / 16)
            | ((aligned / 16) << 32)
            | ((aligned - u64::from(stored)) << 58)
            | (if compressed { 3 } else { 0 } << 62);
        bytes[heap + i * 8..heap + (i + 1) * 8].copy_from_slice(&word.to_le_bytes());
    }
    bytes
}
