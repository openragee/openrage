//! An entry of a bound array taken by a number a run computes, held to the array's length.

use super::*;

/// A pixel program that samples the image and the sampler of the table at `s2` that lie
/// `s9 - 1` dwords in: with noughts for what is bound, as far past the table as a dword goes.
fn picked() -> Lowered {
    let image = smem_at(S_LOAD_X8, 8, 2, 0x40, 30);
    let sampler = smem_at(S_LOAD_X4, 16, 2, 0, 30);
    let texel = sample(8, 16);
    let target = exp(0, [0, 1, 2, 3]);
    let words = vec![
        sop2(S_ADD_U32, 30, 9, I_MINUS_ONE),
        image[0],
        image[1],
        sampler[0],
        sampler[1],
        vop1(V_MOV, 0, F_ONE),
        vop1(V_MOV, 1, F_ONE),
        texel[0],
        texel[1],
        target[0],
        target[1],
        S_ENDPGM,
    ];
    lowered(ShaderKind::Pixel, words)
}

#[test]
fn an_entry_far_past_a_bound_array_is_its_last() {
    let out = picked();
    // The machine stops at an entry an array does not hold.
    let ran = interp::run(&out.module, [1, 1, 1], &|_, _| None).remove(0);
    assert!(out.bound_entries_checked());
    assert_eq!(ran.reg("s30"), u32::MAX);
    assert_eq!(ran.reg("s8_place") / 8, 0x0800_0001);
}

#[test]
fn an_entry_not_held_to_its_array_is_not_checked() {
    let mut module = picked().module;
    // The number a `min` held, taken as it is.
    let (body, _) = module.functions.iter().next().unwrap();
    let function = module.functions.get_mut(body);
    let bare: Vec<_> = (function.expressions.iter())
        .filter_map(|(at, expression)| match *expression {
            naga::Expression::Access { index, .. } => match function.expressions[index] {
                naga::Expression::Math { arg, .. } => Some((at, arg)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(bare.len(), 2);
    for (at, arg) in bare {
        if let naga::Expression::Access { index, .. } = &mut function.expressions[at] {
            *index = arg;
        }
    }
    assert!(!bound_entries_checked(&module));
}
