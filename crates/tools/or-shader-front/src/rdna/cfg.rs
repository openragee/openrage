//! The control-flow graph of one program: the instructions its entry reaches, cut into blocks at
//! each branch and each branch target.

use std::collections::{BTreeMap, BTreeSet};

use super::decode::{decode_one, DecodeError, Inst, Operand};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CfgError {
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error("the branch at word {pc} goes to word {target}, outside the code's {len} words")]
    Target { pc: u32, target: i64, len: usize },
    #[error("the code runs past its last word without ending, from word {pc}")]
    RunsOff { pc: u32 },
    #[error("control reaches word {pc}, {word:#010x}, which is no instruction")]
    Undecoded { pc: u32, word: u32 },
    #[error("the instruction at word {pc} is entered again in its middle, at word {inside}")]
    Overlap { pc: u32, inside: u32 },
}

/// How a block hands control on. A conditional branch is the block's last instruction and names its condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terminator {
    /// The program ends.
    End,
    Jump(u32),
    Branch {
        taken: u32,
        fallthrough: u32,
    },
    /// The next block starts here because something branches to it.
    Fall(u32),
    /// `s_setpc_b64`: the target is a register's value.
    Indirect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub start: u32,
    /// Each instruction with the word it starts at.
    pub insts: Vec<(u32, Inst)>,
    pub terminator: Terminator,
}

/// The blocks by their first word; the entry is word 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cfg {
    pub blocks: BTreeMap<u32, Block>,
}

enum Flow {
    Next,
    End,
    Jump(u32),
    Branch(u32),
    Indirect,
}

fn flow(inst: &Inst, pc: u32, len: usize) -> Result<Flow, CfgError> {
    let to = |imm: u16| {
        let target = i64::from(pc) + 1 + i64::from(imm as i16);
        match (0..len as i64).contains(&target) {
            true => Ok(target as u32),
            false => Err(CfgError::Target { pc, target, len }),
        }
    };
    Ok(match (inst, inst.name()) {
        (Inst::Unknown { word }, _) => return Err(CfgError::Undecoded { pc, word: *word }),
        (
            Inst::Sopp { .. },
            "s_endpgm" | "s_endpgm_saved" | "s_endpgm_ordered_ps_done" | "s_code_end",
        ) => Flow::End,
        (Inst::Sopp { imm, .. }, "s_branch") => Flow::Jump(to(*imm)?),
        (Inst::Sopp { imm, .. }, name) if name.starts_with("s_cbranch_") => Flow::Branch(to(*imm)?),
        (Inst::Sop1 { .. }, "s_setpc_b64") => Flow::Indirect,
        // `s_swappc_b64` and `s_call_b64` come back, so the block goes on.
        _ => Flow::Next,
    })
}

/// The word a far branch lands on: `s_getpc_b64` takes the address of the instruction after
/// it, the next two add a literal to that or take one from it, and `s_setpc_b64` at `pc` jumps there. `None` for a
/// jump through a register filled any other way, or to no word of the code.
fn far_target(insts: &BTreeMap<u32, (Inst, u32)>, pc: u32, len: usize) -> Option<u32> {
    let (Inst::Sop1 { src: through, .. }, _) = insts.get(&pc)? else {
        return None;
    };
    let mut before = insts.range(..pc).rev();
    let (_, (carry, _)) = before.next()?;
    let (_, (add, _)) = before.next()?;
    let (&at, (get, _)) = before.next()?;
    let names = (get.name(), add.name(), carry.name());
    let back = match names {
        ("s_getpc_b64", "s_add_u32", "s_addc_u32") => false,
        ("s_getpc_b64", "s_sub_u32", "s_subb_u32") => true,
        _ => return None,
    };
    let (
        Inst::Sop1 { dst: taken, .. },
        Inst::Sop2 {
            dst,
            src0,
            src1: Operand::Literal(by),
            ..
        },
    ) = (get, add)
    else {
        return None;
    };
    if taken != through || dst != through || src0 != through {
        return None;
    }
    // The code's first word is at address zero, as the lowering of `s_getpc_b64` has it.
    let bytes = match back {
        true => (4 * (at + 1)).wrapping_sub(*by),
        false => (4 * (at + 1)).wrapping_add(*by),
    };
    (bytes % 4 == 0 && ((bytes / 4) as usize) < len).then_some(bytes / 4)
}

impl Cfg {
    /// The graph of what word 0 reaches; code only an indirect jump or call reaches is left out.
    pub fn build(words: &[u32]) -> Result<Self, CfgError> {
        let len = words.len();
        let mut far: BTreeMap<u32, u32> = BTreeMap::new();
        let mut insts: BTreeMap<u32, (Inst, u32)> = BTreeMap::new();
        let mut leaders = BTreeSet::from([0u32]);
        let mut work = vec![0u32];
        while let Some(mut pc) = work.pop() {
            while !insts.contains_key(&pc) {
                if pc as usize >= len {
                    return Err(CfgError::RunsOff { pc });
                }
                let (inst, size) = decode_one(words, pc as usize)?;
                let next = pc + size as u32;
                let flow = flow(&inst, pc, len)?;
                insts.insert(pc, (inst, size as u32));
                let flow = match flow {
                    Flow::Indirect => match far_target(&insts, pc, len) {
                        Some(target) => {
                            far.insert(pc, target);
                            Flow::Jump(target)
                        }
                        None => Flow::Indirect,
                    },
                    other => other,
                };
                match flow {
                    Flow::Next => pc = next,
                    Flow::End | Flow::Indirect => break,
                    Flow::Jump(target) => {
                        leaders.insert(target);
                        work.push(target);
                        break;
                    }
                    Flow::Branch(target) => {
                        leaders.extend([target, next]);
                        work.push(target);
                        pc = next;
                    }
                }
            }
        }
        let mut end = 0;
        let mut last = 0;
        for (&pc, &(_, size)) in &insts {
            if pc < end {
                return Err(CfgError::Overlap {
                    pc: last,
                    inside: pc,
                });
            }
            (last, end) = (pc, pc + size);
        }

        let mut blocks = BTreeMap::new();
        let mut open: Option<Block> = None;
        for (pc, (inst, size)) in insts {
            let next = pc + size;
            let terminator = match flow(&inst, pc, len)? {
                Flow::End => Some(Terminator::End),
                Flow::Indirect => Some(match far.get(&pc) {
                    Some(&target) => Terminator::Jump(target),
                    None => Terminator::Indirect,
                }),
                Flow::Jump(target) => Some(Terminator::Jump(target)),
                Flow::Branch(taken) => Some(Terminator::Branch {
                    taken,
                    fallthrough: next,
                }),
                Flow::Next if leaders.contains(&next) => Some(Terminator::Fall(next)),
                Flow::Next => None,
            };
            let mut block = open.take().unwrap_or(Block {
                start: pc,
                insts: Vec::new(),
                terminator: Terminator::End,
            });
            block.insts.push((pc, inst));
            match terminator {
                Some(terminator) => {
                    block.terminator = terminator;
                    blocks.insert(block.start, block);
                }
                None => open = Some(block),
            }
        }
        Ok(Self { blocks })
    }

    /// The blocks `block` hands control to.
    pub fn successors(&self, block: u32) -> Vec<u32> {
        match self.blocks.get(&block).map(|block| block.terminator) {
            Some(Terminator::Jump(to) | Terminator::Fall(to)) => vec![to],
            Some(Terminator::Branch { taken, fallthrough }) => vec![taken, fallthrough],
            Some(Terminator::End | Terminator::Indirect) | None => Vec::new(),
        }
    }
}
