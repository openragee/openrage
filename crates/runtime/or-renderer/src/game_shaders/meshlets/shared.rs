//! What a scene makes on the device, made once for every surface that binds it alike, and the
//! maps that come in after a scene was prepared, bound where they are read and nowhere else.

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use wgpu::util::DeviceExt;

use super::{bind, view_of, Feed, MeshletScene, MeshletSurface, ViewWrite};
use crate::game_shaders::mesh::Image;

/// What a scene has made on the device since it was prepared, counted where each is made.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Uploads {
    /// Buffers made, and the bytes they were made with.
    pub buffers: u32,
    pub buffer_bytes: u64,
    /// Textures made, and the bytes of the images they were made from.
    pub textures: u32,
    pub image_bytes: u64,
    pub samplers: u32,
    /// Bind groups made.
    pub groups: u32,
    /// Bind groups made again for maps that came in after the scene was prepared.
    pub rebound: u32,
}

/// A buffer found again by its usage, its length and two digests of its bytes.
// ⚠ The bytes are not kept: two digests of 64 bits stand for them, so that buffers the size of
// a model's vertex data are held once, on the device.
type BytesKey = (wgpu::BufferUsages, usize, u64, u64);

/// A texture found again by the image's place in memory, which the entry keeps the image at,
/// the view's dimension and sample type, and the texel it stands in with where it has no image.
type ImageKey = (
    usize,
    wgpu::TextureViewDimension,
    wgpu::TextureSampleType,
    [u8; 4],
);

/// The buffers, textures and samplers a scene has made, by what they were made from.
#[derive(Default)]
pub(super) struct Made {
    buffers: HashMap<BytesKey, wgpu::Buffer>,
    textures: HashMap<ImageKey, (Option<Arc<Image>>, wgpu::TextureView)>,
    samplers: HashMap<bool, wgpu::Sampler>,
    pub(super) uploads: Uploads,
}

fn digests(bytes: &[u8]) -> (u64, u64) {
    let mut first = std::hash::DefaultHasher::new();
    first.write(bytes);
    let mut second = std::hash::DefaultHasher::new();
    second.write_u8(1);
    second.write(bytes);
    (first.finish(), second.finish())
}

impl Made {
    /// A buffer holding `bytes` for `usage`: with `shared`, the one already made of the same
    /// where there is one. A buffer a frame writes, or a stage may write, is never shared.
    pub(super) fn buffer(
        &mut self,
        device: &wgpu::Device,
        usage: wgpu::BufferUsages,
        bytes: &[u8],
        shared: bool,
    ) -> wgpu::Buffer {
        let key = shared.then(|| {
            let (first, second) = digests(bytes);
            (usage, bytes.len(), first, second)
        });
        if let Some(buffer) = key.as_ref().and_then(|key| self.buffers.get(key)) {
            return buffer.clone();
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytes,
            usage,
        });
        self.uploads.buffers += 1;
        self.uploads.buffer_bytes += bytes.len() as u64;
        if let Some(key) = key {
            self.buffers.insert(key, buffer.clone());
        }
        buffer
    }

    /// The view [`view_of`] makes of `image`, or of the texel `blank` for none, made once a scene.
    pub(super) fn view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dimension: wgpu::TextureViewDimension,
        sample: wgpu::TextureSampleType,
        image: Option<&Arc<Image>>,
        blank: [u8; 4],
    ) -> wgpu::TextureView {
        let place = image.map_or(0, |image| Arc::as_ptr(image) as usize);
        let key = (place, dimension, sample, blank);
        if let Some((_, view)) = self.textures.get(&key) {
            return view.clone();
        }
        let view = view_of(device, queue, dimension, sample, image.map(|i| &**i), blank);
        self.uploads.textures += 1;
        self.uploads.image_bytes += image.map_or(0, |image| image.rgba.len() as u64);
        // The image is kept with its view: no other comes to lie where it does while the scene lives.
        self.textures.insert(key, (image.cloned(), view.clone()));
        view
    }

    /// The scene's sampler, of a comparison or not.
    pub(super) fn sampler(&mut self, device: &wgpu::Device, comparison: bool) -> wgpu::Sampler {
        let uploads = &mut self.uploads;
        let made = self.samplers.entry(comparison).or_insert_with(|| {
            uploads.samplers += 1;
            device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                address_mode_w: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                compare: comparison.then_some(wgpu::CompareFunction::LessEqual),
                ..Default::default()
            })
        });
        made.clone()
    }
}

/// A digest of what `feeds` state for group `group`: a group of the same digest is bound to the
/// same. A texture counts by the image's place in memory, which a scene keeps every image it
/// bound at.
pub(super) fn digest(feeds: &BTreeMap<(u32, u32), Feed>, group: u32) -> u64 {
    let mut digest = std::hash::DefaultHasher::new();
    for ((_, binding), feed) in feeds.range((group, 0)..=(group, u32::MAX)) {
        binding.hash(&mut digest);
        match feed {
            Feed::Bytes(bytes) => (0u8, bytes).hash(&mut digest),
            Feed::View { bytes, view_proj } => (1u8, bytes, view_proj).hash(&mut digest),
            Feed::Textures(images) => {
                2u8.hash(&mut digest);
                for image in images {
                    (image.as_ref().map_or(0, |i| Arc::as_ptr(i) as usize)).hash(&mut digest);
                }
            }
        }
    }
    digest.finish()
}

impl MeshletScene {
    /// What the scene has made on the device since it was prepared.
    pub fn uploads(&self) -> Uploads {
        self.made.uploads
    }

    /// Bind again each group whose feeds in `surfaces` differ from those it was bound by, as a map
    /// that came in late makes them: no pass is built, nothing bound alike is made again, and the
    /// other groups are left as they are. `None` where `surfaces` are not those the scene draws;
    /// else how many groups were bound again.
    pub fn rebind(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surfaces: &[MeshletSurface],
    ) -> Result<Option<u32>, String> {
        let same = |draw: &super::Draw| {
            surfaces.get(draw.at).is_some_and(|surface| {
                let (vertex, fragment) = &draw.built._of;
                Arc::ptr_eq(vertex, &surface.vertex) && Arc::ptr_eq(fragment, &surface.fragment)
            })
        };
        let others = self
            .waiting
            .iter()
            .chain(self.refused.iter().map(|(at, _)| at));
        if !self.draws.iter().all(same) || others.max().is_some_and(|at| *at >= surfaces.len()) {
            return Ok(None);
        }
        // ponytail: a scene with forward surfaces is prepared again for a late map; bind theirs
        // here as well when a model of them shows the wait.
        if self.over.is_some() {
            return Ok(None);
        }
        let mut rebound = 0;
        for draw in &mut self.draws {
            let surface = &surfaces[draw.at];
            let built = Arc::clone(&draw.built);
            let mut moved = false;
            for ((group, bound), (was, views)) in draw.groups.iter_mut().zip(&mut draw.bound_by) {
                let now = digest(&surface.feeds, *group);
                if now == *was {
                    continue;
                }
                let Some(Some(layout)) = built.layouts.get(*group as usize) else {
                    continue;
                };
                let module = match *group % 2 {
                    0 => &built.vertex,
                    _ => &built.fragment,
                };
                let fresh = bind(
                    device,
                    queue,
                    module,
                    *group,
                    layout,
                    &surface.feeds,
                    &mut self.made,
                )?;
                *bound = fresh.group;
                *was = now;
                *views = fresh.views;
                moved = true;
                rebound += 1;
            }
            if moved {
                let views = draw
                    .bound_by
                    .iter()
                    .flat_map(|(_, views)| views.iter().cloned());
                draw.views = views.collect::<Vec<ViewWrite>>();
            }
        }
        if rebound > 0 {
            self.made.uploads.rebound += rebound;
            // A group bound again holds no view yet, and the targets lack what it binds.
            self.forget();
        }
        Ok(Some(rebound))
    }
}

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;
