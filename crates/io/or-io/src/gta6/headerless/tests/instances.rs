use super::*;

/// A serialized structure of `size` bytes whose primitives are `primitives`.
fn structure(size: usize, primitives: u32) -> Vec<u8> {
    let mut s = vec![0u8; size];
    let serialized = (size - 0x40) as u64;
    s[0x20..0x28].copy_from_slice(&serialized.to_le_bytes());
    s[0x28..0x30].copy_from_slice(&(serialized - 0x100).to_le_bytes());
    s[0x100..0x104].copy_from_slice(&0x8102_0006u32.to_le_bytes());
    s[0x104..0x108].copy_from_slice(&(primitives + 5).to_le_bytes());
    s[0x108..0x10C].copy_from_slice(&0xA1B2_C3D4u32.to_le_bytes());
    s[0x10C..0x110].copy_from_slice(&primitives.to_le_bytes());
    for (k, f) in [-1.5f32, -2.0, -3.0, 4.0, 5.5, 6.0].iter().enumerate() {
        s[0x110 + 4 * k..0x114 + 4 * k].copy_from_slice(&f.to_le_bytes());
    }
    s[0x128..0x12C].copy_from_slice(&0x80u32.to_le_bytes());
    s[0x13C..0x140].copy_from_slice(&0x8000_0001u32.to_le_bytes());
    s[0x140..serialized as usize].fill(0x77);
    s
}

/// A root of one instance per size, placed as the measured roots of its count are, and the
/// physical section its records tile.
fn instances(sizes: &[u32]) -> (Vec<u8>, Vec<u8>) {
    let n = sizes.len();
    let slots = (8 * n).next_multiple_of(16);
    let t = 0x50 + 48 * n;
    let (map, records) = if n <= 2 {
        (t, t + 0x20)
    } else {
        (t + 3 * slots, t)
    };
    let (a, b) = (records + slots, records + 2 * slots);
    let mut v = vec![0u8; t + 3 * slots + 32];
    for (at, word) in [
        (0x08, 0x5000_0000 + map as u64),
        (0x10, n as u64),
        (0x18, 0x5000_0000 + records as u64),
        (0x20, 0x6000_0000),
        (0x28, 0x5000_0000 + a as u64),
        (0x30, 0x5000_0000 + b as u64),
        (0x38, 0x5000_0050),
        (0x40, 0x1234_5678_9ABC_DEF0),
    ] {
        v[at..at + 8].copy_from_slice(&word.to_le_bytes());
    }
    (v[map + 8], v[map + 9]) = (1, 1);
    let mut physical = Vec::new();
    for (i, size) in sizes.iter().enumerate() {
        let transform: [f32; 12] = std::array::from_fn(|k| if k % 5 == 0 { 1.0 } else { 0.0 });
        for (k, f) in transform.iter().enumerate() {
            v[0x50 + 48 * i + 4 * k..0x54 + 48 * i + 4 * k].copy_from_slice(&f.to_le_bytes());
        }
        let word = (4 * i as u32 % 64) << 16 | 0x100;
        v[records + 8 * i..records + 8 * i + 4].copy_from_slice(&size.to_le_bytes());
        v[records + 8 * i + 4..records + 8 * i + 8].copy_from_slice(&word.to_le_bytes());
        if *size > 0 {
            physical.extend(structure(*size as usize, 10 + i as u32));
        }
    }
    (v, physical)
}

#[test]
fn a_root_of_any_count_reads_its_instances_and_each_structure_s_head() {
    for sizes in [
        &[0x200][..],
        &[0x200, 0x300],
        &[0x400, 0, 0x200],
        &[0x200; 6],
    ] {
        let (v, p) = instances(sizes);
        let read = read_instances(&v, Some(&p)).expect("it reads");
        assert_eq!(read.key, 0x1234_5678_9ABC_DEF0);
        assert_eq!(read.instances.len(), sizes.len());
        assert_eq!(read.records_size(), p.len() as u64);
        for (i, (instance, size)) in read.instances.iter().zip(sizes).enumerate() {
            assert_eq!(instance.size, *size);
            assert_eq!(instance.word, (4 * i as u32 % 64) << 16 | 0x100);
            assert_eq!(instance.transform[0], 1.0);
            match &instance.structure {
                None => assert_eq!(*size, 0),
                Some(s) => {
                    assert_eq!(s.serialized_size, u64::from(*size) - 0x40);
                    assert_eq!((s.primitives, s.count), (10 + i as u32, 15 + i as u32));
                    assert_eq!(
                        (s.box_min, s.box_max),
                        ([-1.5, -2.0, -3.0], [4.0, 5.5, 6.0])
                    );
                    assert_eq!((s.word_108, s.offset_128), (0xA1B2_C3D4, 0x80));
                    assert_eq!(s.words_130, [0, 0, 0, 0x8000_0001]);
                }
            }
        }
        // The section rounded up to 64 KiB reads the same.
        let mut padded = p.clone();
        padded.resize(0x1_0000, 0);
        assert_eq!(read_instances(&v, Some(&padded)).expect("padded"), read);
        padded[p.len()] = 1;
        assert!(read_instances(&v, Some(&padded)).is_err());
    }
}

#[test]
fn a_root_off_the_measured_rules_is_refused() {
    let (v, p) = instances(&[0x200, 0x300, 0x200]);
    let refused = |v: &[u8], p: &[u8]| read_instances(v, Some(p)).is_err();
    assert!(!refused(&v, &p));
    let mut longer = v.clone();
    longer.extend([0; 16]);
    assert!(refused(&longer, &p), "a length its count does not give");
    let mut stray = v.clone();
    stray[0x4C] = 1;
    assert!(refused(&stray, &p), "a byte outside the arrays");
    let mut slot = v.clone();
    let a = (u64::from_le_bytes(v[0x28..0x30].try_into().expect("8")) & 0xFFFF) as usize;
    slot[a] = 1;
    assert!(refused(&slot, &p), "a slot array that is not zero");
    let mut nan = v.clone();
    nan[0x54..0x58].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(refused(&nan, &p), "a transform that is not finite");
    assert!(
        refused(&v, &p[..p.len() - 0x100]),
        "a section shorter than the records"
    );
    let mut serialized = p.clone();
    serialized[0x20] = 0x48;
    assert!(refused(&v, &serialized), "a serialized size off 64");
    let mut huge = p.clone();
    huge[0x20..0x28].copy_from_slice(&0xFFFF_FFFF_FFFF_FFC0u64.to_le_bytes());
    assert!(refused(&v, &huge), "a serialized size past the record");
    let mut deserialized = p.clone();
    deserialized[0x28..0x30].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(refused(&v, &deserialized), "a deserialized size at the top");
    let mut format = p.clone();
    format[0x100] = 0;
    assert!(refused(&v, &format), "another format word");
    let mut offset = p.clone();
    offset[0x128] = 0x40;
    assert!(refused(&v, &offset), "an offset off 128");
    let (zero, _) = instances(&[0]);
    assert!(refused(&zero, &[]), "no record with a structure");
}

#[test]
fn a_root_whose_records_start_where_its_block_map_does_is_refused() {
    let (mut v, p) = instances(&[0x200]);
    let map = v[0x08..0x10].to_vec();
    v[0x18..0x20].copy_from_slice(&map);
    assert!(read_instances(&v, Some(&p)).is_err());
}
