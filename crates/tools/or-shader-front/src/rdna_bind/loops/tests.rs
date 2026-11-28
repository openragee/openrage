use super::*;

/// The game's buffer at binding 3, whose numbers no loop may hang on.
const H: u32 = 0x7B09_BD38;

/// A stage whose `body` runs before `tail`, its loop: `draw` is a draw's dwords (noughts), `game`
/// only the game fills, `rw` and `rv` are writable buffers the editor binds as noughts, `rest`
/// a buffer of the game's whose noughts are its state at rest, `stated` a buffer of stated
/// bytes, and `model` one the renderer fills.
fn module(body: &str, tail: &str) -> naga::Module {
    let text = format!(
        "@group(0) @binding(0) var<uniform> draw: vec4<u32>;
@group(0) @binding(3) var<uniform> game: vec4<u32>;
@group(0) @binding(5) var<storage, read_write> rw: array<atomic<u32>, 4>;
@group(0) @binding(6) var<storage, read_write> rv: array<u32, 4>;
@group(0) @binding(7) var<uniform> rest: array<vec4<u32>, 2>;
@group(0) @binding(8) var<uniform> stated: array<vec4<u32>, 2>;
@group(0) @binding(9) var<storage> model: array<u32>;
var<private> carried: u32;
fn set_carried() {{ carried = game.x; }}
fn run(n: u32) -> u32 {{
    var i: u32 = 0u;
    loop {{ if i >= n {{ break; }} i = i + 1u; }}
    return i;
}}
@compute @workgroup_size(1) fn main() {{
    var count: u32;
    count = 4u;
    var i: u32 = 0u;
    {body}
    {tail}
    rv[3] = i;
}}"
    );
    naga::front::wgsl::parse_str(&text).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&text)))
}

/// The words of `stated`: dword 5 is 3, dword 6 is the float 1.0.
fn stated_bytes() -> Vec<u8> {
    let mut words = [0u32; 8];
    (words[5], words[6]) = (3, 1f32.to_bits());
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

fn bindings() -> Vec<Binding> {
    let bound = |binding, words, game| Binding {
        at: (0, binding),
        words,
        game,
    };
    vec![
        bound(0, Words::Zero, None),
        bound(3, Words::Unknown, Some(H)),
        bound(5, Words::Zero, None),
        bound(6, Words::Zero, None),
        bound(7, Words::Zero, None),
        bound(8, Words::Bytes(stated_bytes()), None),
        bound(9, Words::Unknown, None),
    ]
}

const LOOP: &str = "loop { if i >= count { break; } i = i + 1u; }";

/// How the loops of the entry point end, the helper's loop first.
fn ends(body: &str, tail: &str) -> Vec<Ends> {
    judge(&module(body, tail), &bindings()).ends
}

/// How the entry point's own loop ends.
fn main_loop(body: &str) -> Ends {
    ends(body, LOOP)[1]
}

#[test]
fn a_writable_buffer_bound_as_noughts_decides_no_branch() {
    // Stored before, changed atomically, or written by another invocation or the frame before.
    for body in [
        "rv[0] = 1u; if rv[0] != 0u { count = game.x; }",
        "atomicAdd(&rw[0], 1u); if atomicLoad(&rw[0]) != 0u { count = game.x; }",
        "if rv[1] != 0u { count = game.x; } rv[1] = 1u;",
    ] {
        assert_eq!(main_loop(body), Ends::Game(H), "{body}");
    }
}

#[test]
fn a_float_decides_a_branch_only_where_every_device_computes_it_alike() {
    for body in [
        "let z = bitcast<f32>(draw.x | 0x80000000u); let s = 0.0 + z;
         if (bitcast<u32>(s) ^ 0x80000000u) == 0u { } else { count = game.x; }",
        "if bitcast<f32>(draw.x | 1u) == 0.0 { count = game.x; }",
        "if !(bitcast<f32>(draw.x) < bitcast<f32>(draw.x | 2u)) { count = game.x; }",
        "let s = bitcast<f32>(draw.x | 1u) + 0.0; if bitcast<u32>(s) == 1u { } else { count = game.x; }",
        "let s = bitcast<f32>(draw.x | 0x7F800001u) + 0.0;
         if (bitcast<u32>(s) ^ 0x7F800001u) == 0u { } else { count = game.x; }",
    ] {
        assert_eq!(main_loop(body), Ends::Game(H), "{body}");
    }
    // A plain float is folded.
    let plain = "if bitcast<f32>(draw.x | 0x3F800000u) < 2.0 { } else { count = game.x; }";
    assert_eq!(main_loop(plain), Ends::Literal(5));
}

#[test]
fn an_exact_hit_or_a_count_less_one_of_a_buffer_at_rest_never_leaves_and_is_told() {
    // At rest nought: the counter passes nought after its first step and never meets it again.
    let hit = "loop { i = i + 1u; if i == rest[1].y { break; } }";
    assert_eq!(ends("", hit)[1], Ends::Runaway);
    let less_one = "loop { if i >= rest[1].y - 1u { break; } i = i + 1u; }";
    assert_eq!(ends("", less_one)[1], Ends::Runaway);
    // Any field of the buffer, and an exact hit on a draw's dword.
    let other = "loop { i = i + 1u; if i == rest[0].x { break; } }";
    assert_eq!(ends("", other)[1], Ends::Runaway);
    let draw = "loop { i = i + 1u; if i == draw.x { break; } }";
    assert_eq!(ends("", draw)[1], Ends::Runaway);
    // An exact hit the counter meets is shown, by the words it was met with.
    let met = "loop { i = i + 1u; if i == stated[1].y { break; } }";
    assert_eq!(ends("", met)[1], Ends::Known(3));
    // A hit a step of two passes over.
    let passed = "i = 1u; loop { i = i + 2u; if i == 6u { break; } }";
    assert_eq!(ends("", passed)[1], Ends::Runaway);
    // An ordering exit to a buffer at rest leaves before its first turn.
    let ordered = "loop { if i >= rest[1].y { break; } i = i + 1u; }";
    assert_eq!(ends("", ordered)[1], Ends::Known(1));
}

#[test]
fn a_count_is_followed_through_subgroups_atomics_globals_and_calls() {
    for body in [
        "count = subgroupBroadcastFirst(game.x);",
        "count = subgroupMax(game.x);",
        "count = subgroupShuffle(game.x, 0u);",
        "carried = game.x; count = carried;",
        "set_carried(); count = carried;",
        "count = atomicAdd(&rw[0], game.x);",
    ] {
        assert_eq!(main_loop(body), Ends::Game(H), "{body}");
    }
    // The helper's loop counts to its argument, which a call hands the game's number.
    let called = ends("i = run(game.x);", "");
    assert_eq!(called, [Ends::Game(H)]);
    // Handed a stated number, the helper's loop is not the game's: its argument is not known.
    assert_eq!(ends("i = run(4u);", ""), [Ends::Unshown]);
}

#[test]
fn a_count_to_a_stated_word_is_judged_by_that_word() {
    assert_eq!(main_loop("count = stated[1].y;"), Ends::Known(4));
    // 1.0 read as a count is 0x3F800000 turns.
    assert_eq!(main_loop("count = stated[1].z;"), Ends::Runaway);
    // What the renderer fills is not known.
    assert_eq!(main_loop("count = model[2];"), Ends::Unshown);
}

#[test]
fn the_shapes_a_count_takes_that_end_are_shown() {
    assert_eq!(main_loop(""), Ends::Literal(5));
    // A branch on a draw's dword the device never takes, and one it does.
    assert_eq!(
        main_loop("if (draw.y & 0x8000000u) != 0u { count = game.x; }"),
        Ends::Literal(5)
    );
    assert_eq!(
        main_loop("if (draw.y & 0x8000000u) == 0u { count = game.x; }"),
        Ends::Game(H)
    );
    // A loop in a dead branch.
    let dead = "if draw.x != 0u { loop { } }";
    assert_eq!(ends("", dead)[1], Ends::Dead);
    // Down to nought, by a `continuing` and its `break if`.
    let down = "i = 8u; loop { continuing { i = i - 2u; break if i == 0u; } }";
    assert_eq!(ends("", down)[1], Ends::Literal(4));
    // As the optimiser leaves it: compared as the value stored, under `continue`/`break`.
    let stored = "loop { let next = i + 1u; i = next;
        if !(select(0u, 1u, next == 4u) != 0u) { continue; } else { break; } }";
    assert_eq!(ends("", stored)[1], Ends::Literal(4));
    // One turn, as a lowering leaves a waterfall the optimiser saw through.
    assert_eq!(
        ends("", "loop { i = model[0]; break; }")[1],
        Ends::Literal(1)
    );
    // Nested: the inner loop starts again each outer turn.
    let nested = "var j: u32 = 0u; loop { if j >= 3u { break; } j = j + 1u;
        i = 0u; loop { if i >= 2u { break; } i = i + 1u; } }";
    assert_eq!(ends("", nested)[1..], [Ends::Literal(4), Ends::Literal(3)]);
}

#[test]
fn a_loop_not_shown_to_end_is_told_from_one_shown_to_run_away() {
    // A step that a `continue` before it may skip, a counter stored twice, no exit.
    for tail in [
        "loop { if model[0] != 0u { continue; } i = i + 1u; if i >= 4u { break; } }",
        "loop { i = i + 1u; if model[1] != 0u { i = 0u; } if i >= 4u { break; } }",
        "loop { if model[0] == i { break; } i = i + 1u; }",
    ] {
        assert_eq!(ends("", tail)[1], Ends::Unshown, "{tail}");
    }
    // Every exit known and none taken.
    assert_eq!(
        ends("", "loop { if draw.x != 0u { break; } }")[1],
        Ends::Runaway
    );
    // An exit not tested every turn may yet be taken.
    let deep =
        "loop { if model[0] != 0u { if model[1] != 0u { break; } } if draw.x != 0u { break; } }";
    assert_eq!(ends("", deep)[1], Ends::Unshown);
}

#[test]
fn a_loop_to_the_sixty_fifth_buffer_of_the_game_s_is_named_after_it() {
    let module = module("count = game.x;", LOOP);
    let mut bindings: Vec<Binding> = (0..70)
        .map(|binding| Binding {
            at: (5, binding),
            words: Words::Unknown,
            game: Some(binding),
        })
        .collect();
    bindings[65] = Binding {
        at: (0, 3),
        words: Words::Unknown,
        game: Some(H),
    };
    assert_eq!(judge(&module, &bindings).ends[1], Ends::Game(H));
    let places: Vec<(u32, u32)> = bindings.iter().map(|b| b.at).collect();
    assert_eq!(counted_to_each(&module, &places), BTreeSet::from([(0, 3)]));
}

#[test]
fn the_lane_mask_s_select_keeps_the_count_the_lane_writes() {
    let text = |mask: &str| {
        format!(
            "@group(0) @binding(3) var<uniform> game: vec4<u32>;
@group(0) @binding(4) var<uniform> model: vec4<u32>;
@vertex fn main() -> @builtin(position) vec4<f32> {{
    var {mask}: u32;
    {mask} = model.w;
    var v: u32;
    v = game.x;
    let lanes = {mask};
    let old = v;
    v = select(old, model.x, (lanes & 1u) != 0u);
    var i: u32 = 0u;
    loop {{ if i >= v {{ break; }} i = i + 1u; }}
    return vec4<f32>(f32(i));
}}"
        )
    };
    let bindings = [Binding {
        at: (0, 3),
        words: Words::Unknown,
        game: Some(H),
    }];
    let judged = |mask| {
        judge(
            &naga::front::wgsl::parse_str(&text(mask)).unwrap(),
            &bindings,
        )
        .ends
    };
    assert_eq!(judged(LANE_MASK), [Ends::Unshown]);
    assert_eq!(judged("s4_"), [Ends::Game(H)]);
}
