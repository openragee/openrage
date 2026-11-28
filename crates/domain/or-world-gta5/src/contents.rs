//! What a body holds that its entry-table row does not: the drawables a
//! `.ydd` holds, whether a `.ydr` carries its own bound, and the boxes a map
//! and a static bound span. Only a root and its pointers are read, never a
//! vertex.

use std::collections::{BTreeSet, HashMap};

use crate::MapData;
use glam::Vec3;
use or_io::decl::{Gen8, Gen9, Resource as _};
use or_io::gta5::{GtaDrawable, PgDictionary, PhBoundAny};
use or_title::MountedGame;

use crate::archetype_assets::ArchetypeAssetIndex;
use crate::catalogue::{Catalogue, Kind};

use or_title::vfs::Vfs;

/// The bodies read, by the stem hash a reference names them with.
///
/// ⚠ A name absent here was not read, which is not a name read and found
/// empty: the resolver keeps its entry-table answer for the one and reports
/// the other as missing.
#[derive(Debug, Default, Clone)]
pub struct ModelContents {
    dictionaries: HashMap<u32, BTreeSet<u32>>,
    bounded: HashMap<u32, bool>,
    /// A map's `streamingExtents`, by its stem hash.
    map_boxes: HashMap<u32, (Vec3, Vec3)>,
    /// A `.ybn`'s header box, by its stem hash.
    bound_boxes: HashMap<u32, (Vec3, Vec3)>,
    unreadable: u64,
}

impl ModelContents {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every declared `.ydd`, and the loose `.ydr` of every archetype stating
    /// a `physicsDictionary`, as the mount holds them.
    pub fn read(
        mount: &MountedGame,
        catalogue: &Catalogue,
        archetypes: &ArchetypeAssetIndex,
    ) -> Self {
        let mut out = Self::new();
        for name in archetypes.names() {
            let Some(assets) = archetypes.get(name) else {
                continue;
            };
            let dictionary = assets.drawable_dictionary;
            if dictionary != 0 && !out.dictionaries.contains_key(&dictionary) {
                if let Some(bytes) = catalogue
                    .find(dictionary, Kind::DrawableDict)
                    .and_then(|l| mount.vfs.open(&l.logical))
                {
                    out.add_dictionary(dictionary, &bytes);
                }
            }
            let asset = assets.asset_name;
            if assets.physics_dictionary != 0 && !out.bounded.contains_key(&asset) {
                if let Some(bytes) = catalogue
                    .find(asset, Kind::Drawable)
                    .and_then(|l| mount.vfs.open(&l.logical))
                {
                    out.add_drawable(asset, &bytes);
                }
            }
        }
        out
    }

    /// Record the entry names of the `.ydd` whose stem hashes to `name`.
    pub fn add_dictionary(&mut self, name: u32, file: &[u8]) -> bool {
        let codes = codes_g9(file).or_else(|| codes_g8(file));
        match codes {
            Some(codes) => {
                self.dictionaries.insert(name, codes);
                true
            }
            None => {
                self.unreadable += 1;
                false
            }
        }
    }

    /// Record whether the `.ydr` whose stem hashes to `name` carries a bound.
    pub fn add_drawable(&mut self, name: u32, file: &[u8]) -> bool {
        match bound_g9(file).or_else(|| bound_g8(file)) {
            Some(bounded) => {
                self.bounded.insert(name, bounded);
                true
            }
            None => {
                self.unreadable += 1;
                false
            }
        }
    }

    /// Record the `streamingExtents` of the `.ymap` whose stem hashes to `name`.
    pub fn add_map(&mut self, name: u32, map: MapData<'_>) -> bool {
        match map.stored_streaming_extents() {
            Some(extents) => {
                self.map_boxes.insert(name, extents);
                true
            }
            None => false,
        }
    }

    /// Record the header box of the `.ybn` the catalogue holds under `name`, read from the
    /// front of its file and not the whole of it.
    pub fn read_bound(&mut self, mount: &MountedGame, catalogue: &Catalogue, name: u32) -> bool {
        let Some(path) = catalogue.find(name, Kind::Bounds).map(|l| &l.logical) else {
            return false;
        };
        self.read_bound_from(
            name,
            || mount.vfs.open_head(path, Vfs::BOUND_FRONT),
            || mount.vfs.open(path),
        )
    }

    /// Record the header box of the `.ybn` `name` from what `front` reads of it, and where that
    /// gives no root (a stored payload, or one behind opaque bytes, that inflates only at its exact
    /// length) from what `whole` reads: a bound is not left uncrossed-checked for being cut.
    pub fn read_bound_from(
        &mut self,
        name: u32,
        front: impl FnOnce() -> Option<Vec<u8>>,
        whole: impl FnOnce() -> Option<Vec<u8>>,
    ) -> bool {
        let front = front();
        if let Some(span) = front.as_deref().and_then(bound_box) {
            self.bound_boxes.insert(name, span);
            return true;
        }
        let whole = whole();
        match whole.as_deref().and_then(bound_box) {
            Some(span) => {
                self.bound_boxes.insert(name, span);
                true
            }
            None => {
                // Bytes that were read and hold no root are unreadable; a file nothing opened is not
                self.unreadable += u64::from(front.is_some() || whole.is_some());
                false
            }
        }
    }

    /// Record the header box of the `.ybn` whose stem hashes to `name`; `file` may be only its
    /// front, as `Rsc7::open` keeps what a cut stream inflates to.
    pub fn add_bound(&mut self, name: u32, file: &[u8]) -> bool {
        let Some(span) = bound_box(file) else {
            self.unreadable += 1;
            return false;
        };
        self.bound_boxes.insert(name, span);
        true
    }

    /// The header box read of the `.ybn` whose stem hashes to `name`, as `(min, max)`.
    pub fn bound_box_of(&self, name: u32) -> Option<(Vec3, Vec3)> {
        self.bound_boxes.get(&name).copied()
    }

    /// Whether a bound's box lies inside a map's streaming box; `None` when either was not read.
    pub fn bound_inside_map(&self, bound: u32, map: u32) -> Option<bool> {
        let (min, max) = self.bound_boxes.get(&bound)?;
        let (map_min, map_max) = self.map_boxes.get(&map)?;
        Some(min.cmpge(*map_min).all() && max.cmple(*map_max).all())
    }

    /// Whether a read `.ydd` holds `entry`; `None` when it was not read.
    pub fn dictionary_holds(&self, dictionary: u32, entry: u32) -> Option<bool> {
        self.dictionaries
            .get(&dictionary)
            .map(|c| c.contains(&entry))
    }

    /// Whether a read `.ydr` carries its own bound; `None` when it was not read.
    pub fn drawable_has_bound(&self, name: u32) -> Option<bool> {
        self.bounded.get(&name).copied()
    }

    pub fn dictionaries(&self) -> usize {
        self.dictionaries.len()
    }

    pub fn drawables(&self) -> usize {
        self.bounded.len()
    }

    /// Bodies that opened as neither target's layout.
    pub fn unreadable(&self) -> u64 {
        self.unreadable
    }
}

/// One reader per target: a file at the other target's version is refused by its header.
macro_rules! readers {
    ($codes:ident, $bound:ident, $t:ty) => {
        fn $codes(file: &[u8]) -> Option<BTreeSet<u32>> {
            let peek = PgDictionary::<GtaDrawable<$t>>::peek(file).ok()?;
            let codes = peek.root().ok()?.codes().ok()?;
            Some(codes.iter().copied().collect())
        }

        fn $bound(file: &[u8]) -> Option<bool> {
            let peek = GtaDrawable::<$t>::peek(file).ok()?;
            Some(peek.root().ok()?.bound().ok()?.is_some())
        }
    };
}

readers!(codes_g8, bound_g8, Gen8);
readers!(codes_g9, bound_g9, Gen9);

/// The header box of the `.ybn` `file` holds, or of its front: `None` where no root can be read.
fn bound_box(file: &[u8]) -> Option<(Vec3, Vec3)> {
    let peek = PhBoundAny::peek(file).ok()?;
    let root = peek.root().ok()?;
    let (max, min) = ({ root.bounding_box_max_xyz_margin_w }, {
        root.bounding_box_min_xyz_ref_count_w
    });
    Some((
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, max.y, max.z),
    ))
}
