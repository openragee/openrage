//! What reads a GTA VI effect's programs as shading passes: the translator, or a test's stand-in.

use std::collections::BTreeSet;

use or_cli::Gta6ShadingPass;
use or_io::gta5::shaders::sgd5::Sgd5Table;

/// What reads an effect's programs as shading passes: the translator, or a test's stand-in.
pub(super) trait Translate: Sync {
    /// A pixel stage, which several passes of an effect share.
    type Pixel;
    /// Whether the two inflated containers are the pass that shades a model.
    fn shades(&self, mesh: &[u8], pixel: &[u8]) -> bool;
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

impl Translate for Translator {
    type Pixel = or_cli::Gta6PixelStage;

    fn shades(&self, mesh: &[u8], pixel: &[u8]) -> bool {
        or_cli::is_gta6_shading_pass(mesh, pixel)
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
