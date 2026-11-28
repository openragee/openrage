//! A map's first Uses reads each bound its stem names for its header alone.
//!
//! ⚠ The bounds are padded with bytes nobody reads, as a retail street's are: a
//! sphere is a few hundred bytes whole, and a read of its whole file costs
//! nothing a read of its header would save.

use std::path::Path;
use std::sync::{Arc, Mutex};

use or_cli::DependencyIndex;
use or_io::container::resource::{Compression, ResourceEnvelope as _, Rsc7};
use or_io::container::rpf::crypto::{NoOpCryptoProvider, RpfCryptoProvider};
use or_io::container::rpf::RpfArchive;
use or_title::{build_mounted_game, ArchiveDevice, DlcCutoff, FileDevice, GtaInstall};
use or_world_gta5::Via;

use super::nested_archives_survive_a_save::archive;
use super::uses_reaches_every_pack::{bound_spanning, plain_map};

/// Bytes a padded bound holds past its header, which no box needs.
const PADDING: usize = 100_000;

/// Bytes that deflate to about their own length, so a file holding them is as long stored as whole.
fn noise(len: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9u32;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

/// [`bound_spanning`], then [`PADDING`] bytes of what a bound holds past its header.
fn padded_bound(min: [f32; 3], max: [f32; 3]) -> Vec<u8> {
    let opened = Rsc7::open(&bound_spanning(min, max)).expect("a bound opens");
    let mut payload = opened.data;
    payload.extend(noise(PADDING));
    // ⚠ A seal that carries the version in its flag words: an archive keeps only those.
    let seal = Rsc7::invent_single_chunk_seal(opened.version, payload.len()).expect("a page");
    payload.resize(seal.total_size() as usize, 0);
    Rsc7::reseal(&payload, &seal, Compression::fast()).expect("a padded bound")
}

/// An install holding `plain_map.ymap`, streaming from -100 to 100, and two padded `.ybn` its
/// stem names, one inside that box and one outside it, and one that is no bound at all.
fn install_with_a_map_of_padded_bounds() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("an install folder");
    let x64e = archive(&[
        ("levels/gta5/plain_map.ymap", &plain_map()),
        (
            "levels/gta5/plain_map_0.ybn",
            &padded_bound([0.0; 3], [10.0; 3]),
        ),
        (
            "levels/gta5/ma@plain_map_1.ybn",
            &padded_bound([500.0; 3], [510.0; 3]),
        ),
        ("levels/gta5/plain_map_2.ybn", b"not a bound"),
    ]);
    std::fs::write(
        dir.path().join("common.rpf"),
        archive(&[("data/readme.txt", b"c")]),
    )
    .expect("common.rpf");
    std::fs::write(dir.path().join("x64e.rpf"), x64e).expect("x64e.rpf");
    dir
}

/// One read the mount made of a file.
#[derive(Debug, Clone)]
struct Read {
    path: String,
    /// Bytes the read returned.
    got: usize,
}

/// `inner`, answering as it does and noting what each read of it returned.
#[derive(Debug)]
struct Recorded {
    inner: ArchiveDevice,
    reads: Arc<Mutex<Vec<Read>>>,
}

impl Recorded {
    fn note(&self, path: &str, bytes: Option<Vec<u8>>) -> Option<Vec<u8>> {
        if let (Some(bytes), Ok(mut reads)) = (&bytes, self.reads.lock()) {
            reads.push(Read {
                path: path.to_string(),
                got: bytes.len(),
            });
        }
        bytes
    }
}

impl FileDevice for Recorded {
    fn open(&self, relative_path: &str) -> Option<Vec<u8>> {
        self.note(relative_path, self.inner.open(relative_path))
    }

    fn open_head(&self, relative_path: &str, limit: usize) -> Option<Vec<u8>> {
        self.note(relative_path, self.inner.open_head(relative_path, limit))
    }

    fn exists(&self, relative_path: &str) -> bool {
        self.inner.exists(relative_path)
    }

    fn locate(&self, relative_path: &str) -> Option<String> {
        self.inner.locate(relative_path)
    }

    fn list(&self, relative_dir: &str) -> Vec<String> {
        self.inner.list(relative_dir)
    }

    fn walk(&self) -> Option<Vec<String>> {
        self.inner.walk()
    }
}

/// The index of `root` mounted as a game, with `x64e.rpf`'s reads from then on noted.
fn index_noting_reads(root: &Path) -> (DependencyIndex, Arc<Mutex<Vec<Read>>>) {
    let crypto: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
    let mut game = build_mounted_game(
        &GtaInstall {
            root: root.to_path_buf(),
        },
        Arc::clone(&crypto),
        DlcCutoff::All,
    )
    .expect("the synthetic install mounts");
    let x64e = RpfArchive::open_with(root.join("x64e.rpf"), crypto).expect("x64e.rpf opens");
    let reads = Arc::new(Mutex::new(Vec::new()));
    game.vfs.mount(
        "platform:",
        Arc::new(Recorded {
            inner: ArchiveDevice::new(Arc::new(x64e)).with_label("x64e.rpf"),
            reads: Arc::clone(&reads),
        }),
    );
    (DependencyIndex::read(game, &mut |_| {}), reads)
}

#[test]
fn a_maps_first_uses_reads_each_of_its_bounds_for_its_header_alone() {
    let root = install_with_a_map_of_padded_bounds();
    let (index, reads) = index_noting_reads(root.path());
    reads.lock().expect("the log").clear();

    let map = "plain_map.ymap";
    let boxes = index.map_boxes_of(map, &|| false);
    let set = index
        .uses_checked(map, &boxes)
        .expect("the install holds it");

    let verdict = |bound: &str| {
        (set.edges.iter())
            .find(|edge| edge.name == or_io::string_hash(bound.as_bytes()))
            .map(|edge| edge.via)
            .unwrap_or_else(|| panic!("no edge for {bound}: {:#?}", set.edges))
    };
    for (bound, inside) in [("plain_map_0", true), ("ma@plain_map_1", false)] {
        assert_eq!(
            verdict(bound),
            Via::MapStemConvention {
                inside_streaming_box: Some(inside)
            },
            "{bound}"
        );
    }

    let reads = reads.lock().expect("the log").clone();
    for bound in ["plain_map_0.ybn", "ma@plain_map_1.ybn"] {
        let taken: Vec<&Read> = reads.iter().filter(|r| r.path.ends_with(bound)).collect();
        assert_eq!(taken.len(), 1, "{bound} was read once: {reads:?}");
        assert!(
            taken[0].got < PADDING / 10,
            "{bound} was read for {} bytes of a file of over {PADDING}: its header alone is a \
             few hundred",
            taken[0].got
        );
    }
}
