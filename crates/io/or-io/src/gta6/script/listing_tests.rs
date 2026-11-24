use super::*;
use crate::gta6::script::sample;

/// A listing of one function with one parameter and two locals, whose statements start on line 12.
fn listing_of(body: &str) -> String {
    format!(
        "[VmProgram(0x00000001U)]
public static class Program_00000001
{{
    [VmFunction(0x000000)]
    public static void func_000000(VmValue arg0) // 0x000000
    {{
        VmValue[] parameters = new VmValue[] {{ arg0 }};
        VmValue[] locals = new VmValue[2];
        ref VmValue p0 = ref parameters[0];
        ref VmValue local_0 = ref locals[0];
        ref VmValue local_1 = ref locals[1];
{body}
    }}
}}
"
    )
}

fn read(body: &str) -> Result<Vec<tree::Stmt>, ListingError> {
    let text = listing_of(body);
    let listing = Listing::open(&text)?;
    let mut functions = listing.functions();
    let first = functions.next().expect("the listing holds a function")?;
    if let Some(second) = functions.next() {
        second?;
        panic!("the listing holds one function");
    }
    Ok(first.body)
}

/// The value the one statement `local_0 = <expr>;` assigns.
fn value(expr: &str) -> Expr {
    let body = read(&format!("        local_0 = {expr};")).expect("the statement reads");
    match body.as_slice() {
        [tree::Stmt::Simple(Stmt::Assign {
            target: Var::Local(0),
            value,
        })] => value.clone(),
        other => panic!("not one assignment to local_0: {other:?}"),
    }
}

fn sample_functions() -> Vec<tree::Function> {
    let listing = Listing::open(sample::LISTING).expect("the sample opens");
    listing
        .functions()
        .collect::<Result<_, _>>()
        .expect("every function of the sample reads")
}

#[test]
fn the_sample_reads_with_its_head_and_declarations() {
    let listing = Listing::open(sample::LISTING).expect("the sample opens");
    assert_eq!(listing.program, 0x1122_3344);
    assert_eq!(
        listing.imports,
        [
            Import {
                index: 1,
                program: 0x0A0B_0C0D,
                types: 2..100
            },
            Import {
                index: 2,
                program: 0x1A2B_3C4D,
                types: 100..250
            },
        ]
    );
    assert_eq!(listing.declarations().len(), 12);
    let native = listing.declarations().get("Native.DEMO.GET_THING");
    assert_eq!(
        native,
        Some(&Wrapper {
            target: Target::Call(Callee::Native {
                namespace: Some("DEMO".to_owned()),
                name: "GET_THING".to_owned(),
                hash: 0x5555_6666_7777_8888,
                rva: 0x0102_030C,
            }),
            returns: 1,
            params: 1,
        })
    );

    let functions = sample_functions();
    let names: Vec<&str> = functions.iter().map(|f| f.head.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "func_000000",
            "func_000100",
            "func_000200",
            "func_000300",
            "main"
        ]
    );
    let statements: Vec<usize> = functions
        .iter()
        .map(|f| tree::counts(&f.body).statements)
        .collect();
    assert_eq!(statements, sample::STATEMENTS);
}

#[test]
fn a_function_head_states_its_frames_and_temporaries() {
    let functions = sample_functions();
    let head = &functions[0].head;
    assert_eq!(
        (
            head.program,
            head.at,
            head.returns,
            head.params,
            head.locals
        ),
        (0x1122_3344, 0, Returns::Void, 2, 4)
    );
    let widths: Vec<(String, Width)> = head
        .vars
        .iter()
        .map(|(var, info)| (var.to_string(), info.width))
        .collect();
    assert_eq!(
        widths,
        [
            ("v_000010_0".to_owned(), Width::Cell),
            ("v_000018_1".to_owned(), Width::Cells),
            ("v_000020_2".to_owned(), Width::Cell),
            ("v_000028_3".to_owned(), Width::Cells),
            ("stack_000040_0".to_owned(), Width::Cell),
            ("stack_000040_1".to_owned(), Width::Cell),
        ]
    );
    assert_eq!(functions[1].head.returns, Returns::Cell);
    assert_eq!(functions[2].head.returns, Returns::Cells);
    assert_eq!(functions[4].head.at, 0x400);
}

#[test]
fn a_function_with_no_body_keeps_the_reason() {
    let functions = sample_functions();
    let head = &functions[3].head;
    assert_eq!(
        head.incomplete.as_deref(),
        Some("Widths are underdetermined.")
    );
    assert!(functions[3].body.is_empty());
    assert_eq!(functions[0].head.incomplete, None);
}

#[test]
fn a_literal_takes_the_type_c_sharp_gives_it() {
    let constant = |text: &str| match value(text) {
        Expr::Const(c) => (c.bits, c.kind),
        other => panic!("`{text}` is not a constant: {other:?}"),
    };
    assert_eq!(constant("5"), (5, Kind::I32));
    assert_eq!(constant("0x7FFFFFFF"), (0x7FFF_FFFF, Kind::I32));
    assert_eq!(constant("0xFFFFFFFF"), (0xFFFF_FFFF, Kind::U32));
    assert_eq!(constant("0x100000000"), (0x1_0000_0000, Kind::I64));
    assert_eq!(
        constant("0xFFFFFFFF80000001"),
        (0xFFFF_FFFF_8000_0001, Kind::U64)
    );
    assert_eq!(constant("7U"), (7, Kind::U32));
    assert_eq!(constant("7UL"), (7, Kind::U64));
    assert_eq!(constant("(-1)"), (u64::MAX, Kind::I32));
    assert_eq!(
        constant("(-2147483648)"),
        (i32::MIN as i64 as u64, Kind::I32)
    );
    assert_eq!(constant("1.5f"), (u64::from(1.5f32.to_bits()), Kind::F32));
    assert_eq!(constant("(-1f)"), (u64::from((-1f32).to_bits()), Kind::F32));
    assert_eq!(
        constant("1E-45f"),
        (u64::from(f32::from_bits(1).to_bits()), Kind::F32)
    );
    assert_eq!(constant("float_bits(0xFFFFFFFF)"), (0xFFFF_FFFF, Kind::F32));
    assert_eq!(constant("true"), (1, Kind::Bool));
}

#[test]
fn an_operator_works_at_the_type_its_operands_promote_to() {
    let at = |text: &str| match value(text) {
        Expr::Binary { at, .. } => at,
        other => panic!("`{text}` is not a binary operator: {other:?}"),
    };
    assert_eq!(at("((as_i32(p0) < 7))"), Kind::I32);
    assert_eq!(at("((as_u32(p0) == 0))"), Kind::U32);
    assert_eq!(at("((as_u64(p0) == 0))"), Kind::U64);
    assert_eq!(at("((as_u32(p0) + as_i32(local_1)))"), Kind::I64);
    assert_eq!(at("((as_u8(p0) + as_u8(local_1)))"), Kind::I32);
    assert_eq!(at("((as_f32(p0) < as_f32(0x3F000000)))"), Kind::F32);
    assert_eq!(at("((1UL << unchecked((int)(10))))"), Kind::U64);
    assert_eq!(
        at("((as_i32(1) << unchecked((int)(as_i32(p0)))))"),
        Kind::I32
    );
    assert_eq!(at("(((as_i32(p0) == 1)) != Truth(0))"), Kind::Bool);
    assert_eq!(at("((p0 == local_1))"), Kind::Unknown);
}

#[test]
fn operators_bind_as_c_sharp_binds_them() {
    let var = |n| Expr::cast(Scalar::I32, Expr::Var(Var::Local(n)));
    let one = || Expr::Const(Const::i32(1));
    assert_eq!(
        value("as_i32(local_0) + as_i32(local_1) * 1"),
        Expr::binary(BinOp::Add, var(0), Expr::binary(BinOp::Mul, var(1), one()))
    );
    assert_eq!(
        value("as_i32(local_0) - as_i32(local_1) - 1"),
        Expr::binary(BinOp::Sub, Expr::binary(BinOp::Sub, var(0), var(1)), one())
    );
    assert_eq!(
        value("as_i32(local_0) << 1 < as_i32(local_1) & 1"),
        Expr::binary(
            BinOp::And,
            Expr::binary(BinOp::Lt, Expr::binary(BinOp::Shl, var(0), one()), var(1)),
            one()
        )
    );
}

#[test]
fn the_runtime_helpers_read_as_what_they_do() {
    assert_eq!(
        value("load_i32(Vm.Address(locals, 1), 0x10)"),
        Expr::Load {
            ty: Scalar::I32,
            addr: Box::new(Expr::AddrOf {
                var: Var::Local(1),
                cell: 0
            }),
            offset: 0x10,
        }
    );
    assert_eq!(
        value("Vm.Globals(0x000000AAU, true)[0x18]"),
        Expr::Var(Var::Global {
            program: 0xAA,
            offset: 0x18,
            statics: true
        })
    );
    assert_eq!(
        value("Vm.GlobalAddress(0x000000AAU, 0x18)"),
        Expr::AddrOf {
            var: Var::Global {
                program: 0xAA,
                offset: 0x18,
                statics: false
            },
            cell: 0
        }
    );
    assert_eq!(
        value("unchecked((int)(as_i64(p0)))"),
        Expr::cast(
            Scalar::I32,
            Expr::cast(Scalar::I64, Expr::Var(Var::Param(0)))
        )
    );
    assert_eq!(
        value("(VmValue)(p0)"),
        Expr::Var(Var::Param(0)),
        "a cast to a cell changes nothing"
    );
    assert_eq!(
        value("Vm.Intrinsic(\"example_address\", 1, 1)[0]"),
        Expr::Call {
            callee: Callee::Intrinsic {
                name: "example_address".to_owned(),
                ty: None,
                wrapper: None
            },
            args: vec![Expr::Const(Const::i32(1))],
            returns: Some(1),
        }
    );
    assert_eq!(
        value("Vm.NewObject(new VmType(\"a\\u003Cb\\u003E\"))"),
        Expr::NewObject(VmType {
            name: "a<b>".to_owned(),
            byte_size: 0,
            id: 0,
            flags: 0
        })
    );
    assert_eq!(
        value("func_0000A0(GuidEmpty[1])"),
        Expr::Call {
            callee: Callee::Local {
                at: 0xA0,
                name: "func_0000A0".to_owned()
            },
            args: vec![Expr::Cell {
                base: Box::new(Expr::GuidEmpty),
                index: 1
            }],
            returns: None,
        }
    );
}

#[test]
fn a_call_through_a_declaration_carries_what_it_reaches() {
    let functions = sample_functions();
    let mut calls = Vec::new();
    tree::each_expr(&functions[0].body, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Call {
                callee, returns, ..
            } = e
            {
                calls.push((callee.path(), *returns));
            }
        });
    });
    let path = |text: &str| Some(text.to_owned());
    assert_eq!(
        calls,
        [
            (path("Virtual.type_00000000000000AA.Method_3"), Some(0)),
            (path("Imports.shared.func_0000A0"), Some(2)),
            (path("Virtual.type_00000000000000AA.Method_7"), Some(1)),
            (path("Native.DEMO.DO_THING"), Some(0)),
            (path("Native._0x00000000000000BB"), Some(0)),
            (path("Builtins.Example.Thing.Ctor"), Some(0)),
            (
                path("Imports.module_0x00000000000000CC.func_0000B0"),
                Some(0)
            ),
            (None, Some(0)),
            (None, Some(0)),
            (path("Operations.example_set__4_0"), Some(0)),
            (None, None),
        ]
    );
    let array = value_in(&functions[1], Var::Local(0));
    assert_eq!(
        array,
        Some(&Expr::NewArray {
            ty: VmType {
                name: "type_00000000000000EE".to_owned(),
                byte_size: 8,
                id: 21,
                flags: 0x102
            },
            len: Box::new(Expr::Const(Const::i32(9))),
            wrapper: "NewArray_type_00000000000000EE".to_owned(),
        })
    );
    let cast = value_in(
        &functions[1],
        Var::Value {
            at: 0x104,
            index: 0,
        },
    );
    assert!(
        matches!(cast, Some(Expr::Call { callee: Callee::Intrinsic { name, .. }, .. }) if name == "example_cast<type_00000000000000DD>"),
        "{cast:?}"
    );
}

/// The value the first assignment to `var` at the top of a function's body gives it.
fn value_in(f: &tree::Function, var: Var) -> Option<&Expr> {
    f.body.iter().find_map(|stmt| match stmt {
        tree::Stmt::Simple(Stmt::Assign { target, value }) if *target == var => Some(value),
        _ => None,
    })
}

#[test]
fn a_step_is_an_assignment_of_the_stepped_low_bits() {
    let step_of = |var: Var, op: BinOp| {
        tree::Stmt::Simple(Stmt::Assign {
            target: var,
            value: Expr::binary(
                op,
                Expr::cast(Scalar::I32, Expr::Var(var)),
                Expr::Const(Const::i32(1)),
            ),
        })
    };
    let body = read("        local_0++;\n        p0--;\n        local_1 = p0++;").expect("reads");
    assert_eq!(
        body,
        [
            step_of(Var::Local(0), BinOp::Add),
            step_of(Var::Param(0), BinOp::Sub),
            tree::Stmt::Simple(Stmt::Assign {
                target: Var::Local(1),
                value: Expr::Var(Var::Param(0)),
            }),
            step_of(Var::Param(0), BinOp::Add),
        ]
    );
    assert_eq!(
        read("        local_0 = as_i32(local_0) + 1;").expect("reads"),
        [step_of(Var::Local(0), BinOp::Add)],
        "the spelled-out form is the same statement"
    );
    assert!(
        read("        p0 = p0++;").is_err(),
        "a variable stepped into itself keeps its old value, which two statements would not"
    );
}

#[test]
fn control_flow_reads_as_a_tree() {
    let functions = sample_functions();
    let body = &functions[2].body;
    let [tree::Stmt::Simple(_), tree::Stmt::While { cond, body: looped }, tree::Stmt::Label(label), tree::Stmt::Return(Some(Expr::Cells(cells))), tree::Stmt::Unreachable] =
        body.as_slice()
    else {
        panic!("not an assignment, a loop, a label, a return and the throw: {body:?}");
    };
    assert_eq!(*cond, Expr::Truth(Box::new(Expr::Const(Const::bool(true)))));
    assert_eq!(*label, Label::At(0x280));
    assert_eq!(cells.len(), 2);
    let [tree::Stmt::If { then, els, .. }, tree::Stmt::Simple(_), tree::Stmt::Switch { cases, .. }, tree::Stmt::Simple(_)] =
        looped.as_slice()
    else {
        panic!("not a test, a call, a switch and a step: {looped:?}");
    };
    assert_eq!(
        (then.as_slice(), els.len()),
        ([tree::Stmt::Break].as_slice(), 0)
    );
    let labels: Vec<&[Option<i64>]> = cases.iter().map(|c| c.labels.as_slice()).collect();
    let expected: [&[Option<i64>]; 3] = [&[Some(0), Some(2)], &[Some(1)], &[None]];
    assert_eq!(labels, expected);
    assert_eq!(cases[1].body, [tree::Stmt::Goto(Label::At(0x280))]);
    assert_eq!(cases[2].body.last(), Some(&tree::Stmt::Continue));
}

#[test]
fn a_malformed_listing_is_refused_with_its_line() {
    let line_of = |body: &str| read(body).expect_err("the body is refused").line();
    assert_eq!(
        line_of("        local_0 = 1;\n        local_1 = bogus(2);"),
        13
    );
    assert_eq!(line_of("        local_0 = (1;"), 12);
    assert_eq!(line_of("        local_0 = load_u32(p0, 0x0);"), 12);
    assert_eq!(line_of("        local_0 = 1\n        local_1 = 2;"), 13);
    assert_eq!(line_of("        local_0 = 12x;"), 12);
    assert_eq!(line_of("        1 = local_0;"), 12);
    assert_eq!(line_of("        local_0;"), 12);
    assert_eq!(line_of("        (local_0, local_1) = (1);"), 12);
    assert_eq!(line_of("        goto somewhere;"), 12);
    assert_eq!(line_of("        local_0 = \"open;"), 12);
    assert_eq!(
        line_of("        if (p0)\n        {\n            local_0 = 1;"),
        17,
        "a block left open takes the function's brace, and the listing then ends a brace short"
    );
    assert_eq!(
        line_of("        local_0 = 1;\n        }"),
        15,
        "a brace too many closes the program class early"
    );
    assert_eq!(
        line_of("        local_0 = ((as_i32(1) << unchecked((int)(1.5f))));"),
        12,
        "an `(int)` cast of a float is no reinterpretation"
    );
    assert_eq!(
        read("        local_0 = 1;\n\n        Native.DEMO.MISSING(1);"),
        Err(ListingError::Undeclared {
            line: 14,
            path: "Native.DEMO.MISSING".to_owned()
        })
    );
}

#[test]
fn a_malformed_head_or_declaration_is_refused() {
    let no_program = Listing::open("public static class Program_00000001\n{\n}\n");
    assert_eq!(no_program.expect_err("no program line").line(), 2);
    let stray = Listing::open("[VmProgram(0x00000001U)]\nint x;\n");
    assert_eq!(stray.expect_err("a line no head holds").line(), 2);

    let with = |declaration: &str| {
        let text = format!(
            "[VmProgram(0x00000001U)]\npublic static class Program_00000001\n{{\n    public static class Native\n    {{\n        {declaration}\n    }}\n}}\n"
        );
        Listing::open(&text).map(|listing| listing.declarations().len())
    };
    assert_eq!(
        with("public static void A(VmValue p0) => Vm.Native(0x1UL, 0x2U, 0, p0);"),
        Ok(1)
    );
    let refused = [
        // One cell that is not opened with `[0]`.
        "public static VmValue A(VmValue p0) => Vm.Native(0x1UL, 0x2U, 1, p0);",
        // Declared to yield nothing, and yielding a cell.
        "public static void A(VmValue p0) => Vm.Native(0x1UL, 0x2U, 1, p0)[0];",
        // Arguments out of order.
        "public static void A(VmValue p0, VmValue p1) => Vm.Native(0x1UL, 0x2U, 0, p1, p0);",
        // A script call in the natives' class.
        "public static void A(VmValue p0) => Vm.Script(0x1U, 0x2, 0, p0);",
    ];
    for declaration in refused {
        let error = with(declaration).expect_err(declaration);
        assert_eq!(error.line(), 6, "{declaration}");
    }
}
