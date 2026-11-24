use super::*;

#[test]
fn a_variable_is_named_as_the_listing_names_it() {
    let vars = [
        (Var::Param(0), "p0"),
        (Var::Param(12), "p12"),
        (Var::Local(3), "local_3"),
        (Var::Value { at: 0x1D, index: 0 }, "v_00001D_0"),
        (
            Var::Value {
                at: 0xAB_CDEF,
                index: 14,
            },
            "v_ABCDEF_14",
        ),
        (Var::Stack { at: 0x6C, index: 1 }, "stack_00006C_1"),
    ];
    for (var, name) in vars {
        assert_eq!(var.to_string(), name);
        assert_eq!(Var::from_name(name), Some(var));
    }
    for name in [
        "",
        "p",
        "px",
        "parameters",
        "local_",
        "local_x",
        "v_1D_0",
        "v_00001D",
        "v_00001G_0",
        "stack_00006C_",
        "func_00001D",
    ] {
        assert_eq!(Var::from_name(name), None, "{name}");
    }
    let global = Var::Global {
        program: 0x0A0B_0C0D,
        offset: 0x1230,
        statics: true,
    };
    assert_eq!(global.to_string(), "Vm.Globals(0x0A0B0C0DU, true)[0x1230]");
    assert!(Var::Stack { at: 0, index: 0 }.is_temporary());
    assert!(!Var::Local(0).is_temporary() && !global.is_temporary());
}

#[test]
fn the_names_of_widths_and_operators_read_back() {
    for scalar in Scalar::ALL {
        assert_eq!(Scalar::from_name(scalar.name()), Some(scalar));
    }
    for kind in StoreKind::ALL {
        assert_eq!(StoreKind::from_name(kind.name()), Some(kind));
    }
    for op in BinOp::ALL {
        assert_eq!(BinOp::from_symbol(op.symbol()), Some(op));
    }
    assert_eq!(Scalar::from_name("u128"), None);
    assert_eq!(BinOp::from_symbol("="), None);
    assert_eq!((Scalar::I16.bytes(), StoreKind::Ref.bytes()), (2, 8));
}

#[test]
fn a_callee_says_which_declaration_it_goes_through() {
    let native = |namespace: Option<&str>| Callee::Native {
        namespace: namespace.map(str::to_owned),
        name: "DO_THING".to_owned(),
        hash: 1,
        rva: 2,
    };
    assert_eq!(
        native(Some("DEMO")).path().as_deref(),
        Some("Native.DEMO.DO_THING")
    );
    assert_eq!(native(None).path().as_deref(), Some("Native.DO_THING"));
    let intrinsic = |wrapper: Option<&str>| Callee::Intrinsic {
        name: "box<type_0>".to_owned(),
        ty: None,
        wrapper: wrapper.map(str::to_owned),
    };
    assert_eq!(
        intrinsic(Some("box_type_0_")).path().as_deref(),
        Some("Operations.box_type_0_")
    );
    assert_eq!(intrinsic(None).path(), None);
    let local = Callee::Local {
        at: 0x10,
        name: "func_000010".to_owned(),
    };
    assert_eq!(local.path(), None);
}

#[test]
fn a_walk_visits_every_expression_and_may_rewrite_from_the_inside_out() {
    let p0 = || Expr::Var(Var::Param(0));
    let mut e = Expr::binary(
        BinOp::Add,
        Expr::cast(Scalar::I32, p0()),
        Expr::Cell {
            base: Box::new(Expr::Cells(vec![p0(), Expr::Const(Const::i32(7))])),
            index: 1,
        },
    );
    let mut seen = 0;
    let mut params = 0;
    e.walk(&mut |x| {
        seen += 1;
        params += usize::from(*x == p0());
    });
    assert_eq!((seen, params), (7, 2));

    // Children are rewritten before the expression that holds them is offered.
    let mut order = Vec::new();
    e.walk_mut(&mut |x| {
        if *x == p0() {
            *x = Expr::Var(Var::Local(5));
        }
        order.push(matches!(x, Expr::Binary { .. }));
    });
    assert_eq!(order.last(), Some(&true));
    let mut locals = 0;
    e.walk(&mut |x| locals += usize::from(*x == Expr::Var(Var::Local(5))));
    assert_eq!(locals, 2);
}

#[test]
fn a_statement_names_what_it_assigns_and_offers_what_it_evaluates() {
    let p0 = || Expr::Var(Var::Param(0));
    let assign = Stmt::Assign {
        target: Var::Local(1),
        value: p0(),
    };
    assert_eq!(assign.targets(), [Var::Local(1)]);
    let many = Stmt::AssignMany {
        targets: vec![Var::Local(1), Var::Local(2)],
        values: vec![p0(), p0()],
    };
    assert_eq!(many.targets(), [Var::Local(1), Var::Local(2)]);
    let mut store = Stmt::Store {
        kind: StoreKind::I32,
        addr: p0(),
        offset: 0x10,
        value: Expr::Const(Const::i32(1)),
    };
    assert!(store.targets().is_empty(), "a store names no variable");
    let mut offered = Vec::new();
    store.each_expr(&mut |e| offered.push(e.clone()));
    assert_eq!(offered, [p0(), Expr::Const(Const::i32(1))]);
    store.each_expr_mut(&mut |e| *e = Expr::GuidEmpty);
    let mut rewritten = 0;
    store.each_expr(&mut |e| rewritten += usize::from(*e == Expr::GuidEmpty));
    assert_eq!(rewritten, 2);
}
