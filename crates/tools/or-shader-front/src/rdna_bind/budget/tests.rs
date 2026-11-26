use super::*;

// The translator's test interpreter, which runs a module here: a stage before and after a merge
// is run on the same bytes.
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../../rdna/tests/interp.rs"]
mod interp;

/// What fills a buffer of a stage, in dwords, by its binding.
type Filled = BTreeMap<At, Vec<u32>>;

/// What `fit` did to `module`, and the same of a copy of the stage after the optimiser, its
/// registers kept for whoever reads them: the copy is fitted the same way, to a stage the
/// validator takes.
fn fitted_both_ways(module: &mut Module, fit: impl Fn(&mut Module) -> Fitted) -> Fitted {
    let mut passed = module.clone();
    let names: Vec<String> = (functions(&passed))
        .flat_map(|function| function.local_variables.iter())
        .filter_map(|(_, local)| local.name.clone())
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    crate::rdna::opt::optimise(&mut passed, &names);
    let (optimised, lowered) = (fit(&mut passed), fit(module));
    // The calls a merge writes are counted by the reads left, which the optimiser joins.
    let parts = |fitted: &Fitted| -> Vec<(At, Vec<Part>)> {
        (fitted.merged.iter())
            .map(|merged| (merged.at, merged.parts.clone()))
            .collect()
    };
    assert_eq!(
        (&optimised.own, &optimised.shared, parts(&optimised)),
        (&lowered.own, &lowered.shared, parts(&lowered))
    );
    assert_eq!(bindings(&passed), bindings(module));
    validated(&passed).expect("the optimised stage as fitted");
    // And reads the same: the registers each leaves, its buffers filled alike by a seed.
    for seed in 0..6 {
        let left = |stage: &Module| -> BTreeMap<String, u32> {
            let mut ran = interp::run_whole(stage, seed, 1 << 20);
            ran.threads.remove(0).registers.into_iter().collect()
        };
        assert_eq!(left(&passed), left(module), "seed {seed}");
    }
    lowered
}

/// [`super::within`], [`fitted_both_ways`].
fn within(module: &mut Module, noughts: &[At], budget: &Budget) -> Fitted {
    fitted_both_ways(module, |module| super::within(module, noughts, budget))
}

/// [`super::noughts_within`], [`fitted_both_ways`].
fn noughts_within(module: &mut Module, noughts: &[At], budget: &Budget) -> Fitted {
    fitted_both_ways(module, |module| {
        super::noughts_within(module, noughts, budget)
    })
}

fn wgsl(text: &str) -> Module {
    let module = naga::front::wgsl::parse_str(text).expect("the WGSL parses");
    validated(&module).expect("the stage is valid");
    module
}

fn global_at(module: &Module, at: At) -> Global {
    let found = (module.global_variables.iter()).find(|(_, global)| place(global) == Some(at));
    found
        .unwrap_or_else(|| panic!("nothing is bound at {at:?}"))
        .0
}

/// What a stage binds, sorted.
fn bindings(module: &Module) -> Vec<At> {
    let mut bound: Vec<At> = (module.global_variables.iter())
        .filter_map(|(_, global)| place(global))
        .collect();
    bound.sort_unstable();
    bound
}

/// `module` with its buffers holding `filled` when its entry point starts: each buffer of no
/// stated length is declared as long as what fills it, which the interpreter needs, and the
/// entry point first stores each dword.
fn holding(module: &Module, filled: &Filled) -> Module {
    let mut module = module.clone();
    let mut stores = Vec::new();
    for (at, dwords) in filled {
        let global = global_at(&module, *at);
        let TypeInner::Array { base, stride, .. } =
            module.types[module.global_variables[global].ty].inner
        else {
            panic!("the buffer at {at:?} is no array");
        };
        let elements = std::num::NonZeroU32::new(dwords.len() as u32 * 4 / stride).unwrap();
        let sized = module.types.insert(
            Type {
                name: None,
                inner: TypeInner::Array {
                    base,
                    size: ArraySize::Constant(elements),
                    stride,
                },
            },
            Span::UNDEFINED,
        );
        module.global_variables.get_mut(global).ty = sized;
        let expressions = &mut module.entry_points[0].function.expressions;
        let buffer = expressions.append(E::GlobalVariable(global), Span::UNDEFINED);
        for (dword, value) in dwords.iter().enumerate() {
            let dword = dword as u32;
            let mut cell = expressions.append(
                E::AccessIndex {
                    base: buffer,
                    index: dword * 4 / stride,
                },
                Span::UNDEFINED,
            );
            if stride == 16 {
                cell = expressions.append(
                    E::AccessIndex {
                        base: cell,
                        index: dword % 4,
                    },
                    Span::UNDEFINED,
                );
            }
            let value = expressions.append(E::Literal(Literal::U32(*value)), Span::UNDEFINED);
            stores.push(S::Store {
                pointer: cell,
                value,
            });
        }
    }
    let entry = &mut module.entry_points[0].function;
    let mut body = Block::from_vec(stores);
    body.extend_block(std::mem::take(&mut entry.body));
    entry.body = body;
    module
}

/// The registers `main_body` of `module` leaves, by name, run with the argument `at` on buffers
/// that hold `filled`.
fn registers(module: &Module, filled: &Filled, at: u32) -> BTreeMap<String, u32> {
    let module = holding(module, filled);
    let given = |_: u32, binding: &naga::Binding| {
        matches!(binding, naga::Binding::Location { .. }).then_some(interp::Value::U(at))
    };
    let threads = interp::run(&module, [1, 1, 1], &given);
    threads[0].registers.clone().into_iter().collect()
}

/// `filled` as a stage fitted as `fitted` says is filled: each merged buffer from its parts.
fn refilled(filled: &Filled, fitted: &Fitted) -> Filled {
    let mut filled = filled.clone();
    for merged in &fitted.merged {
        let parts: Vec<Vec<u32>> = (merged.parts.iter())
            .map(|part| filled.remove(&part.was).expect("a part is filled"))
            .collect();
        let whole = match merged.listed {
            // Each part at the byte the merge states.
            None => {
                let mut whole = Vec::new();
                for (part, dwords) in merged.parts.iter().zip(&parts) {
                    assert_eq!(part.bytes as usize, dwords.len() * 4);
                    whole.resize((part.offset / 4) as usize, 0);
                    whole.extend(dwords);
                }
                whole
            }
            // The list, an element a part and one more, then the parts.
            Some(element) => {
                let per = (element / 4) as usize;
                let mut next = parts.len() + 1;
                let mut whole = Vec::new();
                for dwords in &parts {
                    whole.push(next as u32);
                    whole.resize(whole.len() + per - 1, 0);
                    next += dwords.len() / per;
                }
                whole.push(next as u32);
                whole.resize(whole.len() + per - 1, 0);
                parts.iter().for_each(|dwords| whole.extend(dwords));
                whole
            }
        };
        filled.insert(merged.at, whole);
    }
    filled
}

/// `count` dwords from `first` on.
fn dwords(first: u32, count: u32) -> Vec<u32> {
    (first..first + count).collect()
}

const CONSTANT_BUFFERS: &str = "
@group(0) @binding(0) var<uniform> a: array<vec4<u32>, 2>;
@group(0) @binding(1) var<uniform> b: array<vec4<u32>, 3>;
@group(0) @binding(2) var<uniform> c: array<vec4<u32>, 1>;
@group(0) @binding(3) var<uniform> d: array<vec4<u32>, 5>;

fn main_body(at: u32) {
    var stated: u32;
    var row: u32;
    var cell: u32;
    var sum: u32;
    stated = a[1].z;
    row = b[at].y;
    cell = b[at][at & 3u];
    sum = c[0].x + d[at + 2u].w;
}

@fragment fn main(@location(0) @interpolate(flat) at: u32) { main_body(at); }
";

fn constant_buffers_filled() -> Filled {
    Filled::from([
        ((0, 0), dwords(100, 8)),
        ((0, 1), dwords(200, 12)),
        ((0, 2), dwords(300, 4)),
        ((0, 3), dwords(400, 20)),
    ])
}

#[test]
fn a_stage_past_its_constant_buffers_reads_the_same_bytes_from_the_smallest_merged_into_one() {
    let before = wgsl(CONSTANT_BUFFERS);
    let mut after = before.clone();
    let two = Budget {
        uniform: 2,
        ..Budget::DEFAULT
    };
    let fitted = within(&mut after, &[], &two);
    // Two too many: the three smallest are one buffer, bound where the smallest was, each at
    // the byte after the one before; the largest is left.
    assert_eq!(bindings(&after), [(0, 2), (0, 3)]);
    assert!(two.holds(Bound::of(&after)));
    let part = |was, offset, bytes| Part { was, offset, bytes };
    assert_eq!(
        fitted.merged,
        [Merged {
            at: (0, 2),
            parts: vec![
                part((0, 2), 0, 16),
                part((0, 0), 16, 32),
                part((0, 1), 48, 48)
            ],
            listed: None,
            calls: 2,
        }]
    );
    assert_eq!((fitted.own.len(), fitted.shared.len()), (0, 0));
    let filled = constant_buffers_filled();
    let merged = refilled(&filled, &fitted);
    assert_eq!(merged.len(), 2);
    for at in 0..3 {
        let read = registers(&before, &filled, at);
        // Each register holds a dword of a buffer of its own, so one read from a wrong place shows.
        assert_eq!(read["stated"], 106);
        assert_eq!(read["row"], 201 + 4 * at);
        assert_eq!(read["cell"], 200 + 5 * at);
        assert_eq!(read["sum"], 300 + 400 + 4 * (at + 2) + 3);
        assert_eq!(registers(&after, &merged, at), read, "at {at}");
    }
}

#[test]
fn a_place_past_a_merged_part_reads_the_parts_last_element_and_no_other_part() {
    let text = CONSTANT_BUFFERS.replace("d[at + 2u].w", "d[1].w");
    let mut after = wgsl(&text);
    let two = Budget {
        uniform: 2,
        ..Budget::DEFAULT
    };
    let fitted = within(&mut after, &[], &two);
    let merged = refilled(&constant_buffers_filled(), &fitted);
    // Row 7 of a buffer of three rows: its last row, as a device holds the place.
    let read = registers(&after, &merged, 7);
    assert_eq!((read["row"], read["cell"]), (209, 211));
}

const BUFFERS: &str = "
@group(0) @binding(0) var<storage, read> s: array<u32>;
@group(0) @binding(1) var<storage, read> t: array<u32>;
@group(0) @binding(2) var<storage, read> rows: array<vec4<u32>>;
@group(0) @binding(3) var<storage, read> more: array<vec4<u32>>;
@group(0) @binding(4) var<storage, read_write> written: array<u32>;

fn main_body(at: u32) {
    var first: u32;
    var second: u32;
    var stated: u32;
    var wide: u32;
    var other: u32;
    first = s[at];
    second = t[at + 1u];
    stated = t[2];
    wide = rows[at].z;
    other = more[1][at];
    written[0] = first;
}

@fragment fn main(@location(0) @interpolate(flat) at: u32) { main_body(at); }
";

#[test]
fn a_stage_past_its_buffers_reads_the_same_dwords_from_buffers_merged_behind_a_list() {
    let before = wgsl(BUFFERS);
    let mut after = before.clone();
    let three = Budget {
        storage: 3,
        ..Budget::DEFAULT
    };
    let fitted = within(&mut after, &[], &three);
    // Two too many: the two buffers of dwords are one and the two of rows are one; the buffer
    // the stage writes is left.
    assert_eq!(bindings(&after), [(0, 0), (0, 2), (0, 4)]);
    assert!(three.holds(Bound::of(&after)));
    let merged: Vec<_> = (fitted.merged.iter())
        .map(|m| (m.at, m.listed, m.calls, m.parts.len()))
        .collect();
    assert_eq!(merged, [((0, 0), Some(4), 3, 2), ((0, 2), Some(16), 2, 2)]);
    let written = ((0, 4), vec![0]);
    let filled = Filled::from([
        ((0, 0), dwords(100, 3)),
        ((0, 1), dwords(200, 5)),
        ((0, 2), dwords(300, 12)),
        ((0, 3), dwords(400, 8)),
        written,
    ]);
    let refilled = refilled(&filled, &fitted);
    // The list of the buffer of dwords: three entries, the parts after them.
    assert_eq!(refilled[&(0, 0)][..3], [3, 6, 11]);
    for at in 0..3 {
        let read = registers(&before, &filled, at);
        assert_eq!(read["first"], 100 + at);
        assert_eq!(read["second"], 201 + at);
        assert_eq!(read["stated"], 202);
        assert_eq!(read["wide"], 302 + 4 * at);
        assert_eq!(read["other"], 404 + at);
        assert_eq!(registers(&after, &refilled, at), read, "at {at}");
    }
    // Past the first part's three dwords: its last, not the second part's first.
    assert_eq!(registers(&after, &refilled, 3)["first"], 102);
}

#[test]
fn a_merged_buffer_of_dwords_states_each_parts_length() {
    let text = BUFFERS.replace("stated = t[2];", "stated = arrayLength(&t);");
    let mut after = wgsl(&text);
    let three = Budget {
        storage: 3,
        ..Budget::DEFAULT
    };
    let fitted = within(&mut after, &[], &three);
    let filled = Filled::from([
        ((0, 0), dwords(100, 3)),
        ((0, 1), dwords(200, 5)),
        ((0, 2), dwords(300, 12)),
        ((0, 3), dwords(400, 8)),
        ((0, 4), vec![0]),
    ]);
    let read = registers(&after, &refilled(&filled, &fitted), 0);
    assert_eq!(read["stated"], 5);
}

#[test]
fn a_buffer_read_whole_is_not_merged() {
    let whole = CONSTANT_BUFFERS.replace("stated = a[1].z;", "let all = a; stated = all[1].z;");
    let before = wgsl(&whole);
    let mut after = before.clone();
    let two = Budget {
        uniform: 2,
        ..Budget::DEFAULT
    };
    let fitted = within(&mut after, &[], &two);
    // The buffer at binding 0 stays bound as it was, and the three others are merged.
    assert_eq!(bindings(&after), [(0, 0), (0, 2)]);
    assert!(two.holds(Bound::of(&after)));
    let parts: Vec<At> = (fitted.merged.iter())
        .flat_map(|merged| merged.parts.iter().map(|part| part.was))
        .collect();
    assert_eq!(parts, [(0, 2), (0, 1), (0, 3)]);
    let filled = constant_buffers_filled();
    let read = registers(&before, &filled, 1);
    assert_eq!(registers(&after, &refilled(&filled, &fitted), 1), read);
}

#[test]
fn constant_buffers_are_merged_only_as_far_as_one_binds() {
    let mut module = wgsl(CONSTANT_BUFFERS);
    // The three smallest are 96 bytes: with 80 a buffer, only the two smallest are merged.
    let small = Budget {
        uniform: 2,
        uniform_bytes: 80,
        ..Budget::DEFAULT
    };
    let fitted = within(&mut module, &[], &small);
    assert_eq!(bindings(&module), [(0, 1), (0, 2), (0, 3)]);
    assert_eq!(fitted.merged[0].parts.len(), 2);
    // Within the budget nothing is touched.
    let mut module = wgsl(CONSTANT_BUFFERS);
    let before = format!("{module:?}");
    assert_eq!(
        within(&mut module, &[], &Budget::DEFAULT),
        Fitted::default()
    );
    assert_eq!(format!("{module:?}"), before);
}

const NOUGHTS: &str = "
@group(0) @binding(0) var<uniform> small: array<vec4<u32>, 1>;
@group(0) @binding(1) var<uniform> large: array<vec4<u32>, 300>;
@group(0) @binding(2) var<uniform> filled: array<vec4<u32>, 1>;
@group(0) @binding(3) var<uniform> medium: array<vec4<u32>, 2>;
@group(0) @binding(4) var<uniform> larger: array<vec4<u32>, 300>;
@group(0) @binding(5) var<storage, read> s: array<u32>;
@group(0) @binding(6) var<storage, read> t: array<u32>;
@group(0) @binding(7) var<storage, read> u: array<u32>;

fn main_body(at: u32) {
    var sum: u32;
    sum = small[0].x + large[at].x + filled[0].x + medium[1].x + larger[at].y + s[at] + t[at] + u[at];
}

@fragment fn main(@location(0) @interpolate(flat) at: u32) { main_body(at); }
";

#[test]
fn the_noughts_of_a_stage_are_taken_first_the_smallest_first_and_only_as_many_as_needed() {
    let noughts = [(0, 0), (0, 1), (0, 3), (0, 4), (0, 5), (0, 6)];
    let mut module = wgsl(NOUGHTS);
    let four = Budget {
        uniform: 4,
        ..Budget::DEFAULT
    };
    // One too many: the smallest left at nought is the stage's own, and nothing else moves.
    let fitted = noughts_within(&mut module, &noughts, &four);
    assert_eq!(
        (fitted.own.as_slice(), fitted.shared.len()),
        (&[(0, 0)][..], 0)
    );
    assert_eq!(bindings(&module).len(), 7);
    validated(&module).expect("the stage is valid");

    // Four too many: the two small ones are the stage's own, a large one is never its own
    // (each thread would hold a copy) and reads the other large one's buffer; the one the
    // editor fills is left, and the stage is still one past.
    let mut module = wgsl(NOUGHTS);
    let one = Budget {
        uniform: 1,
        ..Budget::DEFAULT
    };
    let fitted = noughts_within(&mut module, &noughts, &one);
    assert_eq!(fitted.own, [(0, 0), (0, 3)]);
    assert_eq!(fitted.shared, [((0, 4), (0, 1))]);
    assert_eq!(bindings(&module), [(0, 1), (0, 2), (0, 5), (0, 6), (0, 7)]);
    assert!(fitted.merged.is_empty());
    assert!(!one.holds(Bound::of(&module)));
    validated(&module).expect("the stage is valid");
    // With merges the one the editor fills joins the buffer of noughts.
    let fitted = within(&mut module, &[(0, 1)], &one);
    assert_eq!(fitted.merged.len(), 1);
    assert!(one.holds(Bound::of(&module)));
}

#[test]
fn buffers_of_dwords_left_at_nought_read_one_buffer_of_noughts() {
    let noughts = [(0, 5), (0, 6)];
    let mut module = wgsl(NOUGHTS);
    let two = Budget {
        storage: 2,
        ..Budget::DEFAULT
    };
    let fitted = noughts_within(&mut module, &noughts, &two);
    assert_eq!(fitted.shared, [((0, 6), (0, 5))]);
    assert!(two.holds(Bound::of(&module)));
    validated(&module).expect("the stage is valid");
    // The stage reads the same: the second buffer's dword is now the first's.
    let filled = Filled::from([((0, 5), dwords(7, 2)), ((0, 7), dwords(40, 2))]);
    assert_eq!(registers(&module, &filled, 1)["sum"], 8 + 8 + 41);
    // One the editor fills is never another's.
    let mut module = wgsl(NOUGHTS);
    let fitted = noughts_within(&mut module, &[(0, 5)], &two);
    assert_eq!(fitted, Fitted::default());
}

#[test]
fn a_stage_s_buffers_are_counted_as_a_backend_with_one_table_of_them_counts() {
    let pixel = wgsl(
        "@group(0) @binding(0) var<uniform> a: vec4<u32>;
@group(0) @binding(1) var<uniform> b: vec4<u32>;
@group(0) @binding(2) var<storage, read> s: array<u32>;
@group(0) @binding(3) var map: texture_2d<f32>;
@group(0) @binding(4) var bilinear: sampler;
@group(1) @binding(0) var maps: binding_array<texture_2d<f32>, 4>;
@fragment fn main() -> @location(0) vec4<f32> {
    return textureSample(maps[a.x + b.x + s[0]], bilinear, vec2<f32>(0.0)) + textureLoad(map, vec2<i32>(0), 0);
}",
    );
    let bound = Bound::of(&pixel);
    let expected = Bound {
        uniform: 2,
        storage: 1,
        arrays: 1,
        sized: true,
    };
    assert_eq!((bound, bound.together()), (expected, 5));
    // A stage with no buffer of dwords binds no buffer of lengths, unless it is a vertex stage.
    let plain = "@group(0) @binding(0) var<uniform> a: vec4<f32>;
STAGE fn main() -> RESULT vec4<f32> { return a; }";
    let stage = |stage: &str, result: &str| {
        Bound::of(&wgsl(
            &plain.replace("STAGE", stage).replace("RESULT", result),
        ))
    };
    assert_eq!(stage("@fragment", "@location(0)").together(), 1);
    assert_eq!(stage("@vertex", "@builtin(position)").together(), 2);
}

#[test]
fn metal_gives_a_vertex_stage_sixteen_buffers_less_its_vertex_buffers_and_another_stage_thirty_one()
{
    let bound = |uniform, sized| Bound {
        uniform,
        sized,
        ..Bound::default()
    };
    assert!(Budget::metal(false, 0).holds(bound(30, true)));
    assert!(!Budget::metal(false, 0).holds(bound(31, true)));
    assert!(Budget::metal(false, 1).holds(bound(31, false)));
    assert!(Budget::metal(true, 0).holds(bound(15, true)));
    assert!(!Budget::metal(true, 0).holds(bound(16, true)));
    assert!(Budget::metal(true, 1).holds(bound(14, true)));
    assert!(!Budget::metal(true, 1).holds(bound(15, true)));
    // A device with default limits counts each kind apart.
    let apart = Bound {
        uniform: 12,
        storage: 8,
        arrays: 3,
        sized: true,
    };
    assert!(Budget::DEFAULT.holds(apart));
    assert!(!Budget::DEFAULT.holds(Bound {
        storage: 9,
        ..apart
    }));
}

#[test]
fn a_vertex_stage_past_metals_sixteen_is_merged_to_them() {
    let mut text = String::new();
    for at in 0..20 {
        text.push_str(&format!(
            "@group(0) @binding({at}) var<uniform> b{at}: array<vec4<u32>, {}>;\n",
            at + 1
        ));
    }
    text.push_str("@vertex fn main(@location(0) at: u32) -> @builtin(position) vec4<f32> {\n    var sum = 0u;\n");
    for at in 0..20 {
        text.push_str(&format!("    sum += b{at}[at % {}u].x;\n", at + 1));
    }
    text.push_str("    return vec4<f32>(f32(sum));\n}");
    let mut module = wgsl(&text);
    let metal = Budget::metal(true, 0);
    assert!(!metal.holds(Bound::of(&module)));
    let fitted = within(&mut module, &[], &metal);
    // Twenty and the buffer of lengths are five past sixteen: the six smallest become one.
    assert_eq!(Bound::of(&module).together(), 16);
    assert!(metal.holds(Bound::of(&module)));
    assert_eq!(fitted.merged.len(), 1);
    assert_eq!(fitted.merged[0].parts.len(), 6);
    assert_eq!(fitted.merged[0].calls, 6);
}
