//! The index of `or_world_gta6` as the app opens it: the configuration folder and the key snapshot
//! are the app's, and so is the method of the viewer that hands the Map screen its land and water.

use std::path::Path;

use or_io::container::rpf::GtaViKeys;
use or_job::{Cancel, Progress};

pub use or_world_gta6::index::*;

/// The key snapshot of this run, where one is found and reads.
pub(crate) fn keys() -> Option<GtaViKeys> {
    or_world_gta6::index::keys(crate::state::prefs::config_root().as_deref())
}

/// [`or_world_gta6::index::open`] with the configuration folder and the key snapshot of this run.
pub fn open(
    root: &Path,
    afresh: bool,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<Option<Opened>, String> {
    let config = crate::state::prefs::config_root();
    let keys = keys();
    or_world_gta6::index::open(
        root,
        config.as_deref(),
        keys.as_ref(),
        afresh,
        cancel,
        progress,
    )
}

/// [`or_world_gta6::index::build`] with the key snapshot of this run.
#[cfg(test)]
pub fn build(
    root: &Path,
    fresh: Option<&Path>,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<Option<WorldIndex>, String> {
    or_world_gta6::index::build(root, fresh, keys().as_ref(), cancel, progress)
}

impl super::Viewer {
    /// The height map and the water of the install shown, as the overhead map shades them: each `None` until its worker has read it.
    pub fn land_and_water(
        &self,
    ) -> (
        Option<std::sync::Arc<or_io::gta6::water::height::WaterGround>>,
        Option<std::sync::Arc<or_io::gta6::water::WaterData>>,
    ) {
        let Some(root) = self.root.as_deref() else {
            return (None, None);
        };
        (super::water::ground(root), super::water::water(root))
    }
}
