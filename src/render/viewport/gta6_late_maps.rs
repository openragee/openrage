//! A GTA VI drawable's maps that come in after its scene was prepared, decoded late or learnt by
//! name as their packs open: bound into the scene where they are read, with no scene prepared
//! again and no pass built for them.

use super::{MeshCacheKey, RenderBridge};
use crate::state::AppState;

/// What came of the active GTA VI drawable's maps since Shaded was asked for it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LateMaps {
    /// The maps decoded by the time its surfaces were last read.
    pub(super) seen: u64,
    /// Scenes of it prepared from nothing.
    pub prepares: u32,
    /// Times maps that came in after its scene was prepared were bound into it.
    pub late: u32,
    /// Whether every map it binds is in, which is said once.
    pub(super) all_in: bool,
}

/// Whether maps came in since a scene prepared by `was` was bound, for the frame of `key`: a map
/// learnt as its pack opened, or one decoded while `decoding`; never where anything else it was
/// prepared from moved, which prepares it from nothing.
fn came_in(was: MeshCacheKey, key: MeshCacheKey, decoding: bool, seen: u64, now: u64) -> bool {
    let learnt = was != key;
    let decoded = decoding && now != seen;
    (learnt || decoded) && was.without_maps() == key.without_maps()
}

impl RenderBridge {
    /// Bind into the active GTA VI drawable's scene the maps learnt or decoded since its surfaces
    /// were read, where nothing else it was prepared from moved; whether it did, which makes `key`
    /// the scene's.
    pub(super) fn take_on_gta6_maps(&mut self, state: &AppState, key: MeshCacheKey) -> bool {
        let Some(was) = self.game_scene_key else {
            return false;
        };
        let now = crate::render::gta6_maps::arrived();
        let drawn = matches!(self.gta6_own, Some(Ok(_)));
        if !drawn || !came_in(was, key, self.gta6_decoding, self.gta6_maps.seen, now) {
            return false;
        }
        let Some(mut scene) = self.renderer.take_gta6_scene() else {
            return false;
        };
        // A map decoded from here on is taken on by a later frame.
        self.gta6_maps.seen = now;
        let read = crate::render::gta6_meshlets::meshlets(state);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let rebound = match &read {
            Some(Ok(meshlets)) => scene.rebind(&self.device, &self.queue, &meshlets.surfaces),
            _ => Ok(None),
        };
        let refused = pollster::block_on(validation.pop());
        self.renderer.set_gta6_scene(Some(scene));
        let (Some(Ok(meshlets)), Ok(Some(groups)), None) = (read, rebound, refused) else {
            // Not the surfaces the scene draws, or not bound: this frame prepares it from nothing.
            self.game_scene_key = None;
            return false;
        };
        self.game_scene_key = Some(key);
        self.gta6_decoding = meshlets.decoding;
        let decoding = meshlets.decoding;
        // A pass a worker still builds is bound with the maps in now.
        if let Some(building) = self.gta6_building.as_mut() {
            *building = meshlets;
        }
        if groups > 0 {
            self.gta6_maps.late += 1;
            let elapsed = self.gta6_since.map(|(_, since, ..)| since.elapsed());
            tracing::debug!(
                groups,
                ?elapsed,
                "maps that came in late were bound into a GTA VI drawable's scene"
            );
        }
        self.said_all_in(decoding);
        true
    }

    /// Say once, with the time since Shaded was asked, that the active GTA VI drawable's maps are all bound, unless one is `decoding`.
    pub(super) fn said_all_in(&mut self, decoding: bool) {
        if decoding || std::mem::replace(&mut self.gta6_maps.all_in, true) {
            return;
        }
        let elapsed = self.gta6_since.map(|(_, since, ..)| since.elapsed());
        tracing::info!(
            ?elapsed,
            late = self.gta6_maps.late,
            prepares = self.gta6_maps.prepares,
            "a GTA VI drawable's last map was bound"
        );
    }
}

#[cfg(test)]
#[path = "gta6_late_maps_tests.rs"]
mod device_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_come_in_only_where_nothing_but_the_maps_moved() {
        let state = AppState::default();
        let key = MeshCacheKey::of(&state, 7, 0);
        let learnt = MeshCacheKey {
            texture_names: 41,
            ..key
        };
        let opened = MeshCacheKey { packs: 3, ..key };
        let other = MeshCacheKey::of(&state, 8, 0);
        assert!(came_in(key, learnt, false, 0, 0), "a name learnt");
        assert!(came_in(key, opened, false, 0, 0), "a pack opened");
        assert!(!came_in(key, key, false, 0, 5), "nothing waited for");
        assert!(came_in(key, key, true, 4, 5), "a map decoded since");
        assert!(!came_in(key, key, true, 5, 5), "none decoded since");
        assert!(
            !came_in(key, MeshCacheKey { packs: 3, ..other }, true, 0, 5),
            "another drawable"
        );
    }
}
