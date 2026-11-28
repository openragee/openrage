//! The textures a ped's pixel program picks from the heap by the numbers of its component's
//! buffer, and that buffer: what the renderer fills them from is the component's, by name.

use or_io::gta5::shaders::sgd5::Sgd5Table;
use or_io::gta6::hash::name_hash32;

use super::frame_records::RECORDS as WIND_FIELDS;
use super::ped_layers::DECORATION;
use super::shading::Feed;
use super::{join, numbered, Class, Join, Place, Record};
use crate::dxil::ShaderKind;
use crate::rdna::{Index, IndexSource, Resource, ResourceKind};

/// The constant buffer of 48 bytes the game fills from a ped's component for each draw.
pub const COMPONENT: u32 = 0xF6C3_FF80;
/// The image instruction's dimension field of a two-dimensional array: the three picked
/// textures are arrays.
const DIM_2D_ARRAY: u8 = 5;
/// The same field of a plain two-dimensional texture: the two decoration textures are.
const DIM_2D: u8 = 1;
/// The bytes of the decoration buffer whose numbers pick its mask and its material's texture.
// A component with no decoration object of its own, and no kind that takes a global one, is
// bound 48 bytes of noughts: both numbers nought, which the pixel program samples neither of.
const DECORATION_PICKS: [u32; 2] = [0, 8];
/// A number shifted by this reaches its heap entry.
const ENTRY_SHIFT: u8 = 5;

/// The texture a number of the component's buffer names, by the byte the number is at: dwords 9,
/// 10 and 11 are the component's wrinkle mask, wrinkle index and wrinkle map.
// Dword 8 is the one the pixel program tests first: the three are sampled only where it is not
// negative, and the game sets it to -1 for a component with no wrinkle map.
const PICKED: [(u32, u32); 3] = [
    (36, name_hash32(b"wrinkleMask")),
    (40, name_hash32(b"wrinkleIndex")),
    (44, name_hash32(b"wrinkle")),
];

/// What fills the texture `resource` a `kind` program picks from the heap by a number of the
/// component's buffer (the component's texture of that name) or of its decoration buffer (the
/// heap's blank first entry, which the numbers at rest pick); `None` for any other pick.
pub fn picked(
    resource: &Resource,
    kind: ShaderKind,
    records: &[Record],
    table: &Sgd5Table,
) -> Option<Feed> {
    if resource.count.is_some() {
        return None;
    }
    let ResourceKind::Texture { dim, .. } = resource.kind else {
        return None;
    };
    let at = resource.at.as_ref()?;
    let draw = records.iter().find(|r| r.class == Class::RootConstants)?;
    let user = |register: u8| match numbered(Place::Words { root: register }, kind) {
        Place::Words { root } => root,
        _ => register,
    };
    // The heap is the table the third dword of a draw points at.
    if !at.path.is_empty() || user(at.root) != draw.root.wrapping_add(2) {
        return None;
    }
    let Some(Index {
        source:
            IndexSource::Data {
                buffer,
                offset,
                plus: None,
            },
        shift: ENTRY_SHIFT,
    }) = &at.index
    else {
        return None;
    };
    if !buffer.path.is_empty() || buffer.index.is_some() {
        return None;
    }
    let place = Place::Table {
        root: buffer.root,
        offset: buffer.offset,
    };
    let Join::Named { record, named } = join(numbered(place, kind), records, table) else {
        return None;
    };
    if record.class != Class::Cbv {
        return None;
    }
    match (named.name_hash, dim) {
        (COMPONENT, DIM_2D_ARRAY) => (PICKED.iter())
            .find_map(|(byte, name)| (byte == offset).then_some(Feed::PedTexture(*name))),
        (DECORATION, DIM_2D) if DECORATION_PICKS.contains(offset) => Some(Feed::Blank),
        _ => None,
    }
}

/// What fills the array of textures a mesh program whose package states no dwords of a draw
/// picks by a number it computes, where its table names the wind field records: the heap, which
/// a field's record numbers its map in.
// The pick lies in the walk of the wind fields, as many as dword 0 of the frame's wind
// constants states, nought at rest (`frame_records::AT_REST`), so the walk runs no turn.
pub fn wind_field_maps(
    resource: &Resource,
    kind: ShaderKind,
    records: &[Record],
    table: &Sgd5Table,
) -> Option<Feed> {
    let ResourceKind::Texture { dim, .. } = resource.kind else {
        return None;
    };
    let at = resource.at.as_ref()?;
    let computed = matches!(
        at.index,
        Some(Index {
            source: IndexSource::Runtime { .. },
            ..
        })
    );
    let drawn = records.iter().any(|r| r.class == Class::RootConstants);
    let fields = table.resources.iter().any(|r| r.name_hash == WIND_FIELDS);
    let heap = kind == ShaderKind::Mesh && resource.count.is_some() && at.path.is_empty();
    (heap && computed && !drawn && fields).then_some(Feed::Heap { dim })
}

#[cfg(test)]
mod tests {
    use or_io::gta5::shaders::sgd5::Sgd5Resource;

    use super::*;
    use crate::rdna::{Loc, Lowered, Program};
    use crate::rdna_bind::shading::{feed, Feed, Unfed};
    use crate::rdna_bind::{Native, Unbound};

    /// The user register of the draw's dwords: the heap is the table of the one two past it.
    const DRAW: u8 = 12;
    /// Where the component's buffer's descriptor is: register 7 of the table in user register 9.
    const TABLE: u8 = 9;
    const AT: u32 = 0x40;

    /// A deferred pixel program: its exports are the five targets whose contents are established.
    fn native() -> Native {
        let exports = [(0u32, 15u32), (1, 15), (2, 15), (3, 15), (4, 3)];
        let words = (exports.iter())
            .flat_map(|(target, en)| [(0x3E << 26) | (target << 4) | en, 0])
            .collect();
        let record = |class, register, root, offset| Record {
            class,
            register,
            root,
            offset,
            dwords: 4,
            stride: 0,
        };
        Native {
            program: Program {
                kind: ShaderKind::Pixel,
                slot: 5,
                words,
                wave32: false,
                inputs: Vec::new(),
                vertex_inputs: Vec::new(),
                position_read: 0,
                front_face: false,
                launch: None,
            },
            records: vec![
                record(Class::Cbv, 7, TABLE, AT),
                record(Class::RootConstants, 0, DRAW, 0),
            ],
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }

    /// The program's own table, which names register 7's constant buffer `buffer`.
    fn table(buffer: u32) -> Sgd5Table {
        Sgd5Table {
            resources: vec![Sgd5Resource {
                code: 0x30,
                flags: 0,
                slot: 7,
                members: 0,
                buffer_size: 48,
                name_hash: buffer,
                semantic_hash: buffer,
            }],
            ..Sgd5Table::default()
        }
    }

    fn loc(root: u8, offset: u32, index: Option<Index>) -> Loc {
        Loc {
            root,
            path: Vec::new(),
            offset,
            index,
        }
    }

    /// The component's buffer, and a texture of dimension field `dim` the program picks from the
    /// heap by the number at `byte` of that buffer, plus `plus`'s register where it is one.
    fn lowered(byte: u32, plus: Option<u8>, dim: u8) -> Lowered {
        let resource = |kind, at, binding| Resource {
            kind,
            at: Some(at),
            words: None,
            group: 0,
            binding,
            count: None,
            written: false,
        };
        let number = Index {
            source: IndexSource::Data {
                buffer: Box::new(loc(TABLE, AT, None)),
                offset: byte,
                plus,
            },
            shift: ENTRY_SHIFT,
        };
        Lowered {
            module: naga::Module::default(),
            resources: vec![
                resource(
                    ResourceKind::ConstantBuffer { rows: 3 },
                    loc(TABLE, AT, None),
                    0,
                ),
                resource(
                    ResourceKind::Texture {
                        dim,
                        compared: false,
                    },
                    loc(DRAW + 2, 0, Some(number)),
                    1,
                ),
            ],
            approximations: Default::default(),
            half_slots: Default::default(),
            subgroups: false,
            intersections: Vec::new(),
            region: None,
        }
    }

    fn fed(byte: u32, plus: Option<u8>, buffer: u32) -> Result<Vec<Feed>, Unfed> {
        fed_as(byte, plus, buffer, DIM_2D_ARRAY)
    }

    fn fed_as(byte: u32, plus: Option<u8>, buffer: u32, dim: u8) -> Result<Vec<Feed>, Unfed> {
        let stage = feed(
            &native(),
            lowered(byte, plus, dim),
            &table(buffer),
            &Sgd5Table::default(),
        )?;
        Ok(stage.feeds.into_iter().map(|fed| fed.feed).collect())
    }

    #[test]
    fn a_ped_pixel_stage_is_fed_the_texture_its_component_s_number_names_and_no_other() {
        // Dwords 9, 10 and 11 of the component's buffer name its wrinkle mask, index and map.
        for (byte, name) in [
            (36, name_hash32(b"wrinkleMask")),
            (40, name_hash32(b"wrinkleIndex")),
            (44, name_hash32(b"wrinkle")),
        ] {
            assert_eq!(
                fed(byte, None, COMPONENT),
                Ok(vec![Feed::PedComponent, Feed::PedTexture(name)])
            );
        }
        // A number nothing establishes: another dword, one placed by a register, another buffer's.
        let refused = Err(Unfed::Unbound(Unbound::Picked));
        assert_eq!(fed(48, None, COMPONENT), refused);
        assert_eq!(fed(32, None, COMPONENT), refused);
        assert_eq!(fed(36, Some(13), COMPONENT), refused);
        assert_eq!(fed(36, None, 0x1234_5678), refused);
        // The refusal every other program meets says what it said.
        assert_eq!(
            Unbound::Picked.to_string(),
            "it picks a resource by a number the game sets, or reaches it through a pointer"
        );
    }

    #[test]
    fn a_head_s_pixel_stage_is_fed_its_two_decoration_picks_as_the_heap_s_blank_entry() {
        // The decoration buffer is the game's 48 noughts, so both numbers pick entry nought.
        for byte in DECORATION_PICKS {
            assert_eq!(
                fed_as(byte, None, DECORATION, DIM_2D),
                Ok(vec![Feed::Zero(Some(DECORATION)), Feed::Blank])
            );
        }
        let refused = Err(Unfed::Unbound(Unbound::Picked));
        assert_eq!(fed_as(4, None, DECORATION, DIM_2D), refused);
        assert_eq!(fed_as(0, None, DECORATION, DIM_2D_ARRAY), refused);
        assert_eq!(fed_as(0, Some(13), DECORATION, DIM_2D), refused);
    }

    /// A skinned mesh program with no dwords of a draw that picks from an array of 256 textures
    /// by a number it computes; its table names the wind field records where `fields` is set.
    fn skinned(fields: bool) -> (Native, Lowered, Sgd5Table) {
        let native = Native {
            program: Program {
                kind: ShaderKind::Mesh,
                slot: 8,
                words: Vec::new(),
                wave32: false,
                inputs: Vec::new(),
                vertex_inputs: Vec::new(),
                position_read: 0,
                front_face: false,
                launch: None,
            },
            records: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        let computed = Index {
            source: IndexSource::Runtime { site: 0 },
            shift: 0,
        };
        let lowered = Lowered {
            module: naga::Module::default(),
            resources: vec![Resource {
                kind: ResourceKind::Texture {
                    dim: DIM_2D,
                    compared: false,
                },
                at: Some(loc(23, 0, Some(computed))),
                words: None,
                group: 0,
                binding: 36,
                count: Some(256),
                written: false,
            }],
            approximations: Default::default(),
            half_slots: Default::default(),
            subgroups: false,
            intersections: Vec::new(),
            region: None,
        };
        let mut table = Sgd5Table::default();
        if fields {
            table.resources.push(Sgd5Resource {
                code: 0x05,
                flags: 0,
                slot: 176,
                members: 0,
                buffer_size: 144,
                name_hash: WIND_FIELDS,
                semantic_hash: WIND_FIELDS,
            });
        }
        (native, lowered, table)
    }

    #[test]
    fn a_skinned_mesh_stage_s_pick_of_a_wind_field_s_map_is_fed_the_heap_and_no_other_pick_is() {
        let fed = |fields| {
            let (native, lowered, table) = skinned(fields);
            feed(&native, lowered, &table, &Sgd5Table::default()).map(|stage| {
                stage
                    .feeds
                    .into_iter()
                    .map(|fed| fed.feed)
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(fed(true), Ok(vec![Feed::Heap { dim: DIM_2D }]));
        assert_eq!(fed(false), Err(Unfed::Unbound(Unbound::Picked)));
    }
}
