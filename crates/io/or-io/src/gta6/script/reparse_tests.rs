use std::collections::BTreeMap;

use super::*;
use crate::gta6::script::ir::{Returns, TypeKey, VmType};
use crate::gta6::script::listing::Listing;
use crate::gta6::script::tree::{Arm, Case, Catch, Group, Guard, Label, Sugar};
use crate::gta6::script::{print, sample, Stats};

fn function(body: Vec<tree::Stmt>) -> tree::Function {
    let head = Head {
        value_types: Default::default(),
        program: 0x1122_3344,
        at: 0x40,
        name: "func_000040".to_owned(),
        returns: Returns::Cell,
        params: 2,
        locals: 4,
        vars: BTreeMap::new(),
        incomplete: None,
    };
    tree::Function { head, body }
}

fn written(f: &tree::Function) -> String {
    let mut out = String::new();
    print::function(&mut out, f, &Names::default());
    out
}

/// Asserts that what the printer writes of `body` reads back as `body`.
fn reads_back(body: Vec<tree::Stmt>) {
    let f = function(body);
    let text = written(&f);
    if let Err(why) = agrees(&f, &text) {
        panic!("{why}\nin:\n{text}");
    }
}

/// `lines` as the body of the function [`function`] makes.
fn text_of(lines: &str) -> String {
    format!("var func_000040(var p0, var p1)\n{{\n{lines}}}\n")
}

/// Whether text written by hand says the same as `body`; a text that does not read is a failure of the test.
fn says(body: Vec<tree::Stmt>, lines: &str) -> bool {
    match agrees(&function(body), &text_of(lines)) {
        Ok(()) => true,
        Err(ReparseError::Differs { .. }) => false,
        Err(other) => panic!("{other}\nin:\n{lines}"),
    }
}

fn p(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn int(value: i32) -> Expr {
    Expr::Const(Const::i32(value))
}

fn constant_of(bits: u64, kind: Kind) -> Expr {
    Expr::Const(Const { bits, kind })
}

fn op(op: BinOp, l: Expr, r: Expr) -> Expr {
    Expr::binary(op, l, r)
}

fn boxed(e: Expr) -> Box<Expr> {
    Box::new(e)
}

fn native(name: &str, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Native {
            namespace: Some("DEMO".to_owned()),
            name: name.to_owned(),
            hash: 0xA1,
            rva: 0,
        },
        args,
        returns: Some(1),
    }
}

fn intrinsic(name: &str, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Intrinsic {
            name: name.to_owned(),
            ty: None,
            wrapper: None,
        },
        args,
        returns: Some(1),
    }
}

fn method(args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Virtual {
            class: "type_00000000000000AA".to_owned(),
            method: "Method_3".to_owned(),
            ty: "type_00000000000000AA".to_owned(),
            slot: 3,
        },
        args,
        returns: Some(1),
    }
}

fn builtin(path: &str, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Builtin {
            path: path.to_owned(),
            ty: 3,
            method: 1,
        },
        args,
        returns: Some(1),
    }
}

fn a_type() -> VmType {
    VmType {
        name: "type_0123456789ABCDEF".to_owned(),
        byte_size: 48,
        id: 7,
        flags: 0,
    }
}

fn simple(s: Stmt) -> tree::Stmt {
    tree::Stmt::Simple(s)
}

fn set(target: Var, value: Expr) -> Stmt {
    Stmt::Assign { target, value }
}

/// A call that takes `e`, so that `e` is written where a value goes.
fn used(e: Expr) -> tree::Stmt {
    simple(Stmt::Eval(native("USE", vec![e])))
}

fn sugared(kind: ExprSugar, e: Expr) -> tree::Stmt {
    used(Expr::sugar(kind, e))
}

fn construct(sugar: Sugar) -> tree::Stmt {
    tree::Stmt::Sugar(Box::new(sugar))
}

#[test]
fn every_function_of_the_sample_reads_back_as_it_was_written() {
    let listing = Listing::open(sample::LISTING).expect("the sample opens");
    let names = Names::default();
    let mut stats = Stats::default();
    let mut source = String::new();
    for read in listing.functions() {
        let read = read.expect("the function reads");
        let done = crate::gta6::script::function(read, &names, &mut stats);
        let done = done.expect("the function decompiles");
        let text = written(&done);
        if let Err(why) = agrees(&done, &text) {
            panic!("{}: {why}\nin:\n{text}", done.head.name);
        }
        source.push_str(&text);
    }
    assert!(parse(&source).is_ok(), "{:?}", parse(&source));
}

#[test]
fn every_simple_statement_reads_back() {
    let store = |kind, offset, value| {
        simple(Stmt::Store {
            kind,
            addr: p(0),
            offset,
            value,
        })
    };
    reads_back(vec![
        simple(set(Var::Local(0), op(BinOp::Add, p(0), int(1)))),
        simple(Stmt::AssignMany {
            targets: vec![Var::Local(1), Var::Local(2)],
            values: vec![p(0), p(1)],
        }),
        store(StoreKind::U8, 0x10, p(1)),
        store(StoreKind::U16, 0x12, int(0x1_00FF)),
        store(StoreKind::I32, 0x14, p(1)),
        store(StoreKind::U64, 0x18, int(5)),
        store(StoreKind::Ref, 0x20, p(1)),
        store(StoreKind::U64, 0, p(1)),
        simple(Stmt::StoreBytes {
            addr: p(0),
            offset: 0x8,
            bytes: 12,
            value: Expr::Cells(vec![p(0), p(1)]),
        }),
        simple(Stmt::StoreBytes {
            addr: p(0),
            offset: 0x30,
            bytes: 16,
            value: Expr::Cells(vec![p(0), p(1)]),
        }),
        simple(Stmt::StoreF32Slots {
            addr: p(0),
            offset: 0,
            value: Expr::Cells(vec![p(0), p(1)]),
        }),
        simple(Stmt::ZeroBytes {
            addr: p(0),
            bytes: 16,
        }),
        simple(Stmt::StepI32 {
            addr: p(0),
            offset: 0xC,
            down: false,
        }),
        simple(Stmt::StepI32 {
            addr: Expr::AddrOf {
                var: Var::Local(3),
                cell: 0,
            },
            offset: 0,
            down: true,
        }),
        simple(Stmt::Eval(intrinsic(
            "array_set_u64",
            vec![p(0), p(1), local(0)],
        ))),
        simple(Stmt::Eval(native("USE", vec![p(0)]))),
    ]);
}

#[test]
fn every_expression_reads_back() {
    let not = |e| Expr::Unary {
        op: UnOp::Not,
        e: boxed(e),
    };
    let load = |ty, addr, offset| Expr::Load {
        ty,
        addr: boxed(addr),
        offset,
    };
    let address = |cell| Expr::AddrOf {
        var: Var::Local(1),
        cell,
    };
    let by = |offset| Expr::ByteOffset {
        addr: boxed(p(0)),
        offset: boxed(offset),
    };
    let less = || op(BinOp::Lt, p(0), p(1));
    reads_back(vec![
        // Constants, by the kind that spells them.
        used(int(5)),
        used(int(-7)),
        used(constant_of(0x3FC0_0000, Kind::F32)),
        used(constant_of(1.25f64.to_bits(), Kind::F64)),
        used(Expr::Const(Const::bool(true))),
        used(constant_of(0, Kind::Pointer)),
        used(constant_of(0xFFFF_FFFF_8E7B_3F2C, Kind::U64)),
        used(constant_of(0x1_FFFF_FFFF, Kind::Unknown)),
        used(constant_of(0x8E7B_3F2C, Kind::Hash)),
        // Values and the readings of them.
        used(Expr::GuidEmpty),
        used(Expr::Cell {
            base: boxed(local(2)),
            index: 1,
        }),
        used(Expr::Cells(vec![p(0), p(1)])),
        used(Expr::cast(Scalar::I32, p(0))),
        used(Expr::cast(Scalar::F32, p(0))),
        used(Expr::cast(Scalar::U8, int(0x1FF))),
        used(Expr::Truth(boxed(p(0)))),
        used(not(p(0))),
        used(not(less())),
        used(Expr::Unary {
            op: UnOp::Neg,
            e: boxed(p(0)),
        }),
        used(Expr::Unary {
            op: UnOp::BitNot,
            e: boxed(p(0)),
        }),
        // Operators, where the parentheses are the tree.
        used(op(BinOp::Sub, p(0), op(BinOp::Sub, p(1), p(0)))),
        used(op(BinOp::Mul, op(BinOp::Add, p(0), p(1)), int(-2))),
        used(op(
            BinOp::LogicalAnd,
            less(),
            op(
                BinOp::LogicalOr,
                op(BinOp::Eq, p(0), int(3)),
                Expr::Truth(boxed(p(1))),
            ),
        )),
        used(Expr::Select {
            cond: boxed(less()),
            then: boxed(int(1)),
            els: boxed(int(2)),
        }),
        // Calls, and the intrinsics C# has an operator for.
        used(Expr::Call {
            callee: Callee::Local {
                at: 0x100,
                name: "func_000100".to_owned(),
            },
            args: vec![p(0)],
            returns: None,
        }),
        used(Expr::Call {
            callee: Callee::Import {
                module: "shared".to_owned(),
                name: "func_0000A0".to_owned(),
                program: 0x0A0B_0C0D,
                at: 0xA0,
            },
            args: vec![p(0)],
            returns: Some(2),
        }),
        used(builtin("System.Thing.Get", vec![p(0)])),
        used(method(vec![p(0), p(1)])),
        used(intrinsic("clamp_f32", vec![p(0), int(0), int(1)])),
        used(intrinsic("array_length", vec![p(0)])),
        used(intrinsic("array_get_i32", vec![p(0), p(1)])),
        used(intrinsic("array_get_u64", vec![p(0), p(1)])),
        used(intrinsic("array_element_address", vec![p(0), p(1)])),
        used(intrinsic("invoke_delegate", vec![p(0), p(1)])),
        used(intrinsic("try_cast<type_00000000000000AA>", vec![p(0)])),
        used(intrinsic("box<type_00000000000000AA>", vec![p(0)])),
        // Memory.
        used(load(Scalar::I32, p(0), 0x10)),
        used(load(Scalar::U64, p(0), 0)),
        used(load(Scalar::U64, address(0), 8)),
        used(load(Scalar::U64, address(0), 0)),
        used(load(Scalar::U64, by(int(8)), 0x10)),
        used(load(Scalar::U8, load(Scalar::U64, p(1), 8), 0x10)),
        used(Expr::LoadBytes {
            addr: boxed(p(0)),
            offset: 0x20,
            bytes: 16,
        }),
        used(Expr::LoadBytes {
            addr: boxed(p(0)),
            offset: 0x20,
            bytes: 12,
        }),
        used(Expr::ExtractBytes {
            value: boxed(Expr::Cells(vec![p(0), p(1)])),
            offset: 4,
            bytes: 8,
        }),
        used(by(int(8))),
        used(by(p(1))),
        used(address(0)),
        used(address(2)),
        // What is made, and what is referred to.
        used(Expr::ZeroSlots(2)),
        used(Expr::Vector3(Box::new([p(0), p(1), p(0)]))),
        used(Expr::NewObject(a_type())),
        used(Expr::NewArray {
            ty: a_type(),
            len: boxed(p(0)),
            wrapper: "new_things".to_owned(),
        }),
        used(Expr::FunctionRef {
            program: 0x1122_3344,
            at: 0x100,
        }),
        used(Expr::FunctionRef {
            program: 0x0A0B_0C0D,
            at: 0x100,
        }),
        used(Expr::BuiltinFunctionRef { ty: 2, method: 1 }),
        used(Expr::VirtualFunctionRef {
            receiver: boxed(p(0)),
            ty: a_type(),
            slot: 4,
        }),
        used(Expr::StringConstant {
            program: 0x1122_3344,
            offset: 0x1F,
        }),
        used(Expr::BuiltinGlobal(3)),
        used(Expr::Var(Var::Global {
            program: 0x0A0B_0C0D,
            offset: 0x1230,
            statics: true,
        })),
    ]);
}

#[test]
fn every_statement_that_nests_reads_back() {
    let test = || Expr::Truth(boxed(p(0)));
    let one = |n| vec![used(int(n))];
    reads_back(vec![
        tree::Stmt::If {
            cond: test(),
            then: one(1),
            els: vec![tree::Stmt::If {
                cond: op(BinOp::Eq, p(1), int(4)),
                then: one(2),
                els: one(3),
            }],
        },
        tree::Stmt::If {
            cond: test(),
            then: Vec::new(),
            els: vec![used(int(4)), tree::Stmt::Break],
        },
        tree::Stmt::While {
            cond: test(),
            body: vec![used(int(5)), tree::Stmt::Continue],
        },
        tree::Stmt::DoWhile {
            body: one(6),
            cond: op(BinOp::Lt, p(0), p(1)),
        },
        tree::Stmt::For {
            init: Some(Box::new(set(Var::Local(0), int(0)))),
            cond: Some(op(BinOp::Lt, local(0), p(0))),
            step: Some(Box::new(set(
                Var::Local(0),
                op(BinOp::Add, local(0), int(1)),
            ))),
            body: one(7),
        },
        tree::Stmt::For {
            init: None,
            cond: None,
            step: None,
            body: vec![tree::Stmt::Break],
        },
        tree::Stmt::Switch {
            value: p(0),
            cases: vec![
                Case {
                    labels: vec![Some(1), Some(-3)],
                    body: vec![used(int(8)), tree::Stmt::Break],
                },
                Case {
                    labels: vec![Some(2)],
                    body: Vec::new(),
                },
                Case {
                    labels: vec![None],
                    body: vec![tree::Stmt::Return(None)],
                },
            ],
        },
        tree::Stmt::Goto(Label::At(0x40)),
        tree::Stmt::Label(Label::At(0x40)),
        tree::Stmt::Label(Label::Block(3)),
        tree::Stmt::Return(Some(p(0))),
        tree::Stmt::Unreachable,
    ]);
}

#[test]
fn every_source_construct_reads_back() {
    let got = |n| native("GET", vec![p(n)]);
    let ctor = builtin("type_0123456789ABCDEF.Ctor", vec![local(0), p(1)]);
    reads_back(vec![
        construct(Sugar::Hidden(vec![used(int(1))])),
        construct(Sugar::Group {
            kind: Group::Compound,
            parts: vec![set(Var::Local(1), op(BinOp::Add, local(1), p(0)))],
        }),
        construct(Sugar::Await {
            before: vec![set(Var::Local(1), got(0))],
            ready: native("READY", vec![local(1)]),
            suspend: vec![tree::Stmt::Return(None)],
            resumed: Label::At(0x50),
            resume: Vec::new(),
            after: vec![set(Var::Local(2), native("RESULT", vec![local(1)]))],
        }),
        construct(Sugar::Yield {
            value: vec![set(Var::Local(2), p(0))],
            suspend: vec![tree::Stmt::Return(None)],
            resumed: Label::At(0x60),
            resume: Vec::new(),
        }),
        construct(Sugar::Foreach {
            setup: vec![set(Var::Local(1), got(1))],
            cond: native("NEXT", vec![local(1)]),
            item: Box::new(set(Var::Local(2), native("CURRENT", vec![local(1)]))),
            body: vec![used(local(2))],
            step: None,
            after: vec![used(int(2))],
        }),
        construct(Sugar::Guarded {
            kind: Guard::Using,
            enter: vec![set(Var::Local(1), got(0))],
            body: vec![used(local(1))],
            exit: vec![used(int(3))],
        }),
        construct(Sugar::Guarded {
            kind: Guard::Lock,
            enter: vec![Stmt::Eval(native("ENTER", vec![p(0)]))],
            body: vec![used(int(4))],
            exit: Vec::new(),
        }),
        construct(Sugar::Try {
            body: vec![used(int(5))],
            catches: vec![
                Catch {
                    ty: Some(TypeKey(0x8000_0005)),
                    var: Some(Var::Local(3)),
                    body: vec![used(local(3))],
                },
                Catch {
                    ty: None,
                    var: None,
                    body: Vec::new(),
                },
            ],
            finally: vec![used(int(6))],
        }),
        construct(Sugar::SwitchOn {
            arms: vec![Arm {
                test: Expr::Truth(boxed(p(0))),
                body: vec![used(int(7))],
            }],
            default: vec![used(int(8))],
        }),
        // An object made and constructed side by side is one statement of the text.
        simple(set(Var::Local(0), Expr::NewObject(a_type()))),
        simple(Stmt::Eval(ctor)),
        sugared(ExprSugar::Method, method(vec![p(0), p(1)])),
        sugared(
            ExprSugar::Property,
            builtin("System.Thing.get_Size", vec![p(0)]),
        ),
        sugared(
            ExprSugar::Field {
                owner: TypeKey(0x8000_0005),
            },
            Expr::Load {
                ty: Scalar::U64,
                addr: boxed(p(0)),
                offset: 0x10,
            },
        ),
        sugared(
            ExprSugar::Lambda,
            Expr::FunctionRef {
                program: 0x1122_3344,
                at: 0x100,
            },
        ),
        sugared(
            ExprSugar::Coalesce,
            Expr::Select {
                cond: boxed(Expr::Truth(boxed(p(0)))),
                then: boxed(p(0)),
                els: boxed(p(1)),
            },
        ),
        sugared(
            ExprSugar::Concat,
            builtin("System.String.Concat", vec![p(0), p(1), p(0)]),
        ),
        sugared(
            ExprSugar::Out,
            Expr::AddrOf {
                var: Var::Local(1),
                cell: 0,
            },
        ),
        sugared(ExprSugar::Tuple, Expr::Cells(vec![p(0), p(1)])),
        sugared(
            ExprSugar::Operator,
            builtin("System.Thing.op_Addition", vec![p(0), p(1)]),
        ),
        construct(Sugar::Throw(Box::new(Stmt::Eval(native(
            "THROW",
            vec![p(0)],
        ))))),
    ]);
}

#[test]
fn a_text_that_states_another_tree_is_told_apart() {
    let nested = || {
        vec![tree::Stmt::Return(Some(op(
            BinOp::Sub,
            p(0),
            op(BinOp::Sub, p(1), p(0)),
        )))]
    };
    // The parentheses are the tree: without them the text subtracts in another order.
    assert!(says(nested(), "    return p0 - (p1 - p0);\n"));
    assert!(!says(nested(), "    return p0 - p1 - p0;\n"));

    let call = || vec![simple(Stmt::Eval(native("USE2", vec![p(0), int(5)])))];
    assert!(says(call(), "    DEMO.USE2(p0, 5);\n"));
    assert!(!says(call(), "    DEMO.USE2(p0, 6);\n"));
    assert!(!says(call(), "    DEMO.USE2(5, p0);\n"));
    assert!(!says(call(), "    DEMO.USE(p0, 5);\n"));
    assert!(!says(call(), "    DEMO.USE2(p0);\n"));

    let store = || {
        vec![simple(Stmt::Store {
            kind: StoreKind::U64,
            addr: p(0),
            offset: 0x10,
            value: p(1),
        })]
    };
    assert!(says(store(), "    p0->f_10 = p1;\n"));
    assert!(!says(store(), "    p0->f_18 = p1;\n"));
    // `.` stays in the variable's own cells and `->` goes through the pointer it holds.
    assert!(!says(store(), "    p0.f_10 = p1;\n"));
    assert!(!says(store(), "    p1->f_10 = p0;\n"));
}

#[test]
fn a_statement_dropped_added_or_moved_out_of_its_block_is_told_apart() {
    let body = || {
        vec![
            tree::Stmt::If {
                cond: Expr::Truth(boxed(p(0))),
                then: vec![used(int(1))],
                els: Vec::new(),
            },
            used(int(2)),
            tree::Stmt::Goto(Label::At(0x40)),
        ]
    };
    let open = "    if (p0)\n    {\n        DEMO.USE(1);\n";
    let written = format!("{open}    }}\n    DEMO.USE(2);\n    goto label_000040;\n");
    assert!(says(body(), &written));
    let moved = format!("{open}        DEMO.USE(2);\n    }}\n    goto label_000040;\n");
    assert!(!says(body(), &moved));
    let dropped = format!("{open}    }}\n    goto label_000040;\n");
    assert!(!says(body(), &dropped));
    let added =
        format!("{open}    }}\n    DEMO.USE(2);\n    DEMO.USE(2);\n    goto label_000040;\n");
    assert!(!says(body(), &added));
    let elsewhere = format!("{open}    }}\n    DEMO.USE(2);\n    goto label_000048;\n");
    assert!(!says(body(), &elsewhere));
    let negated = written.replace("if (p0)", "if (!p0)");
    assert!(!says(body(), &negated));
}

#[test]
fn a_reading_is_not_what_the_text_is_held_to() {
    // ⚠ The stated limit of the check: a cast, a test against zero and a widening are no operand.
    let read = || vec![tree::Stmt::Return(Some(Expr::cast(Scalar::I32, p(0))))];
    assert!(says(read(), "    return (int)p0;\n"));
    assert!(says(read(), "    return p0;\n"));
    let hash = || vec![used(constant_of(0xFFFF_FFFF_8E7B_3F2C, Kind::I64))];
    assert!(says(hash(), "    DEMO.USE((int)0x8E7B3F2C);\n"));
    assert!(says(hash(), "    DEMO.USE(0x8E7B3F2C);\n"));
    assert!(!says(hash(), "    DEMO.USE(0x8E7B3F2D);\n"));
}

#[test]
fn a_declaration_states_no_statement() {
    let text = "var f(var p0)\n{\n    var a, b;\n    DEMO.USE(1);\n    int c = 2;\n}\n";
    assert_eq!(parse(text), Ok(2));
    assert_eq!(parse("// only a comment\n"), Ok(0));
}

#[test]
fn text_the_printer_does_not_write_is_refused_with_its_line() {
    let refused = |lines: &str| match parse(&text_of(lines)) {
        Err(ReparseError::Syntax { line, .. }) => line,
        other => panic!("not refused: {other:?}\nin:\n{lines}"),
    };
    // The body starts on line 3 of the text.
    assert_eq!(refused("    # \n"), 3);
    assert_eq!(refused("    DEMO.USE(1);\n    goto ;\n"), 4);
    refused("    return p0\n");
    refused("    return (var[2]p0;\n");
    refused("    DEMO.USE(\"no end);\n");
    refused("    p0->size = 1;\n");
    refused("    if (p0)\n    {\n");
    refused("    case 1:\n");
    refused("    return 99999999999999999999;\n");
    assert!(matches!(
        parse("var f(\n"),
        Err(ReparseError::Syntax { .. })
    ));
}

#[test]
fn text_nested_past_the_limit_is_refused_and_not_followed() {
    // A stack of a stated size: the limit bounds the recursion, the build says what a level costs.
    let deep = |open: &str, close: &str, levels: usize| {
        let lines = format!(
            "    DEMO.USE({}p0{});\n",
            open.repeat(levels),
            close.repeat(levels)
        );
        let reader = std::thread::Builder::new().stack_size(64 << 20);
        let read = reader.spawn(move || parse(&text_of(&lines)));
        read.expect("a thread starts").join().expect("no panic")
    };
    assert_eq!(deep("(", ")", 40), Ok(1));
    assert_eq!(deep("!", "", 40), Ok(1));
    // `- ` with its space: two signs side by side are the one mark `--`.
    for (open, close) in [
        ("(", ")"),
        ("!", ""),
        ("- ", ""),
        ("(int)", ""),
        ("USE(", ")"),
    ] {
        let refused = deep(open, close, 100_000);
        assert!(
            matches!(&refused, Err(ReparseError::Syntax { what, .. }) if what.contains("deeper")),
            "{open}: {refused:?}"
        );
    }
    let blocks = format!(
        "{}{}",
        "    if (p0)\n    {\n".repeat(1000),
        "    }\n".repeat(1000)
    );
    let reader = std::thread::Builder::new().stack_size(64 << 20);
    let read = reader.spawn(move || parse(&text_of(&blocks)));
    let refused = read.expect("a thread starts").join().expect("no panic");
    assert!(
        matches!(refused, Err(ReparseError::Syntax { .. })),
        "{refused:?}"
    );
}

/// `links` calls of one slot, each on the result of the one before, the first on `p0`.
fn chain(links: usize) -> Expr {
    (0..links).fold(p(0), |receiver, _| method(vec![receiver, p(1)]))
}

#[test]
fn a_chain_of_calls_on_the_result_of_the_one_before_is_held_once() {
    // Sixty links are a few hundred operands. Held in both the forms a slot's call is written
    // in, each link would double what the links under it come to, and the bound would be passed.
    let f = function(vec![used(chain(60))]);
    let text = written(&f);
    assert_eq!(agrees_within(&f, &text, 4096), Ok(()));
    // The other form, a link at a time, reads as the same calls.
    let dotted = format!("    DEMO.USE(p0{});\n", ".Method_3(p1)".repeat(60));
    assert_eq!(agrees_within(&f, &text_of(&dotted), 4096), Ok(()));
    let shorter = format!("    DEMO.USE(p0{});\n", ".Method_3(p1)".repeat(59));
    assert!(!says(vec![used(chain(60))], &shorter));
}

#[test]
fn a_call_of_a_slot_reads_by_its_name_or_on_its_receiver() {
    let call = || vec![used(method(vec![p(0), p(1)]))];
    assert!(says(
        call(),
        "    DEMO.USE(type_00000000000000AA.Method_3(p0, p1));\n"
    ));
    assert!(says(call(), "    DEMO.USE(p0.Method_3(p1));\n"));
    assert!(!says(call(), "    DEMO.USE(p0.Method_4(p1));\n"));
    assert!(!says(call(), "    DEMO.USE(p1.Method_3(p1));\n"));
    assert!(!says(call(), "    DEMO.USE(p0.Method_3(p0));\n"));
    assert!(!says(call(), "    DEMO.USE(p0.Method_3(p0, p1));\n"));
    assert!(!says(
        call(),
        "    DEMO.USE(type_00000000000000AA.Method_3(p1));\n"
    ));
    // A receiver that is no name: the call it is the result of.
    let on_a_result = || vec![used(method(vec![native("GET", vec![p(0)]), p(1)]))];
    assert!(says(
        on_a_result(),
        "    DEMO.USE(DEMO.GET(p0).Method_3(p1));\n"
    ));
    assert!(!says(
        on_a_result(),
        "    DEMO.USE(DEMO.GET(p1).Method_3(p1));\n"
    ));
}

#[test]
fn an_equality_of_two_values_reads_by_its_name_or_with_its_sign() {
    let equal = || builtin("System.Guid.op_Equality", vec![p(0), p(1)]);
    let used_equal = || vec![used(equal())];
    assert!(says(
        used_equal(),
        "    DEMO.USE(System.Guid.op_Equality(p0, p1));\n"
    ));
    assert!(says(used_equal(), "    DEMO.USE(p0 == p1);\n"));
    assert!(!says(used_equal(), "    DEMO.USE(p0 != p1);\n"));
    assert!(!says(used_equal(), "    DEMO.USE(p1 == p0);\n"));
    assert!(!says(
        used_equal(),
        "    DEMO.USE(!System.Guid.op_Equality(p0, p1));\n"
    ));
    // Under a `!` the sign is the opposite one, and the call by name keeps the `!`.
    let not_equal = || {
        vec![used(Expr::Unary {
            op: UnOp::Not,
            e: boxed(equal()),
        })]
    };
    assert!(says(not_equal(), "    DEMO.USE(p0 != p1);\n"));
    assert!(says(
        not_equal(),
        "    DEMO.USE(!System.Guid.op_Equality(p0, p1));\n"
    ));
    assert!(!says(not_equal(), "    DEMO.USE(p0 == p1);\n"));
    assert!(!says(
        not_equal(),
        "    DEMO.USE(System.Guid.op_Equality(p0, p1));\n"
    ));
}

#[test]
fn a_function_of_more_than_the_bound_is_not_compared() {
    let body = || vec![used(int(1)), used(int(2)), used(int(3))];
    let f = function(body());
    let text = written(&f);
    assert_eq!(agrees_within(&f, &text, 64), Ok(()));
    // Three statements of a call and its operand each are more than four of both.
    let refused = agrees_within(&f, &text, 4);
    assert_eq!(refused, Err(ReparseError::TooLarge { limit: 4 }));
    // The bound is one function's: two functions that each come to nine are read within twelve.
    let twice = format!("{text}{}", text.replace("func_000040", "func_000048"));
    assert_eq!(agrees_within(&f, &twice, 12), Ok(()));
}

#[test]
fn a_row_of_comparisons_is_read_once() {
    // ⚠ No clock is read: each `<` after a name is looked past for type arguments, and without
    // the bound on that look every comparison of the row reads the rest of the row.
    const ROW: usize = 40_000;
    let row = vec!["p0 < p1"; ROW].join(", ");
    assert_eq!(parse(&text_of(&format!("    DEMO.USE({row});\n"))), Ok(1));
    // Type arguments within the bound are still read as that.
    assert_eq!(parse(&text_of("    DEMO.USE(box<type_1>(p0));\n")), Ok(1));
}

/// Whether text written by hand as the source level writes `body`, every pass on, says the same as `body`; a form not read back yet is the error it is.
fn source_says(body: Vec<tree::Stmt>, lines: &str) -> Result<bool, ReparseError> {
    method_says(body, lines, false)
}

/// [`source_says`], of the text of a method whose receiver is its first parameter where `receiver` says so.
fn method_says(body: Vec<tree::Stmt>, lines: &str, receiver: bool) -> Result<bool, ReparseError> {
    let text = text_of(lines).replace("var", "Cell");
    match agrees_source(&function(body), &text, Options::default(), receiver) {
        Ok(()) => Ok(true),
        Err(ReparseError::Differs { .. }) => Ok(false),
        Err(other) => Err(other),
    }
}

/// The form `lines` holds that is not read back yet, where reading them back stopped at one.
fn unread_of(body: Vec<tree::Stmt>, lines: &str) -> Option<String> {
    match source_says(body, lines) {
        Err(ReparseError::Unread { form, .. }) => Some(form),
        Ok(_) => None,
        Err(other) => panic!("{other}\nin:\n{lines}"),
    }
}

fn compound(part: Stmt) -> tree::Stmt {
    construct(Sugar::Group {
        kind: Group::Compound,
        parts: vec![part],
    })
}

#[test]
fn a_number_spelled_with_its_float_beside_it_and_an_unchecked_int_read_as_their_bits() {
    let float = || vec![used(constant_of(0x3F80_0000, Kind::Unknown))];
    assert_eq!(
        source_says(float(), "    DEMO.USE(0x3F800000 /* 1.0f */);\n"),
        Ok(true)
    );
    assert_eq!(
        source_says(float(), "    DEMO.USE(0x3F800001 /* 1.0f */);\n"),
        Ok(false)
    );
    // A comment over lines leaves the lines after it counted.
    let refused = parse(&text_of("    /* one\n two */ DEMO.USE(1);\n    # \n"));
    assert!(
        matches!(refused, Err(ReparseError::Syntax { line: 5, .. })),
        "{refused:?}"
    );
    let hashed = || vec![used(int(0x89AB_CDEF_u32 as i32))];
    assert_eq!(
        source_says(hashed(), "    DEMO.USE(unchecked((int)0x89ABCDEF));\n"),
        Ok(true)
    );
}

#[test]
fn a_compound_assignment_reads_as_the_place_it_writes_worked_on() {
    let added = || vec![compound(set(Var::Local(1), op(BinOp::Add, local(1), p(0))))];
    assert_eq!(source_says(added(), "    local_1 += p0;\n"), Ok(true));
    assert_eq!(source_says(added(), "    local_1 -= p0;\n"), Ok(false));
    assert_eq!(source_says(added(), "    local_2 += p0;\n"), Ok(false));
    let stepped = || {
        vec![compound(set(
            Var::Local(1),
            op(BinOp::Add, local(1), int(1)),
        ))]
    };
    assert_eq!(source_says(stepped(), "    local_1++;\n"), Ok(true));
    assert_eq!(source_says(stepped(), "    local_1--;\n"), Ok(false));
    // A whole cell stored through a pointer it was loaded through.
    let load = Expr::Load {
        ty: Scalar::U64,
        addr: boxed(p(0)),
        offset: 0x10,
    };
    let flagged = || {
        vec![compound(Stmt::Store {
            kind: StoreKind::U64,
            addr: p(0),
            offset: 0x10,
            value: op(BinOp::Or, load.clone(), int(4)),
        })]
    };
    assert_eq!(source_says(flagged(), "    p0.f_10 |= 4;\n"), Ok(true));
    assert_eq!(source_says(flagged(), "    p0.f_18 |= 4;\n"), Ok(false));
    // The form of old writes the same group as the statement it holds.
    assert!(says(flagged(), "    p0->f_10 = p0->f_10 | 4;\n"));
}

#[test]
fn an_await_reads_as_what_its_awaiter_is_taken_from_and_what_it_yields() {
    let awaited = || {
        vec![construct(Sugar::Await {
            before: vec![set(Var::Local(1), native("GET", vec![p(0)]))],
            ready: native("READY", vec![local(1)]),
            suspend: vec![tree::Stmt::Return(None)],
            resumed: Label::At(0x50),
            resume: Vec::new(),
            after: vec![set(Var::Local(2), native("RESULT", vec![local(1)]))],
        })]
    };
    assert_eq!(
        source_says(awaited(), "    local_2 = await DEMO.GET(p0);\n"),
        Ok(true)
    );
    assert_eq!(
        source_says(awaited(), "    local_2 = await DEMO.GET(p1);\n"),
        Ok(false)
    );
    assert_eq!(
        source_says(awaited(), "    local_3 = await DEMO.GET(p0);\n"),
        Ok(false)
    );
    // The form of old writes the awaiter taken and what is asked of it after.
    let old =
        "    local_1 = DEMO.GET(p0);\n    await local_1;\n    local_2 = DEMO.RESULT(local_1);\n";
    assert!(says(awaited(), old));
}

#[test]
fn a_yield_reads_as_what_it_yields() {
    let yielded = || {
        vec![construct(Sugar::Yield {
            value: vec![set(Var::Local(2), p(1))],
            suspend: vec![tree::Stmt::Return(None)],
            resumed: Label::At(0x60),
            resume: Vec::new(),
        })]
    };
    assert_eq!(source_says(yielded(), "    yield return p1;\n"), Ok(true));
    assert_eq!(source_says(yielded(), "    yield return p0;\n"), Ok(false));
    assert_eq!(source_says(yielded(), "    yield return;\n"), Ok(false));
}

#[test]
fn this_and_base_are_the_receiver_and_their_fields_are_reached_through_it() {
    let stored = || {
        vec![simple(Stmt::Store {
            kind: StoreKind::U64,
            addr: p(0),
            offset: 0x10,
            value: p(1),
        })]
    };
    assert_eq!(
        method_says(stored(), "    this.f_10 = p1;\n", true),
        Ok(true)
    );
    assert_eq!(
        method_says(stored(), "    base.f_10 = p1;\n", true),
        Ok(true)
    );
    assert_eq!(
        method_says(stored(), "    this.f_18 = p1;\n", true),
        Ok(false)
    );
    // A method's `this` is no `p1`; and outside a method `this` is a name like any.
    let other = || {
        vec![simple(Stmt::Store {
            kind: StoreKind::U64,
            addr: p(1),
            offset: 0x10,
            value: p(1),
        })]
    };
    assert_eq!(
        method_says(other(), "    this.f_10 = p1;\n", true),
        Ok(false)
    );
    assert_eq!(
        method_says(stored(), "    this.f_10 = p1;\n", false),
        Ok(false)
    );
    // A call on the receiver is a call on the first parameter.
    let called = || vec![used(method(vec![p(0), p(1)]))];
    assert_eq!(
        method_says(called(), "    DEMO.USE(this.Method_3(p1));\n", true),
        Ok(true)
    );
}

#[test]
fn the_forms_of_true_csharp_that_say_the_same_read_as_it() {
    // A cast to a type no keyword names, `out` that declares its variable, `?.`, an operator.
    assert_eq!(
        source_says(vec![used(p(0))], "    DEMO.USE((Cells2)p0);\n"),
        Ok(true)
    );
    let out = || {
        let address = Expr::AddrOf {
            var: Var::Local(1),
            cell: 0,
        };
        vec![sugared(ExprSugar::Out, address)]
    };
    assert_eq!(
        source_says(out(), "    DEMO.USE(out Cell local_1);\n"),
        Ok(true)
    );
    assert_eq!(source_says(out(), "    DEMO.USE(out local_1);\n"), Ok(true));
    let asked = || {
        let field = Expr::Load {
            ty: Scalar::U64,
            addr: boxed(p(0)),
            offset: 0x18,
        };
        let nothing = Expr::Const(Const {
            bits: 0,
            kind: Kind::Unknown,
        });
        let select = Expr::Select {
            cond: boxed(Expr::Truth(boxed(p(0)))),
            then: boxed(field),
            els: boxed(nothing),
        };
        vec![sugared(ExprSugar::NullConditional, select)]
    };
    assert_eq!(source_says(asked(), "    DEMO.USE(p0?.f_18);\n"), Ok(true));
    assert_eq!(source_says(asked(), "    DEMO.USE(p1?.f_18);\n"), Ok(false));
    let added = || {
        vec![sugared(
            ExprSugar::Operator,
            builtin("System.Thing.op_Addition", vec![p(0), p(1)]),
        )]
    };
    assert_eq!(source_says(added(), "    DEMO.USE(p0 + p1);\n"), Ok(true));
    assert_eq!(source_says(added(), "    DEMO.USE(p0 - p1);\n"), Ok(false));
    // The control that never comes, said by the exception made for it.
    let never = || vec![tree::Stmt::Unreachable];
    let thrown = "    throw new System.Diagnostics.UnreachableException();\n";
    assert_eq!(source_says(never(), thrown), Ok(true));
    assert_eq!(source_says(vec![used(int(1))], thrown), Ok(false));
}

#[test]
fn the_token_of_a_type_reads_as_typeof_it() {
    let token = || {
        let call = Expr::Call {
            callee: Callee::Intrinsic {
                name: TYPE_TOKEN.to_owned(),
                ty: Some(a_type()),
                wrapper: None,
            },
            args: Vec::new(),
            returns: Some(1),
        };
        vec![used(call)]
    };
    assert!(says(
        token(),
        "    DEMO.USE(typeof(type_0123456789ABCDEF));\n"
    ));
    let handle = "    DEMO.USE(typeof(type_0123456789ABCDEF).TypeHandle);\n";
    assert_eq!(source_says(token(), handle), Ok(true));
    let bare = "    DEMO.USE(typeof(type_0123456789ABCDEF));\n";
    assert_eq!(source_says(token(), bare), Ok(false));
}

#[test]
fn a_form_not_read_back_yet_is_counted_by_name_and_not_compared() {
    let one = || vec![simple(set(Var::Local(0), p(0)))];
    let lambda = "    local_0 = (Cell p1) => DEMO.USE(p1);\n";
    assert_eq!(
        unread_of(one(), lambda).as_deref(),
        Some("a lambda written where it is bound")
    );
    let block = "    local_0 = (Cell p1) =>\n    {\n        DEMO.USE(p1);\n    };\n";
    assert_eq!(
        unread_of(one(), block).as_deref(),
        Some("a lambda written where it is bound")
    );
    // An initialiser of no fields by name, as a collection's is.
    let made = "    local_0 = new type_0123456789ABCDEF { 1, 0.2f };\n";
    assert_eq!(
        unread_of(one(), made).as_deref(),
        Some("an object written with its fields")
    );
    let cached = "    local_0 = Global_28 ??= Global_20.func_000100;\n";
    assert_eq!(source_says(one(), cached), Ok(false));
    let tested = "    local_0 = p0 is type_0123456789ABCDEF;\n";
    assert_eq!(unread_of(one(), tested).as_deref(), Some("is"));
    // A label of a case by its text is not held; one by its number is, and is no assignment.
    let by_text = "    goto case Hash(\"x\");\n";
    assert_eq!(unread_of(one(), by_text).as_deref(), Some("goto case"));
    assert_eq!(source_says(one(), "    goto case 3;\n"), Ok(false));
    // What the function holds of a form not held yet stops the comparison just as well.
    let boxed_pair = || {
        let boxed = |e: Expr| intrinsic("box<int>", vec![e]);
        vec![used(builtin(
            "System.Object.Equals",
            vec![boxed(p(0)), boxed(p(1))],
        ))]
    };
    let written = "    DEMO.USE(p0 == p1);\n";
    assert_eq!(
        unread_of(boxed_pair(), written).as_deref(),
        Some("a boxed value compared or converted")
    );
    // A form read and a statement that differs before it is a difference, not a form unread.
    let before = format!("    local_0 = p1;\n{lambda}");
    let two = || {
        vec![
            simple(set(Var::Local(0), p(0))),
            simple(set(Var::Local(0), p(0))),
        ]
    };
    assert_eq!(source_says(two(), &before), Ok(false));
}

#[test]
fn what_the_source_level_writes_of_a_compound_and_an_await_reads_back() {
    let body = vec![
        compound(set(Var::Local(1), op(BinOp::Add, local(1), p(0)))),
        construct(Sugar::Await {
            before: vec![set(Var::Local(1), native("GET", vec![p(0)]))],
            ready: native("READY", vec![local(1)]),
            suspend: vec![tree::Stmt::Return(None)],
            resumed: Label::At(0x50),
            resume: Vec::new(),
            after: vec![set(Var::Local(2), native("RESULT", vec![local(1)]))],
        }),
        used(local(2)),
    ];
    let f = function(body);
    let names = Names::default();
    let project = crate::gta6::script::Project::empty();
    let cx = crate::gta6::script::Context::new(&names, project, Options::default());
    let mut text = String::new();
    print::function_in(&mut text, &f, &cx);
    if let Err(why) = agrees_source(&f, &text, cx.options, false) {
        panic!("{why}\nin:\n{text}");
    }
}

#[path = "reparse_source_tests.rs"]
mod source;
