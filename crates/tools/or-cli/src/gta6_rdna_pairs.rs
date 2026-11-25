//! Which vertex program each geometry program of an effect container runs after: a geometry pass
//! names both, in two containers of the corpus, and the native front end lowers them as one.
//! Kept as lines of two corpus indices beside the corpus file.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::gta5::shaders::sgd5::Sgd5File;
use or_shader_front::dxil::ShaderKind;
use or_shader_front::rdna::{self, Options, Pair, Program};
use or_shader_front::rdna_corpus::Corpus;

const VERTEX_BANK: usize = 0;
const GEOMETRY_BANK: usize = 2;
/// The slot of a vertex program compiled to run before a geometry program.
const VERTEX_HALF_SLOT: usize = 2;

/// Where the pairs of the corpus at `corpus` are kept.
pub(crate) fn beside(corpus: &Path) -> PathBuf {
    corpus.with_extension("pairs")
}

/// Each (vertex program, geometry program) a pass of `container` draws with, as places among its programs.
pub(crate) fn of(container: &Sgd5File) -> BTreeSet<(usize, usize)> {
    let place = |bank: usize, index: u32| {
        (container.programs.iter()).position(|p| p.bank == Some(bank) && p.index == Some(index))
    };
    let mut pairs = BTreeSet::new();
    for effect in &container.effects {
        let program = |stated: u8, bank: usize| {
            let at = usize::from(stated.checked_sub(1)?);
            place(bank, *effect.programs[bank].get(at)?)
        };
        for pass in &effect.passes {
            let vertex = program(pass.programs[VERTEX_BANK], VERTEX_BANK);
            let geometry = program(pass.programs[GEOMETRY_BANK], GEOMETRY_BANK);
            if let (Some(vertex), Some(geometry)) = (vertex, geometry) {
                pairs.insert((vertex, geometry));
            }
        }
    }
    pairs
}

pub(crate) fn text(pairs: &BTreeSet<(usize, usize)>) -> String {
    (pairs.iter())
        .map(|(vertex, geometry)| format!("{vertex} {geometry}\n"))
        .collect()
}

/// The pairs kept at `path`; none where it does not read.
pub(crate) fn read(path: &Path) -> Vec<(usize, usize)> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    (text.lines())
        .filter_map(|line| {
            let (vertex, geometry) = line.split_once(' ')?;
            Some((vertex.parse().ok()?, geometry.parse().ok()?))
        })
        .collect()
}

fn is_half(program: &Program) -> bool {
    program.kind == ShaderKind::Geometry
        || (program.kind == ShaderKind::Vertex && program.slot == VERTEX_HALF_SLOT)
}

/// The options `program`, of the corpus's container `index`, is translated with: the other half
/// of the first pair that names it, where it is half of one.
pub(crate) fn options(
    corpus: &mut Corpus,
    pairs: &[(usize, usize)],
    index: usize,
    program: &Program,
) -> Options {
    let mut options = Options::default();
    if !is_half(program) {
        return options;
    }
    let geometry = program.kind == ShaderKind::Geometry;
    let other = (pairs.iter()).find_map(|&(v, g)| match geometry {
        true => (g == index).then_some(v),
        false => (v == index).then_some(g),
    });
    let mut read = |at: usize| corpus.program(at).ok().flatten();
    let (Some(other), Some(own)) = (other.and_then(&mut read), read(index)) else {
        return options;
    };
    // The geometry program's container states what the pair emits, whichever half this is.
    let emitted = rdna::emitted(if geometry { &own } else { &other });
    let half = (rdna::unwrap(&other).ok().into_iter().flatten())
        .find(|b| is_half(b) && (b.kind == ShaderKind::Geometry) != geometry);
    if let (Some(other), Ok(emitted)) = (half, emitted) {
        options.pair = Some(Arc::new(Pair { other, emitted }));
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_come_back_from_their_lines() {
        let pairs = BTreeSet::from([(3, 40), (3, 41), (7, 40)]);
        let dir = tempfile::tempdir().unwrap();
        let path = beside(&dir.path().join("corpus.bin"));
        std::fs::write(&path, text(&pairs)).unwrap();
        assert_eq!(read(&path), pairs.into_iter().collect::<Vec<_>>());
        assert!(read(&dir.path().join("absent.pairs")).is_empty());
    }
}
