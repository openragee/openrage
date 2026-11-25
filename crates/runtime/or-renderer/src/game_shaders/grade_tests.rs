use super::*;

#[test]
fn the_shader_validates_and_its_uniforms_fill_the_block_it_declares() {
    let module = naga::front::wgsl::parse_str(SHADER).expect("the WGSL parses");
    naga::valid::Validator::new(Default::default(), naga::valid::Capabilities::empty())
        .validate(&module)
        .expect("the WGSL validates");
    let values = (module.types.iter())
        .find(|(_, t)| t.name.as_deref() == Some("Values"))
        .map(|(_, t)| t.inner.size(module.to_ctx()))
        .expect("a Values");
    assert_eq!(Grade::default().words().len() * 4, values as usize);
    assert_eq!(u64::from(values), UNIFORM_BYTES);
}

#[test]
fn a_moment_that_states_nothing_packs_white_and_the_multipliers_scale_the_points_after_the_power() {
    let none = Grade::of(&|_, default| default);
    assert_eq!(none.vertical, GradeSet::default());
    assert_eq!(none.horizontal, GradeSet::default());
    // A night's: the top at half, the bottom at one and a half, a fade of a quarter on the top's colour.
    let night = Grade::of(&|name, default| match name {
        "postfx_grad_top_col_mult" => 0.5,
        "postfx_grad_bottom_col_mult" => 1.5,
        "postfx_grad_top_col_fade" => 0.25,
        "postfx_grad_midpoint" => 0.4,
        _ => default,
    });
    let top = 0.5 * 0.25f32.powf(1.0 / 2.4);
    assert!(
        (night.vertical.first[0] - top).abs() < 1e-6,
        "{:?}",
        night.vertical
    );
    assert_eq!(night.vertical.last[..3], [1.5; 3]);
    assert_eq!(night.vertical.middle, [1.0, 1.0, 1.0, 0.4]);
    let p = night.vertical.pivot;
    assert!(
        (p[0] - 1.0 / 0.6).abs() < 1e-5 && (p[1] - 2.5).abs() < 1e-5,
        "{p:?}"
    );
}
