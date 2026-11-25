//! A real BK2 movie is read from its RPF9 table and decoded entirely in memory.

use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use or_io::container::rpf::{Gta6Mount, GtaViKeys};
use or_io::gta6::hash::name_hash64;
use or_media::{Codec, Decoders};
use or_test_support::skip_install;

const MOVIE: &str = "movies/arcadegames/bfyb_idle_screen_bink.bk2";

#[test]
fn the_arcade_bk2_decodes_and_seeks_directly_from_its_rpf9_table() {
    let Some(build) = std::env::var_os("OPENRAGE_GTA6_BUILD").map(PathBuf::from) else {
        skip_install(
            "a BK2 movie from its RPF9 table",
            format_args!("set OPENRAGE_GTA6_BUILD and OPENRAGE_GTA6_KEYS"),
        );
        return;
    };
    let keys = GtaViKeys::for_install(None)
        .expect("the keys load")
        .expect("the install needs keys");
    let mut file = File::open(build.join("movies.rpf")).expect("movies.rpf exists");
    let mount = Gta6Mount::open_root(&mut file, &keys).expect("the RPF9 table opens");
    let hash = name_hash64(MOVIE.as_bytes());
    let entry = mount
        .archive
        .entries
        .iter()
        .find(|entry| entry.name64 == hash)
        .expect("the table holds the requested movie's path hash");
    let bytes = mount
        .read_entry_decrypted(&mut file, entry, &keys)
        .expect("the table entry reads");
    assert!(bytes.starts_with(b"KB2"));
    let size = bytes.len();
    let source = or_media::open(Arc::<[u8]>::from(bytes)).expect("the movie opens");
    let info = *source.info();
    let video = info.video.expect("a picture track");
    assert_eq!(video.codec, Codec::Bink2);
    assert!(video.frames > 0);
    let decoders = Decoders::default();
    let mut stream = source.video(&decoders).expect("the video decoder opens");
    let targets = [0, video.frames / 2, video.frames - 1];
    let mut references = Vec::new();
    let mut count = 0;
    while let Some(frame) = stream.next_frame().expect("a frame decodes") {
        assert_eq!((frame.width, frame.height), (video.width, video.height));
        assert_eq!(
            frame.rgba.len(),
            video.width as usize * video.height as usize * 4
        );
        if targets.contains(&count) {
            references.push((count, frame));
        }
        count += 1;
    }
    assert_eq!(count, video.frames);
    for (at, expected) in references.iter().rev() {
        stream.seek(*at).expect("seek from a key frame");
        assert_eq!(
            stream
                .next_frame()
                .expect("the seeked frame decodes")
                .as_ref(),
            Some(expected)
        );
    }
    if let Some(audio) = info.audio {
        let mut stream = source.audio(&decoders).expect("the audio decoder opens");
        let mut samples = 0;
        while let Some(block) = stream.read().expect("the audio decodes") {
            samples += block.len();
        }
        assert_eq!(samples as u64 / u64::from(audio.channels), audio.frames);
    }
    eprintln!("BK2 entry {}: {size} bytes, {}x{}, {} frames at {:?}, audio {:?}; all frames and reverse seeks passed", entry.index, video.width, video.height, count, video.frame_rate, info.audio);
}
