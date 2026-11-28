//! The bindings of a lit forward pass the renderer fills from its own frame, each time the light
//! it draws by moves, where the game fills them from its running world.

/// A binding of a lit forward stage the renderer fills from the frame it lights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Frame {
    /// The directional light's record, 340 bytes.
    // The way the light travels at +0x110, its light at +0x12C, its strength at +0x138 and an
    // interior's tint at +0x148: all a lit program reads of it.
    DirectionalLight,
    /// `g_AmbientMap`: a probe's nine numbers of the light from all around, in seven slices.
    AmbientMap,
    /// The view's lighting record, 2,044 bytes: the directional light's way and roughness, the
    /// clouds' share, the fog's slices, the probe levels and the glass's remap.
    ViewLighting,
    /// The table of what a surface reflects of an even light, by the cosine to the eye across
    /// and the roughness down.
    EnvironmentFit,
    /// The probes' light: a tile of each probe's light along every way, laid out by the view's
    /// lighting record, which picks the texture from the heap by its number at +0x568.
    ProbeRadiance,
    /// How far each probe sees along every way, in tiles laid out as its light's.
    ProbeVisibility,
}

/// The directional light's record.
pub const DIRECTIONAL_LIGHT: u32 = 0xA96F_368C;
/// `g_AmbientMap`.
pub const AMBIENT_MAP: u32 = 0x3094_0D63;
/// The view's lighting record.
pub const VIEW_LIGHTING: u32 = 0x0048_742C;
/// The environment fit's table.
pub const ENVIRONMENT_FIT: u32 = 0x410A_2FC7;
/// The probes' visibility, gathered four texels at a time.
pub const PROBE_VISIBILITY: u32 = 0x856A_CFEF;

/// The bindings the frame fills whose counts a loop may run to: the view's lighting record
/// states two probe levels, so neither probe loop takes more than two turns.
pub const AT_REST: &[u32] = &[VIEW_LIGHTING];

/// The words of the view's lighting record the renderer always writes, by byte: two probe levels.
pub const VIEW_WORDS: [(u32, u32); 1] = [(super::trees::VIEW_LEVELS as u32, 2)];

impl Frame {
    /// The hash of the name of the binding it fills; of a texture picked from the heap, the
    /// record's that picks it.
    pub fn hash(self) -> u32 {
        match self {
            Self::DirectionalLight => DIRECTIONAL_LIGHT,
            Self::AmbientMap => AMBIENT_MAP,
            Self::ViewLighting | Self::ProbeRadiance => VIEW_LIGHTING,
            Self::EnvironmentFit => ENVIRONMENT_FIT,
            Self::ProbeVisibility => PROBE_VISIBILITY,
        }
    }
}

/// The binding named `hash` the frame fills, where it is one.
pub fn frame_of(hash: u32) -> Option<Frame> {
    [
        Frame::DirectionalLight,
        Frame::AmbientMap,
        Frame::ViewLighting,
        Frame::EnvironmentFit,
        Frame::ProbeVisibility,
    ]
    .into_iter()
    .find(|frame| frame.hash() == hash)
}

#[cfg(test)]
mod tests {
    use or_io::gta5::shaders::sgd5::{Sgd5Resource, Sgd5Table};

    use super::*;
    use crate::dxil::ShaderKind;
    use crate::rdna::{Loc, Lowered, Program, Resource, ResourceKind};
    use crate::rdna_bind::forward::CascadeRecord;
    use crate::rdna_bind::shading::{feed, Feed, Unfed};
    use crate::rdna_bind::{Class, Native, Record};

    const TABLE: u8 = 9;
    const CASCADES: u32 = 0xA6D7_79F9;
    const FRAME_RECORD: u32 = 0x6E87_25E4;
    const PROBES: u32 = 0x669A_6134;

    /// A lit forward pixel program (targets 0 and 1 written whole) whose four constant buffers,
    /// at registers 0 to 3, are named `names`.
    fn native(names: &[u32; 4]) -> (Native, Sgd5Table) {
        let exports = [(9u32, 0u32), (0, 15), (1, 15)];
        let words = (exports.iter())
            .flat_map(|(target, en)| [(0x3E << 26) | (target << 4) | en, 0])
            .collect();
        let records = (0..4)
            .map(|register| Record {
                class: Class::Cbv,
                register,
                root: TABLE,
                offset: 0x20 * register,
                dwords: 4,
                stride: 0,
            })
            .collect();
        let resources = (names.iter().zip(0u16..))
            .map(|(name, slot)| Sgd5Resource {
                code: 0x30,
                flags: 0,
                slot,
                members: 0,
                buffer_size: 2048,
                name_hash: *name,
                semantic_hash: *name,
            })
            .collect();
        let native = Native {
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
            records,
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        let table = Sgd5Table {
            resources,
            ..Sgd5Table::default()
        };
        (native, table)
    }

    /// The stage: the probe levels' do-while as the lit program runs it, on binding 0's dword
    /// `count` (it goes on while `count - 1` is past the level), and a read of each other buffer.
    fn lowered(count: &str) -> Lowered {
        let wgsl = format!(
            "@group(0) @binding(0) var<uniform> cb0_: array<vec4<u32>, 128>;
@group(0) @binding(1) var<uniform> cb1_: array<vec4<u32>, 41>;
@group(0) @binding(2) var<uniform> cb2_: array<vec4<u32>, 128>;
@group(0) @binding(3) var<uniform> cb3_: vec4<u32>;
@fragment fn main() -> @location(0) vec4<f32> {{
var s25_: u32 = 0u;
loop {{
if !({count} - 1u > s25_) {{ break; }}
s25_ = s25_ + 1u;
}}
return vec4<f32>(f32(s25_), f32(cb1_[3].w), f32(cb2_[98].x), f32(cb3_.x));
}}"
        );
        let module = naga::front::wgsl::parse_str(&wgsl).expect("the WGSL parses");
        let resource = |binding: u32| Resource {
            kind: ResourceKind::ConstantBuffer { rows: 128 },
            at: Some(Loc {
                root: TABLE,
                path: Vec::new(),
                offset: 0x20 * binding,
                index: None,
            }),
            words: None,
            group: 0,
            binding,
            count: None,
            written: false,
        };
        Lowered {
            module,
            resources: (0..4).map(resource).collect(),
            approximations: Default::default(),
            half_slots: Default::default(),
            subgroups: false,
            intersections: Vec::new(),
            region: None,
        }
    }

    #[test]
    fn a_lit_stage_is_fed_the_view_s_record_the_frame_s_and_the_cascades_and_its_probe_loop_ends() {
        let names = [VIEW_LIGHTING, CASCADES, FRAME_RECORD, PROBES];
        let (native, table) = native(&names);
        // The level count at +0x72C, row 114's fourth dword: stated as one level.
        let stage = feed(
            &native,
            lowered("cb0_[114].w"),
            &table,
            &Sgd5Table::default(),
        )
        .expect("a lit stage is fed whole");
        let feeds: Vec<Feed> = stage.feeds.into_iter().map(|fed| fed.feed).collect();
        let Feed::Stated(frame) = &feeds[2] else {
            panic!("the frame record is not stated: {:?}", feeds[2]);
        };
        assert_eq!(frame[0x620..0x624], 0.5f32.to_le_bytes());
        assert_eq!(
            [&feeds[0], &feeds[1], &feeds[3]],
            [
                &Feed::Frame(Frame::ViewLighting),
                &Feed::Stated(CascadeRecord::AT_REST.bytes()),
                &Feed::Zero(Some(PROBES)),
            ]
        );
    }

    #[test]
    fn a_loop_to_a_frame_s_record_whose_count_is_not_stated_and_a_place_not_established_are_refused(
    ) {
        // The same loop on the directional light's record, whose numbers no count is stated by.
        let names = [DIRECTIONAL_LIGHT, CASCADES, FRAME_RECORD, PROBES];
        let (stage, table) = native(&names);
        let fed = feed(
            &stage,
            lowered("cb0_[114].w"),
            &table,
            &Sgd5Table::default(),
        );
        assert_eq!(fed.err(), Some(Unfed::Loop(DIRECTIONAL_LIGHT)));
        // A place of the view's record nothing states (+0x40) is refused by the record's name.
        let names = [VIEW_LIGHTING, CASCADES, FRAME_RECORD, PROBES];
        let (stage, table) = native(&names);
        let fed = feed(&stage, lowered("cb0_[4].x"), &table, &Sgd5Table::default());
        assert_eq!(fed.err(), Some(Unfed::Unread(VIEW_LIGHTING)));
    }

    #[test]
    fn each_binding_the_frame_fills_is_told_by_its_name() {
        for frame in [
            Frame::DirectionalLight,
            Frame::AmbientMap,
            Frame::ViewLighting,
            Frame::EnvironmentFit,
            Frame::ProbeVisibility,
        ] {
            assert_eq!(frame_of(frame.hash()), Some(frame));
        }
        // The probes' light has no name: the heap's texture the view's record picks.
        assert_eq!(
            frame_of(Frame::ProbeRadiance.hash()),
            Some(Frame::ViewLighting)
        );
        assert_eq!(frame_of(CASCADES), None);
        assert!(AT_REST.contains(&VIEW_LIGHTING) && !AT_REST.contains(&DIRECTIONAL_LIGHT));
    }
}
