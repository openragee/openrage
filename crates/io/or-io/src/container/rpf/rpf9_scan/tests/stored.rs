//! An archive whose header states a platform that stores bodies as they are: what a scan lists of
//! it, and that the same bytes under the other platform's letter go to the zlib strategy.

use super::*;
use crate::gta6::platform::PlatformFacts;
use crate::platform::Platform;

/// A head as `rpf9_head` writes it, with the letter and the header word of `platform`.
fn head_of(platform: Platform, entries: u32, seed: u32) -> Vec<u8> {
    let facts = PlatformFacts::of(platform).expect("a measured platform");
    let mut head = rpf9_head(entries, TABLE, seed);
    head[0x19] = facts.platform.extension_letter() as u8;
    head[0x2E] = facts.header_word;
    head
}

/// The archive `head` starts, which holds every rule of its platform, listed at the end of `l`.
fn archive(l: &mut Layout, head: &[u8]) {
    let find = Rpf9ScanFind {
        offset: l.at(),
        kind: Rpf9BodyKind::Archive(Rpf9Header::parse(head).expect("a head")),
        span: u64::from(TABLE),
        stored_len: None,
        inflated_len: None,
        container: None,
    };
    l.claim(find, head);
}

/// The row of a body stored as it is at `offset`, of `len` bytes where its bytes state them.
fn stored(
    offset: u64,
    content: Rpf9Content,
    signature: Option<Rpf9Signature>,
    len: Option<u64>,
) -> Rpf9ScanFind {
    let run = Rpf9ZlibRun {
        content,
        chunks: 0,
        stored_pieces: len.map_or(0, |len| len.div_ceil(PIECE) as u32),
        first_chunk_len: 0,
        first_chunk_inflated: 0,
        inflated: len.unwrap_or(0),
        signature,
        resource: None,
    };
    Rpf9ScanFind {
        offset,
        kind: Rpf9BodyKind::Zlib(run),
        span: len.map_or(16, |len| len.next_multiple_of(16)),
        stored_len: len,
        inflated_len: len,
        container: None,
    }
}

/// `body`, which states no end, at the end of `l`: its first slot listed and the rest unclaimed.
fn endless(l: &mut Layout, content: Rpf9Content, signature: Option<Rpf9Signature>, body: &[u8]) {
    l.claim(stored(l.at(), content, signature, None), &body[..16]);
    l.unclaimed(&body[16..]);
    l.pad();
}

#[test]
fn a_ps5_archive_lists_its_stored_bodies_and_the_same_bytes_under_the_xbox_letter_list_none() {
    let mut l = Layout::default();
    archive(&mut l, &head_of(Platform::Ps5, 6, 1));
    // Packed from the table's end: a document of two 64 KiB pieces that states its end.
    let document = pso(0x1_2345);
    let document_at = l.at();
    l.claim(
        stored(document_at, Rpf9Content::Pso, None, Some(0x1_2345)),
        &document,
    );
    let mut text = BOM.to_vec();
    text.extend(xml(0x321));
    endless(&mut l, Rpf9Content::Xml, None, &text);
    let mut rbf = noise(0x150, 3);
    rbf[..8].copy_from_slice(b"RBF0\xFB\xFF\x01\0");
    endless(&mut l, Rpf9Content::Rbf, None, &rbf);
    // A resource behind its zero slot, whose root the measured layouts place as a bound.
    l.unclaimed(&[0; 16]);
    let mut section = virtual_section(0x300, (1, 1));
    section[0x18..0x20].fill(0);
    let signature = Rpf9Signature::of(&section, section.len() as u64);
    assert_eq!(signature.and_then(|s| s.role()), Some(Rpf9Role::Bound));
    let section_at = l.at();
    endless(&mut l, Rpf9Content::ResourceShaped, signature, &section);
    // A nested archive of the same platform, and a document after its table.
    archive(&mut l, &head_of(Platform::Ps5, 2, 2));
    l.claim(
        stored(l.at(), Rpf9Content::Pso, None, Some(0x400)),
        &pso(0x400),
    );

    let finds = scan_all(&l.bytes).0;
    assert_eq!(finds, l.finds);
    assert_eq!(scan_in_slices(&l.bytes, 0x400).0, l.finds);
    let Rpf9BodyKind::Zlib(run) = finds[1].kind else {
        panic!("a document, not {:?}", finds[1].kind);
    };
    assert_eq!((run.chunks, run.stored_pieces), (0, 2));
    assert_eq!(Rpf9Family::of(&finds[4]), Rpf9Family::Collision);

    let read = |at| read_rpf9_body(Cursor::new(&l.bytes), at, u64::MAX);
    assert_eq!(
        read(document_at).expect("it states its end"),
        (finds[1], document)
    );
    assert!(matches!(
        read(section_at),
        Err(Rpf9ScanError::ResourceRefused { offset, .. }) if offset == section_at
    ));

    // The top header alone states the platform: under the Xbox's, no stored body is listed.
    let mut xbox = l.bytes.clone();
    xbox[..RPF9_HEADER_SIZE]
        .copy_from_slice(&head_of(Platform::XboxSeries, 6, 1)[..RPF9_HEADER_SIZE]);
    let finds = scan_all(&xbox).0;
    let archives = |finds: &[Rpf9ScanFind]| {
        (finds.iter())
            .filter(|f| matches!(f.kind, Rpf9BodyKind::Archive(_)))
            .count()
    };
    assert_eq!(archives(&finds), 2);
    assert!(!finds
        .iter()
        .any(|f| matches!(f.kind, Rpf9BodyKind::Zlib(_))));
}

#[test]
fn a_document_off_a_slot_and_a_section_behind_no_zero_slot_are_not_listed() {
    let mut l = Layout::default();
    archive(&mut l, &head_of(Platform::Ps5, 4, 1));
    l.claim(
        stored(l.at(), Rpf9Content::Pso, None, Some(0x400)),
        &pso(0x400),
    );
    l.unclaimed(&noise(8, 5));
    l.unclaimed(&pso(0x200));
    l.pad();
    let section = virtual_section(0x200, (1, 0));
    l.unclaimed(&noise(16, 7));
    l.unclaimed(&section);
    // The same section behind its zero slot is.
    l.unclaimed(&[0; 16]);
    let signature = Rpf9Signature::of(&section, section.len() as u64);
    endless(&mut l, Rpf9Content::ResourceShaped, signature, &section);
    assert_eq!(scan_all(&l.bytes).0, l.finds);
}

#[test]
fn the_count_places_no_names_after_a_stored_body_that_states_no_end() {
    let mut l = Layout::default();
    archive(&mut l, &head_of(Platform::Ps5, 2, 1));
    l.claim(
        stored(l.at(), Rpf9Content::Pso, None, Some(0x400)),
        &pso(0x400),
    );
    // A nested archive of one entry that lists as two, its names, and more than a page of zero
    // bytes before the next header, so that the scan places no names.
    archive(&mut l, &head_of(Platform::Ps5, 1, 2));
    let section = virtual_section(0x200, (1, 0));
    let signature = Rpf9Signature::of(&section, section.len() as u64);
    for _ in 0..2 {
        l.unclaimed(&[0; 16]);
        endless(&mut l, Rpf9Content::ResourceShaped, signature, &section);
    }
    l.unclaimed(&noise(NAMES as usize, 109));
    l.unclaimed(&[0; 0x2000]);
    archive(&mut l, &head_of(Platform::Ps5, 1, 3));
    l.claim(
        stored(l.at(), Rpf9Content::Pso, None, Some(0x400)),
        &pso(0x400),
    );
    let finds = scan_all(&l.bytes).0;
    assert_eq!(finds, l.finds);
    let mut count = Rpf9NamesByCount::default();
    finds.iter().for_each(|find| count.push(find));
    assert!(count
        .place(Cursor::new(&l.bytes))
        .expect("counted")
        .is_empty());
}

#[cfg(feature = "awc")]
#[test]
fn an_audio_container_off_a_page_is_listed_only_where_bodies_are_stored_as_they_are() {
    let (container, _) = adat(0xFF02, &[vec![(0x55, noise(64, 61))]]);
    let listed = |platform| {
        let mut l = Layout::default();
        archive(&mut l, &head_of(platform, 2, 1));
        let at = l.at();
        assert_ne!(at % 0x1000, 0);
        l.unclaimed(&container);
        l.pad();
        let awc = Rpf9BodyKind::Awc {
            flags: 0xFF02,
            streams: 1,
            chunks: 1,
        };
        (scan_all(&l.bytes).0.iter()).any(|f| (f.offset, f.kind) == (at, awc))
    };
    assert!(listed(Platform::Ps5));
    assert!(!listed(Platform::XboxSeries));
}
