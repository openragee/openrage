//! Where each binding of a pass that shades a GTA VI model gets its contents: the model's own
//! buffers, the view, the material, or what only the game fills when it runs.

use std::collections::{BTreeMap, BTreeSet};

use or_io::gta5::shaders::sgd5::{Sgd5Resource, Sgd5Table};
use or_io::gta5::shaders::sgd5_pack::constant_buffers;
use or_io::gta5::shaders::spirv_pack::{ConstantBuffer, SignatureElement};
use or_io::hash::string_hash_str;

use super::budget::{self, Budget, Fitted};
use super::loops;
use super::picks::{deferred_targets, material_textures, picks, Unsettled, TEXTURE_ARRAY};
use super::plain::plain_textures;
use super::terrain;
use super::{join, named, numbered, place, stated, Class, Join, Native, Place, Record, Unbound};
use crate::dxil::ShaderKind;
use crate::rdna::{Index, IndexSource, Lowered, Resource, ResourceKind};

/// The first register an effect's table gives the constant buffers a drawable's parameter block fills.
const MATERIAL_REGISTERS: u16 = 256;
/// The element size a binding record states for a buffer of bytes.
const RAW: u32 = 1;

/// What fills a binding of a model's shading pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// The geometry's vertex data as its file stores it: the slot table, the quantisation block
    /// and the streams, which a mesh program reaches by its address.
    VertexData,
    /// The geometry's index data as its file stores it: the header, the meshlet records, the
    /// primitives and the vertex list.
    Meshlets,
    /// The view's matrices: the constant buffer that holds `gViewProj`.
    View,
    /// A constant buffer a drawable's parameter block fills.
    Material,
    /// The dwords the pipeline passes with each draw.
    DrawWords,
    Sampler,
    /// A buffer, an image or a constant buffer only the game fills, by the hash of its name.
    Game(u32),
    /// No one record of the program's table names its register.
    Unnamed,
}

impl Source {
    /// Whether a model's file or the renderer's view holds what fills it.
    pub fn held(self) -> bool {
        matches!(
            self,
            Self::VertexData | Self::Meshlets | Self::View | Self::Material
        )
    }
}

/// The buffer of bytes the game binds a geometry's meshlet list to before it draws the geometry,
/// by the hash of its name.
// ⚠ Told by its name and not as the one buffer of bytes a program binds: a ped's skinned program
// binds its shape data as buffers of bytes beside it, and with the list left at nought every
// triangle is three times the first vertex, which draws nothing.
pub const MESHLET_LIST: u32 = 0xE0BF_99E7;

/// What fills each of `records`, the bindings of one program of a shading pass, in their order;
/// `mesh` where it is the pass's mesh program, `table` its own variable table and `effect` the
/// first table of the effect that runs it.
pub fn sources(
    records: &[Record],
    mesh: bool,
    table: &Sgd5Table,
    effect: &Sgd5Table,
) -> Vec<Source> {
    let own = |hash: u32| effect.resources.iter().find(|r| r.name_hash == hash);
    let view = string_hash_str("gViewProj");
    (records.iter())
        .map(|record| {
            match record.class {
                Class::Sampler => return Source::Sampler,
                Class::RootConstants => return Source::DrawWords,
                Class::Address if mesh => return Source::VertexData,
                Class::Address | Class::Srv | Class::Cbv | Class::Uav => {}
            }
            let Some(hash) = named(table, record).map(|n| n.name_hash) else {
                return Source::Unnamed;
            };
            let members = || {
                table
                    .constants
                    .iter()
                    .filter(move |c| c.cbuffer == Some(hash))
            };
            match record.class {
                Class::Cbv if members().any(|c| c.name_hash == view) => Source::View,
                Class::Cbv if own(hash).is_some_and(|r| r.slot >= MATERIAL_REGISTERS) => {
                    Source::Material
                }
                Class::Srv if mesh && record.stride == RAW && hash == MESHLET_LIST => {
                    Source::Meshlets
                }
                _ => Source::Game(hash),
            }
        })
        .collect()
}

/// What the renderer gives one binding of a lowered program of a model's shading pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Feed {
    /// Noughts: the dwords of a draw, or a binding only the game fills, by the hash of its name.
    Zero(Option<u32>),
    VertexData,
    Meshlets,
    /// The view's constant buffer, by its layout.
    View(ConstantBuffer),
    /// A constant buffer of the material, by its layout.
    Material(ConstantBuffer),
    /// Matrices of sixteen floats: the object's, and again for the frame before, or a bone's.
    Placement,
    /// The records a mesh program moves a vertex's texture coordinates by: eight rows of three
    /// floats each, of which it takes row `k` for u and row `4 + k` for v, each as
    /// `x * u + y * v + z`. Every pair leaves the coordinates where they are.
    UvTransforms,
    /// A skinned program's bones: three rows of four floats each, every bone at rest.
    Bones,
    /// The object's own record: the three rows of three floats that turn a normal to the world
    /// from byte 16, twelve bytes apart, and noughts for the rest.
    Instance,
    /// The records an instanced draw places its instances by, each an instance at rest.
    InstanceRecords,
    /// The numbers of the material's textures: number `i` at dword `i`.
    Numbers,
    /// The head of the material's record, which starts the buffer: each dword of it with the
    /// number of the texture it lists. The rest of the record is the game's, and noughts.
    Listed(Vec<(u32, u32)>),
    /// The heap the numbers pick from, bound whole: every entry a texture of this dimension field.
    Heap {
        dim: u8,
    },
    /// The material's texture of this number, which the program picks from the heap by a number
    /// it reads at a place its code states.
    Texture(u32),
    /// The heap's own dwords, eight an entry: what a program reads of a descriptor as a number,
    /// such as an image's size.
    HeapWords,
    /// The heap's buffers as one buffer of dwords: where each entry's buffer starts, then the
    /// buffers, each empty, since a draw with noughts for its dwords reads none of them.
    HeapBuffers,
    /// The heap's first entry, which a number left at nought picks: a blank texel.
    Blank,
    Sampler,
    /// The bytes the game leaves in a binding only it fills while what writes it is off, the rest
    /// noughts ([`super::forward`]).
    Stated(Vec<u8>),
    /// The one white texel the game binds where what writes a texture is off.
    White,
    /// A record or a texture the renderer fills from the frame it lights ([`super::forward_frame`]).
    Frame(super::forward_frame::Frame),
    /// A buffer or a texture of the terrain's, as the game binds it for a tile with no record of its own.
    Terrain(super::terrain::Terrain),
    /// A ped component's texture of this name, picked by a number of its buffer ([`super::ped`]).
    PedTexture(u32),
    /// A ped component's buffer of 48 bytes, which the renderer fills from the component.
    PedComponent,
    /// A texture of the game's own dictionaries, by the hash of its name, which the game binds
    /// whatever is drawn ([`super::ped_skin`]).
    GameTexture(u32),
    /// The table of skin tone records: record nought alone at rest, the loaded presets' records
    /// where the renderer has them ([`super::ped_skin`]).
    SkinTones,
    /// An object's own record of the pool `OBJECT_DATA` names, at rest, in which the renderer
    /// gives a ped its skin tone ([`super::ped_layers::ObjectData`]).
    ObjectData,
}

/// One binding of a stage with what fills it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fed {
    pub group: u32,
    pub binding: u32,
    pub feed: Feed,
    /// The element size a buffer read by address states before its first element; `None` for a
    /// binding with no such header.
    pub header: Option<u32>,
}

/// Why a shading pass is not one the renderer can feed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unfed {
    #[error(transparent)]
    Unbound(#[from] Unbound),
    #[error(transparent)]
    Numbers(#[from] Unsettled),
    #[error("its pixel program writes targets whose contents are not established")]
    Targets,
    #[error("a loop of it counts up to a number of 0x{0:08X}, which only the game fills and whose state at rest is not known")]
    Loop(u32),
    #[error("{0} of its loops are not shown to end by its translation")]
    NotShown(u32),
    #[error("a loop of it never leaves with what the editor binds: every exit is known, and none is taken within {0} turns")]
    Endless(u32),
    #[error("its translation departs from the hardware where two invocations meet at one texel or pixel: {0}")]
    Approximated(&'static str),
    #[error("it picks an entry of an array of textures or buffers by a number nothing holds to the array's length, and no device checks one")]
    Entry,
    #[error("it is too large and branches too often for the software adapter to draw ({0} stores, {1} branches), so it has not been checked there, and it stays undrawn on a hardware adapter until its owner allows a check")]
    Unchecked(u32, u32),
    #[error(
        "it reads vertices a compute pass of the game's skins first, and the editor runs none"
    )]
    SkinnedBefore,
    #[error("it places the instances of a batch by a list and records only the game fills, at nought here, and draws nothing")]
    Batch,
    #[error("it places each instance of an instanced draw by a record only the game fills, at nought here, and draws nothing")]
    Instanced,
    #[error("it is a forward pass that draws with the engine's own blend state, which no effect states and which is not read")]
    Blend,
    #[error("it is a forward pass that reads 0x{0:08X}, which only the game fills: noughts there change what it draws, and what the game leaves there is not read")]
    Unread(u32),
    #[error("it is a lit forward pass that reads 0x{0:08X}, whose like the editor's frame holds and does not hand a forward pass yet")]
    Unwired(u32),
    #[error("it reads a terrain tile's cells (0xFE9D6ABC), which the game binds only with the tile's own record, and what a stage reads of them without one is not read")]
    TerrainCells,
    #[error("it reads back the terrain's feedback texture, which only the game keeps")]
    TerrainFeedback,
}

/// One stage of a shading pass ready for the renderer: its module, what fills each binding, and
/// the +0x14 hash of the material's texture each number names.
#[derive(Debug, Clone)]
pub struct Stage {
    /// Shared: a renderer keeps what it builds of a stage by where the stage lies.
    pub module: std::sync::Arc<naga::Module>,
    pub feeds: Vec<Fed>,
    pub textures: Vec<(u32, u32)>,
    /// The entries of the vertex data's table of streams a mesh program reads at stated places:
    /// the streams a geometry must hold for the program to be the one that draws it.
    pub streams: BTreeSet<u32>,
    /// The numbers the stage reads as a row of texels: a palette, loaded at a place and not
    /// sampled.
    pub rows: Vec<u32>,
    /// Each way the stage departs from the hardware, counted, as its lowering says them.
    pub approximations: BTreeMap<&'static str, u32>,
}

/// The buffers only the game fills whose state at rest is known, so that a loop may count up to
/// a number of theirs: each owner's list, read as one.
const AT_REST: [&[u32]; 4] = [
    super::trees::AT_REST,
    &[super::ped_layers::DECORATIONS],
    super::frame_records::AT_REST,
    super::forward_frame::AT_REST,
];

/// Whether the buffer named `hash` is one of [`AT_REST`].
fn at_rest(hash: u32) -> bool {
    AT_REST.iter().any(|list| list.contains(&hash))
}

/// Whether the developer's aid `unguarded` of `OPENRAGE_GTA6_SHADED` is given: a count to a number
/// only the game fills, and a resource picked by one, are then let through, for a renderer that
/// draws the stage on a software adapter alone.
fn unguarded() -> bool {
    std::env::var("OPENRAGE_GTA6_SHADED")
        .is_ok_and(|aids| aids.split(',').any(|word| word.trim() == "unguarded"))
}

/// What fills a binding a stage picks by a number: what the heap gives it, or noughts where it
/// is none of the heap's and the developer's aid `unguarded` lets the stage through.
fn picked_feed(found: Option<Feed>, unguarded: bool) -> Result<Feed, Unbound> {
    match (found, unguarded) {
        (Some(feed), _) => Ok(feed),
        (None, true) => Ok(Feed::Zero(None)),
        (None, false) => Err(Unbound::Picked),
    }
}

/// How each loop of a stage ends with `feeds` bound, in the order of [`loops::loops`]; `Err`
/// where one may never end, or ends by a number only the game fills.
// Judged before the rewrites that follow it in `fed`, which move a binding or read it through a
// call and change no loop, no count and no binding's writability; the watchdog is given the
// module drawn, and refuses one whose loops are not those judged.
fn never_ending(
    module: &naga::Module,
    feeds: &[Fed],
    written: &BTreeSet<(u32, u32)>,
    approximations: &BTreeMap<&'static str, u32>,
) -> Result<Vec<loops::Ends>, Unfed> {
    // The lowering's own count comes first: it sees the program's graph, which the module no longer shows.
    let not_shown: u32 = (approximations.iter())
        .filter(|(name, _)| name.starts_with(crate::rdna::LOOP_NOT_SHOWN))
        .map(|(_, loops)| loops)
        .sum();
    if not_shown > 0 {
        return Err(Unfed::NotShown(not_shown));
    }
    if loops::loops(module) == 0 {
        return Ok(Vec::new());
    }
    let judged = loops::judge(module, &loop_bindings(feeds, written));
    // The developer's aid holds every loop to the cap of one not shown to end.
    if unguarded() {
        return Ok(vec![loops::Ends::Unshown; judged.ends.len()]);
    }
    for ends in &judged.ends {
        match ends {
            loops::Ends::Game(hash) => return Err(Unfed::Loop(*hash)),
            loops::Ends::Runaway => return Err(Unfed::Endless(loops::MOST_TURNS)),
            _ => {}
        }
    }
    Ok(judged.ends)
}

/// Every buffer only the game fills that an exit of a loop of a fed stage hangs on.
pub fn loops_count_to(module: &naga::Module, feeds: &[Fed]) -> Vec<u32> {
    let judged = loops::judge(module, &loop_bindings(feeds, &BTreeSet::new()));
    judged.hits.into_iter().collect()
}

/// Each binding of `feeds` with the words the device reads of it and, where no loop may hang on
/// its numbers, the game's hash; `written` are the bindings the stage's lowering writes.
fn loop_bindings(feeds: &[Fed], written: &BTreeSet<(u32, u32)>) -> Vec<loops::Binding> {
    use loops::Words;
    let rest = |hash| match hash {
        super::forward_frame::VIEW_LIGHTING => {
            Words::Places(super::forward_frame::VIEW_WORDS.to_vec())
        }
        _ => Words::Unknown,
    };
    (feeds.iter())
        .map(|fed| {
            let (words, game) = match &fed.feed {
                Feed::Zero(None) => (Words::Zero, None),
                // The game's buffers whose noughts are its state at rest.
                Feed::Zero(Some(hash)) if at_rest(*hash) => (Words::Zero, None),
                // Its noughts are no state of the game's: no branch is decided by them.
                Feed::Zero(Some(hash)) => (Words::Unknown, Some(*hash)),
                // Filled from the frame: a loop ends by it only where its count is stated.
                Feed::Frame(frame) if at_rest(frame.hash()) => (rest(frame.hash()), None),
                Feed::Frame(frame) => (Words::Unknown, Some(frame.hash())),
                // Filled from the component, but of a state no loop is known to end by.
                Feed::PedComponent => (Words::Unknown, Some(super::ped::COMPONENT)),
                // Two words of it are stated, and no count: its noughts are no state of the game's.
                Feed::Stated(_) if fed.feed == super::trees::frame_at_rest() => (
                    Words::Places(super::trees::frame_words()),
                    Some(super::trees::FRAME_RECORD),
                ),
                Feed::Stated(bytes) => (Words::Bytes(bytes.clone()), None),
                // ⚠ Judged at rest: a ped's fill changes only a record's number and a weight,
                // which the skin's pixel programs read as an index and a scale, never a count.
                Feed::ObjectData => (Words::Bytes(super::ped_layers::object_data_at_rest()), None),
                Feed::SkinTones => {
                    let rest = super::ped_skin::SkinToneRecord::AT_REST;
                    (Words::Bytes(rest.bytes()), None)
                }
                _ => (Words::Unknown, None),
            };
            // ⚠ A header comes before what is stated; what the stage writes has no words.
            let at = (fed.group, fed.binding);
            let words = match fed.header.is_some() || written.contains(&at) {
                true => Words::Unknown,
                false => words,
            };
            loops::Binding { at, words, game }
        })
        .collect()
}

/// How many stores `module` holds and how many times it branches (an `if`, a `switch` or a
/// loop), over all its functions: the measure of how much branching code a driver is handed.
pub fn stores_and_branches(module: &naga::Module) -> (u32, u32) {
    let mut count = (0, 0);
    let functions = (module.functions.iter().map(|(_, function)| function))
        .chain(module.entry_points.iter().map(|entry| &entry.function));
    functions.for_each(|function| counted(&function.body, &[], &mut count));
    count
}

/// The stores and branches of `module`'s entry points, each call counted as the body it calls.
// As Direct3D 12's compiler inlines every call.
pub fn inlined_stores_and_branches(module: &naga::Module) -> (u32, u32) {
    // A function calls only functions declared before it.
    let mut called = Vec::with_capacity(module.functions.len());
    for (_, function) in module.functions.iter() {
        let mut count = (0, 0);
        counted(&function.body, &called, &mut count);
        called.push(count);
    }
    let mut count = (0, 0);
    for entry in &module.entry_points {
        counted(&entry.function.body, &called, &mut count);
    }
    count
}

/// Adds the stores and branches of `block` to `count`: a call as what `called` holds for the
/// function it calls, by the function's index, and as nothing where `called` holds none.
fn counted(block: &naga::Block, called: &[(u32, u32)], count: &mut (u32, u32)) {
    use naga::Statement as S;
    let add = |count: &mut (u32, u32), (stores, branches): (u32, u32)| {
        count.0 = count.0.saturating_add(stores);
        count.1 = count.1.saturating_add(branches);
    };
    for statement in block.iter() {
        match statement {
            S::Store { .. } => add(count, (1, 0)),
            S::Block(inner) => counted(inner, called, count),
            S::If { accept, reject, .. } => {
                add(count, (0, 1));
                counted(accept, called, count);
                counted(reject, called, count);
            }
            S::Switch { cases, .. } => {
                add(count, (0, 1));
                (cases.iter()).for_each(|case| counted(&case.body, called, count));
            }
            S::Loop {
                body, continuing, ..
            } => {
                add(count, (0, 1));
                counted(body, called, count);
                counted(continuing, called, count);
            }
            S::Call { function, .. } => {
                if let Some(&body) = called.get(function.index()) {
                    add(count, body);
                }
            }
            _ => {}
        }
    }
}

/// The most a compute stage's stores times its branches, each call counted as the body it
/// calls, come to where Direct3D 12's software adapter still computes it right: under the
/// largest made-up stage that it did.
// ⚠ Past it that adapter goes wrong while it prepares the stage for its first group (a dispatch
// of none is safe): the made-up stages of `picked_blocks` were read back right up to 121 blocks
// (about 2,300,000), and from 122 written with nothing or with other values, or never finished;
// from 250 the process ends at an access violation or a heap corruption inside the adapter. No
// barrier, local array or call is needed for it, and the variables do not count, but the blocks
// picked by a number do: the same updates run one after another, 400 blocks of them, are right.
// The same stages are right on a hardware adapter, under Direct3D 12 with the same compiler and
// under Vulkan.
const COMPUTE_CHECKED_ON_SOFTWARE: u64 = 2_000_000;

/// The inlined stores and branches of a compute stage past what the software adapter runs right.
pub fn dispatch_past_the_software_adapter(module: &naga::Module) -> Option<(u32, u32)> {
    let (stores, branches) = inlined_stores_and_branches(module);
    let past = u64::from(stores) * u64::from(branches) > COMPUTE_CHECKED_ON_SOFTWARE;
    past.then_some((stores, branches))
}

/// The most a mesh stage's stores times its branches come to where Direct3D 12's software
/// adapter still draws it: the largest that drew of the made-up stages tried.
// ⚠ Past it that adapter runs past a buffer of its own while it prepares the stage for its first
// draw, whatever the stage reads: a made-up stage of 4,000 masked stores and 200 branches drew
// and one of 6,000 and 300 ended the process, as did every mesh stage past the measure that was
// drawn (a hedge's, a trunk's, a batch of leaves'). So no such stage can be drawn there first.
// The fault is that adapter's own: a hardware adapter runs such stages to the right answer.
const CHECKED_ON_SOFTWARE: u64 = 800_000;

/// The same for a pixel stage, most of whose branches are the `switch` that picks a texture.
// A four-layer parallax stage of 13,681 stores and 3,515 branches ended the process after its
// first frame; a three-layer one of 7,746 and 2,152, the largest that draws, is under it.
const PIXEL_CHECKED_ON_SOFTWARE: u64 = 17_000_000;

/// Whether the stages bound here are drawn by a hardware adapter, as the program that draws them said.
// ponytail: one switch a process; stages bound before a device is made anew on another kind of
// adapter are not bound again. Hand the kind to `feed` if a session ever changes adapters.
static DRAWN_BY_HARDWARE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Says whether the adapter that draws is a hardware one: a stage past what the software adapter draws is refused only where it is not.
pub fn drawn_by_hardware(hardware: bool) {
    DRAWN_BY_HARDWARE.store(hardware, std::sync::atomic::Ordering::Relaxed);
}

/// Whether a stage is refused for its size: past what the software adapter draws, where that adapter draws it.
fn unchecked(hardware: bool, mesh: bool, stores: u32, branches: u32) -> bool {
    !hardware && past_the_software_adapter(mesh, stores, branches)
}

/// Whether a stage of so many stores and branches is past what the software adapter draws.
fn past_the_software_adapter(mesh: bool, stores: u32, branches: u32) -> bool {
    let most = match mesh {
        true => CHECKED_ON_SOFTWARE,
        false => PIXEL_CHECKED_ON_SOFTWARE,
    };
    u64::from(stores) * u64::from(branches) > most
}

/// The two buffers the game's compute pass that skins a ped writes each frame, by the hashes of
/// the names a mesh program reads them by: a program named for vertices skinned beforehand takes
/// its vertices from them and reads no bone.
const SKINNED_BEFORE: [u32; 2] = [0x3450_B596, 0x580D_0575];

/// Whether a stage fed `feeds` reads the vertices a compute pass skins, which are nought here: a
/// ped's head and a garment drew nothing by such a stage.
fn reads_skinned_vertices(feeds: &[Fed]) -> bool {
    (feeds.iter()).any(|fed| {
        SKINNED_BEFORE
            .iter()
            .any(|hash| fed.feed == Feed::Zero(Some(*hash)))
    })
}

/// The list of a batch's instances a draw takes, by the hash of its name: a mesh program of a
/// batch effect reads an instance's number there and the instance's record, 64 bytes of which
/// three rows are its matrix, by that number.
// ⚠ Eleven batch effects drawn with the list at nought and the identity for the record showed
// nothing where Material shows the model: what the rest of a record states is not read.
const BATCH: u32 = 0x1AB3_F264;

/// Whether a stage fed `feeds` draws the instances of a batch.
fn draws_a_batch(feeds: &[Fed]) -> bool {
    feeds.iter().any(|fed| fed.feed == Feed::Zero(Some(BATCH)))
}

/// The records an instanced draw places its instances by, by the hash of their name: 128 bytes
/// an instance, which start with the three rows of four floats that place it.
// ⚠ A mesh stage that reads them binds no matrix of the object: with the records at nought every
// corner of every triangle is the origin. A stage that also binds what tints an instance reads
// fields of the record no model opened alone states, and is still refused.
const INSTANCE_RECORDS: u32 = 0xB66F_6950;

/// Whether a stage fed `feeds` places the instances of an instanced draw.
fn places_instances(feeds: &[Fed]) -> bool {
    (feeds.iter()).any(|fed| fed.feed == Feed::InstanceRecords)
}

/// The location a lowered stage gives the high halves of a parameter of 16-bit floats, past
/// that of its low halves.
const HIGH_HALVES: u32 = 32;

/// The slot a pixel program reads each parameter of a mesh program at, parameter `n` at place
/// `n`: a pixel program's slot is a register of the signature the two stages share, and a mesh
/// program numbers from nought the registers that are not system values (the position, a clip
/// distance), in their order.
// So the slot is one past the parameter where the position alone comes first, and two past it
// in a pass that also gives a clip distance.
pub fn parameter_slots(outputs: &[SignatureElement]) -> Vec<u32> {
    let mut slots: Vec<u32> = (outputs.iter())
        .filter(|element| element.system_value == 0)
        .map(|element| element.register)
        .collect();
    slots.sort_unstable();
    slots.dedup();
    slots
}

/// [`parameter_slots`] for a mesh program that writes `written` parameters: one that writes one
/// more, its clip distance, numbers it in the order of the registers too, so each parameter after
/// it is one further on.
// ⚠ A pixel stage that reads the clip distance as an input has the mesh program export it to a
// parameter as well as to the clip; numbered past it, every parameter lands one slot early.
pub fn mesh_slots(outputs: &[SignatureElement], written: usize) -> Vec<u32> {
    let plain = parameter_slots(outputs);
    let mut clipped: Vec<u32> = (outputs.iter())
        .filter(|element| matches!(element.system_value, 0 | CLIP_DISTANCE))
        .map(|element| element.register)
        .collect();
    clipped.sort_unstable();
    clipped.dedup();
    match written > plain.len() && clipped.len() == written {
        true => clipped,
        false => plain,
    }
}

/// The system value a signature numbers the clip distances by.
const CLIP_DISTANCE: u32 = 2;

/// The parameter of a mesh program a pixel program reads at `slot`, by [`parameter_slots`].
pub fn parameter_of(slots: &[u32], slot: u8) -> Option<u8> {
    let at = slots.iter().position(|s| *s == u32::from(slot))?;
    u8::try_from(at).ok()
}

/// Move each parameter a mesh stage gives to the slot its pixel stage reads it at; one past
/// those `slots` lists stays where it is.
fn to_pixel_slots(module: &mut naga::Module, slots: &[u32]) {
    use naga::{Binding, TypeInner};
    let results: Vec<_> = (module.entry_points.iter())
        .filter_map(|entry| entry.function.result.as_ref().map(|result| result.ty))
        .collect();
    for handle in results {
        let mut ty = module.types[handle].clone();
        let TypeInner::Struct { members, .. } = &mut ty.inner else {
            continue;
        };
        let mut moved = false;
        for member in members.iter_mut() {
            if let Some(Binding::Location { location, .. }) = &mut member.binding {
                let high = *location / HIGH_HALVES * HIGH_HALVES;
                if let Some(slot) = slots.get((*location - high) as usize) {
                    // A type put back unchanged is one the module already holds, which it refuses.
                    moved |= *location != high + slot;
                    *location = high + slot;
                }
            }
        }
        if moved {
            module.types.replace(handle, ty);
        }
    }
}

/// The entries of the table of streams at the head of a geometry's vertex data.
pub const STREAM_SLOTS: u32 = 23;

/// The element size of the buffer that holds an object's matrix.
const MATRIX: u32 = 64;
/// The element size of the records a mesh program moves a vertex's texture coordinates by.
const UV_TRANSFORMS: u32 = 96;
/// The element size of the buffer that holds a skinned object's bones.
const BONE: u32 = 48;
/// The element size of the buffer that holds an object's own record.
const INSTANCE: u32 = 288;

/// Whether `resource` lies in the heap, the table the third dword of a draw points at.
pub(super) fn heap_root(resource: &Resource, kind: ShaderKind, records: &[Record]) -> bool {
    let draw = records.iter().find(|r| r.class == Class::RootConstants);
    let (Some(at), Some(draw)) = (resource.at.as_ref(), draw) else {
        return false;
    };
    let root = match numbered(Place::Words { root: at.root }, kind) {
        Place::Words { root } => root,
        _ => at.root,
    };
    at.path.is_empty() && root == draw.root.wrapping_add(2)
}

/// What fills the binding of `resource`, which joins nothing by a place of its own: the heap the
/// third dword of a draw points at, bound whole, or the one entry of it picked by the number at a
/// stated dword of `numbers`, the buffer the second dword of a draw places.
fn heap(
    resource: &Resource,
    kind: ShaderKind,
    records: &[Record],
    numbers: Option<Place>,
) -> Option<Feed> {
    let at = resource.at.as_ref()?;
    let draw = records.iter().find(|r| r.class == Class::RootConstants)?;
    let user = |register: u8| match numbered(Place::Words { root: register }, kind) {
        Place::Words { root } => root,
        _ => register,
    };
    if !at.path.is_empty() || user(at.root) != draw.root.wrapping_add(2) {
        return None;
    }
    match (&resource.kind, resource.count, &at.index) {
        (ResourceKind::Texture { dim, .. }, Some(_), _) => Some(Feed::Heap { dim: *dim }),
        (ResourceKind::Buffer { .. }, Some(_), _) => Some(Feed::HeapBuffers),
        (
            ResourceKind::Texture { .. },
            None,
            Some(Index {
                source:
                    IndexSource::Data {
                        buffer,
                        offset,
                        plus: Some(plus),
                    },
                shift: 5,
            }),
        ) => {
            let table = Place::Table {
                root: buffer.root,
                offset: buffer.offset,
            };
            let placed = user(*plus) == draw.root.wrapping_add(1)
                && Some(numbered(table, kind)) == numbers
                && offset % 4 == 0;
            placed.then_some(Feed::Texture(offset / 4))
        }
        _ => None,
    }
}

/// The rows a lowered stage declares for a constant buffer whose length its code does not state.
const UNSIZED_ROWS: u32 = 4096;
/// The buffers of a stage a device binds unasked, as buffers and as constant buffers.
const STORAGE_BUFFERS: usize = 8;

/// Bind each constant buffer of `module` whose length the code does not state as a buffer read
/// whole: a device cuts a constant buffer at 64 KiB, which is less than a model's meshlet list
/// can be, and binds twelve of them a stage, which is fewer than a skinned mesh program has.
/// How many it made so.
// ponytail: stops at the eight buffers a device binds unasked; a stage with more such tables
// keeps the rest as constant buffers, cut as before.
fn whole_tables(module: &mut naga::Module) -> usize {
    use naga::{AddressSpace, ArraySize, StorageAccess, TypeInner};
    let is_buffer = |space: AddressSpace| matches!(space, AddressSpace::Storage { .. });
    let bound = |g: &naga::GlobalVariable| g.binding.is_some();
    let buffers = (module.global_variables.iter())
        .filter(|(_, g)| bound(g) && is_buffer(g.space))
        .count();
    let tables: Vec<_> = (module.global_variables.iter())
        .filter(|(_, g)| bound(g) && g.space == AddressSpace::Uniform)
        .filter_map(|(handle, g)| match module.types[g.ty].inner {
            TypeInner::Array {
                base,
                size: ArraySize::Constant(rows),
                stride,
            } if rows.get() == UNSIZED_ROWS => Some((handle, base, stride)),
            _ => None,
        })
        .collect();
    let made = tables.len().min(STORAGE_BUFFERS.saturating_sub(buffers));
    for (handle, base, stride) in tables.into_iter().take(made) {
        let whole = module.types.insert(
            naga::Type {
                name: None,
                inner: TypeInner::Array {
                    base,
                    size: ArraySize::Dynamic,
                    stride,
                },
            },
            naga::Span::UNDEFINED,
        );
        let global = module.global_variables.get_mut(handle);
        global.ty = whole;
        global.space = AddressSpace::Storage {
            access: StorageAccess::LOAD,
        };
    }
    made
}

/// Give each sample of `module` at the first level of its texture that level as a number: a
/// sample that only says "the first level" is one naga's HLSL writer writes as a method Direct3D
/// has for a compared sample alone. How many it left as they were.
// ponytail: the number is a nought the function states before the sample, which a lowered
// program does long before its first sample; a function with none keeps the sample as it was,
// and a device of Direct3D then refuses the pass. The lowering stating the level itself is the cure.
fn first_levels_stated(module: &mut naga::Module) -> usize {
    use naga::{Expression as E, Literal, SampleLevel};
    let mut left = 0;
    let functions = (module.functions.iter_mut().map(|(_, f)| f))
        .chain(module.entry_points.iter_mut().map(|e| &mut e.function));
    for function in functions {
        let mut nought = None;
        let mut samples = Vec::new();
        for (handle, expression) in function.expressions.iter() {
            match expression {
                E::Literal(Literal::F32(0.0)) if nought.is_none() => nought = Some(handle),
                E::ImageSample {
                    level: SampleLevel::Zero,
                    depth_ref: None,
                    gather: None,
                    ..
                } => samples.push((handle, nought)),
                _ => {}
            }
        }
        for (handle, nought) in samples {
            match (nought, &mut function.expressions[handle]) {
                (Some(nought), E::ImageSample { level, .. }) => *level = SampleLevel::Exact(nought),
                _ => left += 1,
            }
        }
    }
    left
}

/// Whether the textures `module` binds at (`group`, `binding`), alone or as an array, are
/// arrays themselves, and whether they are rows of one dimension.
fn image_shape(module: &naga::Module, group: u32, binding: u32) -> (bool, bool) {
    use naga::TypeInner;
    let found = module.global_variables.iter().find(|(_, global)| {
        (global.binding.as_ref()).is_some_and(|b| (b.group, b.binding) == (group, binding))
    });
    let inner = found.map(|(_, global)| &module.types[global.ty].inner);
    let image = match inner {
        Some(TypeInner::BindingArray { base, .. }) => Some(&module.types[*base].inner),
        other => other,
    };
    match image {
        Some(TypeInner::Image { dim, arrayed, .. }) => (*arrayed, *dim == naga::ImageDimension::D1),
        _ => (false, false),
    }
}

/// Make the array `module` binds at (`group`, `binding`) its one entry, which every index
/// reads: an array of samplers, each of which the editor binds the same sampler for. So a stage
/// binds no array a device may not have for it. Whether it was one.
// ponytail: an entry picked by a stated number, or an access a statement holds, is left as it
// is, and the device then says what it refuses.
fn one_entry(module: &mut naga::Module, group: u32, binding: u32) -> bool {
    use naga::{Expression as E, TypeInner};
    let found = module.global_variables.iter().find(|(_, global)| {
        (global.binding.as_ref()).is_some_and(|b| (b.group, b.binding) == (group, binding))
    });
    let Some((global, _)) = found else {
        return false;
    };
    let TypeInner::BindingArray { base, .. } =
        module.types[module.global_variables[global].ty].inner
    else {
        return false;
    };
    module.global_variables.get_mut(global).ty = base;
    let functions = (module.functions.iter_mut().map(|(_, f)| f))
        .chain(module.entry_points.iter_mut().map(|e| &mut e.function));
    for function in functions {
        // Each entry the code takes of the array, with the expression that is the array.
        let mut entries = std::collections::HashMap::new();
        for (handle, expression) in function.expressions.iter() {
            if let E::Access { base, .. } = expression {
                if function.expressions[*base] == E::GlobalVariable(global) {
                    entries.insert(handle, *base);
                }
            }
        }
        for (handle, expression) in function.expressions.iter_mut() {
            if entries.contains_key(&handle) {
                // Nothing reads the entry now: it is left as a number, which an emit may hold.
                if let E::Access { index, .. } = *expression {
                    *expression = E::Unary {
                        op: naga::UnaryOperator::BitwiseNot,
                        expr: index,
                    };
                }
                continue;
            }
            if let E::Access { base, .. }
            | E::AccessIndex { base, .. }
            | E::Load { pointer: base }
            | E::ArrayLength(base)
            | E::ImageSample { sampler: base, .. } = expression
            {
                if let Some(array) = entries.get(base) {
                    *base = *array;
                }
            }
        }
    }
    true
}

/// `lowered`, a program of a model's shading pass, with what fills each of its bindings; the
/// pixel stage's bindings are moved to group 1, and each stage's arrays to the group two above
/// its own, since a device binds no array beside a constant buffer. `table` is the program's own variable table and
/// `effect` the first table of the effect that runs it.
// The object's matrix is told by shape: the one buffer of 64-byte elements a mesh program binds
// that the effect's own table names.
pub fn feed(
    native: &Native,
    lowered: Lowered,
    table: &Sgd5Table,
    effect: &Sgd5Table,
) -> Result<Stage, Unfed> {
    feed_noting(native, lowered, table, effect).map(|(stage, _)| stage)
}

/// [`feed`], with what it did to the stage's buffers to hold it to a device with default limits.
pub fn feed_noting(
    native: &Native,
    lowered: Lowered,
    table: &Sgd5Table,
    effect: &Sgd5Table,
) -> Result<(Stage, Fitted), Unfed> {
    fed(native, lowered, table, effect, None)
}

/// [`feed`] for a device that gives a stage `budget`: where the bindings the editor leaves at
/// nought are not enough, buffers of the stage are merged, and a renderer fills each merged
/// buffer from the feeds of its parts, as [`Fitted::merged`] says.
pub fn feed_within(
    native: &Native,
    lowered: Lowered,
    table: &Sgd5Table,
    effect: &Sgd5Table,
    budget: &Budget,
) -> Result<(Stage, Fitted), Unfed> {
    fed(native, lowered, table, effect, Some(budget))
}

/// Why [`feed`] refuses a stage of `native` whose lowering binds `resources`, where its program and
/// those bindings decide it: what the optimiser makes of the module changes none of it.
pub fn refused_by_bindings(
    native: &Native,
    resources: &[Resource],
    table: &Sgd5Table,
    effect: &Sgd5Table,
) -> Option<Unfed> {
    let kind = native.program.kind;
    let mesh = kind == ShaderKind::Mesh;
    let picked = picks(&native.program.words);
    let forward = super::forward::forward_of(&picked.exports).filter(|_| !mesh);
    if kind == ShaderKind::Pixel && !deferred_targets(&picked.exports) && forward.is_none() {
        return Some(Unfed::Targets);
    }
    if let Some(Err(why)) = forward.map(super::forward::Forward::blend) {
        return Some(why);
    }
    let material = match material_textures(&picked, &native.records, kind, effect) {
        Ok(material) => material,
        Err(why) => return Some(why.into()),
    };
    for resource in resources {
        let at = numbered(place(resource), kind);
        let why = match join(at, &native.records, table) {
            Join::Picked => {
                let found = heap(resource, kind, &native.records, material.buffer)
                    .or_else(|| terrain::picked(resource, kind, &native.records, table))
                    .or_else(|| super::ped::picked(resource, kind, &native.records, table))
                    .or_else(|| super::ped::wind_field_maps(resource, kind, &native.records, table))
                    .or_else(|| match forward {
                        Some(super::forward::Forward::Lit) => {
                            super::forward::picked(resource, kind, &native.records, table)
                        }
                        _ => None,
                    });
                picked_feed(found, unguarded()).err()
            }
            Join::NoRecord { root, offset } => Some(Unbound::NoRecord { root, offset }),
            Join::NoName { record } => Some(Unbound::NoName {
                register: record.register,
            }),
            _ => None,
        };
        if let Some(why) = why {
            return Some(why.into());
        }
    }
    None
}

/// The stage and what was done to its buffers: held by its noughts to a device with default
/// limits, or with `merging` to that budget by merges too.
fn fed(
    native: &Native,
    lowered: Lowered,
    table: &Sgd5Table,
    effect: &Sgd5Table,
    merging: Option<&Budget>,
) -> Result<(Stage, Fitted), Unfed> {
    // First, so that a caller that asked it before the optimiser ran is told the same.
    if let Some(why) = refused_by_bindings(native, &lowered.resources, table, effect) {
        return Err(why);
    }
    let kind = native.program.kind;
    let mesh = kind == ShaderKind::Mesh;
    let stage_group = u32::from(kind == ShaderKind::Pixel);
    let picked = picks(&native.program.words);
    let forward = super::forward::forward_of(&picked.exports).filter(|_| !mesh);
    if kind == ShaderKind::Pixel && !deferred_targets(&picked.exports) && forward.is_none() {
        return Err(Unfed::Targets);
    }
    if let Some(forward) = forward {
        forward.blend()?;
    }
    let material = material_textures(&picked, &native.records, kind, effect)?;
    let filled = sources(&native.records, mesh, table, effect);
    let layered = terrain::layered(&filled);
    let layouts = constant_buffers(table, &|hash, _| Some(stated(hash, Some(effect))));
    let layout = |named: &Sgd5Resource| {
        let name = stated(named.name_hash, Some(effect));
        (layouts.iter().map(|(_, layout)| layout))
            .find(|layout| layout.name == name)
            .cloned()
            .unwrap_or(ConstantBuffer {
                name,
                size: named.buffer_size,
                variables: Vec::new(),
            })
    };
    let Lowered {
        mut module,
        resources,
        approximations,
        ..
    } = lowered;
    let mut feeds = Vec::with_capacity(resources.len());
    // The bindings the editor leaves at nought.
    let mut noughts = Vec::new();
    // The bindings the stage writes, whose words no loop is judged by.
    let mut written = BTreeSet::new();
    let mut rows = Vec::new();
    // The bindings a heap lowered to plain textures takes: past every one the stage has.
    let mut free = resources.iter().map(|r| r.binding + 1).max().unwrap_or(0);
    // Before the heap is lowered: a sample through an entry of an array of samplers is one the
    // lowering does not take.
    let mut flat = BTreeSet::new();
    for resource in &resources {
        let at = (resource.group, resource.binding);
        let samplers = matches!(resource.kind, ResourceKind::Sampler { .. });
        if samplers && resource.count.is_some() && one_entry(&mut module, at.0, at.1) {
            flat.insert(at);
        }
    }
    for resource in &resources {
        let at = numbered(place(resource), kind);
        let mut header = None;
        let feed = match join(at, &native.records, table) {
            Join::Sampler => Feed::Sampler,
            Join::UserData => Feed::Zero(None),
            // The heap's words are the editor's to state. Another table's are descriptors a
            // program loads into registers on the way to the binding the module reads in their
            // place: noughts, so a length or a size read of one as a number is nought too.
            Join::Words => match heap_root(resource, kind, &native.records) {
                true => Feed::HeapWords,
                false => Feed::Zero(None),
            },
            Join::Picked => {
                let found = heap(resource, kind, &native.records, material.buffer)
                    .or_else(|| terrain::picked(resource, kind, &native.records, table))
                    .or_else(|| super::ped::picked(resource, kind, &native.records, table))
                    .or_else(|| super::ped::wind_field_maps(resource, kind, &native.records, table))
                    .or_else(|| match forward {
                        Some(super::forward::Forward::Lit) => {
                            super::forward::picked(resource, kind, &native.records, table)
                        }
                        _ => None,
                    });
                picked_feed(found, unguarded())?
            }
            Join::NoRecord { root, offset } => {
                return Err(Unbound::NoRecord { root, offset }.into())
            }
            Join::NoName { record } => {
                let register = record.register;
                return Err(Unbound::NoName { register }.into());
            }
            Join::Named { record, named } => {
                if matches!(resource.kind, ResourceKind::Buffer { stride: None }) {
                    header = Some(record.stride);
                }
                let source = (native.records.iter())
                    .position(|r| *r == record)
                    .and_then(|i| filled.get(i).copied());
                match source {
                    _ if material.buffer == Some(at) => Feed::Numbers,
                    _ if material.record == Some(at) => Feed::Listed(material.listed.clone()),
                    Some(Source::VertexData) => Feed::VertexData,
                    Some(Source::Meshlets) => Feed::Meshlets,
                    Some(Source::View) => Feed::View(layout(named)),
                    Some(Source::Material) => Feed::Material(layout(named)),
                    Some(Source::Sampler) => Feed::Sampler,
                    // Before the matrix told by shape: a batch's records are of its size too.
                    Some(Source::Game(hash)) if super::trees::stated(hash).is_some() => {
                        super::trees::stated(hash).unwrap_or(Feed::Zero(Some(hash)))
                    }
                    // A forward pass is handed its own records ([`super::forward::at_rest`]).
                    Some(Source::Game(super::forward_frame::VIEW_LIGHTING))
                        if forward.is_none() =>
                    {
                        super::trees::view_at_rest()
                    }
                    Some(Source::Game(super::trees::FRAME_RECORD)) if forward.is_none() => {
                        super::trees::frame_at_rest()
                    }
                    // The object's matrix, and a skinned program's bones: each element of a
                    // buffer of this size is a matrix, whichever the program takes it for.
                    Some(Source::Game(_))
                        if mesh && record.class == Class::Srv && record.stride == MATRIX =>
                    {
                        Feed::Placement
                    }
                    // A forward pixel program moves its own coordinates by the same records.
                    Some(Source::Game(_))
                        if (mesh || forward.is_some())
                            && record.class == Class::Srv
                            && record.stride == UV_TRANSFORMS =>
                    {
                        Feed::UvTransforms
                    }
                    // Told by shape: a mesh program's buffer of that element size, read as a
                    // buffer and not as a table of records.
                    Some(Source::Game(_))
                        if mesh
                            && record.class == Class::Srv
                            && record.stride == BONE
                            && matches!(resource.kind, ResourceKind::Buffer { stride: None }) =>
                    {
                        Feed::Bones
                    }
                    // Told by shape, as the matrix is: the one size of element, named by the effect.
                    Some(Source::Game(hash))
                        if record.class == Class::Srv
                            && record.stride == INSTANCE
                            && (effect.resources.iter()).any(|r| r.name_hash == hash) =>
                    {
                        Feed::Instance
                    }
                    Some(Source::Game(INSTANCE_RECORDS))
                        if mesh && record.stride == super::instances::BYTES =>
                    {
                        Feed::InstanceRecords
                    }
                    // Its dword 8 decides whether the pixel program samples what it picks.
                    Some(Source::Game(super::ped::COMPONENT))
                        if kind == ShaderKind::Pixel && record.class == Class::Cbv =>
                    {
                        Feed::PedComponent
                    }
                    Some(Source::Game(hash)) => {
                        terrain::buffer(hash).unwrap_or(Feed::Zero(Some(hash)))
                    }
                    Some(Source::DrawWords | Source::Unnamed) | None => Feed::Zero(None),
                }
            }
        };
        // Before the noughts are listed: a binding given the game's rest is no nought to merge.
        let feed = match forward {
            Some(forward) => super::forward::at_rest(forward, feed)?,
            None => feed,
        };
        if let Feed::Heap { .. } = feed {
            let (array, row) = image_shape(&module, resource.group, resource.binding);
            let numbers: Vec<u32> = (material.textures.iter())
                .filter(|(_, texture)| (texture.code == TEXTURE_ARRAY) == array)
                .map(|(number, _)| *number)
                .collect();
            if row {
                rows.extend(&numbers);
            }
            // Entry nought, then each number's: number `i` picks entry `i + 1`; then the
            // terrain's defaults, which a layer's numbers pick.
            let mut defaults = terrain::heap(layered && !array);
            let tiles = filled.contains(&Source::Game(super::trees::WORLD_TILES));
            defaults.extend(super::trees::tile_defaults(tiles && !array, &numbers));
            let entries: Vec<u32> = (std::iter::once(0))
                .chain(numbers.iter().map(|n| n + 1))
                .chain(defaults.iter().map(|(entry, _)| *entry))
                .collect();
            let at = (resource.group, resource.binding);
            let layers = terrain::layer_arrays(layered && !array);
            let ranges: Vec<(u32, u32)> = layers.iter().map(|(range, _)| *range).collect();
            if plain_textures(&mut module, at, &entries, &ranges, stage_group, free).is_ok() {
                let fed = std::iter::once(Feed::Blank)
                    .chain(numbers.into_iter().map(Feed::Texture))
                    .chain(defaults.into_iter().map(|(_, feed)| feed))
                    .chain(layers.into_iter().map(|(_, feed)| feed));
                for (binding, feed) in (free..).zip(fed) {
                    feeds.push(Fed {
                        group: stage_group,
                        binding,
                        feed,
                        header: None,
                    });
                }
                free += (entries.len() + ranges.len()) as u32;
                continue;
            }
        }
        // A table's buffers are one buffer, and an array made its one entry is none.
        let flat = feed == Feed::HeapBuffers || flat.contains(&(resource.group, resource.binding));
        let group = stage_group + 2 * u32::from(resource.count.is_some() && !flat);
        for (_, global) in module.global_variables.iter_mut() {
            let here = (global.binding.as_ref())
                .is_some_and(|b| (b.group, b.binding) == (resource.group, resource.binding));
            if here {
                global.binding = Some(naga::ResourceBinding {
                    group,
                    binding: resource.binding,
                });
            }
        }
        if matches!(feed, Feed::Zero(_)) {
            noughts.push((group, resource.binding));
        }
        if resource.written {
            written.insert((group, resource.binding));
        }
        feeds.push(Fed {
            group,
            binding: resource.binding,
            feed,
            header,
        });
    }
    terrain::held(&mut module, &feeds)?;
    if forward == Some(super::forward::Forward::Lit) {
        if let Some(why) = super::forward::unestablished(&module, &feeds) {
            return Err(why);
        }
    }
    let (stores, branches) = stores_and_branches(&module);
    let hardware = DRAWN_BY_HARDWARE.load(std::sync::atomic::Ordering::Relaxed);
    if !unguarded() && unchecked(hardware, mesh, stores, branches) {
        return Err(Unfed::Unchecked(stores, branches));
    }
    let ends = never_ending(&module, &feeds, &written, &approximations)?;
    if let Some(name) = crate::rdna::refused_for_drawing(&approximations) {
        return Err(Unfed::Approximated(name));
    }
    // ⚠ No backend checks an entry of a bound array: Direct3D 12 and Vulkan read whatever lies
    // past its end, or fault.
    if !crate::rdna::bound_entries_checked(&module) {
        return Err(Unfed::Entry);
    }
    if reads_skinned_vertices(&feeds) {
        return Err(Unfed::SkinnedBefore);
    }
    if mesh && draws_a_batch(&feeds) {
        return Err(Unfed::Batch);
    }
    if mesh && places_instances(&feeds) && super::instances::reads_unsettled(&feeds) {
        return Err(Unfed::Instanced);
    }
    if mesh {
        let written = crate::rdna::link::written(&module);
        to_pixel_slots(&mut module, &mesh_slots(&native.outputs, written));
    }
    whole_tables(&mut module);
    first_levels_stated(&mut module);
    let mut streams = BTreeSet::new();
    let read = reads(&module);
    for fed in feeds.iter().filter(|fed| fed.feed == Feed::VertexData) {
        let places = read.get(&(fed.group, fed.binding)).into_iter().flatten();
        streams.extend(places.filter_map(|place| place.parse::<u32>().ok()));
    }
    streams.retain(|slot| *slot < STREAM_SLOTS);
    // After the tables are bound whole, which takes constant buffers away, and after the
    // streams are read off the bindings, which a merge moves.
    let fitted = match merging {
        Some(budget) => budget::within(&mut module, &noughts, budget),
        None => budget::noughts_within(&mut module, &noughts, &Budget::DEFAULT),
    };
    // Last: every loop of the module drawn held to its cap.
    let caps: Vec<u32> = (ends.iter())
        .filter_map(|ends| super::watchdog::cap(*ends))
        .collect();
    let watched = super::watchdog::watch(&mut module, &caps, kind == ShaderKind::Pixel)
        .map_err(|_| Unfed::NotShown(caps.len() as u32))?;
    tracing::debug!(
        loops = watched.loops,
        expressions = watched.expressions,
        "a GTA VI stage's loops are held to their caps"
    );
    let stage = Stage {
        module: std::sync::Arc::new(module),
        feeds,
        streams,
        rows,
        approximations,
        textures: (material.textures.iter())
            .map(|(number, texture)| (*number, texture.semantic_hash))
            .collect(),
    };
    Ok((stage, fitted))
}

/// `module` as WGSL, for a person to read what a program does with a binding.
pub fn wgsl(module: &naga::Module) -> Result<String, String> {
    use naga::valid::{Capabilities, ValidationFlags, Validator};
    if super::walk_file::holds_ray_query(module) {
        return Err(super::walk_file::NO_WGSL_OF_A_QUERY.to_owned());
    }
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(module)
        .map_err(|e| format!("{e:?}"))?;
    naga::back::wgsl::write_string(module, &info, naga::back::wgsl::WriterFlags::empty())
        .map_err(|e| e.to_string())
}

/// The number `index` states: a literal, or a sum of them.
fn stated_number(function: &naga::Function, index: naga::Handle<naga::Expression>) -> Option<u32> {
    use naga::Expression as E;
    match function.expressions[index] {
        E::Literal(naga::Literal::U32(n)) => Some(n),
        E::Literal(naga::Literal::I32(n)) => u32::try_from(n).ok(),
        E::Binary {
            op: naga::BinaryOperator::Add,
            left,
            right,
        } => stated_number(function, left)?.checked_add(stated_number(function, right)?),
        _ => None,
    }
}

/// What `module` reads of each binding: the places its code takes of the global, a number for a
/// stated index and `*` for a computed one, by (group, binding).
pub fn reads(module: &naga::Module) -> std::collections::BTreeMap<(u32, u32), BTreeSet<String>> {
    use naga::Expression as E;
    let mut out = std::collections::BTreeMap::new();
    let functions = (module.functions.iter().map(|(_, f)| f))
        .chain(module.entry_points.iter().map(|e| &e.function));
    for function in functions {
        let base_of = |e: &E| match e {
            E::Access { base, .. } | E::AccessIndex { base, .. } => Some(*base),
            _ => None,
        };
        let bases: BTreeSet<_> = (function.expressions.iter())
            .filter_map(|(_, e)| base_of(e))
            .collect();
        for (handle, _) in function.expressions.iter() {
            if bases.contains(&handle) {
                continue;
            }
            let (mut at, mut path) = (handle, Vec::new());
            let global = loop {
                match &function.expressions[at] {
                    E::AccessIndex { base, index } => {
                        path.push(index.to_string());
                        at = *base;
                    }
                    E::Access { base, index } => {
                        path.push(match stated_number(function, *index) {
                            Some(n) => n.to_string(),
                            None => "*".to_string(),
                        });
                        at = *base;
                    }
                    E::GlobalVariable(global) => break Some(*global),
                    _ => break None,
                }
            };
            let binding = global.and_then(|g| module.global_variables[g].binding.as_ref());
            if let Some(binding) = binding {
                path.reverse();
                let reads: &mut BTreeSet<String> =
                    out.entry((binding.group, binding.binding)).or_default();
                reads.insert(path.join("."));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use or_io::gta5::shaders::sgd5::{Sgd5Constant, Sgd5Resource};

    use super::*;

    const VIEW: u32 = 0xA000_0001;
    const MATERIAL: u32 = 0xA000_0002;
    const LIST: u32 = MESHLET_LIST;
    const FILLED: u32 = 0xA000_0004;
    const STREAMS: u32 = 0xA000_0005;
    const OTHER: u32 = 0xA000_0006;

    #[test]
    fn a_stage_whose_lowering_counts_a_loop_not_shown_to_end_is_refused() {
        let module = naga::Module::default();
        let counted = |name| BTreeMap::from([(name, 2u32)]);
        for name in [crate::rdna::LOOP_OTHER_LANES, crate::rdna::LOOP_UNCHANGING] {
            let why = never_ending(&module, &[], &BTreeSet::new(), &counted(name));
            assert_eq!(why, Err(Unfed::NotShown(2)));
        }
        let other = counted("a count over the wave's lanes read as this lane's");
        assert_eq!(
            never_ending(&module, &[], &BTreeSet::new(), &other),
            Ok(Vec::new())
        );
    }

    /// A stage of `stores` stores, an `if` around every `every` of them.
    fn branchy(stores: usize, every: usize) -> naga::Module {
        let mut body = String::new();
        for at in 0..stores {
            let open = at % every == 0;
            body += if open {
                "if x > 0.5 { x = x + 1.0; }
"
            } else {
                "x = x * 2.0;
"
            };
        }
        let text = format!(
            "@fragment fn main() -> @location(0) vec4<f32> {{
var x = 0.0;
{body}return vec4<f32>(x);
}}"
        );
        naga::front::wgsl::parse_str(&text).expect("the WGSL parses")
    }

    #[test]
    fn a_picked_binding_the_heap_does_not_give_is_noughts_only_under_the_aid() {
        assert_eq!(picked_feed(Some(Feed::Blank), false), Ok(Feed::Blank));
        assert_eq!(picked_feed(Some(Feed::Blank), true), Ok(Feed::Blank));
        assert_eq!(picked_feed(None, true), Ok(Feed::Zero(None)));
        assert_eq!(picked_feed(None, false), Err(Unbound::Picked));
    }

    #[test]
    fn a_stage_is_measured_by_its_stores_and_its_branches() {
        // The variable's own first value is no store; a loop and a switch are branches too.
        assert_eq!(stores_and_branches(&branchy(40, 10)), (40, 4));
        let text = "@fragment fn main() -> @location(0) vec4<f32> {
    var x = 0.0;
    loop { x = x + 1.0; if x > 3.0 { break; } }
    switch u32(x) { case 1u { x = 0.5; } default { x = 0.25; } }
    return vec4<f32>(x);
}";
        let module = naga::front::wgsl::parse_str(text).expect("the WGSL parses");
        assert_eq!(stores_and_branches(&module), (3, 3));
    }

    #[test]
    fn a_stage_past_what_the_software_adapter_draws_is_told_by_stores_times_branches() {
        // 4,000 stores and 200 branches drew there; 4,020 and 201 are the first past it.
        assert!(!past_the_software_adapter(true, 4000, 200));
        assert!(past_the_software_adapter(true, 4020, 201));
        // Many stores with few branches, and many branches over few stores, are not past it.
        assert!(!past_the_software_adapter(true, 9000, 60));
        assert!(!past_the_software_adapter(true, 1800, 360));
        // A pixel stage branches far more for its size: one that draws, and one that does not.
        assert!(!past_the_software_adapter(false, 7746, 2152));
        assert!(past_the_software_adapter(false, 13681, 3515));
        // The numbers are multiplied past what 32 bits hold.
        assert!(past_the_software_adapter(false, u32::MAX, u32::MAX));
        // The refusal is the software adapter's: a hardware adapter is handed the same stage.
        assert!(unchecked(false, true, 4020, 201) && !unchecked(true, true, 4020, 201));
        assert!(!unchecked(false, true, 4000, 200));
    }

    #[test]
    fn a_call_is_counted_as_the_body_it_calls_where_calls_are_inlined() {
        let text = "fn twice(x: u32) -> u32 {
    var y = x;
    if y > 3u { y = y + 1u; }
    y = y * 2u;
    return y;
}
fn four(x: u32) -> u32 { return twice(twice(x)); }
@compute @workgroup_size(1) fn main() {
    var z = four(1u);
    z = twice(z);
}";
        let module = naga::front::wgsl::parse_str(text).expect("the WGSL parses");
        // A first value that is not a constant is a store.
        assert_eq!(stores_and_branches(&module), (5, 1));
        // `twice` three times over, through `four` and alone, and `main`'s own two stores.
        assert_eq!(inlined_stores_and_branches(&module), (11, 3));
    }

    /// A made-up compute stage of `blocks` blocks that a loop picks by a number, as a dispatch
    /// loop does: each block ten updates of a register under a test of another, a store, and the
    /// next block chosen by a register.
    fn picked_blocks(blocks: u32) -> naga::Module {
        let registers = 10;
        let mut text = String::from(
            "@group(0) @binding(0) var<storage, read_write> data: array<u32>;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_id) thread: vec3<u32>) {
",
        );
        for r in 0..registers {
            text += &format!("var r{r} = data[{r}u * 64u + thread.x];\n");
        }
        text += "var next = 0u;\nloop {\nlet at = next;\n";
        for k in 0..blocks {
            text += &format!("if at == {k}u {{\n");
            for s in 0..10 {
                let (d, a, b) = (
                    (k * 10 + s) % registers,
                    (k * 7 + s * 3 + 1) % registers,
                    (k * 13 + s * 5 + 2) % registers,
                );
                text += &format!(
                    "if (r{a} & {}u) != 0u {{ r{d} = r{a} + (r{b} ^ {}u); }}\n",
                    1 << s,
                    k * 10 + s
                );
            }
            text += &format!(
                "data[{registers}u * 64u + {k}u * 64u + thread.x] = r{};\n",
                k * 11 % registers
            );
            text += &format!(
                "if (r{} & 1u) != 0u {{ next = {}u; }} else {{ next = {}u; }}\n}}\n",
                k * 5 % registers,
                k + 2,
                k + 1
            );
        }
        text += &format!("if at >= {blocks}u {{ break; }}\n}}\n}}\n");
        naga::front::wgsl::parse_str(&text).expect("the WGSL parses")
    }

    #[test]
    fn a_compute_stage_past_what_the_software_adapter_runs_is_told_by_its_inlined_size() {
        // Read back right there at 100 blocks, and wrong at 125.
        assert_eq!(
            dispatch_past_the_software_adapter(&picked_blocks(100)),
            None
        );
        let wrong = picked_blocks(125);
        let past = dispatch_past_the_software_adapter(&wrong);
        assert_eq!(past, Some(inlined_stores_and_branches(&wrong)));
    }

    #[test]
    fn a_stage_that_reads_vertices_skinned_beforehand_is_told_by_either_buffer_and_no_other() {
        let fed = |feed| Fed {
            group: 0,
            binding: 3,
            feed,
            header: None,
        };
        for hash in SKINNED_BEFORE {
            let feeds = [fed(Feed::Bones), fed(Feed::Zero(Some(hash)))];
            assert!(reads_skinned_vertices(&feeds));
        }
        let skins_itself = [fed(Feed::Zero(Some(OTHER))), fed(Feed::Bones)];
        assert!(!reads_skinned_vertices(&skins_itself));
    }

    #[test]
    fn a_stage_that_draws_a_batch_is_told_by_the_list_of_its_instances() {
        let fed = |feed| Fed {
            group: 0,
            binding: 8,
            feed,
            header: Some(4),
        };
        assert!(draws_a_batch(&[
            fed(Feed::Placement),
            fed(Feed::Zero(Some(BATCH)))
        ]));
        // The record of an object drawn alone, and another buffer of the game's, are not it.
        assert!(!draws_a_batch(&[
            fed(Feed::Placement),
            fed(Feed::Zero(Some(OTHER)))
        ]));
    }

    #[test]
    fn a_stage_that_places_instances_is_told_by_their_records() {
        let fed = |feed| Fed {
            group: 0,
            binding: 7,
            feed,
            header: None,
        };
        assert!(places_instances(&[
            fed(Feed::Meshlets),
            fed(Feed::InstanceRecords)
        ]));
        // An object's own record and a batch's list are other draws.
        assert!(!places_instances(&[
            fed(Feed::Instance),
            fed(Feed::Zero(Some(BATCH)))
        ]));
    }

    /// A mesh program that binds nothing a record names.
    fn bare_mesh() -> Native {
        Native {
            program: crate::rdna::Program {
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
        }
    }

    #[test]
    fn a_stage_is_fed_where_each_computed_entry_of_a_bound_array_is_held_to_its_length() {
        let fed = |entry: &str| {
            let text = format!(
                "@group(2) @binding(0) var heap: binding_array<texture_2d<f32>, 256>;
@group(0) @binding(1) var<uniform> number: u32;
@vertex fn main() -> @builtin(position) vec4<f32> {{
    return textureLoad(heap[{entry}], vec2<i32>(0), 0);
}}"
            );
            let lowered = Lowered {
                module: naga::front::wgsl::parse_str(&text).expect("the WGSL parses"),
                resources: Vec::new(),
                approximations: Default::default(),
                half_slots: Default::default(),
                subgroups: false,
                intersections: Vec::new(),
                region: None,
            };
            feed(&bare_mesh(), lowered, &program(), &Sgd5Table::default()).map(|_| ())
        };
        // The last entry stands for any past the array, as the lowering holds one.
        assert_eq!(fed("min(number / 8u, 255u)"), Ok(()));
        assert_eq!(fed("3"), Ok(()));
        assert_eq!(fed("number / 8u"), Err(Unfed::Entry));
        assert_eq!(fed("min(number, 256u)"), Err(Unfed::Entry));
    }

    #[test]
    fn a_stage_translated_with_a_meeting_of_invocations_left_out_is_not_drawn() {
        let fed = |approximations: BTreeMap<&'static str, u32>| {
            let text = "@vertex fn main() -> @builtin(position) vec4<f32> {
    return vec4<f32>(0.0);
}";
            let lowered = Lowered {
                module: naga::front::wgsl::parse_str(text).expect("the WGSL parses"),
                resources: Vec::new(),
                approximations,
                half_slots: Default::default(),
                subgroups: false,
                intersections: Vec::new(),
                region: None,
            };
            feed(&bare_mesh(), lowered, &program(), &Sgd5Table::default()).map(|_| ())
        };
        assert_eq!(fed(BTreeMap::new()), Ok(()));
        assert_eq!(
            fed(BTreeMap::from([("an image loaded as floats", 1)])),
            Ok(())
        );
        for name in crate::rdna::NOT_DRAWN {
            let counted = BTreeMap::from([(name, 1)]);
            assert_eq!(fed(counted), Err(Unfed::Approximated(name)), "{name}");
        }
    }

    fn resource(code: u8, slot: u16, name_hash: u32) -> Sgd5Resource {
        Sgd5Resource {
            code,
            flags: 0,
            slot,
            members: 0,
            buffer_size: 64,
            name_hash,
            semantic_hash: name_hash,
        }
    }

    fn record(class: Class, register: u32, stride: u32) -> Record {
        Record {
            class,
            register,
            root: 9,
            offset: 32 * register,
            dwords: 4,
            stride,
        }
    }

    fn program() -> Sgd5Table {
        Sgd5Table {
            resources: vec![
                resource(0x30, 0, VIEW),
                resource(0x30, 4, MATERIAL),
                resource(0x07, 9, LIST),
                resource(0x05, 12, FILLED),
                resource(0x07, 13, STREAMS),
                resource(0x07, 14, OTHER),
            ],
            constants: vec![Sgd5Constant {
                code: 0x0E,
                count: 0,
                offset: 352,
                default_offset: 0,
                default: None,
                name_hash: string_hash_str("gViewProj"),
                semantic_hash: 0,
                cbuffer: Some(VIEW),
            }],
            ..Sgd5Table::default()
        }
    }

    #[test]
    fn a_binding_is_filled_by_the_model_the_view_the_material_or_the_game() {
        let effect = Sgd5Table {
            resources: vec![
                resource(0x30, 0, VIEW),
                resource(0x30, 260, MATERIAL),
                resource(0x07, 73, LIST),
            ],
            ..Sgd5Table::default()
        };
        let records = [
            record(Class::Cbv, 0, 0),
            record(Class::Cbv, 4, 0),
            record(Class::Srv, 9, 1),
            record(Class::Srv, 12, 48),
            record(Class::Address, 13, 1),
            record(Class::RootConstants, 500, 0),
            record(Class::Sampler, 3, 0),
            record(Class::Srv, 77, 4),
            // A buffer of bytes the effect's own table does not name.
            record(Class::Srv, 14, 1),
        ];
        assert_eq!(
            sources(&records, true, &program(), &effect),
            [
                Source::View,
                Source::Material,
                Source::Meshlets,
                Source::Game(FILLED),
                Source::VertexData,
                Source::DrawWords,
                Source::Sampler,
                Source::Unnamed,
                Source::Game(OTHER),
            ]
        );
        // A pixel program fetches no geometry: the same records are the game's.
        let pixel = sources(&records, false, &program(), &effect);
        assert_eq!(
            (pixel[2], pixel[4]),
            (Source::Game(LIST), Source::Game(STREAMS))
        );
        assert_eq!(pixel[..2], [Source::View, Source::Material]);
        assert!(Source::Meshlets.held() && Source::Material.held());
        assert!(!Source::Game(FILLED).held() && !Source::DrawWords.held());
    }

    #[test]
    fn an_array_of_buffers_a_draw_reads_none_of_becomes_one_buffer() {
        let text = "
struct Words { words: array<u32> }
@group(0) @binding(4) var<storage> lists: binding_array<Words, 256>;
@group(0) @binding(5) var<storage> other: Words;
@fragment fn main(@location(0) @interpolate(flat) at: u32) -> @location(0) vec4<f32> {
    let count = arrayLength(&lists[at].words);
    return vec4<f32>(f32(lists[at / 8u].words[2] + other.words[1] + count));
}";
        let mut module = naga::front::wgsl::parse_str(text).unwrap();
        assert!(one_entry(&mut module, 0, 4));
        let written = wgsl(&module).unwrap();
        assert!(!written.contains("binding_array"), "{written}");
        assert!(written.contains("lists.words[2"), "{written}");
        // A binding that is no array, and a place nothing is bound at, are left.
        let before = format!("{module:?}");
        assert!(!one_entry(&mut module, 0, 5) && !one_entry(&mut module, 0, 9));
        assert_eq!(format!("{module:?}"), before);
    }

    #[test]
    fn a_sample_at_the_first_level_states_the_level_for_a_writer_that_needs_it() {
        let text = "
@group(1) @binding(2) var s: sampler;
@group(1) @binding(3) var map: texture_2d<f32>;
@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let scaled = uv * vec2<f32>(0.0, 1.0);
    return textureSampleLevel(map, s, scaled, 7.0);
}";
        let mut module = naga::front::wgsl::parse_str(text).unwrap();
        // As a lowered program has it: the first level, with no number for it.
        let function = &mut module.entry_points[0].function;
        let sample = (function.expressions.iter())
            .find(|(_, e)| matches!(e, naga::Expression::ImageSample { .. }))
            .map(|(handle, _)| handle)
            .unwrap();
        let naga::Expression::ImageSample { level, .. } = &mut function.expressions[sample] else {
            unreachable!()
        };
        *level = naga::SampleLevel::Zero;
        assert_eq!(first_levels_stated(&mut module), 0);
        let function = &module.entry_points[0].function;
        let naga::Expression::ImageSample { level, .. } = &function.expressions[sample] else {
            unreachable!()
        };
        let naga::SampleLevel::Exact(stated) = *level else {
            panic!("the level is not stated: {level:?}");
        };
        assert!(stated < sample);
        let nought = naga::Expression::Literal(naga::Literal::F32(0.0));
        assert_eq!(function.expressions[stated], nought);
        // Direct3D's language has the sample now.
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let mut hlsl = String::new();
        let options = naga::back::hlsl::Options::default();
        let pipeline = naga::back::hlsl::PipelineOptions::default();
        naga::back::hlsl::Writer::new(&mut hlsl, &options, &pipeline)
            .write(&module, &info, None)
            .unwrap();
        assert!(
            hlsl.contains(".SampleLevel(") && !hlsl.contains("LevelZero"),
            "{hlsl}"
        );
    }

    #[test]
    fn an_array_of_samplers_becomes_the_one_sampler_every_entry_is() {
        let text = "
@group(1) @binding(2) var samplers: binding_array<sampler, 256>;
@group(1) @binding(3) var map: texture_2d<f32>;
@fragment fn main(@location(0) @interpolate(flat) at: u32) -> @location(0) vec4<f32> {
    return textureSample(map, samplers[at], vec2<f32>(0.5));
}";
        let mut module = naga::front::wgsl::parse_str(text).unwrap();
        assert!(one_entry(&mut module, 1, 2));
        let written = wgsl(&module).unwrap();
        assert!(!written.contains("binding_array"), "{written}");
        assert!(
            written.contains("textureSample(map, samplers,"),
            "{written}"
        );
    }

    #[test]
    fn a_table_of_no_stated_length_is_bound_whole_and_the_ninth_buffer_is_left() {
        let text = "
@group(0) @binding(0) var<uniform> table: array<vec4<u32>, 4096>;
@group(0) @binding(1) var<uniform> sized: array<vec4<u32>, 6>;
@group(0) @binding(2) var<storage> words: array<u32>;
@fragment fn main() -> @location(0) vec4<f32> {
    return vec4<f32>(f32(table[5000].x + sized[2].y + words[9]));
}";
        let mut module = naga::front::wgsl::parse_str(text).unwrap();
        assert_eq!(whole_tables(&mut module), 1);
        let written = wgsl(&module).unwrap();
        assert!(
            written.contains("var<storage> table: array<vec4<u32>>;"),
            "{written}"
        );
        assert!(
            written.contains("var<uniform> sized: array<vec4<u32>, 6>;"),
            "{written}"
        );
        // Eight buffers already: a ninth table stays a constant buffer.
        let many: String = (0..8)
            .map(|i| format!("@group(0) @binding({i}) var<storage> b{i}: array<u32>;\n"))
            .collect();
        let full = format!(
            "{many}@group(0) @binding(9) var<uniform> table: array<vec4<u32>, 4096>;
@fragment fn main() -> @location(0) vec4<f32> {{ return vec4<f32>(f32(table[1].x)); }}"
        );
        let mut module = naga::front::wgsl::parse_str(&full).unwrap();
        assert_eq!(whole_tables(&mut module), 0);
    }

    fn element(semantic: &str, register: u32, system_value: u32) -> SignatureElement {
        SignatureElement {
            semantic: semantic.to_owned(),
            index: 0,
            system_value,
            component_type: 3,
            register,
            mask: 15,
        }
    }

    #[test]
    fn a_mesh_parameter_goes_to_the_register_its_pixel_stage_reads_past_the_system_values() {
        // A made-up pass: a clip distance and the position first, two elements sharing a register.
        let outputs = [
            element("SV_ClipDistance", 0, 2),
            element("SV_Position", 1, 1),
            element("TEXCOORD", 3, 0),
            element("COLOR", 2, 0),
            element("TEXCOORD", 3, 0),
            element("TEXCOORD", 5, 0),
        ];
        let slots = parameter_slots(&outputs);
        assert_eq!(slots, [2, 3, 5]);
        assert_eq!(parameter_of(&slots, 3), Some(1));
        assert_eq!(
            parameter_of(&slots, 1),
            None,
            "the position is no parameter"
        );
        // With the position alone before them, a slot is one past its parameter.
        let plain = [element("SV_Position", 0, 1), element("TEXCOORD", 1, 0)];
        assert_eq!(parameter_slots(&plain), [1]);
        // A mesh program that writes its clip distance as a parameter too numbers it in register
        // order: its first texture coordinate is its second parameter.
        let clipped = [
            element("SV_Position", 0, 1),
            element("SV_ClipDistance", 1, 2),
            element("TEXCOORD", 2, 0),
            element("TEXCOORD", 3, 0),
        ];
        assert_eq!(
            mesh_slots(&clipped, 2),
            [2, 3],
            "it writes no clip parameter"
        );
        assert_eq!(mesh_slots(&clipped, 3), [1, 2, 3], "it writes one");
        assert_eq!(mesh_slots(&plain, 1), [1]);

        let mut module = naga::Module::default();
        let vec4 = module.types.insert(
            naga::Type {
                name: None,
                inner: naga::TypeInner::Vector {
                    size: naga::VectorSize::Quad,
                    scalar: naga::Scalar::F32,
                },
            },
            naga::Span::UNDEFINED,
        );
        let member = |location: u32| naga::StructMember {
            name: None,
            ty: vec4,
            binding: Some(naga::Binding::Location {
                location,
                interpolation: None,
                sampling: None,
                blend_src: None,
                per_primitive: false,
            }),
            offset: 0,
        };
        let given = module.types.insert(
            naga::Type {
                name: None,
                inner: naga::TypeInner::Struct {
                    members: vec![member(0), member(1), member(HIGH_HALVES + 1), member(2)],
                    span: 64,
                },
            },
            naga::Span::UNDEFINED,
        );
        module.entry_points.push(naga::EntryPoint {
            name: "main".to_owned(),
            stage: naga::ShaderStage::Vertex,
            early_depth_test: None,
            workgroup_size: [0; 3],
            workgroup_size_overrides: None,
            function: naga::Function {
                result: Some(naga::FunctionResult {
                    ty: given,
                    binding: None,
                }),
                ..Default::default()
            },
            mesh_info: None,
            task_payload: None,
            incoming_ray_payload: None,
        });
        to_pixel_slots(&mut module, &slots);
        let result = module.entry_points[0].function.result.as_ref().unwrap().ty;
        let naga::TypeInner::Struct { members, .. } = &module.types[result].inner else {
            panic!("the result is no struct");
        };
        let locations: Vec<u32> = (members.iter())
            .filter_map(|m| match m.binding {
                Some(naga::Binding::Location { location, .. }) => Some(location),
                _ => None,
            })
            .collect();
        assert_eq!(locations, [2, 3, HIGH_HALVES + 3, 5]);
        // Parameters already at their slots: the module holds the type that would be put back.
        let before = format!("{module:?}");
        to_pixel_slots(&mut module, &[0, 1, 2, 3, 4, 5]);
        assert_eq!(format!("{module:?}"), before);
    }

    #[test]
    fn the_meshlet_list_is_told_by_its_name_among_buffers_of_bytes_and_a_low_register_is_no_material(
    ) {
        let effect = Sgd5Table {
            resources: vec![
                resource(0x30, 4, MATERIAL),
                resource(0x07, 73, LIST),
                resource(0x07, 74, OTHER),
            ],
            ..Sgd5Table::default()
        };
        let records = [
            record(Class::Srv, 9, 1),
            record(Class::Srv, 14, 1),
            record(Class::Cbv, 4, 0),
        ];
        // A ped's skinned program binds its shape data so, beside the list.
        assert_eq!(
            sources(&records, true, &program(), &effect),
            [
                Source::Meshlets,
                Source::Game(OTHER),
                Source::Game(MATERIAL)
            ]
        );
        // The one buffer of bytes a program binds is no list by that alone.
        assert_eq!(
            sources(&records[1..], true, &program(), &effect),
            [Source::Game(OTHER), Source::Game(MATERIAL)]
        );
    }

    #[test]
    fn a_mesh_stage_is_fed_its_model_the_view_and_the_one_matrix_its_shape_tells() {
        use crate::rdna::{Loc, Program};
        let effect = Sgd5Table {
            resources: vec![
                resource(0x30, 0, VIEW),
                resource(0x07, 73, LIST),
                resource(0x05, 74, FILLED),
            ],
            ..Sgd5Table::default()
        };
        let mut matrix = record(Class::Srv, 12, 64);
        matrix.root = 9;
        let mut list = record(Class::Srv, 9, 1);
        list.root = 9;
        let mut view = record(Class::Cbv, 0, 0);
        view.root = 9;
        let mut streams = record(Class::Address, 13, 1);
        streams.root = 11;
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
            records: vec![list, matrix, view, streams],
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        // A mesh program reaches the table of user register 9 through scalar register 17.
        let at = |root: u8, offset: u32| {
            Some(Loc {
                root,
                path: Vec::new(),
                offset,
                index: None,
            })
        };
        let bound = |kind, at, binding| Resource {
            kind,
            at,
            words: None,
            group: 0,
            binding,
            count: None,
            written: false,
        };
        let resources = vec![
            bound(
                ResourceKind::Buffer { stride: None },
                at(17, list.offset),
                0,
            ),
            bound(ResourceKind::Buffer { stride: Some(4) }, at(19, 0), 1),
            bound(
                ResourceKind::ConstantBuffer { rows: 4 },
                at(17, matrix.offset),
                2,
            ),
            bound(
                ResourceKind::ConstantBuffer { rows: 30 },
                at(17, view.offset),
                3,
            ),
            bound(ResourceKind::UserData, None, 4),
        ];
        // The view's constant buffer and the user data, as a lowering declares them.
        let mut module = naga::Module::default();
        let row = module.types.insert(
            naga::Type {
                name: None,
                inner: naga::TypeInner::Vector {
                    size: naga::VectorSize::Quad,
                    scalar: naga::Scalar::U32,
                },
            },
            naga::Span::UNDEFINED,
        );
        // Eleven buffers no resource names, each larger than the user data, then the two.
        let matrix = module.types.insert(
            naga::Type {
                name: None,
                inner: naga::TypeInner::Matrix {
                    columns: naga::VectorSize::Quad,
                    rows: naga::VectorSize::Quad,
                    scalar: naga::Scalar::F32,
                },
            },
            naga::Span::UNDEFINED,
        );
        for binding in (5..5 + Budget::DEFAULT.uniform - 1).chain([3, 4]) {
            let ty = if binding > 4 { matrix } else { row };
            module.global_variables.append(
                naga::GlobalVariable {
                    name: None,
                    space: naga::AddressSpace::Uniform,
                    binding: Some(naga::ResourceBinding { group: 0, binding }),
                    ty,
                    init: None,
                    memory_decorations: Default::default(),
                },
                naga::Span::UNDEFINED,
            );
        }
        let lowered = |resources: Vec<Resource>| Lowered {
            module: module.clone(),
            resources,
            approximations: Default::default(),
            half_slots: Default::default(),
            subgroups: false,
            intersections: Vec::new(),
            region: None,
        };
        let stage = feed(&native, lowered(resources.clone()), &program(), &effect).unwrap();
        const CONSTANT_BUFFERS: usize = Budget::DEFAULT.uniform as usize;
        let fed: Vec<_> = stage.feeds.iter().map(|f| (&f.feed, f.header)).collect();
        assert_eq!(fed[0], (&Feed::Meshlets, Some(1)));
        assert_eq!(fed[1], (&Feed::VertexData, None));
        assert_eq!(fed[2], (&Feed::Placement, None));
        let Feed::View(layout) = fed[3].0 else {
            panic!("the view is not told: {:?}", fed[3]);
        };
        assert!(layout.variables.iter().any(|v| v.name == "gViewProj"));
        assert_eq!(fed[4], (&Feed::Zero(None), None));
        // Thirteen constant buffers: the one the editor leaves at nought binds nothing, and
        // the stage's own noughts stand for it; the twelve others stay bound.
        let globals: Vec<_> = (stage.module.global_variables.iter())
            .map(|(_, global)| (global.space, global.binding.as_ref().map(|b| b.binding)))
            .collect();
        assert_eq!(globals.len(), CONSTANT_BUFFERS + 1);
        assert_eq!(
            globals[CONSTANT_BUFFERS],
            (naga::AddressSpace::Private, None)
        );
        let bound = &globals[..CONSTANT_BUFFERS];
        assert!(bound
            .iter()
            .all(|g| g.0 == naga::AddressSpace::Uniform && g.1.is_some()));
        // With twelve, none is over and the one at nought stays bound.
        let mut twelve = module.clone();
        let first = twelve.global_variables.iter().next().unwrap().0;
        twelve.global_variables.get_mut(first).space = naga::AddressSpace::Private;
        let before = format!("{twelve:?}");
        budget::noughts_within(&mut twelve, &[(0, 4)], &Budget::DEFAULT);
        assert_eq!(format!("{twelve:?}"), before);
        assert!(stage.textures.is_empty());

        assert!(stage.streams.is_empty());

        // With the records of an instanced draw where its matrix was, the stage is fed them.
        let mut instanced = native.clone();
        instanced.records[1].stride = super::super::instances::BYTES;
        let mut named = program();
        named.resources[3].name_hash = INSTANCE_RECORDS;
        let placed = feed(&instanced, lowered(resources.clone()), &named, &effect).unwrap();
        assert_eq!(placed.feeds[2].feed, Feed::InstanceRecords);
        // One that also binds what tints an instance reads fields no model states: refused.
        named.resources[2].name_hash = 0x8E78_E380;
        let refused = feed(&instanced, lowered(resources.clone()), &named, &effect).map(|_| ());
        assert_eq!(refused, Err(Unfed::Instanced));
        // For a device that gives a stage five constant buffers, with each declared as rows, as
        // a lowering declares them: the one at nought is the stage's own first, and the eight
        // smallest of the others become one buffer, the view's among them.
        let mut rows = naga::Module::default();
        let row = rows
            .types
            .insert(module.types[row].clone(), naga::Span::UNDEFINED);
        for binding in (5..5 + Budget::DEFAULT.uniform - 1).chain([3, 4]) {
            let ty = rows.types.insert(
                naga::Type {
                    name: None,
                    inner: naga::TypeInner::Array {
                        base: row,
                        size: naga::ArraySize::Constant(
                            std::num::NonZeroU32::new(binding + 1).unwrap(),
                        ),
                        stride: 16,
                    },
                },
                naga::Span::UNDEFINED,
            );
            rows.global_variables.append(
                naga::GlobalVariable {
                    name: None,
                    space: naga::AddressSpace::Uniform,
                    binding: Some(naga::ResourceBinding { group: 0, binding }),
                    ty,
                    init: None,
                    memory_decorations: Default::default(),
                },
                naga::Span::UNDEFINED,
            );
        }
        let lowered = Lowered {
            module: rows,
            resources,
            approximations: Default::default(),
            half_slots: Default::default(),
            subgroups: false,
            intersections: Vec::new(),
            region: None,
        };
        let five = Budget {
            uniform: 5,
            ..Budget::DEFAULT
        };
        let (stage, fitted) = feed_within(&native, lowered, &program(), &effect, &five).unwrap();
        assert_eq!(fitted.own, [(0, 4)]);
        assert!(five.holds(budget::Bound::of(&stage.module)));
        let [merged] = fitted.merged.as_slice() else {
            panic!("one buffer holds the merged ones: {:?}", fitted.merged);
        };
        assert_eq!((merged.at, merged.parts.len()), ((0, 3), 8));
        let part = |was, offset, bytes| budget::Part { was, offset, bytes };
        assert_eq!(
            merged.parts[..2],
            [part((0, 3), 0, 64), part((0, 5), 64, 96)]
        );
        // A part is still fed as its own binding says: the view's is the view.
        let view = &stage.feeds[3];
        assert!(matches!(view.feed, Feed::View(_)));
        assert_eq!((view.group, view.binding), (0, 3));
    }
}
