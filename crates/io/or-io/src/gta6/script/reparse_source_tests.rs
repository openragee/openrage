//! What the source level writes of a construct, a place and a name, read back against what the function holds.

use super::*;
use crate::gta6::script::ir::{VarInfo, Width};

/// [`source_says`] of a function whose variables are declared as `vars` says: each with its width and kind.
fn declared_says(
    body: Vec<tree::Stmt>,
    vars: &[(Var, Width, Kind)],
    lines: &str,
) -> Result<bool, ReparseError> {
    let mut f = function(body);
    for (var, width, kind) in vars {
        let mut info = VarInfo::new(*width);
        info.kind = kind.clone();
        f.head.vars.insert(*var, info);
    }
    let text = text_of(lines).replace("var", "Cell");
    match agrees_source(&f, &text, Options::default(), false) {
        Ok(()) => Ok(true),
        Err(ReparseError::Differs { .. }) => Ok(false),
        Err(other) => Err(other),
    }
}

/// A variable of two cells whose layout nothing states.
fn two_cells(n: u32) -> (Var, Width, Kind) {
    (Var::Local(n), Width::Cells, Kind::Aggregate(2))
}

fn an_object(n: u32) -> (Var, Width, Kind) {
    let ty = "type_00000000000000AA".to_owned();
    (Var::Local(n), Width::Cell, Kind::Object(ty))
}

fn address(n: u32) -> Expr {
    Expr::AddrOf {
        var: Var::Local(n),
        cell: 0,
    }
}

#[test]
fn bytes_taken_out_of_a_value_read_as_its_field() {
    let taken = |value: Expr, offset: u32, bytes: u32| Expr::ExtractBytes {
        value: boxed(value),
        offset,
        bytes,
    };
    let pair = [two_cells(1)];
    let four = || vec![used(taken(local(1), 4, 4))];
    let field = "    DEMO.USE((uint)local_1.f_4);\n";
    assert_eq!(declared_says(four(), &pair, field), Ok(true));
    let other = "    DEMO.USE((uint)local_1.f_8);\n";
    assert_eq!(declared_says(four(), &pair, other), Ok(false));
    let named = "    DEMO.USE(extract_bytes(local_1, 4, 4));\n";
    assert_eq!(declared_says(four(), &pair, named), Ok(true));
    // The form of old writes the bytes by name alone.
    assert!(!says(four(), "    DEMO.USE(local_1.f_4);\n"));
    // A whole cell at the start of a variable of one cell is the variable.
    let whole = || vec![used(taken(local(2), 0, 8))];
    assert_eq!(
        declared_says(whole(), &[], "    DEMO.USE(local_2);\n"),
        Ok(true)
    );
    // Of a reference the bytes are no field: `.f_4` would be a member of what it refers to.
    assert_eq!(declared_says(four(), &[an_object(1)], field), Ok(false));
    // Of a group, the field of the cell the bytes lie in.
    let group = || vec![used(taken(Expr::Cells(vec![local(2), local(3)]), 12, 4))];
    let third = "    DEMO.USE((uint)local_3.f_4);\n";
    let wide = [two_cells(2), two_cells(3)];
    assert_eq!(declared_says(group(), &wide, third), Ok(false));
    // ⚠ A cell of a group that is a variable of one cell is shifted down: a dot after it goes
    // through what it holds.
    let shifted = "    DEMO.USE((uint)(local_3 >> 32));\n";
    assert_eq!(declared_says(group(), &[], shifted), Ok(true));
    assert_eq!(declared_says(group(), &[], third), Ok(false));
    let second = "    DEMO.USE((uint)(local_2 >> 32));\n";
    assert_eq!(declared_says(group(), &[], second), Ok(false));
}

#[test]
fn a_dot_after_a_variable_of_one_cell_goes_through_it_and_its_own_bytes_are_reached_by_its_address()
{
    let read = || {
        vec![used(Expr::Load {
            ty: Scalar::I32,
            addr: boxed(p(0)),
            offset: 0x10,
        })]
    };
    assert_eq!(
        source_says(read(), "    DEMO.USE((int)p0.f_10);\n"),
        Ok(true)
    );
    assert_eq!(
        source_says(read(), "    DEMO.USE((int)p0->f_10);\n"),
        Ok(false)
    );
    let stored = || {
        vec![simple(Stmt::Store {
            kind: StoreKind::U64,
            addr: address(1),
            offset: 8,
            value: p(0),
        })]
    };
    assert_eq!(
        source_says(stored(), "    (&local_1)->f_8 = p0;\n"),
        Ok(true)
    );
    assert_eq!(source_says(stored(), "    local_1.f_8 = p0;\n"), Ok(false));
    // A variable of several cells keeps its dot for its own bytes and its arrow through it.
    let pair = [two_cells(1)];
    assert_eq!(
        declared_says(stored(), &pair, "    local_1.f_8 = p0;\n"),
        Ok(true)
    );
    let through = || {
        vec![simple(Stmt::Store {
            kind: StoreKind::U64,
            addr: local(1),
            offset: 8,
            value: p(0),
        })]
    };
    assert_eq!(
        declared_says(through(), &pair, "    local_1->f_8 = p0;\n"),
        Ok(true)
    );
    assert_eq!(
        declared_says(through(), &[], "    local_1.f_8 = p0;\n"),
        Ok(true)
    );
    // With `fields` off the arrow stays.
    let mut f = function(through());
    f.head.vars.clear();
    let off = Options::default().without(Pass::Fields);
    let arrow = text_of("    local_1->f_8 = p0;\n");
    assert!(agrees_source(&f, &arrow, off, false).is_ok());
}

#[test]
fn fewer_bytes_than_a_variable_of_several_cells_holds_are_a_part_of_it() {
    let pair = [two_cells(1)];
    let stored = || {
        vec![simple(Stmt::Store {
            kind: StoreKind::U64,
            addr: address(1),
            offset: 0,
            value: p(0),
        })]
    };
    let part = "    local_1.f_0 = p0;\n";
    assert_eq!(declared_says(stored(), &pair, part), Ok(true));
    let all = "    local_1 = p0;\n";
    assert_eq!(declared_says(stored(), &pair, all), Ok(false));
    // Of a variable of one cell the eight bytes are the variable.
    assert_eq!(declared_says(stored(), &[], all), Ok(true));
    assert_eq!(declared_says(stored(), &[], part), Ok(false));
    let read = || {
        vec![used(Expr::Load {
            ty: Scalar::F32,
            addr: boxed(address(1)),
            offset: 0,
        })]
    };
    let used_part = "    DEMO.USE(local_1.f_0);\n";
    assert_eq!(declared_says(read(), &pair, used_part), Ok(true));
    let used_all = "    DEMO.USE(local_1);\n";
    assert_eq!(declared_says(read(), &pair, used_all), Ok(false));
}

#[test]
fn a_whole_clear_and_a_whole_comparison_read_as_true_csharp_writes_them() {
    let pair = [two_cells(1), two_cells(3)];
    let cleared = |bytes: u32| {
        vec![simple(Stmt::ZeroBytes {
            addr: address(1),
            bytes,
        })]
    };
    let default = "    local_1 = default;\n";
    assert_eq!(declared_says(cleared(16), &pair, default), Ok(true));
    let other = "    local_3 = default;\n";
    assert_eq!(declared_says(cleared(16), &pair, other), Ok(false));
    // Fewer bytes than the variable holds are no default of it.
    assert_eq!(declared_says(cleared(8), &pair, default), Ok(false));
    let named = "    zero_bytes(&local_1, 8);\n";
    assert_eq!(declared_says(cleared(8), &pair, named), Ok(true));
    let compared = || {
        let equal = intrinsic("bytes_equal_16", vec![address(1), address(3)]);
        vec![used(Expr::Unary {
            op: UnOp::Not,
            e: boxed(equal),
        })]
    };
    let differ = "    DEMO.USE(local_1 != local_3);\n";
    assert_eq!(declared_says(compared(), &pair, differ), Ok(true));
    let equal = "    DEMO.USE(local_1 == local_3);\n";
    assert_eq!(declared_says(compared(), &pair, equal), Ok(false));
    // Variables of one cell are compared by their bytes, by name.
    assert_eq!(declared_says(compared(), &[], differ), Ok(false));
}

#[test]
fn a_type_of_the_runtime_is_written_by_its_keyword_and_a_slot_by_its_type() {
    let object = || {
        let call = Expr::Call {
            callee: Callee::Virtual {
                class: "System_Object".to_owned(),
                method: "Method_3".to_owned(),
                ty: "System.Object".to_owned(),
                slot: 3,
            },
            args: vec![p(0), p(1)],
            returns: Some(1),
        };
        vec![used(call)]
    };
    let keyword = "    DEMO.USE(object.Method_3(p0, p1));\n";
    assert_eq!(source_says(object(), keyword), Ok(true));
    let wrapper = "    DEMO.USE(System_Object.Method_3(p0, p1));\n";
    assert_eq!(source_says(object(), wrapper), Ok(false));
    assert!(says(object(), wrapper));
    let hashed = || vec![used(builtin("System.Int64.Method_7", vec![p(0)]))];
    let long = "    DEMO.USE(long.Method_7(p0));\n";
    assert_eq!(source_says(hashed(), long), Ok(true));
    let made = || {
        let ty = VmType {
            name: "System.Object".to_owned(),
            byte_size: 8,
            id: 1,
            flags: 0,
        };
        vec![simple(set(Var::Local(1), Expr::NewObject(ty)))]
    };
    let new = "    local_1 = new object();\n";
    assert_eq!(source_says(made(), new), Ok(true));
    let listed = "    local_1 = new System.Object();\n";
    assert_eq!(source_says(made(), listed), Ok(false));
    let tried = || vec![used(intrinsic("try_cast<System.String>", vec![p(0)]))];
    assert_eq!(
        source_says(tried(), "    DEMO.USE(p0 as string);\n"),
        Ok(true)
    );
}

#[test]
fn an_unsigned_test_above_zero_reads_as_a_test_for_null_where_it_may_be_one() {
    let above = |l: Expr| Expr::Binary {
        op: BinOp::Gt,
        at: Kind::U64,
        l: boxed(l),
        r: boxed(int(0)),
    };
    // What a call yields may be a reference: either form says it.
    let called = || vec![used(above(native("GET", vec![p(0)])))];
    let null = "    DEMO.USE(DEMO.GET(p0) != null);\n";
    assert_eq!(source_says(called(), null), Ok(true));
    let zero = "    DEMO.USE(DEMO.GET(p0) > 0);\n";
    assert_eq!(source_says(called(), zero), Ok(true));
    let other = "    DEMO.USE(DEMO.GET(p1) != null);\n";
    assert_eq!(source_says(called(), other), Ok(false));
    // A variable says which by its kind.
    let held = || vec![used(above(local(1)))];
    let object = "    DEMO.USE(local_1 != null);\n";
    assert_eq!(declared_says(held(), &[an_object(1)], object), Ok(true));
    assert_eq!(declared_says(held(), &[], object), Ok(false));
    let number = "    DEMO.USE(local_1 > 0);\n";
    assert_eq!(declared_says(held(), &[], number), Ok(true));
    // The test turned round is the test for null, or for at most zero.
    let turned = || {
        let test = above(native("GET", vec![p(0)]));
        vec![used(Expr::Unary {
            op: UnOp::Not,
            e: boxed(test),
        })]
    };
    let none = "    DEMO.USE(DEMO.GET(p0) == null);\n";
    assert_eq!(source_says(turned(), none), Ok(true));
    let most = "    DEMO.USE(DEMO.GET(p0) <= 0);\n";
    assert_eq!(source_says(turned(), most), Ok(true));
    assert_eq!(source_says(turned(), null), Ok(false));
}

#[test]
fn what_stands_at_an_elements_address_reads_as_the_element_where_it_is_as_wide() {
    let read = |array: Expr, ty: Scalar| {
        let at = intrinsic("array_element_address", vec![array, p(1)]);
        vec![used(Expr::Load {
            ty,
            addr: boxed(at),
            offset: 0,
        })]
    };
    let ints = [(Var::Local(1), Width::Cell, Kind::Array(Box::new(Kind::I32)))];
    let element = "    DEMO.USE(local_1[p1]);\n";
    let at = "    DEMO.USE((&local_1[p1])->f_0);\n";
    assert_eq!(
        declared_says(read(local(1), Scalar::I32), &ints, element),
        Ok(true)
    );
    assert_eq!(
        declared_says(read(local(1), Scalar::I32), &ints, at),
        Ok(false)
    );
    assert_eq!(
        declared_says(read(local(1), Scalar::U64), &ints, element),
        Ok(false)
    );
    assert_eq!(
        declared_says(read(local(1), Scalar::U64), &ints, at),
        Ok(true)
    );
    // An array of no kind the function states reads either way.
    let field = || Expr::Load {
        ty: Scalar::U64,
        addr: boxed(p(0)),
        offset: 0x10,
    };
    let in_field = "    DEMO.USE(p0.f_10[p1]);\n";
    assert_eq!(
        declared_says(read(field(), Scalar::I32), &[], in_field),
        Ok(true)
    );
    let at_field = "    DEMO.USE((&p0.f_10[p1])->f_0);\n";
    assert_eq!(
        declared_says(read(field(), Scalar::I32), &[], at_field),
        Ok(true)
    );
    let other = "    DEMO.USE(p0.f_18[p1]);\n";
    assert_eq!(
        declared_says(read(field(), Scalar::I32), &[], other),
        Ok(false)
    );
}

#[test]
fn a_case_that_runs_off_its_end_is_closed_as_true_csharp_must() {
    let switch = || {
        let cases = vec![
            Case {
                labels: vec![Some(1)],
                body: vec![used(p(0))],
            },
            Case {
                labels: vec![Some(2)],
                body: Vec::new(),
            },
            Case {
                labels: vec![Some(3)],
                body: vec![used(p(1)), tree::Stmt::Return(None)],
            },
            Case {
                labels: vec![None],
                body: vec![used(p(1))],
            },
        ];
        vec![tree::Stmt::Switch { value: p(0), cases }]
    };
    let written = |into: &str, last: &str| {
        format!(
            "    switch (p0)\n    {{\n        case 1:\n            DEMO.USE(p0);\n{into}        case 2:\n        case 3:\n            DEMO.USE(p1);\n            return;\n        default:\n            DEMO.USE(p1);\n{last}    }}\n"
        )
    };
    let into = "            goto case 2;\n";
    let last = "            break;\n";
    assert_eq!(source_says(switch(), &written(into, last)), Ok(true));
    assert_eq!(source_says(switch(), &written("", last)), Ok(false));
    assert_eq!(source_says(switch(), &written(into, "")), Ok(false));
    let elsewhere = "            goto case 3;\n";
    assert_eq!(source_says(switch(), &written(elsewhere, last)), Ok(false));
    // The form of old lets a case run on unsaid.
    assert!(says(switch(), &written("", "")));
}

#[test]
fn an_arm_nothing_is_written_for_is_no_else_and_the_other_stands_under_the_test_turned() {
    let hidden = || construct(Sugar::Hidden(vec![simple(set(Var::Local(1), p(0)))]));
    let branch = || {
        vec![tree::Stmt::If {
            cond: p(0),
            then: vec![hidden()],
            els: vec![used(p(1))],
        }]
    };
    let turned = "    if (!p0)\n    {\n        DEMO.USE(p1);\n    }\n";
    assert_eq!(source_says(branch(), turned), Ok(true));
    let both = "    if (p0)\n    {\n    }\n    else\n    {\n        DEMO.USE(p1);\n    }\n";
    assert_eq!(source_says(branch(), both), Ok(false));
}

#[test]
fn a_value_given_to_the_discard_and_one_cell_gathered_alone_are_that_value() {
    let element = || {
        let read = intrinsic("array_get_value", vec![p(0), p(1)]);
        vec![simple(Stmt::Eval(read))]
    };
    assert_eq!(source_says(element(), "    _ = p0[p1];\n"), Ok(true));
    assert_eq!(source_says(element(), "    _ = p1[p0];\n"), Ok(false));
    let group = || vec![used(Expr::Cells(vec![p(1)]))];
    assert_eq!(source_says(group(), "    DEMO.USE(p1);\n"), Ok(true));
    assert!(!says(group(), "    DEMO.USE(p1);\n"));
}

#[test]
fn a_function_run_on_an_object_its_call_makes_reads_as_that_object_made() {
    let made = || {
        let call = Expr::Call {
            callee: Callee::Local {
                at: 0x80,
                name: "func_000080".to_owned(),
            },
            args: vec![Expr::NewObject(a_type()), p(1)],
            returns: Some(0),
        };
        vec![simple(Stmt::Eval(call))]
    };
    let new = "    new type_0123456789ABCDEF(p1);\n";
    assert_eq!(source_says(made(), new), Ok(true));
    let other = "    new type_0123456789ABCDEF(p0);\n";
    assert_eq!(source_says(made(), other), Ok(false));
    let called = "    func_000080(new type_0123456789ABCDEF(), p1);\n";
    assert_eq!(source_says(made(), called), Ok(true));
}

#[test]
fn an_object_made_then_constructed_apart_reads_as_the_two_statements_at_the_source_level() {
    let pair = || {
        let ty = a_type();
        let ctor = format!("{}.Ctor", ty.name);
        vec![
            simple(set(Var::Local(1), Expr::NewObject(ty))),
            simple(Stmt::Eval(builtin(&ctor, vec![local(1), p(0)]))),
        ]
    };
    let apart = "    local_1 = new type_0123456789ABCDEF();\n    type_0123456789ABCDEF.Ctor(local_1, p0);\n";
    assert_eq!(source_says(pair(), apart), Ok(true));
    let one = "    local_1 = new type_0123456789ABCDEF(p0);\n";
    assert_eq!(source_says(pair(), one), Ok(false));
    // The form of old pairs them itself.
    assert!(says(pair(), one));
}

#[test]
fn a_difference_is_known_by_the_innermost_forms_where_the_two_part() {
    let form = |body: Vec<tree::Stmt>, lines: &str| match agrees(&function(body), &text_of(lines)) {
        Err(ReparseError::Differs { form, .. }) => form,
        other => panic!("{other:?}\nin:\n{lines}"),
    };
    let sum = vec![used(op(BinOp::Add, p(0), p(1)))];
    let taken = "    DEMO.USE(p0 + extract_bytes(p1, 0, 8));\n";
    assert_eq!(form(sum, taken), "`extract_bytes` against a name");
    let word = form(vec![tree::Stmt::Break], "    continue;\n");
    assert_eq!(word, "`continue` against `break`");
}

#[test]
fn an_object_constructed_and_filled_reads_as_its_initialiser() {
    let store = |object: u32, offset: u32, value: Expr| Stmt::Store {
        kind: StoreKind::U64,
        addr: local(object),
        offset,
        value,
    };
    let built = |fields: Vec<Stmt>| {
        let ctor = format!("{}.Ctor", a_type().name);
        let mut parts = vec![
            set(Var::Local(1), Expr::NewObject(a_type())),
            Stmt::Eval(builtin(&ctor, vec![local(1), p(0)])),
        ];
        parts.extend(fields);
        vec![construct(Sugar::Group {
            kind: Group::Construct,
            parts,
        })]
    };
    let both = || built(vec![store(1, 8, p(1)), store(1, 0x10, int(3))]);
    let text = "    local_1 = new type_0123456789ABCDEF(p0) { f_8 = p1, f_10 = 3 };\n";
    assert_eq!(source_says(both(), text), Ok(true));
    let other = "    local_1 = new type_0123456789ABCDEF(p0) { f_8 = p1, f_18 = 3 };\n";
    assert_eq!(source_says(both(), other), Ok(false));
    // With no constructor run the object is another.
    let unrun = "    local_1 = new type_0123456789ABCDEF { f_8 = p1, f_10 = 3 };\n";
    assert_eq!(source_says(both(), unrun), Ok(false));
    // With no field, the object made with the arguments.
    let made = "    local_1 = new type_0123456789ABCDEF(p0);\n";
    assert_eq!(source_says(built(Vec::new()), made), Ok(true));
    // A store that is no field of the object leaves the fields as the statements they are.
    let apart = || built(vec![store(1, 8, p(1)), store(3, 8, p(0))]);
    let written = format!("{made}    local_1.f_8 = p1;\n    local_3.f_8 = p0;\n");
    assert_eq!(source_says(apart(), &written), Ok(true));
    // An object made inside, stored whole to a field, is that field's value.
    let inner = || {
        let parts = vec![
            set(Var::Local(1), Expr::NewObject(a_type())),
            set(Var::Local(2), Expr::NewObject(a_type())),
            store(2, 8, p(0)),
            store(1, 0x10, local(2)),
        ];
        vec![construct(Sugar::Group {
            kind: Group::Construct,
            parts,
        })]
    };
    let nested = "    local_1 = new type_0123456789ABCDEF { f_10 = new type_0123456789ABCDEF { f_8 = p0 } };\n";
    assert_eq!(source_says(inner(), nested), Ok(true));
    let flat = "    local_1 = new type_0123456789ABCDEF { f_10 = new type_0123456789ABCDEF() };\n";
    assert_eq!(source_says(inner(), flat), Ok(false));
}

#[test]
fn a_static_property_of_the_runtime_reads_by_the_type_that_declares_it() {
    let got = || {
        let call = builtin("System.Thing.get_Count", Vec::new());
        vec![sugared(ExprSugar::Property, call)]
    };
    let property = "    DEMO.USE(System.Thing.Count);\n";
    assert_eq!(source_says(got(), property), Ok(true));
    let called = "    DEMO.USE(System.Thing.get_Count());\n";
    assert_eq!(source_says(got(), called), Ok(false));
}

#[test]
fn an_infinity_spelled_by_name_reads_as_its_bits() {
    let stored = |bits: u64, kind: Kind| vec![used(constant_of(bits, kind))];
    assert_eq!(
        source_says(
            stored(0x7F80_0000, Kind::F32),
            "    DEMO.USE(float.PositiveInfinity);\n"
        ),
        Ok(true)
    );
    assert_eq!(
        source_says(
            stored(0xFF80_0000, Kind::F32),
            "    DEMO.USE(float.NegativeInfinity);\n"
        ),
        Ok(true)
    );
    // ⚠ Another float's bits are not it.
    assert_eq!(
        source_says(
            stored(0x3F80_0000, Kind::F32),
            "    DEMO.USE(float.PositiveInfinity);\n"
        ),
        Ok(false)
    );
}

#[test]
fn a_field_of_a_variables_own_bytes_reads_as_the_place_they_are() {
    let owner = crate::gta6::script::ir::TypeKey(0x0000_0001_0000_00AB);
    let field = || {
        let load = Expr::Load {
            ty: Scalar::U64,
            addr: Box::new(Expr::AddrOf {
                var: Var::Local(0),
                cell: 0,
            }),
            offset: 0x18,
        };
        vec![used(Expr::sugar(ExprSugar::Field { owner }, load))]
    };
    assert_eq!(
        source_says(field(), "    DEMO.USE((&local_0)->f_18);\n"),
        Ok(true)
    );
    assert_eq!(
        source_says(field(), "    DEMO.USE((&local_0).f_18);\n"),
        Ok(false)
    );
    // A variable of several cells holds its fields in place.
    assert_eq!(
        declared_says(field(), &[two_cells(0)], "    DEMO.USE(local_0.f_18);\n"),
        Ok(true)
    );
}

#[test]
fn a_delegate_made_and_bound_reads_as_the_group_or_the_object_it_is_written_as() {
    let own = 0x1122_3344;
    let bound = |to: Expr, function: Expr| {
        let ctor = format!("{}.Ctor", a_type().name);
        vec![construct(Sugar::Group {
            kind: Group::Delegate,
            parts: vec![
                set(Var::Local(1), Expr::NewObject(a_type())),
                Stmt::Eval(builtin(&ctor, vec![local(1), to, function])),
            ],
        })]
    };
    let function = |program: u32| Expr::FunctionRef { program, at: 0x80 };
    // A function of the program, on what it is bound to.
    let member = "    local_1 = p0.func_000080;\n";
    assert_eq!(source_says(bound(p(0), function(own)), member), Ok(true));
    let other = "    local_1 = p1.func_000080;\n";
    assert_eq!(source_says(bound(p(0), function(own)), other), Ok(false));
    let another = "    local_1 = p0.func_000090;\n";
    assert_eq!(source_says(bound(p(0), function(own)), another), Ok(false));
    // Bound to nothing, the function alone.
    let alone = "    local_1 = func_000080;\n";
    assert_eq!(source_says(bound(int(0), function(own)), alone), Ok(true));
    assert_eq!(source_says(bound(p(0), function(own)), alone), Ok(false));
    // Another program's function is a group only where it is bound to nothing.
    let far = "    local_1 = program_55667788.func_000080;\n";
    assert_eq!(
        source_says(bound(int(0), function(0x5566_7788)), far),
        Ok(true)
    );
    let made = "    local_1 = new type_0123456789ABCDEF(p0, &program_55667788.func_000080);\n";
    assert_eq!(
        source_says(bound(p(0), function(0x5566_7788)), made),
        Ok(true)
    );
    assert_eq!(
        source_says(bound(p(0), function(0x5566_7788)), far),
        Ok(false)
    );
    // A slot of the object the delegate is bound to, and of another object the delegate made whole.
    let slot = |receiver: Expr| Expr::VirtualFunctionRef {
        receiver: boxed(receiver),
        ty: a_type(),
        slot: 5,
    };
    let group = "    local_1 = p0.Method_5;\n";
    assert_eq!(source_says(bound(p(0), slot(p(0))), group), Ok(true));
    let apart = "    local_1 = new type_0123456789ABCDEF(p0, VirtualFunction(p1, type_0123456789ABCDEF, 5));\n";
    assert_eq!(source_says(bound(p(0), slot(p(1))), apart), Ok(true));
    assert_eq!(source_says(bound(p(0), slot(p(1))), group), Ok(false));
    // A lambda in the text stays a form not read.
    let lambda = "    local_1 = (a) => a;\n";
    let unread = unread_of(bound(p(0), function(own)), lambda);
    assert_eq!(
        unread.as_deref(),
        Some("a lambda written where it is bound")
    );
}

#[test]
fn an_array_made_and_every_element_set_reads_as_the_array_written_with_them() {
    let stored = |index: i32, value: Expr| {
        Stmt::Eval(intrinsic(
            "array_set_ref",
            vec![local(1), int(index), value],
        ))
    };
    let made = |len: i32, sets: Vec<Stmt>| {
        let array = Expr::NewArray {
            ty: a_type(),
            len: boxed(int(len)),
            wrapper: String::new(),
        };
        let mut parts = vec![set(Var::Local(1), array)];
        parts.extend(sets);
        vec![construct(Sugar::Group {
            kind: Group::Collection,
            parts,
        })]
    };
    let both = || made(2, vec![stored(0, p(0)), stored(1, p(1))]);
    let text = "    local_1 = new type_0123456789ABCDEF[] { p0, p1 };\n";
    assert_eq!(source_says(both(), text), Ok(true));
    let turned = "    local_1 = new type_0123456789ABCDEF[] { p1, p0 };\n";
    assert_eq!(source_says(both(), turned), Ok(false));
    let fewer = "    local_1 = new type_0123456789ABCDEF[] { p0 };\n";
    assert_eq!(source_says(both(), fewer), Ok(false));
    // An element left out is zero, which the elements alone would not say: the statements stand apart.
    let apart =
        "    local_1 = new type_0123456789ABCDEF[3];\n    local_1[0] = p0;\n    local_1[1] = p1;\n";
    let short = || made(3, vec![stored(0, p(0)), stored(1, p(1))]);
    assert_eq!(source_says(short(), apart), Ok(true));
    // And so do elements set out of their order.
    let swapped = || made(2, vec![stored(1, p(1)), stored(0, p(0))]);
    assert_eq!(source_says(swapped(), text), Ok(false));
}

#[test]
fn a_call_that_fills_what_was_only_made_zero_reads_as_the_call_alone() {
    let out = |n: u32| Expr::sugar(ExprSugar::Out, address(n));
    let filled = |zero: Expr, argument: Expr| {
        vec![construct(Sugar::Group {
            kind: Group::Out,
            parts: vec![
                set(Var::Local(1), zero),
                Stmt::Eval(native("FILL", vec![p(0), argument])),
            ],
        })]
    };
    let call = "    DEMO.FILL(p0, out local_1);\n";
    assert_eq!(source_says(filled(int(0), out(1)), call), Ok(true));
    let declared = "    DEMO.FILL(p0, out Cell local_1);\n";
    assert_eq!(source_says(filled(int(0), out(1)), declared), Ok(true));
    let other = "    DEMO.FILL(p0, out local_2);\n";
    assert_eq!(source_says(filled(int(0), out(1)), other), Ok(false));
    // A variable given anything but zero, or one the call does not fill, keeps its statement.
    let both = "    local_1 = 5;\n    DEMO.FILL(p0, out local_1);\n";
    assert_eq!(source_says(filled(int(5), out(1)), both), Ok(true));
    assert_eq!(source_says(filled(int(5), out(1)), call), Ok(false));
    let zeroed = "    local_1 = 0;\n    DEMO.FILL(p0, out local_2);\n";
    assert_eq!(source_says(filled(int(0), out(2)), zeroed), Ok(true));
}

#[test]
fn an_events_loop_reads_as_the_handler_added_to_its_place_or_taken_from_it() {
    let runtime = |path: &str, args: Vec<Expr>| builtin(path, args);
    let field = || Expr::Load {
        ty: Scalar::U64,
        addr: boxed(p(0)),
        offset: 0x30,
    };
    let place = || Expr::ByteOffset {
        addr: boxed(p(0)),
        offset: boxed(int(0x30)),
    };
    let event = |join: &str| {
        let joined = runtime(join, vec![local(1), p(1)]);
        let swap = runtime(
            "Demo.Cells.CompareExchange",
            vec![place(), joined, local(1)],
        );
        let again = Expr::binary(
            BinOp::Ne,
            Expr::cast(Scalar::I64, local(0)),
            Expr::cast(Scalar::I64, local(1)),
        );
        vec![construct(Sugar::Event {
            seed: Box::new(set(Var::Local(0), field())),
            round: vec![set(Var::Local(1), local(0)), set(Var::Local(0), swap)],
            again,
        })]
    };
    let added = "    p0.f_30 += p1;\n";
    let taken = "    p0.f_30 -= p1;\n";
    assert_eq!(source_says(event("Demo.Delegate.Combine"), added), Ok(true));
    assert_eq!(
        source_says(event("Demo.Delegate.Combine"), taken),
        Ok(false)
    );
    assert_eq!(source_says(event("Demo.Delegate.Remove"), taken), Ok(true));
    let another = "    p0.f_38 += p1;\n";
    assert_eq!(
        source_says(event("Demo.Delegate.Combine"), another),
        Ok(false)
    );
    let handler = "    p0.f_30 += p0;\n";
    assert_eq!(
        source_says(event("Demo.Delegate.Combine"), handler),
        Ok(false)
    );
}
