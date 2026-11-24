//! How a GTA VI native program's resources are bound: the records its package lists its bindings
//! by, each resource of a translated module joined to its record and to the name the effect
//! container's table gives that register, and the module named and placed for the pack path.

pub mod kept;

use std::collections::BTreeSet;

use or_io::gta5::shaders::sgd5::{Sgd5Resource, Sgd5Table};
use or_io::gta5::shaders::sgd5_pack::constant_buffers;
use or_io::gta5::shaders::spirv_pack::{
    Binding, BindingClass, ConstantBuffer, Fidelity, Reflection, ShaderStageKind, SignatureElement,
};
use or_io::gta5::shaders::xdat;

use crate::dxil::container::{Container, ContainerError};
use crate::dxil::plan::hash_name;
use crate::dxil::{ShaderKind, Translated};
use crate::rdna::{self, Lowered, Options, Program, RdnaError, Resource, ResourceKind};

/// Where a package states its binding list: the list's offset from [`PACKAGE_BASE`], then its count.
const LIST_AT: usize = 0xF0;
const LIST_COUNT: usize = 0xFC;
const PACKAGE_BASE: usize = 0x10;
/// What a package holds in place of an offset where it has no list.
const NO_LIST: u64 = 0xDEAD_BEEF_DEAD_BEEF;
/// Five dwords: the type, the table offset in dwords, the register, the element size, and the
/// descriptor's dwords with its user register in the next byte.
const RECORD: usize = 20;
/// The package slot of the block a vertex, a pixel and a mesh program draw with.
const VERTEX_SLOT: usize = 4;
const PIXEL_SLOT: usize = 5;
const MESH_SLOT: usize = 8;
/// The effect container's resource code of a constant buffer, and of each unordered-access kind;
/// bit 7 of a code adds 256 to its record's register.
const CONSTANT_BUFFER: u8 = 0x30;
const UNORDERED: [u8; 5] = [0x14, 0x16, 0x18, 0x1C, 0x5C];
const HIGH_REGISTER: u8 = 0x80;
/// A mesh program's code reaches the table of the binding records' user register `r` through
/// scalar register `r + 8`; a vertex or a pixel program's through register `r`.
const MESH_USER_SHIFT: u8 = 8;

/// What a binding record binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// A shader resource in a table: a texture or a buffer.
    Srv,
    Cbv,
    Uav,
    /// A sampler, which no table of the program's holds.
    Sampler,
    /// A buffer whose address two user registers hold.
    Address,
    /// Dwords the pipeline loads into user registers.
    RootConstants,
}

impl Class {
    fn of(word: u32) -> Option<Self> {
        Some(match word {
            0x300 => Self::Srv,
            0x302 => Self::Cbv,
            0x303 => Self::Uav,
            0x401 => Self::Sampler,
            0x100 => Self::Address,
            2 => Self::RootConstants,
            _ => return None,
        })
    }

    /// Whether a table holds its descriptor.
    fn in_a_table(self) -> bool {
        matches!(self, Self::Srv | Self::Cbv | Self::Uav)
    }

    /// Whether the effect container's resource code `code` is of this class.
    fn takes(self, code: u8) -> bool {
        let code = code & !HIGH_REGISTER;
        match self {
            Self::Cbv | Self::RootConstants => code == CONSTANT_BUFFER,
            Self::Uav => UNORDERED.contains(&code),
            Self::Srv | Self::Address => code != CONSTANT_BUFFER && !UNORDERED.contains(&code),
            Self::Sampler => false,
        }
    }
}

/// One binding a program's package lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    pub class: Class,
    /// The register its `PSV0` binding and the effect container's table state.
    pub register: u32,
    /// The user register that holds its table's pointer, or its address or first dword.
    pub root: u8,
    /// Where its descriptor starts in its table, in bytes.
    pub offset: u32,
    /// Its descriptor's dwords: 8 for an image, 4 for a buffer.
    pub dwords: u8,
    /// A buffer's element size in bytes; 4 for an image.
    pub stride: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecordError {
    #[error("the package ends before the words that state its binding list")]
    Short,
    #[error("the package states {count} binding records at +{at:#X}, past its end")]
    Outside { at: u64, count: u32 },
    #[error("binding record {index} is of type {word:#X}, which is none this reads")]
    Type { index: usize, word: u32 },
}

fn dword(bytes: &[u8], at: usize) -> Option<u32> {
    let word = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
}

/// The bindings `package` lists, in its order; none where it states no list.
pub fn records(package: &[u8]) -> Result<Vec<Record>, RecordError> {
    let stated = (
        dword(package, LIST_AT),
        dword(package, LIST_AT + 4),
        dword(package, LIST_COUNT),
    );
    let (Some(low), Some(high), Some(count)) = stated else {
        return Err(RecordError::Short);
    };
    let at = u64::from(low) | (u64::from(high) << 32);
    if at == NO_LIST {
        return Ok(Vec::new());
    }
    let list = usize::try_from(at)
        .ok()
        .and_then(|at| at.checked_add(PACKAGE_BASE))
        .and_then(|start| Some(start..start.checked_add((count as usize).checked_mul(RECORD)?)?))
        .and_then(|range| package.get(range))
        .ok_or(RecordError::Outside { at, count })?;
    (list.as_chunks::<RECORD>().0.iter().enumerate())
        .map(|(index, record)| {
            let word = |i: usize| dword(record, 4 * i).unwrap_or(0);
            let class = Class::of(word(0)).ok_or(RecordError::Type {
                index,
                word: word(0),
            })?;
            Ok(Record {
                class,
                register: word(2),
                root: (word(4) >> 8) as u8,
                // A dword offset past a quarter of the address space is no table's.
                offset: word(1).saturating_mul(4),
                dwords: word(4) as u8,
                stride: word(3),
            })
        })
        .collect()
}

/// Where a translated module's resource comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// A descriptor at a constant byte offset of the table a user register points at.
    Table {
        root: u8,
        offset: u32,
    },
    /// A buffer at the address the user registers `root` and `root + 1` hold.
    Address {
        root: u8,
    },
    Sampler,
    /// The user registers read as numbers.
    UserData,
    /// The dwords of the table a user register points at, read as numbers: the sizes and lengths
    /// its descriptors state.
    Words {
        root: u8,
    },
    /// A descriptor picked by a number the game sets, or reached through a pointer.
    Picked,
}

pub fn place(resource: &Resource) -> Place {
    match (&resource.kind, &resource.at) {
        (ResourceKind::UserData, _) => Place::UserData,
        (ResourceKind::Sampler { .. }, _) => Place::Sampler,
        (ResourceKind::TableWords, Some(at)) if at.path.is_empty() => {
            Place::Words { root: at.root }
        }
        (ResourceKind::Buffer { stride: Some(_) }, Some(at)) => Place::Address { root: at.root },
        (_, Some(at)) if at.path.is_empty() && at.index.is_none() => Place::Table {
            root: at.root,
            offset: at.offset,
        },
        _ => Place::Picked,
    }
}

/// `place` with its user register numbered as the binding records of a `kind` program number it.
// A register below the shift is no user register of the program's: it wraps to one no record states.
pub fn numbered(place: Place, kind: ShaderKind) -> Place {
    let shift = match kind {
        ShaderKind::Mesh => MESH_USER_SHIFT,
        _ => 0,
    };
    match place {
        Place::Table { root, offset } => Place::Table {
            root: root.wrapping_sub(shift),
            offset,
        },
        Place::Address { root } => Place::Address {
            root: root.wrapping_sub(shift),
        },
        Place::Words { root } => Place::Words {
            root: root.wrapping_sub(shift),
        },
        Place::Sampler | Place::UserData | Place::Picked => place,
    }
}

/// What a resource joins to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Join<'t> {
    /// The one record at its place, and the one record of the program's table of that class and register.
    Named {
        record: Record,
        named: &'t Sgd5Resource,
    },
    Sampler,
    UserData,
    Words,
    Picked,
    /// No record, or more than one, is at its place.
    NoRecord {
        root: u8,
        offset: u32,
    },
    /// No record of the table, or more than one, is of its record's class and register.
    NoName {
        record: Record,
    },
}

impl Join<'_> {
    /// Whether the resource is tied to a parameter, or needs none.
    pub fn joins(&self) -> bool {
        matches!(self, Self::Named { .. } | Self::Sampler)
    }
}

fn only<T>(mut found: impl Iterator<Item = T>) -> Option<T> {
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// The record of `table` a binding record names: the one of its class whose slot is its register.
pub fn named<'t>(table: &'t Sgd5Table, record: &Record) -> Option<&'t Sgd5Resource> {
    let register =
        |r: &Sgd5Resource| u32::from(r.slot) + 256 * u32::from(r.code & HIGH_REGISTER != 0);
    only(
        (table.resources.iter())
            .filter(|r| record.class.takes(r.code) && register(r) == record.register),
    )
}

/// A resource at `place` against the program's binding `records` and its own variable `table`.
pub fn join<'t>(place: Place, records: &[Record], table: &'t Sgd5Table) -> Join<'t> {
    let (root, offset, found) = match place {
        Place::Sampler => return Join::Sampler,
        Place::UserData => return Join::UserData,
        Place::Words { .. } => return Join::Words,
        Place::Picked => return Join::Picked,
        Place::Table { root, offset } => (
            root,
            offset,
            only(
                (records.iter())
                    .filter(|r| r.class.in_a_table() && (r.root, r.offset) == (root, offset)),
            ),
        ),
        Place::Address { root } => (
            root,
            0,
            only((records.iter()).filter(|r| r.class == Class::Address && r.root == root)),
        ),
    };
    match found {
        None => Join::NoRecord { root, offset },
        Some(record) => match named(table, record) {
            Some(named) => Join::Named {
                record: *record,
                named,
            },
            None => Join::NoName { record: *record },
        },
    }
}

/// Why a native program is not handed to the renderer; [`Unbound::name`] is the word a census counts by.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unbound {
    #[error(transparent)]
    Container(#[from] ContainerError),
    #[error("its package's binding list does not read: {0}")]
    Records(#[from] RecordError),
    #[error("it holds no block a {0:?} program draws with")]
    Stage(ShaderKind),
    #[error(transparent)]
    NotTranslated(#[from] RdnaError),
    #[error("naga's validator refuses the module: {0}")]
    Invalid(String),
    #[error("its vertex program takes no vertex attribute: it fetches its vertices from buffers the game fills when it runs")]
    NoVertexInput,
    #[error("it reads numbers the game passes with each draw")]
    UserData,
    #[error("it reads its descriptors' own words, an image's size or a buffer's length, which the game's heap states")]
    Words,
    #[error("it reads the buffer {0}, which the game fills when it runs")]
    EngineBuffer(String),
    #[error("it picks a resource by a number the game sets, or reaches it through a pointer")]
    Picked,
    #[error("the descriptor at +{offset:#X} of the table in user register {root} is in no binding record of its package")]
    NoRecord { root: u8, offset: u32 },
    #[error(
        "no one record of its effect container's table names register {register} of its class"
    )]
    NoName { register: u32 },
    #[error("its pixel program reads parameters as 16-bit halves, which its vertex program is not translated for")]
    Halves,
    #[error("it binds, but alone: its outputs and the next stage's inputs are made to agree only where a pass is translated as one")]
    Alone,
}

impl Unbound {
    /// `no-vertex-input`, `engine-buffer`, `not-translated:control flow` …
    pub fn name(&self) -> String {
        match self {
            Self::Container(_) => "container".into(),
            Self::Records(_) => "records".into(),
            Self::Stage(_) => "stage".into(),
            Self::NotTranslated(RdnaError::NotTranslated(why)) => {
                format!("not-translated:{}", why.name())
            }
            Self::NotTranslated(RdnaError::Cfg(_)) => "not-translated:graph".into(),
            Self::NotTranslated(RdnaError::Container(_)) => "container".into(),
            Self::Invalid(_) => "invalid".into(),
            Self::NoVertexInput => "no-vertex-input".into(),
            Self::UserData => "user-data".into(),
            Self::Words => "descriptor-words".into(),
            Self::EngineBuffer(_) => "engine-buffer".into(),
            Self::Picked => "picked".into(),
            Self::NoRecord { .. } => "no-record".into(),
            Self::NoName { .. } => "no-name".into(),
            Self::Halves => "halves".into(),
            Self::Alone => "alone".into(),
        }
    }
}

/// One program of a container ready to translate: its code, its binding records and its signatures.
#[derive(Debug, Clone)]
pub struct Native {
    pub program: Program,
    pub records: Vec<Record>,
    pub inputs: Vec<SignatureElement>,
    pub outputs: Vec<SignatureElement>,
}

/// The block of `container` its stage draws with; a stage other than vertex, pixel and mesh is refused.
pub fn native(container: &[u8]) -> Result<Native, Unbound> {
    let parts = Container::read(container)?;
    let part = (parts.part(b"DXIL")).ok_or(ContainerError::NoPart("DXIL"))?;
    let stages = xdat::read(part).map_err(ContainerError::from)?.stages;
    let programs = rdna::unwrap(container)?;
    let kind = programs.first().map_or(ShaderKind::Other(0), |p| p.kind);
    let slot = match kind {
        ShaderKind::Vertex => VERTEX_SLOT,
        ShaderKind::Pixel => PIXEL_SLOT,
        ShaderKind::Mesh => MESH_SLOT,
        _ => return Err(Unbound::Stage(kind)),
    };
    // `unwrap` gives the blocks in the directory's order, which is the stages'.
    let (program, stage) = (programs.into_iter().zip(stages))
        .find(|(program, _)| program.slot == slot)
        .ok_or(Unbound::Stage(kind))?;
    Ok(Native {
        records: records(&stage.package)?,
        program,
        // A container with no signature part takes or gives nothing.
        inputs: parts.inputs().unwrap_or_default(),
        outputs: parts.outputs().unwrap_or_default(),
    })
}

/// `native` lowered, exporting as halves the parameter slots its pass's pixel program reads so.
pub fn lower(native: &Native, half_slots: &BTreeSet<u8>) -> Result<Lowered, Unbound> {
    lower_by(half_slots, |options| {
        Ok(rdna::translate_with(&native.program, options)?)
    })
}

/// A program lowered by `translate`, then again with the kind of each texture it loads texels
/// of stated, where it loads any: only a lowering shows which those are.
// Every texture the renderer binds is of a float or a normalised format: a material's map, the
// blank texel, and the noughts that stand for a texture the game fills.
pub(crate) fn lower_by<E>(
    half_slots: &BTreeSet<u8>,
    translate: impl Fn(&Options) -> Result<Lowered, E>,
) -> Result<Lowered, E> {
    let mut options = Options::default();
    options.half_slots.extend(half_slots);
    let lowered = translate(&options)?;
    let loaded = lowered.image_loads();
    if loaded.is_empty() {
        return Ok(lowered);
    }
    (options.image_kinds).extend(loaded.into_iter().map(|at| (at, naga::ScalarKind::Float)));
    translate(&options)
}

/// What each resource of `lowered` joins to, in the order of its bindings.
pub fn joins<'t>(native: &Native, lowered: &Lowered, table: &'t Sgd5Table) -> Vec<Join<'t>> {
    (lowered.resources.iter())
        .map(|resource| {
            let at = numbered(place(resource), native.program.kind);
            join(at, &native.records, table)
        })
        .collect()
}

/// The name the renderer knows a parameter by: the GTA V engine name its hash is of, else `0x` and
/// the hash a drawable states for it, which `effect`'s table gives for a program's +0x10.
// A program's table states no +0x14 for a constant, and a drawable never states a +0x10.
pub fn stated(hash: u32, effect: Option<&Sgd5Table>) -> String {
    let engine = hash_name(hash);
    if !engine.starts_with("0x") {
        return engine;
    }
    let drawable = effect.and_then(|table| {
        let constant = (table.constants.iter()).find(|c| c.name_hash == hash);
        let resource = || (table.resources.iter()).find(|r| r.name_hash == hash);
        (constant.map(|c| c.semantic_hash)).or_else(|| resource().map(|r| r.semantic_hash))
    });
    hash_name(drawable.filter(|hash| *hash != 0).unwrap_or(hash))
}

/// What a joined resource is to the renderer.
struct Bound {
    name: String,
    kind: &'static str,
    class: BindingClass,
    register: u32,
    layout: Option<ConstantBuffer>,
}

/// A translated program with each binding named and placed as the pack path reads it: the pixel
/// stage's in group 1, a constant buffer by its table's layout, a texture by the hash a drawable
/// states. `table` is the program's own variable table, `effect` the first table of an effect
/// that runs it. Refused where the renderer has nothing to give a resource.
pub fn bind_lowered(
    native: &Native,
    lowered: Lowered,
    table: &Sgd5Table,
    effect: Option<&Sgd5Table>,
) -> Result<Translated, Unbound> {
    let kind = native.program.kind;
    // A mesh program is lowered to a vertex module: three corners a thread, an instance a group.
    let stage = match kind {
        ShaderKind::Mesh => ShaderStageKind::Vertex,
        _ => kind.stage().ok_or(Unbound::Stage(kind))?,
    };
    // An element with no system value is a vertex attribute.
    if kind == ShaderKind::Vertex && !native.inputs.iter().any(|e| e.system_value == 0) {
        return Err(Unbound::NoVertexInput);
    }
    let group = u32::from(kind == ShaderKind::Pixel);
    let layouts = constant_buffers(table, &|hash, _| Some(stated(hash, effect)));
    let Lowered {
        mut module,
        resources,
        approximations,
        ..
    } = lowered;

    let mut bindings = Vec::with_capacity(resources.len());
    let mut buffers = Vec::new();
    for (index, resource) in resources.iter().enumerate() {
        let bound = match join(numbered(place(resource), kind), &native.records, table) {
            Join::Sampler => Bound {
                name: format!("sampler{index}"),
                kind: "sampler",
                class: BindingClass::Sampler,
                register: 0,
                layout: None,
            },
            Join::UserData => return Err(Unbound::UserData),
            Join::Words => return Err(Unbound::Words),
            Join::Picked => return Err(Unbound::Picked),
            Join::NoRecord { root, offset } => return Err(Unbound::NoRecord { root, offset }),
            Join::NoName { record } => {
                return Err(Unbound::NoName {
                    register: record.register,
                })
            }
            Join::Named { record, named } => {
                let name = stated(named.name_hash, effect);
                match (record.class, resource.kind) {
                    (Class::Cbv, ResourceKind::ConstantBuffer { .. }) => {
                        let layout = (layouts.iter().map(|(_, layout)| layout))
                            .find(|layout| layout.name == name)
                            .cloned()
                            .unwrap_or_else(|| ConstantBuffer {
                                name: name.clone(),
                                size: named.buffer_size,
                                variables: Vec::new(),
                            });
                        Bound {
                            name,
                            kind: "cbuffer",
                            class: BindingClass::Cbv,
                            register: record.register,
                            layout: Some(layout),
                        }
                    }
                    (Class::Srv, ResourceKind::Texture { .. }) => Bound {
                        name,
                        kind: "texture",
                        class: BindingClass::Srv,
                        register: record.register,
                        layout: None,
                    },
                    // A buffer of any class, and a texture the program writes: only the game fills them.
                    _ => return Err(Unbound::EngineBuffer(name)),
                }
            }
        };
        for (_, global) in module.global_variables.iter_mut() {
            let here = (global.binding.as_ref())
                .is_some_and(|b| (b.group, b.binding) == (resource.group, resource.binding));
            if here {
                global.name = Some(bound.name.clone());
                global.binding = Some(naga::ResourceBinding {
                    group,
                    binding: resource.binding,
                });
            }
        }
        bindings.push(Binding {
            name: bound.name,
            kind: bound.kind.to_string(),
            class: bound.class,
            register: bound.register,
            set: group,
            binding: resource.binding,
        });
        buffers.extend(bound.layout);
    }

    // The capabilities every backend the app enables grants, so a module that passes here is one a device takes.
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .map_err(|error| Unbound::Invalid(error.to_string()))?;

    Ok(Translated {
        stage,
        module,
        reflection: Reflection {
            bindings,
            constant_buffers: Vec::new(),
            inputs: native.inputs.clone(),
            outputs: native.outputs.clone(),
        },
        constant_buffers: buffers,
        fidelity: match approximations.is_empty() {
            true => Fidelity::Equivalent,
            false => Fidelity::Approximated,
        },
        lowerings: (approximations.iter())
            .map(|(what, count)| ((*what).to_string(), *count))
            .collect(),
    })
}

/// Why the native program of `container` is not handed to the renderer alone, as a program of
/// a pack is: the first thing that stops it, and where nothing does, that it is alone.
// ⚠ A pack keeps one module a program and pairs them when it draws, by location. A native
// program's parameters are not at the places the next stage reads them, and the link that
// moves them changes both stages: a pair is translated as one pass (`shading`) or not drawn.
pub fn bind(
    container: &[u8],
    table: &Sgd5Table,
    effect: Option<&Sgd5Table>,
) -> Result<Translated, Unbound> {
    bind_alone(&native(container)?, table, effect)
}

fn bind_alone(
    native: &Native,
    table: &Sgd5Table,
    effect: Option<&Sgd5Table>,
) -> Result<Translated, Unbound> {
    let lowered = lower(native, &BTreeSet::new())?;
    if !lowered.half_slots.is_empty() {
        return Err(Unbound::Halves);
    }
    bind_lowered(native, lowered, table, effect)?;
    Err(Unbound::Alone)
}

pub mod picks;
pub mod plain;
pub mod shading;

#[cfg(test)]
mod tests;
