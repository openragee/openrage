//! The occluders of the maps around the camera, streamed by a tier of their own: an occluder map
//! holds nothing that draws, so the maps the forest wants never name it.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::sync::Arc;

use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9ZlibRun};
use or_io::gta6::map::occluders::{self, MapOccluders};
use or_world::streaming::View;

use super::index::{Enabled, MapRow, WorldIndex};
use super::pool::Pool;

/// The content flag of a map that holds occluders.
const OCCLUDERS: u32 = 1 << 5;
/// The bytes an occluder map's body is read to.
const BODY_LIMIT: u64 = 256 << 20;
/// How many workers read occluder maps.
const READERS: usize = 2;

/// Whether a camera of `view` streams in the occluders of `row`, with what `enabled` switches on.
fn wanted(row: &MapRow, enabled: &Enabled, view: &View) -> bool {
    // ⚠ Holding nothing that draws, an occluder map is held back as empty (`held` 3).
    let on = enabled.allows(row) || (row.held == 3 && row.modes & u32::from(enabled.modes) != 0);
    row.content_flags & OCCLUDERS != 0 && on && row.boxes().wants(view.eye)
}

/// The occluders of the map `name`, its archive opened through `files`.
fn read(index: &WorldIndex, files: &mut HashMap<u32, File>, name: u32) -> Option<MapOccluders> {
    let row = index.maps.get(index.map_named(name)?)?;
    let file = match files.entry(row.archive) {
        Entry::Occupied(open) => open.into_mut(),
        Entry::Vacant(slot) => {
            let path = index.archives.get(row.archive as usize)?;
            slot.insert(File::open(path).ok()?)
        }
    };
    let (find, bytes) = read_rpf9_body(&*file, row.offset, BODY_LIMIT).ok()?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        ..
    }) = find.kind
    else {
        return None;
    };
    let (virtual_section, physical) = resource.sections(&bytes);
    match occluders::open(virtual_section, physical) {
        Ok(read) => Some(read),
        Err(error) => {
            tracing::debug!(%error, map = name, "an occluder map did not read");
            None
        }
    }
}

/// The occluder maps around the camera.
pub(super) struct Occluders {
    index: Arc<WorldIndex>,
    /// The maps of the index that hold occluders, by their place in it.
    maps: Vec<usize>,
    /// The maps the camera wants, by name, with how far it is from their entities' box.
    wanted: HashMap<u32, f32>,
    loaded: HashMap<u32, Arc<MapOccluders>>,
    /// A map that did not read is not asked for again.
    failed: HashSet<u32>,
    readers: Pool<u32, Option<MapOccluders>>,
    held: Arc<[Arc<MapOccluders>]>,
}

impl Occluders {
    pub(super) fn new(index: Arc<WorldIndex>) -> Self {
        let maps = (index.maps.iter().enumerate())
            .filter(|(_, row)| row.content_flags & OCCLUDERS != 0)
            .map(|(at, _)| at)
            .collect();
        let shared = Arc::clone(&index);
        let readers = Pool::new(
            READERS,
            HashMap::<u32, File>::new,
            move |files, name: u32| read(&shared, files, name),
        );
        Self {
            index,
            maps,
            wanted: HashMap::new(),
            loaded: HashMap::new(),
            failed: HashSet::new(),
            readers,
            held: Arc::default(),
        }
    }

    /// Take the occluders that arrived and, where `moved`, work out again which maps the camera of
    /// `view` wants with what `enabled` switches on; whether what is held changed.
    pub(super) fn stream(&mut self, view: &View, enabled: &Enabled, moved: bool) -> bool {
        let mut changed = false;
        for (name, read) in self.readers.take(|| true) {
            match read {
                Some(read)
                    if self.wanted.contains_key(&name) && !self.loaded.contains_key(&name) =>
                {
                    self.loaded.insert(name, Arc::new(read));
                    changed = true;
                }
                Some(_) => {}
                None => {
                    self.failed.insert(name);
                }
            }
        }
        if moved {
            self.wanted.clear();
            for row in (self.maps.iter()).filter_map(|at| self.index.maps.get(*at)) {
                if wanted(row, enabled, view) {
                    let (min, max) = row.boxes().entities;
                    let away = view.eye.distance(view.eye.clamp(min, max));
                    self.wanted.insert(row.name, away);
                }
            }
            let before = self.loaded.len();
            let kept = &self.wanted;
            self.loaded.retain(|name, _| kept.contains_key(name));
            changed |= self.loaded.len() != before;
        }
        // Asked on every pass, so a map that arrived leaves the pool's queue.
        let asked = (self.wanted.iter())
            .filter(|(name, _)| !self.loaded.contains_key(name) && !self.failed.contains(name))
            .map(|(name, away)| (*away, *name))
            .collect();
        self.readers.want(asked);
        if changed {
            self.held = self.loaded.values().cloned().collect();
        }
        changed
    }

    /// The occluders of every map loaded; the same list until one comes or goes.
    pub(super) fn held(&self) -> Arc<[Arc<MapOccluders>]> {
        Arc::clone(&self.held)
    }
}

#[cfg(test)]
mod tests {
    use bytemuck::Zeroable;
    use glam::Vec3;

    use super::*;

    /// A map of `content_flags` whose boxes both run from the origin to 100 m in every axis.
    fn row(content_flags: u32, held: u32) -> MapRow {
        let mut boxes = [0.0; 12];
        for at in [3, 4, 5, 9, 10, 11] {
            boxes[at] = 100.0;
        }
        MapRow {
            content_flags,
            held,
            modes: 1,
            group: 7,
            boxes,
            ..MapRow::zeroed()
        }
    }

    #[test]
    fn an_occluder_map_is_wanted_near_its_box_though_it_holds_nothing_that_draws() {
        let (enabled, near) = (Enabled::default(), View::at(Vec3::splat(50.0), 1.0));
        // Empty, and at rest.
        assert!(wanted(&row(OCCLUDERS | 16, 3), &enabled, &near));
        assert!(wanted(&row(OCCLUDERS, 0), &enabled, &near));
        // No occluders, or too far.
        assert!(!wanted(&row(16, 3), &enabled, &near));
        let far = View::at(Vec3::splat(-200.0), 1.0);
        assert!(!wanted(&row(OCCLUDERS, 3), &enabled, &far));
        // Within the sphere every map streams in by: its half side and 50 m from its middle.
        let at = |x: f32| View::at(Vec3::new(x, 50.0, 50.0), 1.0);
        assert!(wanted(&row(OCCLUDERS, 3), &enabled, &at(-40.0)));
        assert!(!wanted(&row(OCCLUDERS, 3), &enabled, &at(-60.0)));
        // Past a corner of the box grown by 50 m, but outside the sphere.
        assert!(!wanted(
            &row(OCCLUDERS, 3),
            &enabled,
            &View::at(Vec3::splat(-40.0), 1.0)
        ));
    }

    #[test]
    fn a_script_managed_or_unregistered_occluder_map_waits_for_what_switches_it_on() {
        let near = View::at(Vec3::splat(50.0), 1.0);
        let mut enabled = Enabled::default();
        assert!(!wanted(&row(OCCLUDERS, 2), &enabled, &near));
        assert!(!wanted(&row(OCCLUDERS, 1), &enabled, &near));
        enabled.groups.push(7);
        assert!(wanted(&row(OCCLUDERS, 2), &enabled, &near));
        // A mode that is off.
        let story = Enabled {
            modes: 2,
            groups: Vec::new(),
        };
        assert!(!wanted(&row(OCCLUDERS, 3), &story, &near));
    }
}
