//! What the far stand-ins take of the install's visual settings.

use or_renderer::world::cards::CardLook;

use super::sky;
use super::Viewer;

impl Viewer {
    /// The stand-ins' diffuse scale and the distances it is taken over, by the names the executable hashes to find them in the visual settings; its own where they state none or none are read.
    pub fn card_look(&self) -> CardLook {
        let image = CardLook::default();
        let Some(tables) = self.root.as_deref().and_then(sky::tables) else {
            return image;
        };
        let get = |name: &str, default: f32| tables.visual_settings.get_f32(name, default);
        let [r, g, b] = image.diffuse_scale;
        CardLook {
            diffuse_scale: [
                get("imposter.trees.gbuffDiffuseScaleRed", r),
                get("imposter.trees.gbuffDiffuseScaleGreen", g),
                get("imposter.trees.gbuffDiffuseScaleBlue", b),
            ],
            scale_start: get("imposter.trees.gbuffScaleStart", image.scale_start),
            scale_range: get("imposter.trees.gbuffScaleRange", image.scale_range),
        }
    }
}
