//! GTA VI's Xbox programs, which ship as native gfx10 GPU code and no bitcode: the container
//! unwrapped to instruction words, the words decoded, their control flow, and a lowering to naga IR.

pub mod cfg;
pub mod decode;
pub mod link;
mod opcodes;
pub mod opt;
pub mod sym;
pub mod to_naga;

use or_io::gta5::shaders::xdat;

pub use cfg::{Block, Cfg, CfgError, Terminator};
pub use decode::{decode, decode_one, DecodeError, Family, Inst, Op, Operand};
pub use sym::{Index, IndexSource, Loc};
pub use to_naga::{
    arena_layout, bound_entries_checked, emitted, intersections, lower, lower_with, Emitted,
    Intersection, Lowered, NotTranslated, Options, Pair, Region, Registers, Resource, ResourceKind,
    LOOP_NOT_SHOWN, LOOP_OTHER_LANES, LOOP_UNCHANGING,
};

use crate::dxil::container::{Container, ContainerError};
use crate::dxil::ShaderKind;

/// Two 12-byte records open a block's code range; the first instruction follows them.
const CODE_DESCRIPTOR: usize = 0x18;
/// The filler after a program's last instruction, to the end of its code range.
const S_CODE_END: u32 = 0xBF9F_0000;
/// The package word whose bit 5 says the code was compiled for 32 lanes rather than 64.
const PACKAGE_FLAGS: usize = 0xD4;
const WAVE32: u32 = 0x20;
/// A pixel package's input records: eight bytes each, from here while the first byte is 6, or
/// 7 as the record of the parameter in slot 0 is tagged where a program reads one.
const INPUT_TABLE: usize = 0x138;
const INPUT_RECORD: usize = 8;
const INPUT_TAGS: [u8; 2] = [6, 7];

/// One input a pixel program interpolates; its place in [`Program::inputs`] is the attribute number the code names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelInput {
    /// The parameter the vertex stage exported it as.
    pub slot: u8,
    /// Bit 0: read flat; bit 1: interpolated as 32-bit; bit 3: a 16-bit parameter; bit 4: interpolated as 16-bit.
    pub flags: u8,
}

/// One element of a vertex program's input signature, which a fetch routine outside the program loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexInput {
    pub register: u32,
    pub mask: u8,
    /// 1 unsigned, 2 signed, 3 float, as the signature states it.
    pub component_type: u32,
}

/// What a compute package states of its launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Launch {
    /// The scalar registers the pipeline loads; the group's numbers follow them.
    pub user_registers: u8,
    /// Bit `i`: the group's number along axis `i` is loaded.
    pub group_axes: u8,
    /// Whether the group's size is loaded too, after its numbers.
    pub group_size: bool,
    /// The threads of a group along each axis.
    pub threads: [u32; 3],
    /// The dwords of memory the threads of a group share.
    pub shared_dwords: u32,
}

/// One `XBCH` block's code: a container holds one, or two where its `XDAT` directory lists two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub kind: ShaderKind,
    /// The one used slot of the block's package, which tells two blocks of a container apart.
    pub slot: usize,
    /// The instruction words, the trailing filler dropped; data the code reads may follow the last instruction.
    pub words: Vec<u32>,
    pub wave32: bool,
    pub inputs: Vec<PixelInput>,
    pub vertex_inputs: Vec<VertexInput>,
    /// The components of the position a pixel program is handed, as a mask: those its signature
    /// says it reads, and the depth where it writes a depth held to one side of the rasterizer's.
    pub position_read: u8,
    /// Whether a pixel program's signature says it reads which way its triangle faces.
    pub front_face: bool,
    /// A compute program's launch, as its package states it.
    pub launch: Option<Launch>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RdnaError {
    #[error(transparent)]
    Container(#[from] ContainerError),
    #[error(transparent)]
    Cfg(#[from] CfgError),
    #[error(transparent)]
    NotTranslated(#[from] NotTranslated),
}

/// An input signature's element size, and where an element holds its system value, and the mask of the components the program reads.
const ELEMENT: usize = 32;
const ELEMENT_SYSTEM_VALUE: usize = 12;
const ELEMENT_READ_MASK: usize = 25;
/// The system values of a position and of a triangle's facing.
const SV_POSITION: u32 = 1;
const SV_IS_FRONT_FACE: u32 = 9;
/// The system values of a depth output that may only grow or only shrink from the rasterizer's:
/// the code clamps what it writes against the position's third component, which it is handed.
const SV_DEPTH_ONE_SIDED: [u32; 2] = [67, 68];

/// Whether an output signature holds a depth held to one side of the rasterizer's.
fn one_sided_depth(signature: &[u8]) -> bool {
    let word = |at: usize| {
        (signature.get(at..at + 4)).map_or(0, |w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
    };
    let (count, first) = (word(0) as usize, word(4) as usize);
    (0..count).any(|element| {
        (element.checked_mul(ELEMENT))
            .and_then(|by| by.checked_add(first + ELEMENT_SYSTEM_VALUE))
            .is_some_and(|at| at + 4 <= signature.len() && SV_DEPTH_ONE_SIDED.contains(&word(at)))
    })
}

/// The position components an input signature says are read, and whether the facing is.
fn system_reads(signature: &[u8]) -> (u8, bool) {
    let word = |at: usize| {
        (signature.get(at..at + 4)).map_or(0, |w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
    };
    let (count, first) = (word(0) as usize, word(4) as usize);
    let (mut position, mut facing) = (0, false);
    for element in 0..count {
        let Some(at) = element
            .checked_mul(ELEMENT)
            .and_then(|by| by.checked_add(first))
        else {
            break;
        };
        let Some(&read) = signature.get(at + ELEMENT_READ_MASK) else {
            break;
        };
        match word(at + ELEMENT_SYSTEM_VALUE) {
            SV_POSITION => position |= read & 0xF,
            SV_IS_FRONT_FACE => facing |= read != 0,
            _ => {}
        }
    }
    (position, facing)
}

/// A compute package's launch: the word at the input table's place holds the user registers in
/// bits 5:1, the group axes in 9:7, the group size's flag in 10 and the shared memory in 23:15,
/// in units of 512 bytes; the three words after it are the threads of a group.
fn launch(package: &[u8]) -> Launch {
    let word = |at: usize| {
        (package.get(at..at + 4)).map_or(0, |w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
    };
    let state = word(INPUT_TABLE);
    Launch {
        user_registers: ((state >> 1) & 0x1F) as u8,
        group_axes: ((state >> 7) & 7) as u8,
        group_size: (state >> 10) & 1 != 0,
        threads: [1, 2, 3].map(|k| word(INPUT_TABLE + 4 * k)),
        shared_dwords: ((state >> 15) & 0x1FF) * 128,
    }
}

fn pixel_inputs(package: &[u8]) -> Vec<PixelInput> {
    (package.get(INPUT_TABLE..).unwrap_or_default())
        .as_chunks::<INPUT_RECORD>()
        .0
        .iter()
        .take_while(|record| INPUT_TAGS.contains(&record[0]))
        .map(|record| PixelInput {
            slot: record[6],
            flags: record[7],
        })
        .collect()
}

/// Every block of a DXBC container's `DXIL` part, in directory order, as instruction words.
pub fn unwrap(dxbc: &[u8]) -> Result<Vec<Program>, ContainerError> {
    let container = Container::read(dxbc)?;
    let part = (container.part(b"DXIL")).ok_or(ContainerError::NoPart("DXIL"))?;
    let program = xdat::read(part)?;
    let kind = ShaderKind::from_program_version(program.program_version);
    // A container with no input signature fetches nothing.
    let vertex_inputs: Vec<VertexInput> = (container.inputs().unwrap_or_default().iter())
        .map(|e| VertexInput {
            register: e.register,
            mask: e.mask,
            component_type: e.component_type,
        })
        .collect();
    let (position_read, front_face) = match kind {
        ShaderKind::Pixel => {
            let (read, facing) = system_reads(container.part(b"ISG1").unwrap_or_default());
            let depth = one_sided_depth(container.part(b"OSG1").unwrap_or_default());
            (read | u8::from(depth) << 2, facing)
        }
        _ => (0, false),
    };
    (program.stages.iter().enumerate())
        .map(|(block, stage)| {
            let code = (stage.package.get(stage.code.clone()))
                .and_then(|code| code.get(CODE_DESCRIPTOR..))
                .unwrap_or_default();
            let mut words: Vec<u32> = (code.as_chunks::<4>().0.iter())
                .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
                .collect();
            while words.last() == Some(&S_CODE_END) {
                words.pop();
            }
            if words.is_empty() {
                return Err(ContainerError::Malformed {
                    part: "DXIL",
                    why: format!(
                        "block {block}'s code range holds no instruction after its descriptor"
                    ),
                });
            }
            let flags = (stage.package.get(PACKAGE_FLAGS..PACKAGE_FLAGS + 4))
                .map_or(0, |w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]));
            Ok(Program {
                kind,
                slot: stage.slot,
                words,
                wave32: flags & WAVE32 != 0,
                inputs: match kind {
                    ShaderKind::Pixel => pixel_inputs(&stage.package),
                    _ => Vec::new(),
                },
                vertex_inputs: vertex_inputs.clone(),
                position_read,
                front_face,
                launch: (kind == ShaderKind::Compute).then(|| launch(&stage.package)),
            })
        })
        .collect()
}

/// `program` as a naga module with what it binds, or why this front end does not translate it.
pub fn translate(program: &Program) -> Result<Lowered, RdnaError> {
    translate_with(program, &Options::default())
}

/// [`translate`], told what the program's neighbours need of it.
pub fn translate_with(program: &Program, options: &Options) -> Result<Lowered, RdnaError> {
    let cfg = Cfg::build(&program.words)?;
    Ok(lower_with(program, &cfg, options)?)
}

/// Whether naga's validator takes a translated module, with every capability allowed; `Err` is a fault of the lowering.
pub fn check(module: &naga::Module) -> Result<(), String> {
    use naga::valid::{Capabilities, ValidationFlags, Validator};
    Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(module)
        .map(|_| ())
        .map_err(|e| e.into_inner().to_string())
}

#[cfg(test)]
mod tests;
