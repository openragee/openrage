use super::*;
use crate::gta6::script::listing::Listing;
use crate::gta6::script::passes::values;
use crate::gta6::script::project::signatures::tests::{function, listing};

/// The settled reaches of the functions of one listing of program 1, each read as the indexer reads it: as lowered, before any pass.
fn reaches(functions: &[String]) -> Project {
    let mut project = Project::default();
    let text = listing(1, functions);
    let listing = Listing::open(&text).expect("the listing opens");
    for read in listing.functions() {
        let f = cfg::lower(read.expect("the function reads")).expect("the function lowers");
        of_function(&mut project, &f);
    }
    settle(&mut project);
    project
}

fn key(at: u32) -> FunctionKey {
    FunctionKey::new(1, at)
}

#[test]
fn a_cell_reaches_what_the_functions_loads_and_stores_through_it_and_its_callees_reach() {
    let functions = [
        // Eight bytes at 8, and the four at 0x10 of the second cell.
        function(
            "void",
            0x10,
            2,
            "store_u64(p0, 0x8, 1UL); Native.DEMO.USE(load_i32(p1, 0x10)); return;",
        ),
        // Handed on moved by 0x20 bytes, and tested against zero.
        function(
            "void",
            0x20,
            1,
            "if (((as_u64(p0) != 0))) { func_000010(byte_offset(p0, 0x20), p0); } return;",
        ),
        // Kept, and handed to a native.
        function("void", 0x30, 1, "Native.DEMO.USE(p0); return;"),
        function("void", 0x40, 1, "func_000030(p0); return;"),
        // Handed to a function the project holds no reach of.
        function("void", 0x50, 1, "func_000090(p0); return;"),
        // A ring that moves the address on every round grows without end.
        function(
            "void",
            0x60,
            1,
            "func_000060(byte_offset(p0, 0x8)); return;",
        ),
        // A ring that does not is bounded by what it touches.
        function(
            "void",
            0x70,
            1,
            "store_i32(p0, 0x4, 1); func_000070(p0); return;",
        ),
    ];
    let project = reaches(&functions);
    let reach = |at: u32, cell: u32| project.reach(key(at), cell);
    assert_eq!((reach(0x10, 0), reach(0x10, 1)), (Some(16), Some(20)));
    // Its own load says nothing, and what it hands on reaches 0x20 + 16 bytes, or 20 as the second cell.
    assert_eq!(reach(0x20, 0), Some(0x30));
    assert_eq!(reach(0x30, 0), Some(UNBOUNDED));
    assert_eq!(reach(0x40, 0), Some(UNBOUNDED));
    assert_eq!(reach(0x50, 0), Some(UNBOUNDED));
    assert_eq!(reach(0x60, 0), Some(UNBOUNDED));
    assert_eq!(reach(0x70, 0), Some(8));
    assert_eq!(project.reach(key(0x90), 0), None);
}

#[test]
fn a_cell_is_in_out_or_ref_by_what_every_path_does_with_its_bytes_before_it_reads_them() {
    let functions = [
        // Reads four bytes at 8 of the first cell; writes the first four of the second.
        function("void", 0x10, 2, "store_i32(p1, 0x0, load_i32(p0, 0x8)); return;"),
        // Reads only what it wrote first.
        function(
            "void",
            0x20,
            1,
            "store_u64(p0, 0x0, 1UL); Native.DEMO.USE(load_i32(p0, 0x0)); return;",
        ),
        // Reads what the caller left, then writes.
        function(
            "void",
            0x30,
            1,
            "Native.DEMO.USE(load_i32(p0, 0x0)); store_i32(p0, 0x0, 2); return;",
        ),
        // Writes on both ways.
        function(
            "void",
            0x40,
            2,
            "if (((as_u64(p1) != 0))) { store_u64(p0, 0x0, 1UL); } else { store_u64(p0, 0x0, 2UL); } return;",
        ),
        // Writes on one way only.
        function(
            "void",
            0x50,
            2,
            "if (((as_u64(p1) != 0))) { store_u64(p0, 0x0, 1UL); } return;",
        ),
        // A call that writes the bytes comes before the read.
        function(
            "void",
            0x60,
            1,
            "func_000020(p0); Native.DEMO.USE(load_i32(p0, 0x0)); return;",
        ),
        // A ring that writes on the way that returns.
        function(
            "void",
            0xA0,
            2,
            "if (((as_u64(p1) != 0))) { func_0000A0(p0, p1); } else { store_u64(p0, 0x0, 1UL); } return;",
        ),
        // Only held against zero.
        function(
            "void",
            0xB0,
            1,
            "if (((as_u64(p0) != 0))) { Native.DEMO.USE(1); } return;",
        ),
        // Hands on to one that reads what the caller left.
        function("void", 0xC0, 1, "func_000030(p0); return;"),
        // Hands on to a native, to a function the project holds no reach of, and to one that keeps it.
        function("void", 0xD0, 1, "Native.DEMO.USE(p0); return;"),
        function("void", 0xE0, 1, "func_000900(p0); return;"),
        function("void", 0xF0, 1, "func_0000D0(p0); return;"),
        // Hands on moved by 8 bytes: the eight bytes after the first are written.
        function("void", 0x100, 1, "func_000020(byte_offset(p0, 0x8)); return;"),
    ];
    let project = reaches(&functions);
    let mode = |at: u32, cell: u32| project.param_mode(key(at), cell);
    let covered = |at: u32, cell: u32| project.param_covered(key(at), cell);
    let cells = |at: u32, cell: u32| project.reach_cells(key(at), cell);
    assert_eq!((mode(0x10, 0), cells(0x10, 0)), (Some(Mode::In), Some(2)));
    assert_eq!(
        (mode(0x10, 1), covered(0x10, 1)),
        (Some(Mode::Out), Some(4))
    );
    assert_eq!(cells(0x10, 1), Some(1));
    assert_eq!(
        (mode(0x20, 0), covered(0x20, 0)),
        (Some(Mode::Out), Some(8))
    );
    assert_eq!((mode(0x30, 0), covered(0x30, 0)), (Some(Mode::Ref), None));
    assert_eq!(
        (mode(0x40, 0), covered(0x40, 0)),
        (Some(Mode::Out), Some(8))
    );
    assert_eq!((mode(0x50, 0), covered(0x50, 0)), (Some(Mode::Ref), None));
    assert_eq!(
        (mode(0x60, 0), covered(0x60, 0)),
        (Some(Mode::Out), Some(8))
    );
    assert_eq!(
        (mode(0xA0, 0), covered(0xA0, 0)),
        (Some(Mode::Out), Some(8))
    );
    assert_eq!(mode(0xB0, 0), Some(Mode::Untouched));
    assert_eq!(mode(0xC0, 0), Some(Mode::Ref));
    assert_eq!(
        (mode(0xD0, 0), mode(0xE0, 0), mode(0xF0, 0)),
        (None, None, None)
    );
    assert_eq!(
        (mode(0x100, 0), covered(0x100, 0)),
        (Some(Mode::Out), Some(0))
    );
    assert_eq!(cells(0x100, 0), Some(2));
    assert_eq!(project.param_mode(key(0x900), 0), None);
    // Settled, nothing of the order of the uses is kept.
    let kept = project.reach.values().flat_map(|cells| cells.values());
    assert!(kept
        .clone()
        .all(|held| held.flow.is_none() && held.uses.is_none()));
    assert!(kept.clone().all(|held| held.handed.is_empty()));
}

#[test]
fn a_cell_at_or_past_a_parameter_whose_address_is_taken_holds_no_callers_address() {
    // `&p0` reaches every cell from p0 on, so what the native does may overwrite p1.
    let functions = [function(
        "void",
        0x10,
        2,
        "Native.DEMO.USE(Vm.Address(parameters, 0)); store_u64(p1, 0x0, 0UL); return;",
    )];
    let project = reaches(&functions);
    assert_eq!(project.param_mode(key(0x10), 1), None);
    assert_eq!(project.reach(key(0x10), 1), Some(UNBOUNDED));
    // An address taken in the value returned reaches as far.
    let returned = [function(
        "VmValue",
        0x20,
        3,
        "return load_u64(load_u64(Vm.Address(parameters, 1), 0x8), 0x0);",
    )];
    let project = reaches(&returned);
    assert_eq!(project.reach(key(0x20), 2), Some(UNBOUNDED));
}

#[test]
fn an_address_dies_with_the_call_unless_the_function_or_one_it_hands_it_to_may_keep_it() {
    let functions = [
        function("void", 0x20, 1, "store_u64(p0, 0x0, 1UL); return;"),
        function(
            "void",
            0x30,
            1,
            "func_000020(byte_offset(p0, 0x8)); return;",
        ),
        // A ring that keeps nothing.
        function(
            "void",
            0x40,
            2,
            "if (((as_u64(p1) != 0))) { func_000040(p0, p1); } return;",
        ),
        function("VmValue", 0x50, 1, "return p0;"),
        function("void", 0x60, 1, "Native.DEMO.USE(p0); return;"),
        function("void", 0x70, 1, "func_000900(p0); return;"),
        function("void", 0x80, 1, "func_000030(p0); func_000060(p0); return;"),
        function("void", 0x90, 1, "func_000050(p0); return;"),
    ];
    let project = reaches(&functions);
    let dies = |at: u32| project.address_dies(key(at), 0);
    assert_eq!(
        (dies(0x20), dies(0x30), dies(0x40)),
        (Some(true), Some(true), Some(true))
    );
    assert_eq!(
        (dies(0x50), dies(0x60), dies(0x70)),
        (Some(false), Some(false), Some(false))
    );
    assert_eq!((dies(0x80), dies(0x90)), (Some(false), Some(false)));
    assert_eq!(dies(0x900), None);
}

#[test]
fn byte_runs_join_meet_and_part_as_sets_of_bytes() {
    let span = |runs: &[(u32, u32)]| Span::of(runs.iter().copied());
    // Runs that overlap or touch are one, and an empty run is none.
    let joined = span(&[(8, 12), (0, 4), (4, 6), (10, 16), (20, 20)]);
    assert_eq!(joined.0, [(0, 6), (8, 16)]);
    let a = span(&[(0, 8), (16, 24)]);
    let b = span(&[(4, 20)]);
    assert_eq!(a.union(&b).0, [(0, 24)]);
    assert_eq!(a.meet(&b).0, [(4, 8), (16, 20)]);
    assert_eq!(a.minus(&b).0, [(0, 4), (20, 24)]);
    assert_eq!(b.minus(&a).0, [(8, 16)]);
    assert!(Span::all().covers(&a) && !b.covers(&a) && a.union(&b).covers(&a));
    assert_eq!(a.shift(8).0, [(8, 16), (24, 32)]);
    assert_eq!(Span::all().shift(8).0, [(8, u32::MAX)]);
    assert_eq!((a.prefix(), b.prefix()), (8, 0));
    // Past the runs a set is kept to, one that may be larger is taken whole, and one that must be smaller loses its last runs.
    let many = Span::of((0..40u32).map(|n| (n * 16, n * 16 + 8)));
    assert_eq!(many.clone().widened().0, [(0, 39 * 16 + 8)]);
    let narrowed = many.clone().narrowed();
    assert_eq!(narrowed.0.len(), RANGES);
    assert!(many.covers(&narrowed) && !narrowed.covers(&many));
}

#[test]
fn statements_with_no_call_in_a_row_are_one_step_that_reads_only_what_they_did_not_write_first() {
    let text = listing(1, &[function("void", 0x10, 1, "return;")]);
    let opened = Listing::open(&text).expect("the listing opens");
    let read = opened
        .functions()
        .next()
        .expect("the listing holds a function");
    let f = cfg::lower(read.expect("the function reads")).expect("the function lowers");
    let mut reading = Reading {
        head: &f.head,
        holds: HashMap::from([(Var::Param(0), (0, 0))]),
        shown: BTreeMap::new(),
        step: BTreeMap::new(),
    };
    let mut uses = BTreeMap::new();
    let p0 = || Expr::Var(Var::Param(0));
    let load = |offset: u32| Expr::Load {
        ty: crate::gta6::script::ir::Scalar::I32,
        addr: Box::new(p0()),
        offset,
    };
    let store = |offset: u32| Stmt::Store {
        kind: crate::gta6::script::ir::StoreKind::U64,
        addr: p0(),
        offset,
        value: Expr::Const(crate::gta6::script::ir::Const::i32(1)),
    };
    reading.stmt(&store(0));
    reading.close(0, &mut uses);
    reading.stmt(&Stmt::Assign {
        target: Var::Local(0),
        value: Expr::binary(BinOp::Add, load(0x4), load(0x8)),
    });
    reading.close(0, &mut uses);
    let steps = &uses[&0][&0];
    assert_eq!(steps.len(), 1);
    // The read at 4 is of what the store wrote; the one at 8 is of what the caller left.
    assert_eq!(steps[0].reads, [(8, 12)]);
    assert_eq!(steps[0].writes, [(0, 8)]);
}

/// The addresses of locals the functions of `text` hand to a function: the bytes the passes take each to reach, beside what the callee reaches.
fn sites(text: &str, project: &Project) -> Vec<(u64, Option<u32>)> {
    let listing = Listing::open(text).expect("the listing opens");
    let mut sites = Vec::new();
    for read in listing.functions() {
        let mut f = cfg::lower(read.expect("the function reads")).expect("the function lowers");
        values::widths(&mut f);
        let singles: BTreeSet<u32> = (f.head.vars.iter())
            .filter_map(|(var, info)| match var {
                Var::Local(n) if info.single => Some(*n),
                _ => None,
            })
            .collect();
        let frame = f.head.locals;
        // The local alone where it is proved one cell wide, else every local up to the next one so proved.
        let taken = |start: u32| -> u64 {
            let next = singles.range(start.saturating_add(1)..).next().copied();
            let end = match singles.contains(&start) {
                true => start.saturating_add(1),
                false => next.unwrap_or(frame),
            };
            u64::from(end.clamp(start, frame.max(start)) - start) * 8
        };
        let head = f.head.clone();
        let mut note = |top: &Expr| {
            top.walk(&mut |e| {
                let Expr::Call { callee, args, .. } = e else {
                    return;
                };
                let Some(to) = key_of(head.program, callee) else {
                    return;
                };
                let mut cell = Some(0u32);
                for arg in args {
                    if let (
                        Some(n),
                        Expr::AddrOf {
                            var: Var::Local(k),
                            cell: c,
                        },
                    ) = (cell, arg)
                    {
                        sites.push((taken(k.saturating_add(*c)), project.reach(to, n)));
                    }
                    cell = cell
                        .zip(cells_of(arg, &head))
                        .and_then(|(a, b)| a.checked_add(b));
                }
            });
        };
        for block in &f.cfg.blocks {
            for stmt in &block.stmts {
                stmt.each_expr(&mut note);
            }
            if let Some(e) = block.term.expr() {
                note(e);
            }
        }
    }
    sites
}

#[test]
fn every_address_of_a_local_a_hand_written_function_hands_on_reaches_no_further_than_the_passes_take_it(
) {
    let functions = [
        // A local proved one cell wide, to a function that writes four bytes of it.
        function(
            "void",
            0x200,
            1,
            "local_1 = p0; func_000210(Vm.Address(locals, 1)); Native.DEMO.USE(local_1); return;",
        ),
        function("void", 0x210, 1, "store_i32(p0, 0x0, 1); return;"),
        // Two locals assigned in a row are proved nothing: the address is taken to reach the rest of the frame.
        function(
            "void",
            0x220,
            1,
            "local_2 = p0; local_3 = p0; func_000230(Vm.Address(locals, 2)); Native.DEMO.USE(local_2); Native.DEMO.USE(local_3); return;",
        ),
        function("void", 0x230, 1, "zero_bytes(p0, 16); return;"),
        // What a function the address is handed on to reaches counts, moved as it is moved.
        function(
            "void",
            0x240,
            1,
            "local_2 = p0; local_3 = p0; func_000250(Vm.Address(locals, 2)); Native.DEMO.USE(local_2); Native.DEMO.USE(local_3); return;",
        ),
        function("void", 0x250, 1, "func_000210(byte_offset(p0, 0x8)); return;"),
        // The control: a local proved one cell wide, to a function that writes two cells.
        function(
            "void",
            0x260,
            1,
            "local_1 = p0; func_000230(Vm.Address(locals, 1)); Native.DEMO.USE(local_1); return;",
        ),
    ];
    let project = reaches(&functions);
    let found = sites(&listing(1, &functions), &project);
    assert_eq!(
        found,
        [(8, Some(4)), (16, Some(16)), (16, Some(12)), (8, Some(16))]
    );
    let further: Vec<_> = (found.iter())
        .filter(|(taken, reach)| reach.is_none_or(|bytes| u64::from(bytes) > *taken))
        .collect();
    // Only the control reaches past what the passes take, and the measure sees it.
    assert_eq!(further, [&(8, Some(16))]);
}
