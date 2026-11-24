//! Where each binding of a pass that shades a GTA VI model gets its contents: the model's own
//! buffers, the view, the material, or what only the game fills when it runs.

use std::collections::{BTreeMap, BTreeSet};

use or_io::gta5::shaders::sgd5::{Sgd5Resource, Sgd5Table};
use or_io::gta5::shaders::sgd5_pack::constant_buffers;
use or_io::gta5::shaders::spirv_pack::{ConstantBuffer, SignatureElement};
use or_io::hash::string_hash_str;

use super::loops;
use super::picks::{deferred_targets, material_textures, picks, Unsettled, TEXTURE_ARRAY};
use super::plain::plain_textures;
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

/// What fills each of `records`, the bindings of one program of a shading pass, in their order;
/// `mesh` where it is the pass's mesh program, `table` its own variable table and `effect` the
/// first table of the effect that runs it.
// A mesh program's meshlet list is told by its shape: the one buffer of bytes the program binds
// that its effect's own table names. With two such buffers neither is taken for it.
pub fn sources(
    records: &[Record],
    mesh: bool,
    table: &Sgd5Table,
    effect: &Sgd5Table,
) -> Vec<Source> {
    let own = |hash: u32| effect.resources.iter().find(|r| r.name_hash == hash);
    let raw_own = |record: &Record| {
        record.class == Class::Srv
            && record.stride == RAW
            && named(table, record).is_some_and(|n| own(n.name_hash).is_some())
    };
    let one_raw = mesh && records.iter().filter(|r| raw_own(r)).count() == 1;
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
                Class::Srv if one_raw && raw_own(record) => Source::Meshlets,
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
    #[error("it picks an entry of an array of textures or buffers by a number it computes, which no device checks")]
    Entry,
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
/// a number of theirs: none yet.
const AT_REST: &[u32] = &[];

/// Whether the developer's aid `unguarded` of `OPENRAGE_GTA6_SHADED` is given: a count to a number
/// only the game fills is then let through, for a renderer that draws it on a software adapter alone.
fn unguarded() -> bool {
    std::env::var("OPENRAGE_GTA6_SHADED")
        .is_ok_and(|aids| aids.split(',').any(|word| word.trim() == "unguarded"))
}

/// Why a stage that loops is not drawn, where one of its loops may never end.
fn never_ending(
    module: &naga::Module,
    feeds: &[Fed],
    approximations: &BTreeMap<&'static str, u32>,
) -> Option<Unfed> {
    // The lowering's own count comes first: it sees the program's graph, which the module no longer shows.
    let not_shown: u32 = (approximations.iter())
        .filter(|(name, _)| name.starts_with(crate::rdna::LOOP_NOT_SHOWN))
        .map(|(_, loops)| loops)
        .sum();
    if not_shown > 0 {
        return Some(Unfed::NotShown(not_shown));
    }
    if loops::loops(module) == 0 {
        return None;
    }
    let game: Vec<((u32, u32), u32)> = (feeds.iter())
        .filter_map(|fed| match fed.feed {
            Feed::Zero(Some(hash)) if !AT_REST.contains(&hash) => {
                Some(((fed.group, fed.binding), hash))
            }
            _ => None,
        })
        .collect();
    if unguarded() {
        return None;
    }
    loops::counted_to(module, &game).map(Unfed::Loop)
}

/// Whether `module` takes an entry of an array it binds by a number it computes, or by a stated
/// one past the array's end.
// ⚠ No backend checks that number: Direct3D 12 and Vulkan read whatever lies there, or fault.
fn picks_an_unchecked_entry(module: &naga::Module) -> bool {
    use naga::{ArraySize, Expression as E, TypeInner};
    let entries = |global: naga::Handle<naga::GlobalVariable>| match module.types
        [module.global_variables[global].ty]
        .inner
    {
        TypeInner::BindingArray { size, .. } => Some(match size {
            ArraySize::Constant(entries) => entries.get(),
            _ => 0,
        }),
        _ => None,
    };
    let functions = (module.functions.iter().map(|(_, function)| function))
        .chain(module.entry_points.iter().map(|entry| &entry.function));
    for function in functions {
        let array = |base: naga::Handle<E>| match function.expressions[base] {
            E::GlobalVariable(global) => entries(global),
            _ => None,
        };
        for (_, expression) in function.expressions.iter() {
            let unchecked = match *expression {
                E::Access { base, .. } => array(base).is_some(),
                E::AccessIndex { base, index } => array(base).is_some_and(|held| index >= held),
                _ => false,
            };
            if unchecked {
                return true;
            }
        }
    }
    false
}

/// The constant buffers a device with default limits gives a stage.
const CONSTANT_BUFFERS: usize = 12;
/// The most bytes of a constant buffer left at nought that a stage holds as its own variable.
// ⚠ Each thread of a stage has its own copy: with one of 64 KiB, the size a buffer read at a
// computed place is declared with, a Vulkan device was lost at the first draw.
const OWN_NOUGHTS: u32 = 4096;

/// Keep `module` to the constant buffers a device with default limits gives a stage: while it
/// binds more, the smallest it binds at one of `noughts`, the bindings the editor leaves at
/// nought, becomes the stage's own variable of noughts, which binds nothing and reads the same.
// Only as many as it takes: a driver may fold what a program computes from noughts it can see,
// a division by one say, otherwise than the device computes it from a buffer.
fn within_constant_buffers(module: &mut naga::Module, noughts: &[(u32, u32)]) {
    use naga::AddressSpace;
    let bound = (module.global_variables.iter())
        .filter(|(_, global)| global.space == AddressSpace::Uniform && global.binding.is_some())
        .count();
    let mut own: Vec<_> = (module.global_variables.iter())
        .filter(|(_, global)| global.space == AddressSpace::Uniform)
        .filter(|(_, global)| {
            (global.binding.as_ref()).is_some_and(|b| noughts.contains(&(b.group, b.binding)))
        })
        .map(|(handle, global)| (module.types[global.ty].inner.size(module.to_ctx()), handle))
        .filter(|(size, _)| *size <= OWN_NOUGHTS)
        .collect();
    own.sort();
    for (_, handle) in own.into_iter().take(bound.saturating_sub(CONSTANT_BUFFERS)) {
        let global = module.global_variables.get_mut(handle);
        global.space = AddressSpace::Private;
        global.binding = None;
    }
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
fn heap_root(resource: &Resource, kind: ShaderKind, records: &[Record]) -> bool {
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
    let kind = native.program.kind;
    let mesh = kind == ShaderKind::Mesh;
    let stage_group = u32::from(kind == ShaderKind::Pixel);
    let picked = picks(&native.program.words);
    if kind == ShaderKind::Pixel && !deferred_targets(&picked.exports) {
        return Err(Unfed::Targets);
    }
    let material = material_textures(&picked, &native.records, kind, effect)?;
    let filled = sources(&native.records, mesh, table, effect);
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
                heap(resource, kind, &native.records, material.buffer).ok_or(Unbound::Picked)?
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
                    // The object's matrix, and a skinned program's bones: each element of a
                    // buffer of this size is a matrix, whichever the program takes it for.
                    Some(Source::Game(_))
                        if mesh && record.class == Class::Srv && record.stride == MATRIX =>
                    {
                        Feed::Placement
                    }
                    Some(Source::Game(_))
                        if mesh && record.class == Class::Srv && record.stride == UV_TRANSFORMS =>
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
                    Some(Source::Game(hash)) => Feed::Zero(Some(hash)),
                    Some(Source::DrawWords | Source::Unnamed) | None => Feed::Zero(None),
                }
            }
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
            // Entry nought, then each number's: number `i` picks entry `i + 1`.
            let entries: Vec<u32> = (std::iter::once(0))
                .chain(numbers.iter().map(|n| n + 1))
                .collect();
            let at = (resource.group, resource.binding);
            if plain_textures(&mut module, at, &entries, stage_group, free).is_ok() {
                let fed =
                    std::iter::once(Feed::Blank).chain(numbers.into_iter().map(Feed::Texture));
                for (binding, feed) in (free..).zip(fed) {
                    feeds.push(Fed {
                        group: stage_group,
                        binding,
                        feed,
                        header: None,
                    });
                }
                free += entries.len() as u32;
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
        feeds.push(Fed {
            group,
            binding: resource.binding,
            feed,
            header,
        });
    }
    if let Some(why) = never_ending(&module, &feeds, &approximations) {
        return Err(why);
    }
    if picks_an_unchecked_entry(&module) {
        return Err(Unfed::Entry);
    }
    if mesh {
        to_pixel_slots(&mut module, &parameter_slots(&native.outputs));
    }
    whole_tables(&mut module);
    first_levels_stated(&mut module);
    // After the tables are bound whole, which takes constant buffers away.
    within_constant_buffers(&mut module, &noughts);
    let mut streams = BTreeSet::new();
    let read = reads(&module);
    for fed in feeds.iter().filter(|fed| fed.feed == Feed::VertexData) {
        let places = read.get(&(fed.group, fed.binding)).into_iter().flatten();
        streams.extend(places.filter_map(|place| place.parse::<u32>().ok()));
    }
    streams.retain(|slot| *slot < STREAM_SLOTS);
    Ok(Stage {
        module: std::sync::Arc::new(module),
        feeds,
        streams,
        rows,
        approximations,
        textures: (material.textures.iter())
            .map(|(number, texture)| (*number, texture.semantic_hash))
            .collect(),
    })
}

/// `module` as WGSL, for a person to read what a program does with a binding.
pub fn wgsl(module: &naga::Module) -> Result<String, String> {
    use naga::valid::{Capabilities, ValidationFlags, Validator};
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
    const LIST: u32 = 0xA000_0003;
    const FILLED: u32 = 0xA000_0004;
    const STREAMS: u32 = 0xA000_0005;
    const OTHER: u32 = 0xA000_0006;

    #[test]
    fn a_stage_whose_lowering_counts_a_loop_not_shown_to_end_is_refused() {
        let module = naga::Module::default();
        let counted = |name| BTreeMap::from([(name, 2u32)]);
        for name in [crate::rdna::LOOP_OTHER_LANES, crate::rdna::LOOP_UNCHANGING] {
            let why = never_ending(&module, &[], &counted(name));
            assert_eq!(why, Some(Unfed::NotShown(2)));
        }
        let other = counted("a count over the wave's lanes read as this lane's");
        assert_eq!(never_ending(&module, &[], &other), None);
    }

    #[test]
    fn an_entry_of_a_bound_array_taken_by_a_computed_number_or_past_its_end_is_told() {
        let stage = |entry: &str| {
            let text = format!(
                "@group(3) @binding(0) var heap: binding_array<texture_2d<f32>, 4>;
@group(1) @binding(1) var<uniform> number: u32;
@fragment fn main() -> @location(0) vec4<f32> {{
    return textureLoad(heap[{entry}], vec2<i32>(0), 0);
}}"
            );
            naga::front::wgsl::parse_str(&text).expect("the WGSL parses")
        };
        assert!(picks_an_unchecked_entry(&stage("number")));
        assert!(picks_an_unchecked_entry(&stage("number / 8u")));
        assert!(!picks_an_unchecked_entry(&stage("3")));
        // A stated entry past the end is one the reader of WGSL refuses, so it is made by hand.
        let mut past = stage("3");
        for (_, expression) in past.entry_points[0].function.expressions.iter_mut() {
            if let naga::Expression::AccessIndex { index, .. } = expression {
                *index = 4;
            }
        }
        assert!(picks_an_unchecked_entry(&past));
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
    fn two_own_buffers_of_bytes_name_no_meshlet_list_and_a_low_register_no_material() {
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
        assert_eq!(
            sources(&records, true, &program(), &effect),
            [
                Source::Game(LIST),
                Source::Game(OTHER),
                Source::Game(MATERIAL)
            ]
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
        for binding in (5..5 + CONSTANT_BUFFERS as u32 - 1).chain([3, 4]) {
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
        };
        let stage = feed(&native, lowered(resources.clone()), &program(), &effect).unwrap();
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
        within_constant_buffers(&mut twelve, &[(0, 4)]);
        assert_eq!(format!("{twelve:?}"), before);
        assert!(stage.textures.is_empty());

        assert!(stage.streams.is_empty());
    }
}
