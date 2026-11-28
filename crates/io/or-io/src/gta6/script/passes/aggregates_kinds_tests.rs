use super::*;
use crate::gta5::script::natives::{NativeDb, NativeName, NativeParam};
use crate::gta6::script::cfg::{Block, Cfg, Function, Terminator};
use crate::gta6::script::ir::{Const, FunctionKey, Returns, StoreKind, VarInfo};
use crate::gta6::script::project::Signature;
use crate::gta6::script::{Level, Options};

const PROGRAM: u32 = 0x5566_7788;
/// A value type no list names.
const VALUE_TYPE: &str = "type_00C0FFEE00C0FFEE";

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn set(n: u32, value: Expr) -> Stmt {
    Stmt::Assign {
        target: Var::Local(n),
        value,
    }
}

/// `&local_n`.
fn own(n: u32) -> Expr {
    Expr::AddrOf {
        var: Var::Local(n),
        cell: 0,
    }
}

/// A call of the program's function at `at`, yielding `returns` cells.
fn call(at: u32, args: Vec<Expr>, returns: Option<u32>) -> Expr {
    let callee = Callee::Local {
        at,
        name: format!("func_{at:06X}"),
    };
    Expr::Call {
        callee,
        args,
        returns,
    }
}

fn used(at: u32, args: Vec<Expr>) -> Stmt {
    Stmt::Eval(call(at, args, Some(0)))
}

/// A native no list names, yielding two cells.
fn engine() -> Expr {
    let callee = Callee::Native {
        namespace: None,
        name: "_0x00000000DEADBEEF".to_owned(),
        hash: 0xDEAD_BEEF,
        rva: 0,
    };
    Expr::Call {
        callee,
        args: Vec::new(),
        returns: Some(2),
    }
}

/// `name(a, b)` of one of the runtime's operations.
fn compare(name: &str, a: Expr, b: Expr) -> Stmt {
    let callee = Callee::Intrinsic {
        name: name.to_owned(),
        ty: None,
        wrapper: None,
    };
    Stmt::Eval(Expr::Call {
        callee,
        args: vec![a, b],
        returns: Some(1),
    })
}

/// A call of the runtime's builtin method at `path`.
fn builtin(path: &str, args: Vec<Expr>) -> Stmt {
    let callee = Callee::Builtin {
        path: path.to_owned(),
        ty: 0x15,
        method: 4,
    };
    Stmt::Eval(Expr::Call {
        callee,
        args,
        returns: Some(1),
    })
}

/// A function of one block whose locals of several cells are these, of these kinds.
fn function(wide: &[(u32, Kind)], stmts: Vec<Stmt>) -> Function {
    let mut vars = BTreeMap::new();
    for (n, kind) in wide {
        let mut info = VarInfo::new(Width::Cells);
        info.kind = kind.clone();
        vars.insert(Var::Local(*n), info);
    }
    let head = Head {
        value_types: Default::default(),
        program: PROGRAM,
        at: 0x40,
        name: "func_000040".to_owned(),
        returns: Returns::Void,
        params: 2,
        locals: 8,
        vars,
        incomplete: None,
    };
    let block = Block {
        at: None,
        stmts,
        term: Terminator::Return(None),
    };
    Function {
        head,
        cfg: Cfg {
            blocks: vec![block],
        },
    }
}

fn on() -> Options {
    Options::at(Level::Structured)
}

fn kinded(mut f: Function, project: &Project, options: Options) -> Function {
    let names = Names::default();
    let cx = Context::new(&names, project, options);
    copied_kinds(&mut f, &cx, &mut Stats::default());
    f
}

fn kind_of(f: &Function, var: Var) -> Option<Kind> {
    f.head.vars.get(&var).map(|info| info.kind.clone())
}

const TWO: Kind = Kind::Aggregate(2);

#[test]
fn a_value_copied_whole_from_a_handle_is_a_handle_and_so_is_one_copied_on_from_it() {
    let wide = [(0, TWO), (2, Kind::Handle), (4, TWO)];
    let body = || {
        vec![
            set(0, Expr::ZeroSlots(2)),
            set(0, local(2)),
            set(4, local(0)),
            used(0x100, vec![local(4)]),
        ]
    };
    let project = Project::default();
    let done = kinded(function(&wide, body()), &project, on());
    assert_eq!(kind_of(&done, Var::Local(0)), Some(Kind::Handle));
    assert_eq!(kind_of(&done, Var::Local(4)), Some(Kind::Handle));
    // The code is as it was: a kind is the declaration alone.
    assert_eq!(done.cfg, function(&wide, body()).cfg);

    // Switched off by either name, nothing is given a kind.
    for off in [Pass::Kinds, Pass::Aggregates] {
        let same = function(&wide, body());
        assert_eq!(kinded(same.clone(), &project, on().without(off)), same);
    }
}

#[test]
fn a_value_copied_whole_into_a_vector_is_a_vector() {
    let wide = [(0, TWO), (2, Kind::Vector)];
    let body = vec![set(0, Expr::ZeroSlots(2)), set(2, local(0))];
    let done = kinded(function(&wide, body), &Project::default(), on());
    assert_eq!(kind_of(&done, Var::Local(0)), Some(Kind::Vector));
}

#[test]
fn a_value_two_kinds_reach_or_a_way_in_no_kind_is_read_off_keeps_its_cells_alone() {
    let project = Project::default();
    let untouched = |wide: &[(u32, Kind)], body: Vec<Stmt>| {
        let f = function(wide, body);
        assert_eq!(kinded(f.clone(), &project, on()), f);
    };
    // Copied from a handle and from a vector.
    let wide = [(0, TWO), (2, Kind::Handle), (4, Kind::Vector)];
    untouched(&wide, vec![set(0, local(2)), set(0, local(4))]);
    // A copy of a handle whose address is handed to a function, which may write anything there.
    let wide = [(0, TWO), (2, Kind::Handle)];
    untouched(&wide, vec![set(0, local(2)), used(0x100, vec![own(0)])]);
    // Assigned a group of cells, or among other targets at once.
    let cells = Expr::Cells(vec![Expr::Const(Const::i32(1)), Expr::Const(Const::i32(2))]);
    untouched(&wide, vec![set(0, local(2)), set(0, cells)]);
    let many = Stmt::AssignMany {
        targets: vec![Var::Local(0), Var::Local(6)],
        values: vec![local(2), Expr::Const(Const::i32(0))],
    };
    untouched(&wide, vec![set(0, local(2)), many]);
    // A handle has two cells, not three.
    untouched(
        &[(0, Kind::Aggregate(3)), (4, Kind::Handle)],
        vec![set(0, local(4))],
    );
    // Zero alone says nothing of what the cells are.
    untouched(&[(0, TWO)], vec![set(0, Expr::ZeroSlots(2))]);

    // A parameter is of what its signature says, whatever it is handed.
    let handed = Stmt::Assign {
        target: Var::Param(0),
        value: local(2),
    };
    let mut f = function(&wide, vec![handed]);
    let mut info = VarInfo::new(Width::Cells);
    info.kind = TWO;
    f.head.vars.insert(Var::Param(0), info);
    assert_eq!(kinded(f.clone(), &project, on()), f);
}

#[test]
fn a_zero_compared_whole_with_a_handle_is_a_handle() {
    let wide = [(0, TWO), (2, Kind::Handle)];
    let project = Project::default();
    let body = |name: &str| vec![set(0, Expr::ZeroSlots(2)), compare(name, own(0), own(2))];
    let done = kinded(function(&wide, body("bytes_equal_16")), &project, on());
    assert_eq!(kind_of(&done, Var::Local(0)), Some(Kind::Handle));
    // Twelve bytes of two cells compare no whole value.
    let f = function(&wide, body("bytes_equal_12"));
    assert_eq!(kinded(f.clone(), &project, on()), f);
}

#[test]
fn a_value_the_runtimes_guid_comparison_takes_is_a_handle() {
    let wide = [(0, TWO)];
    let project = Project::default();
    let body = |path: &str| {
        vec![
            set(0, engine()),
            builtin(path, vec![local(0), Expr::ZeroSlots(2)]),
            used(0x100, vec![local(0)]),
        ]
    };
    let done = kinded(
        function(&wide, body("System.Guid.op_Equality")),
        &project,
        on(),
    );
    assert_eq!(kind_of(&done, Var::Local(0)), Some(Kind::Handle));
    // A builtin no list names says nothing.
    let f = function(&wide, body("type_80000015.Method_4"));
    assert_eq!(kinded(f.clone(), &project, on()), f);
}

#[test]
fn a_value_a_function_returns_as_a_value_type_is_of_that_type() {
    let value = Kind::Value {
        ty: VALUE_TYPE.to_owned(),
        cells: 2,
    };
    let mut project = Project::default();
    let returning = |returns: u32, result: Kind| Signature {
        returns,
        result,
        ..Signature::default()
    };
    let key = FunctionKey::new(PROGRAM, 0x200);
    project.signatures.insert(key, returning(2, value.clone()));
    let body = || vec![set(0, call(0x200, Vec::new(), Some(2)))];
    let done = kinded(function(&[(0, TWO)], body()), &project, on());
    assert_eq!(kind_of(&done, Var::Local(0)), Some(value));

    // A result of another count of cells than the variable's says nothing it can take.
    let three = Kind::Value {
        ty: VALUE_TYPE.to_owned(),
        cells: 3,
    };
    project.signatures.insert(key, returning(2, three));
    let f = function(&[(0, TWO)], body());
    assert_eq!(kinded(f.clone(), &project, on()), f);
}

/// A list's natives: `void WAVE_AT_FRIEND(Ped friend, int times)`, `void POINT_AT_SPOT(Vector3 spot)`, `Vehicle PICK_FRIEND()`.
fn listed() -> Names {
    let takes = |ty: &str, name: &str| NativeParam {
        ty: ty.to_owned(),
        name: name.to_owned(),
    };
    let native = |name: &str, params: Vec<NativeParam>, returns: &str| NativeName {
        namespace: Some("DEMO".to_owned()),
        name: name.to_owned(),
        params,
        returns: Some(returns.to_owned()),
    };
    let wave = native(
        "WAVE_AT_FRIEND",
        vec![takes("Ped", "friend"), takes("int", "times")],
        "void",
    );
    let point = native("POINT_AT_SPOT", vec![takes("Vector3", "spot")], "void");
    let pick = native("PICK_FRIEND", Vec::new(), "Vehicle");
    let natives = [(WAVE, wave), (POINT, point), (PICK, pick)];
    Names {
        natives: Some(NativeDb::from_names("a test's", natives)),
        ..Names::default()
    }
}

const WAVE: u64 = 0x7E57_0001;
const POINT: u64 = 0x7E57_0002;
const PICK: u64 = 0x7E57_0003;

/// A call of the native of hash `hash`.
fn native(hash: u64, args: Vec<Expr>, returns: u32) -> Expr {
    let callee = Callee::Native {
        namespace: Some("DEMO".to_owned()),
        name: format!("_0x{hash:016X}"),
        hash,
        rva: 0,
    };
    Expr::Call {
        callee,
        args,
        returns: Some(returns),
    }
}

fn int(n: i32) -> Expr {
    Expr::Const(Const::i32(n))
}

fn kinded_with(mut f: Function, names: &Names, options: Options) -> Function {
    let project = Project::default();
    let cx = Context::new(names, &project, options);
    copied_kinds(&mut f, &cx, &mut Stats::default());
    f
}

/// `store_T(&local_n[cell], offset, value)`.
fn field(n: u32, cell: u32, kind: StoreKind, offset: u32, value: Expr) -> Stmt {
    Stmt::Store {
        kind,
        addr: Expr::AddrOf {
            var: Var::Local(n),
            cell,
        },
        offset,
        value,
    }
}

#[test]
fn a_value_handed_whole_to_a_native_a_list_says_takes_a_handle_or_a_vector_is_of_that_kind() {
    let names = listed();
    let body = || {
        vec![
            set(0, engine()),
            Stmt::Eval(native(WAVE, vec![local(0), int(3)], 0)),
            set(2, Expr::ZeroSlots(2)),
            Stmt::Eval(native(POINT, vec![local(2)], 0)),
        ]
    };
    let wide = [(0, TWO), (2, TWO)];
    let done = kinded_with(function(&wide, body()), &names, on());
    assert_eq!(kind_of(&done, Var::Local(0)), Some(Kind::Handle));
    assert_eq!(kind_of(&done, Var::Local(2)), Some(Kind::Vector));
    assert_eq!(done.cfg, function(&wide, body()).cfg);

    // With no list, or with either name off, nothing is given a kind.
    let f = function(&wide, body());
    assert_eq!(kinded_with(f.clone(), &Names::default(), on()), f);
    for off in [Pass::Kinds, Pass::Aggregates] {
        assert_eq!(kinded_with(f.clone(), &names, on().without(off)), f);
    }

    // A call that passes other cells than its list's parameters take says nothing of any.
    let more = vec![
        set(0, engine()),
        Stmt::Eval(native(WAVE, vec![local(0), int(3), int(4)], 0)),
    ];
    let f = function(&[(0, TWO)], more);
    assert_eq!(kinded_with(f.clone(), &names, on()), f);

    // Handed to a handle and to a vector: neither.
    let both = vec![
        Stmt::Eval(native(WAVE, vec![local(0), int(3)], 0)),
        Stmt::Eval(native(POINT, vec![local(0)], 0)),
    ];
    let f = function(&[(0, TWO)], both);
    assert_eq!(kinded_with(f.clone(), &names, on()), f);
}

#[test]
fn a_value_one_way_in_of_which_is_a_native_a_list_says_yields_a_handle_is_a_handle() {
    let names = listed();
    let body = vec![set(0, native(PICK, Vec::new(), 2)), set(0, engine())];
    let done = kinded_with(function(&[(0, TWO)], body), &names, on());
    assert_eq!(kind_of(&done, Var::Local(0)), Some(Kind::Handle));
}

#[test]
fn a_value_filled_field_by_field_and_handed_whole_to_a_vector_is_a_vector() {
    let names = listed();
    let x = Expr::Const(Const::f32(1.0));
    let filled = |handed: u64, last: Stmt| {
        vec![
            set(0, Expr::ZeroSlots(2)),
            field(0, 0, StoreKind::I32, 0, x.clone()),
            field(0, 0, StoreKind::I32, 4, x.clone()),
            last,
            Stmt::Eval(native(handed, vec![local(0)], 0)),
        ]
    };
    let third = || field(0, 1, StoreKind::I32, 0, x.clone());
    let done = kinded_with(function(&[(0, TWO)], filled(POINT, third())), &names, on());
    assert_eq!(kind_of(&done, Var::Local(0)), Some(Kind::Vector));

    let untouched = |body: Vec<Stmt>| {
        let f = function(&[(0, TWO)], body);
        assert_eq!(kinded_with(f.clone(), &names, on()), f);
    };
    // A handle is whole wherever it is used: one whose fields are written is none.
    untouched(filled(WAVE, third()));
    // A store that runs past the value's end writes what lies beyond it.
    untouched(filled(POINT, field(0, 1, StoreKind::U64, 4, x.clone())));
    // A store of more than one cell's bytes may bring in part of a value of another kind.
    let wide = Stmt::StoreBytes {
        addr: own(0),
        offset: 0,
        bytes: 12,
        value: local(4),
    };
    untouched(filled(POINT, wide));
}
