//! GTA VI's world session: a shell over the viewer of `gta6_worldview`, which streams and selects
//! with GTA VI's own streamer and is one value for the process.
//!
//! ⚠ The World screen and the bridge draw GTA VI through that viewer themselves: this shell
//! answers the seam's questions from it and hands the bridge no feed.

use or_renderer::FlyCamera;
use or_world::caps::{Capability, WorldCaps};
use or_world::slots::OptionKind;

use super::{Standing, Unread, Waiting, WorldFeed, WorldKey, WorldSession};
use crate::state::gta6_world::worldview::viewer;

/// The session of a GTA VI build's folder.
pub struct Gta6Session {
    key: WorldKey,
}

impl Gta6Session {
    pub fn new(key: WorldKey) -> Self {
        Self { key }
    }
}

/// What a GTA VI world states it has: a height of the ground, from the install's height map.
pub const CAPS: WorldCaps = WorldCaps::none().with(Capability::Ground);

impl WorldSession for Gta6Session {
    fn key(&self) -> &WorldKey {
        &self.key
    }

    fn poll(&mut self) {
        let mut viewer = viewer();
        viewer.open(&self.key.install);
        viewer.poll();
    }

    fn standing(&self) -> Standing {
        let viewer = viewer();
        match (viewer.building(), &viewer.failed) {
            (Some(done), _) => Standing::Opening(done),
            (None, Some(why)) => Standing::Failed(why.clone()),
            (None, None) if viewer.index.is_some() => Standing::Open,
            (None, None) => Standing::Opening(0.0),
        }
    }

    fn caps(&self) -> WorldCaps {
        CAPS
    }

    fn option_kinds(&self) -> Vec<OptionKind> {
        Vec::new()
    }

    fn camera(&self) -> FlyCamera {
        viewer().camera.clone()
    }

    fn set_camera(&mut self, camera: FlyCamera) {
        viewer().camera = camera;
    }

    fn hour(&self) -> f32 {
        viewer().hour
    }

    fn set_hour(&mut self, hour: f32) {
        viewer().hour = hour;
    }

    fn waiting(&self) -> Waiting {
        let viewer = viewer();
        Waiting {
            maps: viewer.streaming().maps_waiting,
            models: viewer.models_waiting,
            textures: viewer.textures_waiting,
        }
    }

    // ⚠ What GTA VI could not read is said by its own panel, from the viewer.
    fn unread(&self) -> Vec<Unread> {
        Vec::new()
    }

    fn feed(&mut self) -> Option<&mut dyn WorldFeed> {
        None
    }
}
