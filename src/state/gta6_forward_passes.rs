//! A GTA VI forward pass in its technique: the state the engine draws that technique's list with,
//! and the technique's later passes, which draw the same geometry after it in their order.

use std::sync::Arc;

use or_cli::Gta6ShadingPass;
use or_io::gta5::shaders::sgd5::{Sgd5Program, Sgd5Table};

use super::translator::Translate;

/// A technique's name and the two programs of each of its passes, in their order.
pub(super) type Technique<'f> = (String, Vec<(&'f Sgd5Program, &'f Sgd5Program)>);

/// The technique of `techniques` whose first pass is the two programs at `at`.
pub(super) fn technique_at<'t, 'f>(
    techniques: &'t [Technique<'f>],
    at: (usize, usize),
) -> Option<&'t Technique<'f>> {
    (techniques.iter()).find(|(_, passes)| {
        (passes.first()).is_some_and(|(first, pixel)| (first.at, pixel.at) == at)
    })
}

/// `pass`, the first of `technique`, laid on as that technique's list is, with the technique's
/// later passes translated after it; `inflated` gives a program's container. A pass that is no
/// forward one is left as it is. `Err` says why the renderer cannot be handed it.
pub(super) fn in_technique<T: Translate>(
    translate: &T,
    pass: Gta6ShadingPass,
    technique: &Technique<'_>,
    parameters: &Sgd5Table,
    inflated: &mut dyn FnMut(&Sgd5Program) -> Result<Vec<u8>, String>,
) -> Result<Gta6ShadingPass, String> {
    let (name, passes) = technique;
    let (Some(_), Some((_, pixel))) = (pass.forward, passes.first()) else {
        return Ok(pass);
    };
    let placed = |pass: Gta6ShadingPass, pixel: &Sgd5Program| {
        or_cli::gta6_in_technique(pass, name, &pixel.name)
    };
    // Made from the last, so that each holds the one after it.
    let mut later = None;
    for (first, pixel) in passes.iter().skip(1).rev() {
        let (mesh, fragment) = (inflated(first)?, inflated(pixel)?);
        let stage = translate.pixel(&fragment, &pixel.variables, parameters)?;
        let next = translate.mesh(&mesh, &first.variables, parameters, &stage)?;
        let mut next = placed(next, pixel)?;
        next.then = later;
        later = Some(Arc::new(next));
    }
    let mut pass = placed(pass, pixel)?;
    pass.then = later;
    Ok(pass)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use or_cli::{Gta6Blend, Gta6Faces, Gta6Factor};

    use super::*;

    /// A lit pass's blend, as the translator gives it before its technique is known.
    const LIT: Gta6Blend = Gta6Blend {
        colour: [Gta6Factor::One, Gta6Factor::SecondColour],
        alpha: [Gta6Factor::One, Gta6Factor::SecondAlpha],
        writes: 0b1111,
        faces: Gta6Faces::Front,
    };

    /// A stand-in whose every pass is a lit forward one.
    struct Lit;

    fn lit_pass() -> Gta6ShadingPass {
        let stage = || or_cli::Gta6Stage {
            approximations: Default::default(),
            module: Arc::new(wgpu::naga::Module::default()),
            feeds: Vec::new(),
            textures: Vec::new(),
            streams: BTreeSet::new(),
            rows: Vec::new(),
        };
        Gta6ShadingPass {
            mesh: stage(),
            pixel: stage(),
            linked: Default::default(),
            forward: Some(LIT),
            then: None,
        }
    }

    impl Translate for Lit {
        type Pixel = ();

        fn shades(&self, _: &[u8], _: &[u8]) -> bool {
            false
        }

        fn streams(&self, _: &[u8], _: &Sgd5Table, _: &Sgd5Table) -> Result<BTreeSet<u32>, String> {
            Ok(BTreeSet::new())
        }

        fn pixel(&self, _: &[u8], _: &Sgd5Table, _: &Sgd5Table) -> Result<(), String> {
            Ok(())
        }

        fn mesh(
            &self,
            _: &[u8],
            _: &Sgd5Table,
            _: &Sgd5Table,
            _: &(),
        ) -> Result<Gta6ShadingPass, String> {
            Ok(lit_pass())
        }
    }

    fn program(name: &str, at: usize) -> Sgd5Program {
        Sgd5Program {
            bank: None,
            index: None,
            name: name.to_owned(),
            kind: 0,
            key: 0,
            at,
            stream: 0..0,
            inflated_len: 0,
            variables: Sgd5Table::default(),
        }
    }

    #[test]
    fn both_passes_of_a_glass_s_technique_are_drawn_its_inside_first() {
        let mesh = program("MS_VS_Transform", 1);
        let (inside, outside) = (
            program("PS_Textured_Eight_BackFace", 2),
            program("PS_Textured_Eight", 3),
        );
        let technique = (
            "drawmeshlets".to_owned(),
            vec![(&mesh, &inside), (&mesh, &outside)],
        );
        let techniques = [technique];
        let found = technique_at(&techniques, (1, 2)).expect("its first pass");
        assert!(
            technique_at(&techniques, (1, 3)).is_none(),
            "not a first pass"
        );
        let mut read = Vec::new();
        let mut inflated = |program: &Sgd5Program| -> Result<Vec<u8>, String> {
            read.push(program.at);
            Ok(Vec::new())
        };
        let table = Sgd5Table::default();
        let pass = in_technique(&Lit, lit_pass(), found, &table, &mut inflated).expect("drawn");
        drop(inflated);
        assert_eq!(read, [1, 3], "the second pass's two programs are read");
        let first = pass.forward.expect("a forward pass");
        assert_eq!(first.faces, Gta6Faces::Back, "the inside first");
        let then = pass.then.as_ref().expect("the second pass");
        assert_eq!(
            then.forward.map(|blend| blend.faces),
            Some(Gta6Faces::Front)
        );
        assert!(then.then.is_none());
    }

    #[test]
    fn a_lit_pass_of_a_list_whose_state_is_not_read_is_refused() {
        let (mesh, pixel) = (program("MS", 1), program("PS", 2));
        let technique = (
            "alphaPrepass_drawmeshlets".to_owned(),
            vec![(&mesh, &pixel)],
        );
        let table = Sgd5Table::default();
        let mut inflated = |_: &Sgd5Program| -> Result<Vec<u8>, String> { Ok(Vec::new()) };
        let refused = in_technique(&Lit, lit_pass(), &technique, &table, &mut inflated);
        assert!(refused.is_err());
        // A deferred pass is no forward one, and is left as it is.
        let deferred = Gta6ShadingPass {
            forward: None,
            ..lit_pass()
        };
        let left = in_technique(&Lit, deferred, &technique, &table, &mut inflated);
        assert!(left.is_ok_and(|pass| pass.forward.is_none() && pass.then.is_none()));
    }
}
