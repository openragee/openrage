//! The maps a GTA VI drawable's own passes bind, decoded off the frame on a bounded pool: a map a
//! pack holds is read from the pack there as well, so that several packs are read side by side.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use or_renderer::game_shaders::mesh::Image;

use crate::state::{AppState, DocPayload};

/// Maps decoded so far this session: a scene bound before the count moved may lack one.
static ARRIVED: AtomicU64 = AtomicU64::new(0);

/// The stack a map is decoded on.
const STACK: usize = 8 << 20;

/// How many maps workers have decoded so far this session.
pub(super) fn arrived() -> u64 {
    ARRIVED.load(Ordering::Acquire)
}

/// Where the first level of a map is read from.
pub(super) enum Stored {
    /// Copied out of a texture the app has read already.
    Level(super::gta6_shaded::StoredLevel),
    /// A texture of an open pack, by its index there, read by the worker that decodes it.
    Pack(Arc<crate::state::ttpf_document::TtpfDocument>, usize),
}

impl Stored {
    /// The first level decoded, in the colour space it states; `None` where it will not read or decode.
    pub(super) fn decode(&self) -> Option<Image> {
        match self {
            Self::Level(level) => level.decode(),
            Self::Pack(pack, index) => {
                let texture = pack.texture(*index).ok()?;
                or_renderer::game_shaders::drawable::first_level(&*texture)
            }
        }
    }
}

/// Where the first level of the texture of `key` is read from, found as the app finds it for a
/// mesh: in an open pack, its texture is read later on a worker, never on the frame's thread.
// ⚠ The pack is the first place the ranking finds the name in; where its read is refused the map
// is not looked for further, which a read on this thread would do.
pub(super) fn stored(state: &AppState, selection: Option<u64>, key: u64) -> Option<Stored> {
    use crate::state::texture_resolution::probe;
    let name = crate::state::gta6_resource::texture_name(key);
    let found = probe(state, None, selection, &name).found;
    let payload = found
        .and_then(|found| state.document(found.document))
        .map(|d| &d.payload);
    if let Some(DocPayload::Ttpf(pack)) = payload {
        if let Some(index) = pack.index_of(&name) {
            return Some(Stored::Pack(Arc::clone(pack), index));
        }
    }
    super::gta6_shaded::stored_level(state, selection, key).map(Stored::Level)
}

/// Run `task` once; whether it was there to run.
fn run_once<F: FnOnce()>(task: &Mutex<Option<F>>) -> bool {
    let task = task.lock().unwrap_or_else(|p| p.into_inner()).take();
    let ran = task.is_some();
    if let Some(task) = task {
        task();
    }
    ran
}

/// Have a worker run `decode`, which fills a map's slot: no more at once than [`or_job::workers`],
/// and the count of [`arrived`] moved once it has.
pub(super) fn decode(decode: impl FnOnce() + Send + 'static) {
    static POOL: OnceLock<or_job::Pool> = OnceLock::new();
    let pool = POOL.get_or_init(|| or_job::Pool::new(or_job::workers(), STACK, || {}));
    let task = Arc::new(Mutex::new(Some(decode)));
    let theirs = Arc::clone(&task);
    let ran = pool.run(move || {
        if run_once(&theirs) {
            ARRIVED.fetch_add(1, Ordering::AcqRel);
        }
    });
    // With no worker to take it the map is decoded here, as late as it is.
    if let Err(error) = ran {
        tracing::warn!(%error, "no worker to decode a GTA VI map on, so it is decoded on this thread");
        if run_once(&task) {
            ARRIVED.fetch_add(1, Ordering::AcqRel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_is_decoded_on_no_more_workers_than_the_pool_has_and_each_is_counted_once_in() {
        use std::sync::atomic::AtomicUsize;
        let (now, most) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let done = Arc::new(AtomicUsize::new(0));
        let before = arrived();
        let asked = 4 * or_job::workers() + 3;
        for _ in 0..asked {
            let (now, most, done) = (Arc::clone(&now), Arc::clone(&most), Arc::clone(&done));
            decode(move || {
                let running = now.fetch_add(1, Ordering::SeqCst) + 1;
                most.fetch_max(running, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(2));
                now.fetch_sub(1, Ordering::SeqCst);
                done.fetch_add(1, Ordering::SeqCst);
            });
        }
        // The count moves once a task has returned.
        for _ in 0..5000 {
            if done.load(Ordering::SeqCst) == asked && arrived() - before >= asked as u64 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(done.load(Ordering::SeqCst), asked, "every map was decoded");
        let most = most.load(Ordering::SeqCst);
        assert!(most <= or_job::workers(), "{most} at once");
        // Another test's maps may come in meanwhile: the count moves at least once a map.
        assert!(arrived() - before >= asked as u64);
        // A task is run once, whoever asks for it after.
        let task = Mutex::new(Some(|| ()));
        assert!(run_once(&task) && !run_once(&task));
    }
}
