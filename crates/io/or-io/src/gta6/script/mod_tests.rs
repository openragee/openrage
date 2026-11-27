use super::*;
use crate::gta5::script::natives::{NativeDb, NativeName};

#[test]
fn a_listing_decompiles_from_end_to_end() {
    let done = decompile(sample::LISTING, &Names::default()).expect("the sample decompiles");
    let statements: usize = sample::STATEMENTS.iter().sum();
    assert_eq!(
        done.stats,
        Stats {
            functions: 5,
            incomplete: 1,
            statements_in: statements,
            // Three tuples are six assignments; three values nothing reads and a load of a
            // global nothing reads go, one temporary moves into the switch that reads it, and
            // the clear of a local that is only ever zero goes with the local.
            statements_out: statements + 3 - 4 - 1 - 1,
            gotos_in: 3,
            gotos_out: 0,
            temporaries_in: 12,
            temporaries_out: 7,
            native_calls: 3,
            natives_named: 2,
            constructs: [0; CONSTRUCTS.len()],
        }
    );

    // The first line says what was read: a lifted listing, and no compiled script.
    let first = done.text.lines().next().unwrap_or_default();
    assert!(
        first.starts_with("// Readable source of a lifted listing")
            && first.ends_with("not from the compiled script."),
        "{first}"
    );
    let head = "\
// program 0x11223344
// imports: 0x0A0B0C0D, 0x1A2B3C4D
";
    assert!(done.text.contains(head), "{}", done.text);
    for line in [
        "void func_000000(void* p0, void* p1)",
        "var func_000100(var p0)",
        "var[] func_000200()",
        "// func_000300 at 0x000300: the listing holds no body (Widths are underdetermined.)",
        "void main()",
        "    DEMO.DO_THING(0);",
        "    _0x00000000000000BB(p0, BuiltinGlobal(0)[0]);",
        // A store says how wide it is, and a pointer is gone through with `->`.
        "    p0->f_10 = (byte)1;",
        "        if ((byte)p1->f_8->f_10 == 0)",
        "    v_000028_3.f_0 = 0x1234ABCD;",
        // The loop the two gotos made, and the switch without the one its value was biased by.
        "    while ((int)local_1 < (int)example_length(local_0));",
        "    while (local_0 < 10)",
        "        switch ((int)DEMO.GET_THING(local_0))",
        "            case 2:",
        "                return { local_0, local_1 };",
        "    return { local_0, local_1 };",
    ] {
        assert!(
            done.text.lines().any(|l| l == line),
            "no line `{line}` in:\n{}",
            done.text
        );
    }
    // The temporaries nothing reads are gone with what they held.
    for gone in ["v_00010C_1", "v_000114_3", "v_000118_4", "v_000110_2"] {
        assert!(!done.text.contains(gone), "{gone} in:\n{}", done.text);
    }
}

#[test]
fn a_native_takes_the_name_the_users_list_gives_its_hash() {
    let listed = NativeName {
        namespace: Some("LISTED".to_owned()),
        name: "NAMED_BY_THE_LIST".to_owned(),
        ..NativeName::default()
    };
    let names = Names {
        natives: Some(NativeDb::from_names("a test's", [(0xBB, listed)])),
        ..Names::default()
    };
    let done = decompile(sample::LISTING, &names).expect("the sample decompiles");
    assert_eq!((done.stats.native_calls, done.stats.natives_named), (3, 3));
    assert!(
        done.text
            .contains("LISTED.NAMED_BY_THE_LIST(p0, BuiltinGlobal(0)[0]);"),
        "{}",
        done.text
    );
}

#[test]
fn a_listing_that_does_not_read_or_lower_is_refused() {
    let unread = decompile("not a listing\n", &Names::default());
    assert!(
        matches!(&unread, Err(DecompileError::Listing(e)) if e.line() == 1),
        "{unread:?}"
    );

    let text = "[VmProgram(0x00000001U)]
public static class Program_00000001
{
    [VmFunction(0x000010)]
    public static void func_000010() // 0x000010
    {
        VmValue[] parameters = new VmValue[] {  };
        VmValue[] locals = new VmValue[0];
        goto label_000020;
    }
}
";
    let unlowered = decompile(text, &Names::default());
    let Err(DecompileError::Lower { function, source }) = &unlowered else {
        panic!("a goto to no label lowered: {unlowered:?}");
    };
    assert_eq!(function, "func_000010");
    assert_eq!(
        *source,
        cfg::LowerError::UnknownLabel(tree::Label::At(0x20))
    );
}

#[test]
fn a_function_of_more_blocks_than_the_passes_are_given_is_written_as_its_blocks() {
    // A chain of jumps whose first block names a constant and hands it to a call.
    let temporaries = |blocks: usize| {
        let named = ir::Var::Value { at: 0, index: 0 };
        let callee = ir::Callee::Local {
            at: 0x100,
            name: "func_000100".to_owned(),
        };
        let used = ir::Expr::Call {
            callee,
            args: vec![ir::Expr::Var(named)],
            returns: None,
        };
        let first = vec![
            ir::Stmt::Assign {
                target: named,
                value: ir::Expr::Const(ir::Const::i32(5)),
            },
            ir::Stmt::Eval(used),
        ];
        let block = |n: usize| cfg::Block {
            at: Some(n as u32),
            stmts: match n {
                0 => first.clone(),
                _ => Vec::new(),
            },
            term: match n + 1 < blocks {
                true => cfg::Terminator::Jump(n + 1),
                false => cfg::Terminator::Return(None),
            },
        };
        let head = ir::Head {
            program: 1,
            at: 0,
            name: "func_000000".to_owned(),
            returns: ir::Returns::Void,
            params: 0,
            locals: 0,
            vars: [(named, ir::VarInfo::new(ir::Width::Cell))].into(),
            incomplete: None,
            value_types: Default::default(),
        };
        let cfg = cfg::Cfg {
            blocks: (0..blocks).map(block).collect(),
        };
        let names = Names::default();
        let cx = Context::new(&names, Project::empty(), Options::default());
        let mut stats = Stats::default();
        blocks_in(cfg::Function { head, cfg }, &cx, &mut stats);
        stats.temporaries_out
    };
    // The passes fold the constant into the call: past the bound none runs, and its name stays.
    assert_eq!(temporaries(64), 0);
    assert_eq!(temporaries(BLOCKS + 1), 1);
}

#[test]
fn stats_add_up() {
    let one = Stats {
        functions: 1,
        incomplete: 2,
        statements_in: 3,
        statements_out: 4,
        gotos_in: 5,
        gotos_out: 6,
        temporaries_in: 7,
        temporaries_out: 8,
        native_calls: 9,
        natives_named: 10,
        constructs: [11; CONSTRUCTS.len()],
    };
    let mut sum = one;
    sum.add(&one);
    assert_eq!(
        sum,
        Stats {
            functions: 2,
            incomplete: 4,
            statements_in: 6,
            statements_out: 8,
            gotos_in: 10,
            gotos_out: 12,
            temporaries_in: 14,
            temporaries_out: 16,
            native_calls: 18,
            natives_named: 20,
            constructs: [22; CONSTRUCTS.len()],
        }
    );
}

#[test]
fn a_compiled_program_is_refused_until_it_can_be_read() {
    let names = Names::default();
    let refused = decompile_program(&[], &Context::baseline(&names));
    assert!(
        matches!(
            &refused,
            Err(DecompileError::Program(_) | DecompileError::Lift(_))
        ),
        "{refused:?}"
    );
    // The refusal says what it is of, for a command to print.
    let said = refused.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(!said.is_empty());
}

#[test]
fn the_passes_asked_for_are_the_passes_run() {
    let names = Names::default();
    let with = |options: Options| {
        let cx = Context::new(&names, Project::empty(), options);
        decompile_listing(sample::LISTING, &cx).expect("the sample decompiles")
    };
    let always = decompile(sample::LISTING, &names).expect("the sample decompiles");

    // The four first passes through the entry that takes options are `decompile`, byte for byte.
    let baseline = with(Options::baseline());
    assert_eq!(baseline.files.len(), 1);
    assert_eq!(baseline.files[0].text, always.text);
    assert_eq!(baseline.stats, always.stats);

    // With `structure` switched off no construct is rebuilt: the jumps of the blocks stay jumps.
    let flat = with(Options::baseline().without(Pass::Structure));
    assert!(
        flat.stats.gotos_out > always.stats.gotos_out,
        "{:?}",
        flat.stats
    );
    assert!(
        flat.files[0].text.contains("goto "),
        "{}",
        flat.files[0].text
    );

    // At `Lifted` no pass runs and the listing's own syntax is written again.
    let lifted = with(Options::at(Level::Lifted));
    assert!(
        lifted.files[0].text.contains("[VmProgram("),
        "{}",
        lifted.files[0].text
    );
    assert_eq!(lifted.stats.functions, always.stats.functions);
    assert_eq!(lifted.stats.statements_out, 0);

    // At `Source` every function is still written, whatever the reconstructions make of it.
    let source = with(Options::default());
    assert!(!source.files.is_empty());
    assert_eq!(source.stats.functions, always.stats.functions);
}

#[test]
fn the_constructs_given_to_the_printer_are_counted_and_a_hidden_is_no_statement_of_the_source() {
    use tree::{Group, Label, Stmt as S, Sugar};
    let set = |n: u32| ir::Stmt::Assign {
        target: ir::Var::Local(n),
        value: Expr::Const(ir::Const::i32(1)),
    };
    let hidden = vec![
        S::Simple(set(0)),
        S::Goto(Label::Block(1)),
        S::Label(Label::Block(1)),
    ];
    let compound = Sugar::Group {
        kind: Group::Compound,
        parts: vec![set(1)],
    };
    let read = Expr::sugar(ExprSugar::Method, Expr::Var(ir::Var::Local(0)));
    let body = vec![
        S::Sugar(Box::new(Sugar::Hidden(hidden))),
        S::Sugar(Box::new(compound)),
        S::Return(Some(read)),
    ];
    let head = ir::Head {
        program: 1,
        at: 0,
        name: "func_000000".to_owned(),
        returns: ir::Returns::Cell,
        params: 0,
        locals: 2,
        vars: Default::default(),
        incomplete: None,
        value_types: Default::default(),
    };
    let f = tree::Function { head, body };
    let names = Names::default();
    let count = |level: Level| {
        let cx = Context::new(&names, Project::empty(), Options::at(level));
        let mut stats = Stats::default();
        counted(&f, &cx, &mut stats);
        stats
    };
    let made = |stats: &Stats, kind: &str| {
        let at = CONSTRUCTS.iter().position(|held| *held == kind);
        at.map(|at| stats.constructs[at])
    };
    let (structured, source) = (count(Level::Structured), count(Level::Source));
    for stats in [&structured, &source] {
        for kind in ["hidden", "compound", "method"] {
            assert_eq!(made(stats, kind), Some(1), "{kind}");
        }
        assert_eq!(stats.constructs.iter().sum::<usize>(), 3);
    }
    // Below `Source` a `Hidden` is written as its statements, and at `Source` as nothing.
    assert_eq!((structured.statements_out, structured.gotos_out), (2, 1));
    assert_eq!((source.statements_out, source.gotos_out), (1, 0));
}
