//! The maps the Material view binds from an open GTA VI texture pack, read from the pack and
//! decoded on a bounded pool: a frame never reads one, and a mesh is drawn without its map until
//! the map is in.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use crate::render::gpu_budget::TextureStamp;
use crate::state::ttpf_document::TtpfDocument;
use crate::state::{AppState, DocPayload, MeshSource};

/// The stack a map is decoded on.
const STACK: usize = 8 << 20;

/// The bytes of maps one build of the meshes uploads, past which the rest wait for the next.
pub(super) const UPLOAD_BYTES: usize = 64 << 20;

/// The bytes of decoded maps kept, past which they are all let go.
const KEPT_BYTES: usize = 1 << 30;

/// A map's first level as the Material view uploads it.
pub(super) struct Prepared {
    pub(super) size: (u32, u32),
    pub(super) rgba: Vec<u8>,
    pub(super) srgb: bool,
}

/// A map as far as it is decoded: nothing while a worker decodes it, `None` once it would not.
type Slot = Arc<Mutex<Option<Option<Arc<Prepared>>>>>;

/// Texture `index` of an open pack, as the pack shows it now.
pub(super) struct PackMap {
    pack: Arc<TtpfDocument>,
    index: usize,
    /// What it is read from: the pack, its edits, the texture and the slice shown.
    stamp: u64,
}

impl PackMap {
    /// The stamp its upload is kept under, which moves when the pack's texture does.
    pub(super) fn stamp(&self) -> TextureStamp {
        TextureStamp::of_read(self.stamp)
    }

    /// What its read `bump` is kept as, which [`all_in`] is asked of.
    pub(super) fn read(&self, bump: bool) -> (u64, bool) {
        (self.stamp, bump)
    }
}

/// Where the texture `name` binds for `selection` is a texture of an open GTA VI pack: found as
/// the app finds it for a mesh, without reading it.
pub(super) fn in_pack(
    state: &AppState,
    from: Option<MeshSource>,
    selection: Option<u64>,
    name: &str,
) -> Option<PackMap> {
    let found = crate::state::texture_resolution::probe(state, from, selection, name).found?;
    let doc = state.document(found.document)?;
    let DocPayload::Ttpf(pack) = &doc.payload else {
        return None;
    };
    let index = pack.index_of(name)?;
    let mut hasher = std::hash::DefaultHasher::new();
    (Arc::as_ptr(pack) as usize, doc.payload_epoch(), index).hash(&mut hasher);
    // ⚠ Asked only of a texture of slices: the slice shown is behind the lock a worker holds while it
    // reads the pack, which would hold this frame until that read ends.
    if pack.slices(index) > 1 {
        pack.shown_slice(index).hash(&mut hasher);
    }
    Some(PackMap {
        pack: Arc::clone(pack),
        index,
        stamp: hasher.finish(),
    })
}

/// The map of `map`, read `bump` (linear) or as colour: `Err` while a worker reads it, which is
/// started the first time it is asked for; `Ok(None)` where it will not read or decode.
pub(super) fn prepared(map: PackMap, bump: bool) -> Result<Option<Arc<Prepared>>, ()> {
    prepared_with(
        (map.stamp, bump),
        move || {
            let texture = map.pack.texture(map.index).ok()?;
            let texture =
                crate::state::textures::Texture::Owned(std::borrow::Cow::Borrowed(&*texture));
            super::texture_upload::prepare(&texture, bump)
        },
        run,
    )
}

/// The maps asked for this session, by what each is read from and whether it is read as a normal map.
static KEPT: Mutex<Option<HashMap<(u64, bool), Slot>>> = Mutex::new(None);

/// [`prepared`] of the map kept as `key`, made by `make` on a worker `spawn` runs it on.
fn prepared_with(
    key: (u64, bool),
    make: impl FnOnce() -> Option<Prepared> + Send + 'static,
    spawn: impl FnOnce(Box<dyn FnOnce() + Send>),
) -> Result<Option<Arc<Prepared>>, ()> {
    let mut kept = KEPT.lock().unwrap_or_else(|p| p.into_inner());
    let kept = kept.get_or_insert_with(HashMap::new);
    if let Some(slot) = kept.get(&key) {
        return slot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or(());
    }
    let bytes = |slot: &Slot| match &*slot.lock().unwrap_or_else(|p| p.into_inner()) {
        Some(Some(map)) => map.rgba.len(),
        _ => 0,
    };
    // ponytail: past the cap every map is let go at once; the least lately used when a session
    // that opens many models shows the read again.
    if kept.values().map(bytes).sum::<usize>() > KEPT_BYTES {
        kept.clear();
    }
    let slot = Slot::default();
    kept.insert(key, Arc::clone(&slot));
    spawn(Box::new(move || {
        let map = make().map(Arc::new);
        *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(map);
    }));
    Err(())
}

/// Whether no worker still reads the map kept as any of `reads`; none is started.
pub(super) fn all_in(reads: &[(u64, bool)]) -> bool {
    let kept = KEPT.lock().unwrap_or_else(|p| p.into_inner());
    // A map let go past the cap is in: a build asks for it again.
    let read = |read| kept.as_ref()?.get(read).cloned();
    (reads.iter()).all(|r| {
        read(r).is_none_or(|slot| slot.lock().unwrap_or_else(|p| p.into_inner()).is_some())
    })
}

/// Have a worker of the bounded pool run `task`; with none to take it, it runs here.
fn run(task: Box<dyn FnOnce() + Send>) {
    static POOL: OnceLock<or_job::Pool> = OnceLock::new();
    let pool = POOL.get_or_init(|| or_job::Pool::new(or_job::workers(), STACK, || {}));
    let task = Arc::new(Mutex::new(Some(task)));
    let theirs = Arc::clone(&task);
    let ran = pool.run(move || {
        if let Some(task) = theirs.lock().unwrap_or_else(|p| p.into_inner()).take() {
            task();
        }
    });
    if let Err(error) = ran {
        tracing::warn!(%error, "no worker to read a texture pack's map on, so it is read on this thread");
        if let Some(task) = task.lock().unwrap_or_else(|p| p.into_inner()).take() {
            task();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> Option<Prepared> {
        Some(Prepared {
            size: (1, 1),
            rgba: vec![1, 2, 3, 4],
            srgb: true,
        })
    }

    #[test]
    fn a_map_is_made_once_off_the_asking_thread_and_waited_for_until_it_is_in() {
        let key = (0x5EED_0001, false);
        let mut held = None;
        let asked = prepared_with(key, map, |task| held = Some(task));
        assert!(asked.is_err(), "nothing is made on the asking thread");
        let spawned =
            |_: Box<dyn FnOnce() + Send>| panic!("a map asked for again is not made again");
        assert!(
            prepared_with(key, map, spawned).is_err(),
            "still being made"
        );
        held.expect("the map was handed to a worker")();
        let made = prepared_with(key, map, spawned).expect("the map is in");
        assert_eq!(made.expect("it decoded").rgba, [1, 2, 3, 4]);
        // Read as a normal map, the same texture is another upload.
        let mut other = false;
        let _ = prepared_with((key.0, true), map, |_| other = true);
        assert!(other);
        assert!(
            !all_in(&[key, (key.0, true)]),
            "the normal map is still being made"
        );
        assert!(all_in(&[key]));
    }

    #[test]
    fn a_map_that_does_not_decode_is_in_as_none() {
        let key = (0x5EED_0002, false);
        let _ = prepared_with(key, || None, |task| task());
        assert!(matches!(prepared_with(key, || None, |_| ()), Ok(None)));
    }
}
