//! A GTA VI table-form PSO whose root holds one `StringHash`, its text in the enciphered category-1
//! `LSNS` run, and a key snapshot whose string tables are synthetic and of the shape the encrypt side
//! inverts, so a save that enciphers a new text is tested with no game byte. Shared by the app's and
//! the CLI's tests.

use or_io::container::rpf::GtaViKeys;
use or_io::meta::binary::pso::string_blob::CATEGORY_1_RUN;
use or_io::meta::instance::HashedString;
use or_io::meta::Value;

/// The text the document's run holds and its member names.
pub(crate) const HELD: &str = "zq_held";

const ROOT: u32 = 0x1111_0001;
const MEMBER: u32 = 0x2222_0000;
const STRING_HASH: u8 = 0x4A;
/// The round tables feeding each output lane of the cipher.
const LANES: [[usize; 4]; 4] = [[0, 13, 10, 7], [1, 14, 4, 11], [5, 2, 8, 15], [9, 6, 12, 3]];

fn mix8(b: u8) -> u8 {
    let (mut y, mut acc) = (0u8, 0u8);
    for i in (0..8).rev() {
        acc ^= (b >> i) & 1;
        y |= acc << i;
    }
    y
}

/// An `ORGTAVIK` snapshot with no index or content key and a string section of synthetic tables:
/// permutation maps and invertible round tables. Never the real material.
pub(crate) fn string_keys() -> GtaViKeys {
    let mut out = b"ORGTAVIK".to_vec();
    out.extend(1u32.to_le_bytes()); // version
    out.extend(0x4342u32.to_le_bytes()); // CBC mode tag
    out.extend([0u8; 32 + 16]); // exe sha256, bootstrap iv
    for tag in [None, Some(0x5243u32)] {
        if let Some(tag) = tag {
            out.extend(tag.to_le_bytes()); // content mode tag
        }
        out.extend(4096u32.to_le_bytes());
        out.extend((0..4096).map(|i| i as u8));
        out.extend(16384u32.to_le_bytes());
        out.extend((0..16384).map(|i| i as u8));
        out.extend(0u32.to_le_bytes()); // no keys
    }
    out.extend(0x5453u32.to_le_bytes()); // string mode tag
    out.extend(4096u32.to_le_bytes());
    for m in 0..16 {
        out.extend((0..256).map(|x| ((x * 5 + m * 11 + 1) % 256) as u8));
    }
    out.extend(16384u32.to_le_bytes());
    let mut slot_of = [0usize; 16];
    for lane in LANES {
        for (slot, &k) in lane.iter().enumerate() {
            slot_of[k] = slot;
        }
    }
    for (k, &slot) in slot_of.iter().enumerate() {
        let constant = 0x0A0B_0C0Du32.wrapping_mul(k as u32 + 1);
        for x in 0..256u32 {
            let s = (x as u8).rotate_left(3) ^ (k as u8).wrapping_mul(0x5B);
            out.extend((constant ^ (u32::from(mix8(s)) << (slot * 8))).to_le_bytes());
        }
    }
    out.extend([0x5A; 16]); // iv
    out.extend(4340u32.to_le_bytes());
    out.extend([0x50, 0x4f, 0x86, 0x5e]); // string blob magic
    out.extend((0..60u32).flat_map(|i| i.wrapping_mul(0x9E37_79B1).to_le_bytes()));
    for m in 0..16 {
        out.extend((0..256).map(|x| ((x * 7 + m * 3) % 256) as u8));
    }
    GtaViKeys::parse(&out).expect("the synthetic snapshot parses")
}

fn frame(out: &mut Vec<u8>, ident: &[u8; 4], content: &[u8]) {
    out.extend_from_slice(ident);
    out.extend_from_slice(&(content.len() as u32 + 8).to_be_bytes());
    out.extend_from_slice(content);
}

/// The table-form document: one root of one `StringHash` holding [`HELD`], whose text is the one
/// string of its category-1 run, enciphered with `keys`.
pub(crate) fn document(keys: &GtaViKeys) -> Vec<u8> {
    let mut psin = [ROOT.to_le_bytes(), 4u32.to_le_bytes()].concat();
    psin.extend([0xAA; 8]);
    psin.extend([0x70; 8]);
    psin.extend(or_io::string_hash(HELD.as_bytes()).to_le_bytes());
    psin.resize(40, 0);
    let mut lsch = [ROOT.to_le_bytes(), 0x5151_5151u32.to_le_bytes()].concat();
    lsch.extend([0, 3, 0, 0, 0, 0, 0, 0]);
    lsch.extend([0xAA; 8]);
    lsch.extend([4, 0, 0, 0, 0, 0, 1, 0]);
    lsch.extend(MEMBER.to_le_bytes());
    lsch.extend([0, 0, 0, 0, STRING_HASH, 0, 0, 0, 0, 0, 0, 0]);
    let mut run = [HELD.as_bytes(), b"\0"].concat();
    run.resize(16, 0);
    keys.string_encrypt(&mut run)
        .expect("the synthetic tables encipher");
    let mut lsns = 1u32.to_le_bytes().to_vec();
    lsns.extend(CATEGORY_1_RUN.to_le_bytes());
    lsns.extend((run.len() as u32).to_le_bytes());
    lsns.extend(run);
    let mut lrsc = vec![0u8; 0x50];
    lrsc[0..4].copy_from_slice(b"0DRP");
    lrsc[4..6].copy_from_slice(&0x7Au16.to_le_bytes());
    lrsc[0x10..0x14].copy_from_slice(&1u32.to_le_bytes());
    lrsc[0x18..0x38].fill(0xBB);
    lrsc[0x38..0x3A].copy_from_slice(&1u16.to_le_bytes());
    lrsc[0x3C..0x3E].copy_from_slice(&1u16.to_le_bytes());
    lrsc[0x40..0x48].fill(0xBB);
    let mut out = Vec::new();
    frame(&mut out, b"PSIN", &psin);
    frame(&mut out, b"LSCH", &lsch);
    frame(&mut out, b"LENM", &[]);
    frame(&mut out, b"LSNS", &lsns);
    frame(&mut out, b"LGID", &[]);
    frame(&mut out, b"LRSC", &lrsc);
    let mut chks = (out.len() as u32 + 20).to_be_bytes().to_vec();
    chks.extend([0; 4]);
    chks.extend(b"zppp");
    frame(&mut out, b"CHKS", &chks);
    out
}

/// The 32-bit hash a `StringHash` value holds.
pub(crate) fn hash_of(value: &Value) -> Option<u32> {
    match value {
        Value::Hash(h) => Some(h.hash),
        Value::String(s) => Some(HashedString::from_xml_text(s).hash),
        _ => None,
    }
}
