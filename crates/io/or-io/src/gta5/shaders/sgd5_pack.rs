//! An `SGD5` effect in the shapes of a shader pack's manifest: the passes the first version draws,
//! an effect of the techniques the renderer picks by name, and a program's constant buffers.

use crate::gta5::shaders::sgd5::{constant_units, Sgd5Effect, Sgd5File, Sgd5Program, Sgd5Table};
use crate::gta5::shaders::spirv_pack::{ConstantBuffer, Effect, Pass, Technique, Variable};

/// `SGD2`'s resource type code of a constant buffer.
const CONSTANT_BUFFER: u8 = 0x30;
/// The banks of a vertex and a pixel program.
const VERTEX_BANK: usize = 0;
const PIXEL_BANK: usize = 1;

/// Each (pass, bank-0 program, bank-1 program) of `effect` binding those two alone and on disk.
pub fn vertex_pixel_passes<'f>(
    file: &'f Sgd5File,
    effect: &Sgd5Effect,
) -> Vec<(usize, &'f Sgd5Program, &'f Sgd5Program)> {
    let program = |stated: u8, bank: usize| -> Option<&'f Sgd5Program> {
        let at = usize::from(stated.checked_sub(1)?);
        let index = *effect.programs.get(bank)?.get(at)?;
        file.program(bank, index).ok()
    };
    (effect.passes.iter().enumerate())
        .filter(|(_, pass)| pass.programs[PIXEL_BANK + 1..].iter().all(|p| *p == 0))
        .filter_map(|(i, pass)| {
            let vertex = program(pass.programs[VERTEX_BANK], VERTEX_BANK)?;
            let pixel = program(pass.programs[PIXEL_BANK], PIXEL_BANK)?;
            Some((i, vertex, pixel))
        })
        .collect()
}

/// The bank-0 and bank-1 programs of the first pass of `effect`'s technique `technique`, where
/// both are on disk and the pass binds no other.
// ⚠ A technique's passes follow those of the techniques before it; the index byte it states is not
// its first pass.
pub fn technique_programs<'f>(
    file: &'f Sgd5File,
    effect: &Sgd5Effect,
    technique: &str,
) -> Option<(&'f Sgd5Program, &'f Sgd5Program)> {
    let at = (effect.techniques.iter()).position(|t| t.name == technique)?;
    if effect.techniques[at].passes == 0 {
        return None;
    }
    let first: usize = (effect.techniques[..at].iter())
        .map(|t| usize::from(t.passes))
        .sum();
    vertex_pixel_passes(file, effect)
        .into_iter()
        .find(|(pass, _, _)| *pass == first)
        .map(|(_, vertex, pixel)| (vertex, pixel))
}

/// The bank-0 and bank-1 programs of the first pass of each technique of `effect`, in the
/// techniques' order, where both are on disk and the pass binds no other.
pub fn first_passes<'f>(
    file: &'f Sgd5File,
    effect: &Sgd5Effect,
) -> Vec<(&'f Sgd5Program, &'f Sgd5Program)> {
    let passes = vertex_pixel_passes(file, effect);
    let mut first = 0usize;
    let mut out = Vec::new();
    for technique in &effect.techniques {
        let found = passes.iter().find(|(pass, _, _)| *pass == first);
        if let (true, Some((_, vertex, pixel))) = (technique.passes > 0, found) {
            out.push((*vertex, *pixel));
        }
        first += usize::from(technique.passes);
    }
    out
}

/// The manifest effect: `draw` the first of `passes` not skinned, `drawskinned` the first that is.
// GTA VI's own technique names are not the renderer's.
pub fn pack_effect(name: &str, passes: &[(Pass, bool)]) -> Effect {
    let technique = |technique: &str, skinned: bool| {
        (passes.iter().find(|(_, s)| *s == skinned)).map(|(pass, _)| Technique {
            name: technique.to_string(),
            passes: vec![pass.clone()],
        })
    };
    Effect {
        name: name.to_string(),
        techniques: [technique("draw", false), technique("drawskinned", true)]
            .into_iter()
            .flatten()
            .collect(),
        ..Effect::default()
    }
}

/// The program's constant buffers as manifest layouts, named by `name_of` or else `0x` and +0x14.
// `name_of` takes a buffer's or variable's +0x10 and +0x14; a member of no fixed size is left out.
pub fn constant_buffers(
    table: &Sgd5Table,
    name_of: &dyn Fn(u32, u32) -> Option<String>,
) -> Vec<(u16, ConstantBuffer)> {
    let name = |hash: u32, semantic: u32| {
        name_of(hash, semantic).unwrap_or_else(|| format!("0x{semantic:08X}"))
    };
    (table.resources.iter())
        .filter(|r| r.code == CONSTANT_BUFFER)
        .map(|r| {
            let variables = (table.constants.iter())
                .filter(|c| c.cbuffer == Some(r.name_hash))
                .filter_map(|c| {
                    let units = u16::try_from(constant_units(c.code)?).ok()?;
                    let float = matches!(c.code, 0x09..=0x0E);
                    let (rows, columns) = if units == 16 { (4, 4) } else { (1, units) };
                    Some(Variable {
                        name: name(c.name_hash, c.semantic_hash),
                        offset: u32::from(c.offset),
                        size: 4 * u32::from(units) * u32::from(c.count.max(1)),
                        used: true,
                        scalar: if float { "float" } else { "int" }.to_string(),
                        rows,
                        columns,
                        elements: c.count,
                    })
                })
                .collect();
            let buffer = ConstantBuffer {
                name: name(r.name_hash, r.semantic_hash),
                size: r.buffer_size,
                variables,
            };
            (r.slot, buffer)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gta5::shaders::sgd5::{Sgd5Constant, Sgd5Pass, Sgd5Resource, BANKS};

    fn program(bank: usize, index: u32, at: usize) -> Sgd5Program {
        Sgd5Program {
            bank: Some(bank),
            index: Some(index),
            name: format!("p{bank}_{index}"),
            kind: 0,
            key: 0,
            at,
            stream: at + 16..at + 48,
            inflated_len: 64,
            variables: Sgd5Table::default(),
        }
    }

    fn pass(programs: [u8; BANKS]) -> Sgd5Pass {
        Sgd5Pass {
            name: String::new(),
            programs,
        }
    }

    #[test]
    fn only_the_passes_of_a_vertex_and_a_pixel_program_alone_are_drawn() {
        let file = Sgd5File {
            head_word: 0,
            layouts: Vec::new(),
            globals: Sgd5Table::default(),
            bank_counts: [Some(3), Some(2), Some(1), Some(0), Some(0), Some(0)],
            programs: vec![
                program(0, 0, 0x100),
                program(0, 2, 0x200),
                program(1, 1, 0x300),
                program(2, 0, 0x400),
            ],
            gaps: Vec::new(),
            effect_count: Some(1),
            effects: Vec::new(),
        };
        let effect = Sgd5Effect {
            name: "standard_x".into(),
            at: 0,
            word: 0,
            programs: [vec![2, 0, 1], vec![1], vec![0], vec![], vec![], vec![]],
            state_counts: [0; 4],
            techniques: Vec::new(),
            passes: vec![
                // Bank 0 index 2 and bank 1 index 1.
                pass([1, 1, 0, 0, 0, 0]),
                // With a geometry program: left out.
                pass([1, 1, 1, 0, 0, 0]),
                // No pixel program: left out.
                pass([2, 0, 0, 0, 0, 0]),
                // Bank 0 index 0 and bank 1 index 1.
                pass([2, 1, 0, 0, 0, 0]),
                // Bank 0 index 1 is not on disk: left out.
                pass([3, 1, 0, 0, 0, 0]),
                // A pass past the effect's list: left out.
                pass([9, 1, 0, 0, 0, 0]),
            ],
            bound: Vec::new(),
            parameters: Sgd5Table::default(),
            second: Sgd5Table::default(),
            layout_flag: 0,
            layout_refs: Vec::new(),
            layouts: Vec::new(),
        };
        let found: Vec<(usize, usize, usize)> = vertex_pixel_passes(&file, &effect)
            .into_iter()
            .map(|(i, v, p)| (i, v.at, p.at))
            .collect();
        assert_eq!(found, vec![(0, 0x200, 0x300), (3, 0x100, 0x300)]);
    }

    #[test]
    fn a_technique_s_passes_follow_those_of_the_techniques_before_it() {
        use crate::gta5::shaders::sgd5::Sgd5Technique;
        let file = Sgd5File {
            head_word: 0,
            layouts: Vec::new(),
            globals: Sgd5Table::default(),
            bank_counts: [Some(2), Some(2), Some(0), Some(0), Some(0), Some(0)],
            programs: vec![
                program(0, 0, 0x100),
                program(0, 1, 0x200),
                program(1, 0, 0x300),
                program(1, 1, 0x400),
            ],
            gaps: Vec::new(),
            effect_count: Some(1),
            effects: Vec::new(),
        };
        // Every stated index is 0, as if each technique began at the first pass.
        let technique = |name: &str, passes: u8| Sgd5Technique {
            name: name.into(),
            flag: false,
            index: 0,
            passes,
        };
        let effect = Sgd5Effect {
            name: "made_up".into(),
            at: 0,
            word: 0,
            programs: [vec![0, 1], vec![0, 1], vec![], vec![], vec![], vec![]],
            state_counts: [0; 4],
            techniques: vec![
                technique("depth", 2),
                technique("none", 0),
                technique("shade", 1),
                technique("alone", 1),
            ],
            passes: vec![
                pass([1, 1, 0, 0, 0, 0]),
                pass([1, 2, 0, 0, 0, 0]),
                pass([2, 2, 0, 0, 0, 0]),
                // No pixel program.
                pass([2, 0, 0, 0, 0, 0]),
            ],
            bound: Vec::new(),
            parameters: Sgd5Table::default(),
            second: Sgd5Table::default(),
            layout_flag: 0,
            layout_refs: Vec::new(),
            layouts: Vec::new(),
        };
        let at = |name: &str| technique_programs(&file, &effect, name).map(|(v, p)| (v.at, p.at));
        assert_eq!(at("depth"), Some((0x100, 0x300)));
        assert_eq!(at("shade"), Some((0x200, 0x400)));
        assert_eq!(at("none"), None);
        assert_eq!(at("alone"), None);
        assert_eq!(at("absent"), None);
    }

    #[test]
    fn an_effect_draws_with_its_first_plain_pass_and_skins_with_its_first_skinned_one() {
        let pass_of = |key: &str| Pass {
            stages: [(
                crate::gta5::shaders::spirv_pack::ShaderStageKind::Vertex,
                key.to_string(),
            )]
            .into(),
        };
        let effect = pack_effect(
            "standard_y",
            &[
                (pass_of("a"), true),
                (pass_of("b"), false),
                (pass_of("c"), false),
                (pass_of("d"), true),
            ],
        );
        assert_eq!(effect.name, "standard_y");
        let names: Vec<(&str, &Pass)> = (effect.techniques.iter())
            .map(|t| (t.name.as_str(), &t.passes[0]))
            .collect();
        assert_eq!(
            names,
            vec![("draw", &pass_of("b")), ("drawskinned", &pass_of("a"))]
        );
        assert!(pack_effect("none", &[]).techniques.is_empty());
    }

    #[test]
    fn a_constant_buffer_holds_the_members_that_name_it_at_their_offsets() {
        let constant =
            |code: u8, count: u16, offset: u16, name_hash: u32, cbuffer: u32| Sgd5Constant {
                code,
                count,
                offset,
                default_offset: 0,
                default: None,
                name_hash,
                semantic_hash: name_hash ^ 0xFFFF_0000,
                cbuffer: Some(cbuffer),
            };
        let resource = |code: u8, slot: u16, name_hash: u32| Sgd5Resource {
            code,
            flags: 0,
            slot,
            members: 0,
            buffer_size: 0x40 + u32::from(slot),
            name_hash,
            semantic_hash: name_hash.rotate_left(8),
        };
        let table = Sgd5Table {
            resources: vec![
                resource(0x30, 3, 0xA000_0001),
                resource(0x02, 5, 0xB000_0002),
                resource(0x30, 7, 0xC000_0003),
            ],
            constants: vec![
                constant(0x0C, 0, 0x10, 0x1111_0001, 0xA000_0001),
                constant(0x0E, 2, 0x20, 0x2222_0002, 0xA000_0001),
                constant(0x02, 0, 0x04, 0x3333_0003, 0xC000_0003),
                // No fixed size: left out.
                constant(0x0F, 0, 0x08, 0x4444_0004, 0xC000_0003),
            ],
            ..Sgd5Table::default()
        };
        let named = |hash: u32, _: u32| (hash == 0x2222_0002).then(|| "gWorld".to_string());
        let buffers = constant_buffers(&table, &named);
        assert_eq!(buffers.len(), 2);
        let (slot, first) = &buffers[0];
        assert_eq!(*slot, 3);
        assert_eq!(
            first.name,
            format!("0x{:08X}", 0xA000_0001u32.rotate_left(8))
        );
        assert_eq!(first.size, 0x43);
        let shape: Vec<(&str, u32, u32, u16, u16, u16)> = (first.variables.iter())
            .map(|v| {
                (
                    v.name.as_str(),
                    v.offset,
                    v.size,
                    v.rows,
                    v.columns,
                    v.elements,
                )
            })
            .collect();
        let semantic = format!("0x{:08X}", 0x1111_0001u32 ^ 0xFFFF_0000);
        assert_eq!(
            shape,
            vec![
                (semantic.as_str(), 0x10, 16, 1, 4, 0),
                ("gWorld", 0x20, 128, 4, 4, 2)
            ]
        );
        let (slot, second) = &buffers[1];
        assert_eq!((*slot, second.size), (7, 0x47));
        assert_eq!(second.variables.len(), 1);
        assert_eq!(
            (
                second.variables[0].size,
                second.variables[0].scalar.as_str()
            ),
            (8, "int")
        );
    }
}
