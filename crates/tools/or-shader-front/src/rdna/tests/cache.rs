//! What the translation cache keeps of a program: the module the optimiser leaves, which
//! computes what the lowering computes, and which still joins, packs and binds.

use or_io::gta5::shaders::sgd5::Sgd5Table;

use super::equal::{compare, SEEDS};
use super::*;
use crate::rdna::link::{self, Options as LinkOptions};
use crate::rdna::opt;
use crate::rdna_cache::{Cache, Cached};

/// The translation `cache` keeps of `program` under `options`.
fn kept(cache: &Cache, program: &Program, options: &Options) -> Lowered {
    match &*cache.get_or_translate(program, options).0 {
        Cached::Translated(lowered) => (**lowered).clone(),
        Cached::Refused { message, .. } => panic!("the program is refused: {message}"),
    }
}

/// A pixel program: `v0` and `v1` are the first attribute's first two components; three times
/// round, `v0 = v0 * v1 + v0`; the lane stays on where one > `v0`; the target takes `v0`, the
/// last product and `v1`.
fn looped() -> Program {
    let target = exp(0, [0, 2, 1, 1]);
    program(
        ShaderKind::Pixel,
        vec![
            vintrp(1, 0, 0, 0, 1),
            vintrp(1, 1, 0, 1, 1),
            sopk(S_MOVK, 0, 3),
            vop2(V_MUL_F32, 2, V, 1),
            vop2(V_ADD_F32, 0, V + 2, 0),
            sop2(S_ADD_I32, 0, 0, I_MINUS_ONE),
            sopc(S_CMP_LG_U32, 0, I_ZERO),
            // Back to the product, five words up.
            sopp(S_CBRANCH_SCC1, 0xFFFB),
            vopc(V_CMPX_GT_F32, F_ONE, 0),
            target[0],
            target[1],
            S_ENDPGM,
        ],
    )
}

#[test]
fn a_miss_keeps_the_module_the_optimiser_leaves_and_it_computes_what_the_lowering_does() {
    let program = looped();
    let lowered = translate(&program).unwrap().module;
    let cache = Cache::in_memory();
    let optimised = kept(&cache, &program, &Options::default()).module;
    let (before, after) = (opt::emitted(&lowered), opt::emitted(&optimised));
    assert!(after < before, "{after} of {before} expressions kept");
    validate(&optimised);
    let compared = compare(&lowered, &optimised, SEEDS).unwrap();
    assert!(compared.equal > 0, "{compared:?}");
    // What is kept is what the next caller gets, and the optimiser finds nothing more in it.
    assert!(cache.get_or_translate(&program, &Options::default()).1);
    let mut again = optimised.clone();
    assert_eq!(opt::optimise(&mut again, &[]), opt::Changed::default());
    // The optimiser off keeps the lowering as it is.
    let off = Options {
        unoptimised: true,
        ..Options::default()
    };
    let (as_lowered, kept_before) = cache.get_or_translate(&program, &off);
    assert!(
        !kept_before,
        "the optimised translation was served for the lowering"
    );
    let Cached::Translated(as_lowered) = &*as_lowered else {
        panic!("the program is refused");
    };
    assert_eq!(opt::emitted(&as_lowered.module), before);
}

#[test]
fn a_pass_whose_stages_the_cache_optimised_is_joined_and_packed() {
    // Twenty parameters each written to the one register the target takes: the last is read.
    let mut words: Vec<u32> = (0..20).map(|attr| vintrp(1, 4, attr, 0, 1)).collect();
    words.extend(exp(0, [4, 4, 4, 4]));
    words.push(S_ENDPGM);
    let mut pixel = program(ShaderKind::Pixel, words);
    pixel.inputs = (1..=20).map(|slot| PixelInput { slot, flags: 2 }).collect();
    let mut words = vec![vop1(V_MOV, 0, F_ONE)];
    words.extend(exp(POS0, [0, 0, 0, 0]));
    for parameter in 1..=20 {
        words.extend(exp(32 + parameter, [0, 0, 0, 0]));
    }
    words.push(S_ENDPGM);
    let vertex = program(ShaderKind::Vertex, words);
    let cache = Cache::in_memory();
    let stages = || {
        let options = Options::default();
        let geometry = kept(&cache, &vertex, &options).module;
        (geometry, kept(&cache, &pixel, &options).module)
    };
    let (mut geometry, mut fragment) = stages();
    let linked = link::link(&mut geometry, &mut fragment, &LinkOptions::default()).unwrap();
    validate(&geometry);
    validate(&fragment);
    assert_eq!((linked.carried.len(), linked.locations), (1, 1));
    assert_eq!(linked.carried[0].slot, 20);
    // Alone, the pixel stage packs the same way.
    let (_, mut alone) = stages();
    let packed = link::pack(&mut alone).unwrap();
    assert_eq!((packed.carried.len(), packed.locations), (1, 1));
    validate(&alone);
}

#[test]
fn a_stage_the_cache_optimised_binds_as_its_lowering_does() {
    let native = crate::rdna_bind::Native {
        program: looped(),
        records: Vec::new(),
        inputs: Vec::new(),
        outputs: Vec::new(),
    };
    let table = Sgd5Table::default();
    let bound = |lowered: Lowered| {
        crate::rdna_bind::bind_lowered(&native, lowered, &table, None)
            .map(|translated| translated.reflection.bindings.len())
    };
    let cache = Cache::in_memory();
    let optimised = kept(&cache, &native.program, &Options::default());
    let lowered = translate(&native.program).unwrap();
    assert!(opt::emitted(&optimised.module) < opt::emitted(&lowered.module));
    let bound_optimised = bound(optimised);
    assert!(bound_optimised.is_ok(), "{bound_optimised:?}");
    assert_eq!(bound_optimised, bound(lowered));
}
