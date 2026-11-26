use super::*;
use crate::gta6::script::ir::{Const, Expr, Head, Scalar, Stmt, StoreKind};
use crate::gta6::script::print::method;
use crate::gta6::script::print::tests::{at, head, local, native, param, simple};
use crate::gta6::script::{Context, Level, Names, Options, Pass, Project};

const OWNER: TypeKey = TypeKey(0xAB);

fn member(role: Role, head: Head, body: Vec<tree::Stmt>) -> Method {
    Method {
        function: tree::Function { head, body },
        role,
        is_static: false,
        is_async: false,
        slot: None,
        overrides: false,
    }
}

/// A head of `params` parameters, the receiver among them, that hands back what `returns` says.
fn headed(params: u32, returns: Returns) -> Head {
    let mut head = head();
    head.params = params;
    head.returns = returns;
    head
}

/// `member` as true C# writes it in a class named `CThing`, one level in.
fn written_in(member: &Method, names: &Names) -> String {
    let mut out = String::new();
    let cx = at(names, Level::Source);
    method(&mut out, member, Some((OWNER, "CThing")), 1, &cx);
    out
}

fn written(member: &Method) -> String {
    written_in(member, &Names::default())
}

/// The first line of what is written.
fn first(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

fn kept() -> tree::Stmt {
    simple(Stmt::Store {
        kind: StoreKind::U64,
        addr: param(0),
        offset: 0x8,
        value: param(1),
    })
}

#[test]
fn a_constructor_and_a_finalizer_go_by_the_name_of_their_class() {
    let made = member(Role::Constructor, headed(2, Returns::Void), vec![kept()]);
    let expected = "    CThing(Cell p1) // func_000040
    {
        this.f_8 = p1;
    }
";
    assert_eq!(written(&made), expected);

    let mut readied = member(
        Role::StaticConstructor,
        headed(0, Returns::Void),
        Vec::new(),
    );
    readied.is_static = true;
    let expected = "    static CThing() // func_000040\n    {\n    }\n";
    assert_eq!(written(&readied), expected);

    let ended = member(Role::Finalizer, headed(1, Returns::Void), Vec::new());
    let expected = "    ~CThing() // func_000040\n    {\n    }\n";
    assert_eq!(written(&ended), expected);
}

#[test]
fn a_role_is_taken_only_by_a_function_of_its_shape() {
    // ⚠ A getter takes nothing, a setter yields nothing, and a constructor has an object to
    // run on and hands nothing back: any other function is written as a method.
    let method_head = "    Cell func_000040(Cell p1)";
    let asks = member(Role::Getter, headed(2, Returns::Cell), Vec::new());
    assert_eq!(first(&written(&asks)), method_head);
    let answers = member(Role::Setter, headed(2, Returns::Cell), Vec::new());
    assert_eq!(first(&written(&answers)), method_head);
    let yields = member(Role::Constructor, headed(2, Returns::Cell), Vec::new());
    assert_eq!(first(&written(&yields)), method_head);

    let mut apart = member(Role::Constructor, headed(2, Returns::Void), Vec::new());
    apart.is_static = true;
    let expected = "    static void func_000040(Cell p0, Cell p1)";
    assert_eq!(first(&written(&apart)), expected);
    let mut taking = member(
        Role::StaticConstructor,
        headed(1, Returns::Void),
        Vec::new(),
    );
    taking.is_static = true;
    let expected = "    static void func_000040(Cell p0)";
    assert_eq!(first(&written(&taking)), expected);
}

#[test]
fn an_accessor_is_a_property_and_a_getter_that_only_yields_is_its_value() {
    let field = Expr::Load {
        ty: Scalar::U64,
        addr: Box::new(param(0)),
        offset: 0x10,
    };
    let yields = vec![tree::Stmt::Return(Some(field))];
    let get = member(Role::Getter, headed(1, Returns::Cell), yields);
    assert_eq!(written(&get), "    Cell func_000040 => this.f_10;\n");

    let set = member(Role::Setter, headed(2, Returns::Void), vec![kept()]);
    let expected = "    Cell func_000040
    {
        set
        {
            this.f_8 = value;
        }
    }
";
    assert_eq!(written(&set), expected);

    // ⚠ A getter that reads a local declares it, which an expression alone cannot.
    let read = vec![tree::Stmt::Return(Some(local(0)))];
    let reads = member(Role::Getter, headed(1, Returns::Cell), read);
    let expected = "    Cell func_000040
    {
        get
        {
            Cell local_0;
            return local_0;
        }
    }
";
    assert_eq!(written(&reads), expected);
}

#[test]
fn a_method_says_what_is_proved_of_its_slot_and_goes_by_the_users_name() {
    let method_of = |slot: Option<u32>, overrides: bool| {
        let mut held = member(Role::Method, headed(1, Returns::Void), Vec::new());
        held.slot = slot;
        held.overrides = overrides;
        held
    };
    let declared = "    virtual void Method_2() // func_000040";
    assert_eq!(first(&written(&method_of(Some(2), false))), declared);
    let overriding = "    override void Method_2() // func_000040";
    assert_eq!(first(&written(&method_of(Some(2), true))), overriding);
    assert_eq!(
        first(&written(&method_of(None, false))),
        "    void func_000040()"
    );

    let mut waits = method_of(None, false);
    waits.is_async = true;
    assert_eq!(first(&written(&waits)), "    async void func_000040()");
    // ⚠ A function that takes no receiver fills no slot, whatever its member says.
    let mut fixed = method_of(Some(2), false);
    fixed.is_static = true;
    assert_eq!(
        first(&written(&fixed)),
        "    static void func_000040(Cell p0)"
    );

    // The user's name for the slot, on the type or on a base of it; their name for the function before either.
    let mut names = Names::default();
    names.methods.insert((OWNER, 2), "Update".to_owned());
    let named = written_in(&method_of(Some(2), false), &names);
    assert_eq!(first(&named), "    virtual void Update() // func_000040");
    let key = FunctionKey::new(0x1122_3344, 0x40);
    names.functions.insert(key, "Tick".to_owned());
    let named = written_in(&method_of(Some(2), false), &names);
    assert_eq!(first(&named), "    virtual void Tick() // func_000040");
}

#[test]
fn the_form_of_old_knows_no_class_and_a_function_no_type_owns_is_written_free() {
    let made = member(Role::Constructor, headed(2, Returns::Void), vec![kept()]);
    let names = Names::default();
    let mut old = String::new();
    let class = Some((OWNER, "CThing"));
    method(&mut old, &made, class, 1, &Context::baseline(&names));
    let expected = "    void func_000040(var p0, var p1)
    {
        p0->f_8 = p1;
    }
";
    assert_eq!(old, expected);

    let mut free = String::new();
    method(&mut free, &made, None, 0, &at(&names, Level::Source));
    let expected = "\
void func_000040(Cell p0, Cell p1)
{
    p0->f_8 = p1;
}
";
    assert_eq!(free, expected);
}

#[test]
fn a_member_is_broken_at_the_width_from_where_it_stands() {
    let args = (0..30).map(local).collect();
    let body = vec![simple(Stmt::Eval(native("USE", args)))];
    let wide = member(Role::Method, headed(1, Returns::Void), body);
    let mut out = String::new();
    let names = Names::default();
    let class = Some((OWNER, "CThing"));
    method(&mut out, &wide, class, 3, &at(&names, Level::Source));
    assert!(out.lines().count() > 6, "{out}");
    assert!(out.lines().all(|line| line.chars().count() <= 120), "{out}");
    // Three levels in, the body one more and what a statement runs on to one more again.
    assert!(out.contains("\n                DEMO.USE(local_0,"), "{out}");
    assert!(out.contains(",\n                    local_"), "{out}");
}

#[test]
fn what_the_receiver_holds_is_written_with_a_dot_and_its_own_cell_through_its_address() {
    let read = |ty: Scalar, offset: u32| Expr::Load {
        ty,
        addr: Box::new(param(0)),
        offset,
    };
    let inside = |offset: i32| Expr::ByteOffset {
        addr: Box::new(param(0)),
        offset: Box::new(Expr::Const(Const::i32(offset))),
    };
    let embedded = Expr::Load {
        ty: Scalar::I32,
        addr: Box::new(inside(0x20)),
        offset: 4,
    };
    let args = vec![
        read(Scalar::U64, 0),
        read(Scalar::I32, 0xC),
        embedded,
        inside(0x38),
    ];
    let body = vec![
        simple(Stmt::Eval(native("USE", args))),
        simple(Stmt::StepI32 {
            addr: param(0),
            offset: 0x14,
            down: false,
        }),
        // ⚠ Four bytes of the cell the receiver stands in are no member of the object.
        simple(Stmt::Store {
            kind: StoreKind::I32,
            addr: Expr::AddrOf {
                var: Var::Param(0),
                cell: 0,
            },
            offset: 0,
            value: param(1),
        }),
    ];
    let held = member(Role::Method, headed(2, Returns::Void), body);
    let expected = "    void func_000040(Cell p1)
    {
        DEMO.USE(this.f_0, (int)this.f_C, (int)this.f_20.f_4, &this.f_38);
        this.f_14++;
        (&this)->f_0 = (int)p1;
    }
";
    assert_eq!(written(&held), expected);

    // With `fields` off the receiver is written as the pointer the code holds.
    let names = Names::default();
    let off = Options::default().without(Pass::Fields);
    let cx = Context::new(&names, Project::empty(), off);
    let mut out = String::new();
    method(&mut out, &held, Some((OWNER, "CThing")), 1, &cx);
    let expected = "    void func_000040(Cell p1)
    {
        DEMO.USE(*this, (int)this->f_C, (int)this->f_20.f_4, &this->f_38);
        this->f_14++;
        this.f_0 = (int)p1;
    }
";
    assert_eq!(out, expected);

    // A function no class owns has no receiver: its first parameter stays a pointer.
    let mut free = String::new();
    method(&mut free, &held, None, 0, &at(&names, Level::Source));
    assert!(free.contains("    DEMO.USE(*p0, (int)p0->f_C, "), "{free}");
    assert!(free.contains("    p0.f_0 = (int)p1;\n"), "{free}");
}

#[test]
fn an_async_method_that_hands_its_task_back_early_is_written_leaving_with_a_bare_return() {
    let task = || Expr::Load {
        ty: Scalar::U64,
        addr: Box::new(local(0)),
        offset: 0x10,
    };
    let hidden = |stmts| tree::Stmt::Sugar(Box::new(tree::Sugar::Hidden(stmts)));
    let early = tree::Stmt::If {
        cond: param(1),
        then: vec![tree::Stmt::Return(Some(task()))],
        els: Vec::new(),
    };
    let other = tree::Stmt::If {
        cond: param(1),
        then: vec![tree::Stmt::Return(Some(param(1)))],
        els: Vec::new(),
    };
    let last = hidden(vec![tree::Stmt::Return(Some(task()))]);
    let body = vec![early, other, kept(), last];
    let mut kicked = member(Role::Method, headed(2, Returns::Cell), body.clone());
    kicked.is_async = true;
    let text = written(&kicked);
    // The task is what the hidden last `return` hands back: a `return` of it is the source's `return;`.
    let expected = "    async Cell func_000040(Cell p1)
    {
        Cell local_0;
        if (p1)
        {
            return;
        }
        if (p1)
        {
            return p1;
        }
        this.f_8 = p1;
    }
";
    assert_eq!(text, expected);
    // A method that is not async returns what it returns.
    let plain = member(Role::Method, headed(2, Returns::Cell), body);
    assert!(
        written(&plain).contains("return local_0->f_10;"),
        "{}",
        written(&plain)
    );
}
