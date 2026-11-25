//! The colour a list of far stand-ins states for its trees, which stands in for a tree's colour map no file holds.
//!
//! ⚠ A stand-in, not the game's arithmetic: bytes 8 to 10 of a far stand-in's record are a
//! colour its program reads, how is not read; most records state white, which says nothing.

use std::collections::HashMap;

use wgpu::util::DeviceExt;

use crate::state::gta6_world::worldview::stream::slod::CardList;

/// White, which a record states where it says nothing of its tree's colour.
const WHITE: [u8; 3] = [255; 3];
/// What a material remembers a colour map no file holds by once a stated colour stands in for it: no texture's stamp.
pub(crate) const STATED: u64 = u64::MAX;

/// The colour most of the records of species `id` state in `lists`, where more than half of them state the same colour and it is not white.
pub(super) fn stated(lists: &[CardList], id: u16) -> Option<[u8; 3]> {
    let mut counts: HashMap<[u8; 3], usize> = HashMap::new();
    let mut total = 0usize;
    for list in lists.iter().filter(|list| list.ids.contains(&id)) {
        for record in list.records.iter() {
            if u16::from_le_bytes([record[14], record[15]]) == id {
                *counts
                    .entry([record[8], record[9], record[10]])
                    .or_default() += 1;
                total += 1;
            }
        }
    }
    let (colour, most) = counts.into_iter().max_by_key(|(_, n)| *n)?;
    (colour != WHITE && most * 2 > total).then_some(colour)
}

/// A one-texel colour map of `colour`, stored as sRGB as a colour map is.
pub(super) fn view_of(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    colour: [u8; 3],
) -> wgpu::TextureView {
    let [r, g, b] = colour;
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("world-stated-colour"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &[r, g, b, 255],
    );
    texture.create_view(&Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: u16, colour: [u8; 3]) -> [u8; 16] {
        let mut r = [0u8; 16];
        r[8..11].copy_from_slice(&colour);
        r[14..16].copy_from_slice(&id.to_le_bytes());
        r
    }

    fn list(records: Vec<[u8; 16]>) -> CardList {
        let mut ids: Vec<u16> = records
            .iter()
            .map(|r| u16::from_le_bytes([r[14], r[15]]))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        CardList {
            centre: [0.0; 3],
            radius: 1.0,
            min: [0.0; 3],
            size: [1.0; 3],
            scale_range: [1.0; 2],
            distance: 1.0,
            fade: 1.0,
            records: records.into(),
            ids: ids.into(),
        }
    }

    #[test]
    fn a_species_takes_the_colour_most_of_its_records_state_and_none_where_they_state_white_or_disagree(
    ) {
        let olive = [68, 91, 22];
        let lists = [
            list(vec![record(3, olive), record(3, olive), record(4, WHITE)]),
            list(vec![
                record(3, [255, 255, 128]),
                record(5, olive),
                record(5, WHITE),
            ]),
            list(vec![record(4, WHITE), record(4, olive)]),
        ];
        assert_eq!(
            stated(&lists, 3),
            Some(olive),
            "two of three, over two lists"
        );
        assert_eq!(stated(&lists, 4), None, "white says nothing");
        assert_eq!(stated(&lists, 5), None, "half is not most");
        assert_eq!(stated(&lists, 9), None, "a species no list names");
    }
}
