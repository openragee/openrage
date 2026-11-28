//! What reads a GTA VI effect's programs as shading passes: the translator, or a test's stand-in.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};

use or_cli::Gta6ShadingPass;
use or_io::gta5::shaders::sgd5::Sgd5Table;

/// What reads an effect's programs as shading passes: the translator, or a test's stand-in.
pub(super) trait Translate: Sync {
    /// A pixel stage, which several passes of an effect share.
    type Pixel;
    /// Whether the two inflated containers are the pass that shades a model.
    fn shades(&self, mesh: &[u8], pixel: &[u8]) -> bool;
    /// Whether they are a forward pass, laid over the lit frame: one a model is drawn by only
    /// where its effect has no pass that shades it.
    fn forward(&self, _mesh: &[u8], _pixel: &[u8]) -> bool {
        false
    }
    /// The streams the mesh program reads.
    fn streams(
        &self,
        mesh: &[u8],
        table: &Sgd5Table,
        effect: &Sgd5Table,
    ) -> Result<BTreeSet<u32>, String>;
    fn pixel(
        &self,
        pixel: &[u8],
        table: &Sgd5Table,
        effect: &Sgd5Table,
    ) -> Result<Self::Pixel, String>;
    fn mesh(
        &self,
        mesh: &[u8],
        table: &Sgd5Table,
        effect: &Sgd5Table,
        pixel: &Self::Pixel,
    ) -> Result<Gta6ShadingPass, String>;
}

pub(super) struct Translator;

/// A job's stop, and the workers it spares beyond the one each effect is read on.
pub(super) struct Workers<'c> {
    pub(super) cancel: &'c or_job::Cancel,
    pub(super) spare: AtomicUsize,
}

impl Workers<'_> {
    /// Up to `wanted` of the spare workers, until [`Workers::give`] gives them back.
    pub(super) fn take(&self, wanted: usize) -> usize {
        let spare = (self.spare).try_update(Ordering::SeqCst, Ordering::SeqCst, |spare| {
            Some(spare - spare.min(wanted))
        });
        spare.unwrap_or(0).min(wanted)
    }

    pub(super) fn give(&self, workers: usize) {
        self.spare.fetch_add(workers, Ordering::SeqCst);
    }
}

impl Translate for Translator {
    type Pixel = or_cli::Gta6PixelStage;

    fn shades(&self, mesh: &[u8], pixel: &[u8]) -> bool {
        or_cli::is_gta6_shading_pass(mesh, pixel)
    }

    fn forward(&self, mesh: &[u8], pixel: &[u8]) -> bool {
        or_cli::is_gta6_forward_pass(mesh, pixel)
    }

    fn streams(
        &self,
        mesh: &[u8],
        table: &Sgd5Table,
        effect: &Sgd5Table,
    ) -> Result<BTreeSet<u32>, String> {
        or_cli::gta6_mesh_streams(mesh, table, effect)
    }

    fn pixel(
        &self,
        pixel: &[u8],
        table: &Sgd5Table,
        effect: &Sgd5Table,
    ) -> Result<Self::Pixel, String> {
        or_cli::translate_gta6_pixel_stage(pixel, table, effect)
    }

    fn mesh(
        &self,
        mesh: &[u8],
        table: &Sgd5Table,
        effect: &Sgd5Table,
        pixel: &Self::Pixel,
    ) -> Result<Gta6ShadingPass, String> {
        or_cli::translate_gta6_mesh_stage(mesh, table, effect, pixel)
    }
}
