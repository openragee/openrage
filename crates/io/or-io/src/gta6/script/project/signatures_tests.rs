use super::*;
use crate::gta6::script::ir::TypeKey;
use crate::gta6::script::listing::Listing;
use crate::gta6::script::passes::{types, values};
use crate::gta6::script::program::{EntryRow, FunctionRow};
use crate::gta6::script::project::TypeFacts;
use crate::gta6::script::Stats;

/// What the listings of these tests call through, declared as a listing declares it.
const DECLARATIONS: &str = r#"
    public static class Builtins
    {
        public static class System
        {
            public static class Guid
            {
                public static VmValue op_Equality(VmValue p0, VmValue p1, VmValue p2, VmValue p3) => Vm.Builtin(3, 1, 1, p0, p1, p2, p3)[0];
            }
        }
    }
    public static class Imports
    {
        public static class other
        {
            public static VmValue func_000040(VmValue p0) => Vm.Script(0x00000002U, 0x000040, 1, p0)[0];
        }
    }
    public static class Virtual
    {
        public static class type_00000001000000C1
        {
            public static VmValue Method_3(VmValue p0, VmValue p1) => Vm.Virtual("type_00000001000000C1", 3, 1, p0, p1)[0];
        }
        public static class type_00000001000000D1
        {
            public static VmValue Method_3(VmValue p0, VmValue p1) => Vm.Virtual("type_00000001000000D1", 3, 1, p0, p1)[0];
        }
    }
    public static class Native
    {
        public static class DEMO
        {
            public static void USE(VmValue p0) => Vm.Native(0x0000000000000003UL, 0x00000003U, 0, p0);
            public static VmValue ASK(VmValue p0) => Vm.Native(0x0000000000000005UL, 0x00000005U, 1, p0)[0];
            public static VmValue GET(VmValue p0) => Vm.Native(0x0000000000000006UL, 0x00000006U, 1, p0)[0];
        }
    }
}
"#;

/// A listing of the program `program`, written by hand, that holds `functions`.
pub(crate) fn listing(program: u32, functions: &[String]) -> String {
    let head =
        format!("[VmProgram(0x{program:08X}U)]\npublic static class Program_{program:08X}\n{{\n");
    [head, functions.concat(), DECLARATIONS.to_owned()].concat()
}

/// One function of such a listing: `ty func_<at>` of `params` parameters and four locals, which runs `body`.
pub(crate) fn function(ty: &str, at: u32, params: u32, body: &str) -> String {
    let args: Vec<String> = (0..params).map(|n| format!("arg{n}")).collect();
    let typed: Vec<String> = args.iter().map(|arg| format!("VmValue {arg}")).collect();
    let lines = [
        format!("    [VmFunction(0x{at:06X})]"),
        format!("    public static {ty} func_{at:06X}({}) // 0x{at:06X}", typed.join(", ")),
        "    {".to_owned(),
        format!("        VmValue[] parameters = new VmValue[] {{ {} }};", args.join(", ")),
        "        VmValue[] locals = new VmValue[4];".to_owned(),
        "        VmValue v_000001_0 = default; VmValues v_000002_1 = null!;".to_owned(),
        format!("        {body}"),
        "        throw new global::System.InvalidOperationException(\"Unexpected bytecode fallthrough.\");".to_owned(),
        "    }".to_owned(),
        String::new(),
    ];
    lines.join("\n")
}

/// What the functions of the listings show, each read as the indexer reads it, with nothing carried along the calls yet.
fn parts_of(listings: &[String]) -> Project {
    let names = Names::default();
    let mut stats = Stats::default();
    let mut project = Project::default();
    for text in listings {
        let listing = Listing::open(text).expect("the listing opens");
        for read in listing.functions() {
            let read = read.expect("the function reads");
            let mut f = cfg::lower(read).expect("the function lowers");
            let first = first_read(&f);
            values::run(&mut f, &names, &mut stats);
            types::run(&mut f, &names, &mut stats);
            of_function_read(&mut project, &f, first);
        }
    }
    project
}

fn project_of(listings: &[String]) -> Project {
    let mut project = parts_of(listings);
    settle(&mut project);
    project
}

fn key(at: u32) -> FunctionKey {
    FunctionKey::new(1, at)
}

fn result(project: &Project, at: u32) -> Kind {
    project.signatures[&key(at)].result.clone()
}

/// The cells of each parameter of the function at `at`, in order.
fn cells(project: &Project, at: u32) -> Vec<u32> {
    let params = &project.signatures[&key(at)].params;
    params.iter().map(|param| param.cells).collect()
}

const ORDERED: &str = "return ((as_i32(p0) < as_i32(p1)));";

#[test]
fn a_result_takes_the_kind_every_cell_it_can_be_is_widened_as() {
    let or_zero = format!("if (((as_u32(p0) == 0))) {{ return 0; }} {ORDERED}");
    let or_two = format!("if (((as_u32(p0) == 0))) {{ return 2; }} {ORDERED}");
    let or_given = format!("if (((as_u32(p0) == 0))) {{ return p1; }} {ORDERED}");
    let functions = [
        function("VmValue", 0x10, 2, &or_zero),
        function("VmValue", 0x20, 2, &or_two),
        function("VmValue", 0x30, 2, &or_given),
        function("void", 0x40, 0, "return;"),
    ];
    let project = project_of(&[listing(1, &functions)]);
    assert_eq!(result(&project, 0x10), Kind::Bool);
    // Two is no truth: read as one, the result would be another cell.
    assert_eq!(result(&project, 0x20), Kind::Unknown);
    // A parameter is whatever the caller made it.
    assert_eq!(result(&project, 0x30), Kind::Unknown);
    let returns = |at| project.signatures[&key(at)].returns;
    assert_eq!((returns(0x10), returns(0x40)), (1, 0));
}

#[test]
fn a_variable_returned_is_what_its_assignments_make_it_unless_an_address_reaches_it() {
    let test = "local_0 = ((as_i32(p0) < as_i32(p1)));";
    let kept = format!(
        "local_1 = 0; if (((as_u32(p0) != 0))) {{ {test} local_1 = local_0; }} return local_1;"
    );
    let reached = format!("{test} Native.DEMO.USE(Vm.Address(locals, 0)); return local_0;");
    let functions = [
        function("VmValue", 0x10, 2, &kept),
        function("VmValue", 0x20, 2, &reached),
    ];
    let project = project_of(&[listing(1, &functions)]);
    assert_eq!(result(&project, 0x10), Kind::Bool);
    // The callee may have written the local through its address.
    assert_eq!(result(&project, 0x20), Kind::Unknown);
}

#[test]
fn a_result_handed_on_is_the_result_of_the_function_it_came_from() {
    let functions = [
        function("VmValue", 0x10, 1, "return func_000020(p0);"),
        function("VmValue", 0x20, 1, "return ((as_i32(p0) < 5));"),
        function(
            "VmValue",
            0x30,
            1,
            "if (((as_u32(p0) == 0))) { return 3; } return func_000020(p0);",
        ),
        function("VmValue", 0x50, 1, "return func_000060(p0);"),
        function("VmValue", 0x60, 1, "return func_000050(p0);"),
        function("VmValue", 0x70, 1, "return func_000080(p0);"),
    ];
    let listings = [listing(1, &functions)];

    let parts = parts_of(&listings);
    assert!(!settled(&parts.signatures[&key(0x10)]));
    assert!(settled(&parts.signatures[&key(0x20)]));

    let project = project_of(&listings);
    assert!(project.signatures.values().all(settled));
    assert_eq!(cells(&project, 0x10), [1]);
    assert_eq!(result(&project, 0x10), Kind::Bool);
    // Its own constant is no truth, whatever the function it also hands on returns.
    assert_eq!(result(&project, 0x30), Kind::Unknown);
    // Two functions that only hand each other's result on return nothing that types either.
    assert_eq!(result(&project, 0x50), Kind::Unknown);
    assert_eq!(result(&project, 0x60), Kind::Unknown);
    // A function the project does not hold returns what nothing knows.
    assert_eq!(result(&project, 0x70), Kind::Unknown);
    assert_eq!(project.calls[&key(0x10)], [key(0x20)]);
}

#[test]
fn what_a_signature_waited_on_and_lost_proves_nothing() {
    let lost = Param {
        kind: Kind::Value {
            ty: "not what was put".to_owned(),
            cells: 0,
        },
        cells: 0,
    };
    let waiting = Signature {
        params: vec![lost],
        returns: 1,
        result: Kind::Bool,
    };
    let mut project = Project::default();
    project.signatures.insert(key(0x10), waiting);
    settle(&mut project);
    let held = &project.signatures[&key(0x10)];
    assert_eq!((held.params.len(), &held.result), (0, &Kind::Unknown));
}

#[test]
fn parameter_cells_only_ever_used_together_are_one_parameter() {
    let stored = "store_bytes(p0, 0x20, 16, new VmValues(p1, p2));";
    let alone = format!("{stored} Native.DEMO.USE(p2); return;");
    let compared = "return Builtins.System.Guid.op_Equality(p0, p1, p2, p3);";
    let functions = [
        function("void", 0x10, 3, &format!("{stored} return;")),
        function("void", 0x20, 3, &alone),
        function("VmValue", 0x30, 4, compared),
    ];
    let project = project_of(&[listing(1, &functions)]);
    assert_eq!(cells(&project, 0x10), [1, 2]);
    let params = &project.signatures[&key(0x10)].params;
    assert_eq!(params[1].kind, Kind::Aggregate(2));
    // The second cell is also read on its own: the two are two parameters.
    assert_eq!(cells(&project, 0x20), [1, 1, 1]);
    let handles = &project.signatures[&key(0x30)].params;
    assert_eq!(cells(&project, 0x30), [2, 2]);
    assert!(handles.iter().all(|param| param.kind == Kind::Handle));
}

#[test]
fn a_parameter_only_passed_on_is_read_as_its_callee_reads_it() {
    let reads = "Native.DEMO.USE(((as_f32(p0) * as_f32(2.5f)))); return;";
    let functions = [
        function("void", 0x10, 1, "func_000020(p0); return;"),
        function("void", 0x20, 1, reads),
        function(
            "void",
            0x30,
            1,
            "func_000020(p0); Native.DEMO.USE(p0); return;",
        ),
    ];
    let project = project_of(&[listing(1, &functions)]);
    let kind = |at| project.signatures[&key(at)].params[0].kind.clone();
    assert_eq!(kind(0x20), Kind::F32);
    assert_eq!(kind(0x10), Kind::F32);
    // A native reads it too, and nothing says as what.
    assert_eq!(kind(0x30), Kind::Unknown);
}

#[test]
fn the_cells_of_a_result_of_several_come_from_its_returns_or_from_the_function_handed_on() {
    let functions = [
        function("VmValues", 0x10, 0, "return func_000020();"),
        function("VmValues", 0x20, 0, "return new VmValues(1, 2);"),
        function("VmValues", 0x30, 0, "return func_000090();"),
        function("VmValue", 0x50, 1, "return Imports.other.func_000040(p0);"),
    ];
    let project = project_of(&[listing(1, &functions)]);
    let returns = |at| project.signatures.get(&key(at)).map(|held| held.returns);
    assert_eq!((returns(0x20), returns(0x10)), (Some(2), Some(2)));
    // Nothing states the count: no signature is better than a wrong one.
    assert_eq!(returns(0x30), None);
    assert_eq!(project.calls[&key(0x50)], [FunctionKey::new(2, 0x40)]);
}

#[test]
fn a_compiled_program_states_the_cells_of_each_function() {
    let row = |at, params, returns| FunctionRow {
        at,
        params,
        returns,
        ..FunctionRow::default()
    };
    let program = Program {
        program: 7,
        code: vec![0; 64],
        functions: vec![
            row(0x10, 2, 3),
            row(0x20, MAX_PARAMS + 1, 0),
            row(0x30, 63, 0),
        ],
        ..Program::default()
    };
    let mut project = Project::default();
    of_program(&mut project, &program);
    let held = &project.signatures[&FunctionKey::new(7, 0x10)];
    assert_eq!((held.params.len(), held.returns), (2, 3));
    assert!(held.params.iter().all(|param| param.cells == 1));
    // More parameter cells than a function has, and more in all than the code has bytes: not believed.
    assert!(!project.signatures.contains_key(&FunctionKey::new(7, 0x20)));
    assert!(!project.signatures.contains_key(&FunctionKey::new(7, 0x30)));
}

#[test]
fn a_function_the_engine_runs_has_a_caller_no_code_shows() {
    let program = Program {
        program: 1,
        code: vec![0; 64],
        entries: vec![EntryRow { name: 9, at: 0x10 }],
        main: Some(0x20),
        ..Program::default()
    };
    let functions = [
        function("void", 0x10, 1, "return;"),
        function("void", 0x20, 1, "return;"),
        function("void", 0x30, 1, "return;"),
        function(
            "void",
            0x40,
            0,
            "func_000010(1); func_000020(2); func_000030(3); return;",
        ),
    ];
    let mut project = parts_of(&[listing(1, &functions)]);
    of_program(&mut project, &program);
    let marks = BTreeSet::from([key(0x10), key(0x20)]);
    assert_eq!(project.unseen, marks);
    settle(&mut project);
    let cells = |at: u32| project.widened.get(&key(at)).cloned().unwrap_or_default();
    assert!(cells(0x10).is_empty() && cells(0x20).is_empty());
    assert_eq!(cells(0x30), BTreeSet::from([0]));
}

const FLAG: &str = "if (((as_u32(p0) == 0))) { return 0; } return 1;";
const FLOAT: &str = "((as_f32(p0) * as_f32(2.5f)))";
const TAKES: &str = "Native.DEMO.USE(p0); return;";

/// The kind of the parameter that starts at cell `cell` of the function at `at`.
fn param(project: &Project, at: u32, cell: u32) -> Kind {
    let held = &project.signatures[&key(at)];
    let n = param_at(held, cell).expect("a parameter starts there");
    held.params[n].kind.clone()
}

#[test]
fn a_result_of_constants_alone_takes_the_kind_its_uses_read() {
    let tested =
        |at: u32, then: &str| format!("if (((as_u32(func_{at:06X}(p0)) != 0))) {{ {then} }}");
    let counted = "Native.DEMO.USE(((as_i32(func_000040(p0)) + as_i32(1))));";
    let taken = "Native.DEMO.USE(Vm.Function(0x00000001U, 0x000050));";
    let caller = [
        tested(0x10, "Native.DEMO.USE(p0);"),
        tested(0x30, "Native.DEMO.USE(p0);"),
        tested(0x40, counted),
        tested(0x50, taken),
        "return;".to_owned(),
    ];
    let functions = [
        function("VmValue", 0x10, 1, FLAG),
        function("VmValue", 0x20, 1, FLAG),
        function(
            "VmValue",
            0x30,
            1,
            "if (((as_u32(p0) == 0))) { return 0; } return 2;",
        ),
        function("VmValue", 0x40, 1, FLAG),
        function("VmValue", 0x50, 1, FLAG),
        function("void", 0x100, 1, &caller.join(" ")),
    ];
    let project = project_of(&[listing(1, &functions)]);
    assert_eq!(result(&project, 0x10), Kind::Bool);
    // Nothing calls it: zero and one are of no kind by themselves.
    assert_eq!(result(&project, 0x20), Kind::Unknown);
    // Two is no truth, however the result is tested.
    assert_eq!(result(&project, 0x30), Kind::Unknown);
    // Tested and counted with: a number.
    assert_eq!(result(&project, 0x40), Kind::I32);
    // A reference to it is taken, so the calls that name it are not all its calls.
    assert_eq!(result(&project, 0x50), Kind::Unknown);
}

#[test]
fn a_result_of_the_bits_of_floats_read_as_a_float_is_one() {
    let half = "if (((as_u32(p0) == 0))) { return 0; } return 0x3F000000;";
    let scaled =
        |at: u32| format!("Native.DEMO.USE(((as_f32(func_{at:06X}(p0)) * as_f32(2.5f))));");
    let functions = [
        function("VmValue", 0x10, 1, half),
        function(
            "VmValue",
            0x20,
            1,
            "if (((as_u32(p0) == 0))) { return 5; } return 0x3F000000;",
        ),
        function(
            "void",
            0x100,
            1,
            &format!("{} {} return;", scaled(0x10), scaled(0x20)),
        ),
    ];
    let project = project_of(&[listing(1, &functions)]);
    assert_eq!(result(&project, 0x10), Kind::F32);
    // Five is no float a source holds.
    assert_eq!(result(&project, 0x20), Kind::Unknown);
}

#[test]
fn a_result_handed_on_or_kept_is_read_as_what_holds_it_is() {
    let kept = [
        "local_0 = func_000040(p0);",
        "if (((as_u32(local_0) != 0))) { Native.DEMO.USE(local_0); }",
        "return;",
    ];
    let functions = [
        function("VmValue", 0x10, 1, "return func_000020(p0);"),
        function("VmValue", 0x20, 1, FLAG),
        function(
            "void",
            0x30,
            1,
            "if (((as_u32(func_000010(p0)) != 0))) { Native.DEMO.USE(p0); } return;",
        ),
        function("VmValue", 0x40, 1, FLAG),
        function("void", 0x50, 1, &kept.join(" ")),
    ];
    let project = project_of(&[listing(1, &functions)]);
    // Only the function that hands it on is tested: the test reads both.
    assert_eq!(result(&project, 0x20), Kind::Bool);
    assert_eq!(result(&project, 0x10), Kind::Bool);
    // ⚠ The variable that keeps the result is tested, and handed whole to a native, which may
    // read it as anything.
    assert_eq!(result(&project, 0x40), Kind::Unknown);
}

#[test]
fn readings_of_one_result_join_to_the_widest_or_to_none() {
    use Usage::{Int, Mixed, Truth, Unsaid, F32, I32, U32};
    assert_eq!(Unsaid.join(Truth), Truth);
    assert_eq!(Truth.join(Int), Int);
    assert_eq!(Int.join(U32), U32);
    assert_eq!(I32.join(Truth), I32);
    assert_eq!(I32.join(U32), Mixed);
    assert_eq!(Mixed.join(Truth), Mixed);
    assert_eq!(F32.join(Truth), F32);
    // 32 bits compared as an integer are no float.
    assert_eq!(F32.join(Int), Mixed);

    let bits = |cell: u64| Found {
        lo: (cell as i64).min(0),
        hi: (cell as i64).max(0),
        ..Found::default()
    };
    assert_eq!(Truth.kind(&bits(1)), Some(Kind::Bool));
    assert_eq!(Truth.kind(&bits(2)), None);
    // 32 bits of either sign may count or may name: no kind is stated on that.
    assert_eq!(Int.kind(&bits(1)), None);
    assert_eq!(I32.kind(&bits(2)), Some(Kind::I32));
    assert_eq!(I32.kind(&bits(0xF000_0000)), None);
    assert_eq!(U32.kind(&bits(0xF000_0000)), Some(Kind::U32));
    assert_eq!(Unsaid.kind(&bits(1)), None);
    // An integer among floats is said by `odd`, which a float's bits leave unset.
    assert_eq!(F32.kind(&bits(0x3F00_0000)), Some(Kind::F32));
    let five = Found {
        odd: true,
        ..bits(5)
    };
    assert_eq!(F32.kind(&five), None);
}

#[test]
fn a_parameter_nothing_types_takes_the_kind_every_call_passes() {
    let taken = "Native.DEMO.USE(Vm.Function(0x00000001U, 0x000030));";
    let caller = [
        format!("func_000010({FLOAT}); func_000010(0x3F000000);"),
        format!("func_000020({FLOAT}); func_000020(1);"),
        format!("func_000030({FLOAT}); {taken}"),
        format!("func_000050({FLOAT}); return;"),
    ];
    let functions = [
        function("void", 0x10, 1, TAKES),
        function("void", 0x20, 1, TAKES),
        function("void", 0x30, 1, TAKES),
        function("void", 0x40, 1, "func_000010(p0); return;"),
        function(
            "void",
            0x50,
            1,
            "Native.DEMO.USE(p0); func_000060(p0); return;",
        ),
        function("void", 0x60, 1, TAKES),
        function("void", 0x100, 1, &caller.join(" ")),
    ];
    let project = project_of(&[listing(1, &functions)]);
    // A float, and an integer that is a float's bits.
    assert_eq!(param(&project, 0x10, 0), Kind::F32);
    // One is no float a source holds: two kinds are no kind.
    assert_eq!(param(&project, 0x20, 0), Kind::Unknown);
    // A reference to it is taken, so the calls that name it are not all its calls.
    assert_eq!(param(&project, 0x30, 0), Kind::Unknown);
    // Only passed on, to a parameter the calls type.
    assert_eq!(param(&project, 0x40, 0), Kind::F32);
    // What a parameter is goes on to the parameter it is passed for.
    assert_eq!(param(&project, 0x50, 0), Kind::F32);
    assert_eq!(param(&project, 0x60, 0), Kind::F32);
}

#[test]
fn a_parameter_takes_the_kind_of_the_parameter_passed_for_it() {
    let functions = [
        function(
            "void",
            0x10,
            1,
            "store_i32(p0, 0x10, 5); func_000020(p0); return;",
        ),
        function("void", 0x20, 1, TAKES),
    ];
    let project = project_of(&[listing(1, &functions)]);
    assert_eq!(param(&project, 0x10, 0), Kind::Pointer);
    assert_eq!(param(&project, 0x20, 0), Kind::Pointer);
}

#[test]
fn an_integer_among_floats_is_a_float_only_as_the_bits_of_one() {
    let or = |constant: &str| {
        format!("if (((as_u32(p1) == 0))) {{ return {constant}; }} return {FLOAT};")
    };
    let functions = [
        function("VmValue", 0x10, 2, &or("0x3F000000")),
        function("VmValue", 0x20, 2, &or("5")),
    ];
    let project = project_of(&[listing(1, &functions)]);
    assert_eq!(result(&project, 0x10), Kind::F32);
    assert_eq!(result(&project, 0x20), Kind::Unknown);

    let minus_one = (-1.0f32).to_bits();
    assert!(float_bits(u64::from(minus_one), true));
    // Widened as the `int` of those bits, the cell is no float's: only an argument is read so.
    let widened = minus_one as i32 as i64 as u64;
    assert!(!float_bits(widened, true));
    assert!(float_bits(widened, false));
    // A small integer is a denormal, and bits above a float's are no sign.
    assert!(!float_bits(5, false));
    assert!(!float_bits(0x1_0000_0000 | u64::from(minus_one), false));
}

/// A type that fills slot 3 with the function at `at`, under `base`.
fn fills(base: Option<TypeKey>, at: u32) -> TypeFacts {
    TypeFacts {
        base,
        slots: BTreeMap::from([(3, key(at))]),
        ..TypeFacts::default()
    }
}

#[test]
fn the_functions_of_one_slot_share_what_they_agree_on() {
    let ordered = "return ((as_f32(p1) < as_f32(2.5f)));";
    let functions = [
        function("VmValue", 0x10, 2, ordered),
        function("VmValue", 0x20, 2, "return 0;"),
        function("VmValue", 0x30, 2, ordered),
        function("VmValue", 0x40, 2, "return p1;"),
        function("VmValue", 0x50, 2, "return 0;"),
        function("VmValue", 0x60, 2, ordered),
        function("VmValue", 0x70, 2, "return 1;"),
    ];
    let mut project = parts_of(&[listing(1, &functions)]);
    let ty = |n: u64| TypeKey(0x0000_0001_0000_0000 | n);
    project.types.insert(ty(1), fills(None, 0x10));
    project.types.insert(ty(2), fills(Some(ty(1)), 0x20));
    project.types.insert(ty(3), fills(None, 0x30));
    project.types.insert(ty(4), fills(Some(ty(3)), 0x40));
    project.types.insert(ty(5), fills(Some(ty(4)), 0x50));
    let both = BTreeSet::from([key(0x60), key(0x70)]);
    let slots = BTreeMap::from([(0, both)]);
    project.implemented.insert(ty(6), slots);
    settle(&mut project);

    // The override returns zero alone, and the method it overrides a truth.
    assert_eq!(result(&project, 0x10), Kind::Bool);
    assert_eq!(result(&project, 0x20), Kind::Bool);
    // It reads no parameter, and the method it overrides reads its second as a `float`.
    assert_eq!(param(&project, 0x10, 1), Kind::F32);
    assert_eq!(param(&project, 0x20, 1), Kind::F32);
    // One of the line returns what nothing knows: the line agrees on nothing, and each keeps its own.
    assert_eq!(result(&project, 0x30), Kind::Bool);
    assert_eq!(result(&project, 0x40), Kind::Unknown);
    assert_eq!(result(&project, 0x50), Kind::Unknown);
    // Two types fill one slot of an interface.
    assert_eq!(result(&project, 0x60), Kind::Bool);
    assert_eq!(result(&project, 0x70), Kind::Bool);

    // A call through the slot yields a truth where the whole line proves one, whichever type the object is.
    let slot = |n: u64| project.slot_result(ty(n), 3).cloned();
    assert_eq!((slot(1), slot(2)), (Some(Kind::Bool), Some(Kind::Bool)));
    assert_eq!((slot(3), slot(4), slot(5)), (None, None, None));
    let held = |at: u32| project.slot_results.get(&key(at)).cloned();
    assert_eq!(
        (held(0x60), held(0x70)),
        (Some(Kind::Bool), Some(Kind::Bool))
    );
}

#[test]
fn an_integer_is_no_kind_a_caller_states() {
    let counted = "((as_i32(p0) + as_i32(1)))";
    let caller = [
        format!("func_000010({counted});"),
        format!("func_000020({counted}); func_000020({FLOAT});"),
        "return;".to_owned(),
    ];
    let functions = [
        function("void", 0x10, 1, TAKES),
        function("void", 0x20, 1, TAKES),
        function("void", 0x100, 1, &caller.join(" ")),
        // The same slot: one reads its second parameter as an `int`, the other not at all.
        function("VmValue", 0x30, 2, "return ((as_i32(p1) < 5));"),
        function("VmValue", 0x40, 2, "return 0;"),
    ];
    let mut project = parts_of(&[listing(1, &functions)]);
    let ty = |n: u64| TypeKey(0x0000_0001_0000_0000 | n);
    project.types.insert(ty(1), fills(None, 0x30));
    project.types.insert(ty(2), fills(Some(ty(1)), 0x40));
    settle(&mut project);
    // Its sign, and whether it counts or names, is for the callee's own reads to say.
    assert_eq!(param(&project, 0x10, 0), Kind::Unknown);
    // An integer and a float are two kinds.
    assert_eq!(param(&project, 0x20, 0), Kind::Unknown);
    assert_eq!(param(&project, 0x30, 1), Kind::I32);
    assert_eq!(param(&project, 0x40, 1), Kind::Unknown);
}

#[test]
fn a_native_whose_result_every_function_only_tests_for_zero_is_a_truth() {
    let functions = [
        // Tested, negated, and kept in a variable that is only tested.
        function(
            "void",
            0x10,
            1,
            "if (((as_u32(Native.DEMO.ASK(p0)) != 0))) { Native.DEMO.USE(1); } return;",
        ),
        function(
            "void",
            0x20,
            1,
            "local_0 = Native.DEMO.ASK(p0); if (((as_u32(local_0) == 0))) { return; } Native.DEMO.ASK(2); return;",
        ),
        // Tested once, and handed on as a value once.
        function(
            "void",
            0x30,
            1,
            "if (((as_u32(Native.DEMO.GET(p0)) != 0))) { Native.DEMO.USE(Native.DEMO.GET(p0)); } return;",
        ),
    ];
    let project = project_of(&[listing(1, &functions)]);
    assert_eq!(project.native_truths, BTreeSet::from([5]));
    assert!(project.native_uses.is_empty());
}

#[test]
fn a_parameter_every_call_passes_an_int_widened_by_its_sign_is_proved_one() {
    let functions = [
        function(
            "void",
            0x10,
            2,
            "Native.DEMO.USE(p0); Native.DEMO.USE(p1); return;",
        ),
        // A constant of an `int`'s range, a comparison, and the caller's own such parameter.
        function(
            "void",
            0x20,
            1,
            "func_000010(-5, ((as_i32(p0) < as_i32(3)))); return;",
        ),
        function("void", 0x30, 1, "func_000010(7, p0); return;"),
        function(
            "void",
            0x40,
            0,
            "func_000030(((as_i32(local_0) == as_i32(2)))); return;",
        ),
        // A variable, of which nothing here says how it widened.
        function(
            "void",
            0x50,
            1,
            "func_000060(p0); func_000060(local_1); return;",
        ),
        function("void", 0x60, 1, "Native.DEMO.USE(p0); return;"),
        // A 32-bit reading and a load, as the code first reads, are what the cell holds.
        function(
            "void",
            0x70,
            1,
            "func_000080(as_i32(p0), load_i32(local_0, 0x8)); return;",
        ),
        function(
            "void",
            0x80,
            2,
            "Native.DEMO.USE(p0); Native.DEMO.USE(p1); return;",
        ),
    ];
    let project = project_of(&[listing(1, &functions)]);
    let cells = |at: u32| project.widened.get(&key(at)).cloned().unwrap_or_default();
    assert_eq!(cells(0x10), BTreeSet::from([0, 1]));
    assert_eq!(cells(0x30), BTreeSet::from([0]));
    assert!(cells(0x60).is_empty());
    assert_eq!(cells(0x80), BTreeSet::from([0, 1]));
    // No call names the function at 0x20 or 0x50: their callers are not known.
    assert!(cells(0x20).is_empty() && cells(0x50).is_empty());
}

#[test]
fn a_parameter_changed_before_it_is_passed_on_and_an_argument_of_unsaid_cells_prove_nothing() {
    let functions = [
        function("void", 0x10, 1, "Native.DEMO.USE(p0); return;"),
        // The caller's parameter is an `int` by its own callers, and is passed on as it came.
        function("void", 0x20, 1, "func_000010(p0); return;"),
        function(
            "void",
            0x30,
            0,
            "func_000020(4); func_000040(4); func_000060(4); return;",
        ),
        // Assigned before it is passed on: what goes on is no longer what came.
        function(
            "void",
            0x40,
            1,
            "p0 = Native.DEMO.GET(1); func_000050(p0); return;",
        ),
        function("void", 0x50, 1, "Native.DEMO.USE(p0); return;"),
        // Its address taken: a store through it changes it the same.
        function(
            "void",
            0x60,
            1,
            "Native.DEMO.USE(Vm.Address(parameters, 0)); func_000070(p0); return;",
        ),
        function("void", 0x70, 1, "Native.DEMO.USE(p0); return;"),
        // The result of a call that states no cells is the last argument: no cell after it says so.
        function("VmValue", 0x80, 1, "return Native.DEMO.GET(p0);"),
        function("void", 0x90, 0, "func_0000A0(3, func_000080(1)); return;"),
        function(
            "void",
            0xA0,
            2,
            "Native.DEMO.USE(p0); Native.DEMO.USE(p1); return;",
        ),
        // Another call passes a constant where that result lands.
        function("void", 0xB0, 0, "func_0000A0(3, 9); return;"),
    ];
    let project = project_of(&[listing(1, &functions)]);
    let cells = |at: u32| project.widened.get(&key(at)).cloned().unwrap_or_default();
    assert_eq!(cells(0x20), BTreeSet::from([0]));
    assert_eq!(cells(0x10), BTreeSet::from([0]));
    assert!(cells(0x50).is_empty(), "a parameter assigned in its caller");
    assert!(
        cells(0x70).is_empty(),
        "a parameter whose address its caller takes"
    );
    assert!(
        !cells(0xA0).contains(&1),
        "the cell a call's result lands in is proved nothing, whatever another call passes there"
    );
}

#[test]
fn a_native_a_function_hands_back_is_a_truth_where_every_call_of_the_function_only_tests_it() {
    let functions = |read: &str| {
        let tests = format!(
            "if (((as_u32(func_000010(p0)) != 0))) {{ Native.DEMO.USE(1); }} {read} return;"
        );
        [
            function("VmValue", 0x10, 1, "return Native.DEMO.ASK(p0);"),
            function(
                "VmValue",
                0x20,
                1,
                "local_0 = Native.DEMO.GET(p0); return local_0;",
            ),
            function("void", 0x30, 1, &tests),
            function(
                "void",
                0x40,
                1,
                "if (((as_u32(func_000020(p0)) == 0))) { return; } return;",
            ),
        ]
    };
    let project = project_of(&[listing(1, &functions(""))]);
    assert_eq!(project.native_truths, BTreeSet::from([5, 6]));
    // A call that counts with what the function hands back reads the native as a number.
    let counted = "Native.DEMO.USE(((as_i32(func_000010(p0)) + as_i32(1))));";
    let project = project_of(&[listing(1, &functions(counted))]);
    assert_eq!(project.native_truths, BTreeSet::from([6]));
    // A reference taken to the function lets calls no code names read what it hands back.
    let taken = "Native.DEMO.USE(Vm.Function(0x00000001U, 0x000020));";
    let project = project_of(&[listing(1, &functions(taken))]);
    assert_eq!(project.native_truths, BTreeSet::from([5]));
    // The function's result handed whole to a call, or kept and handed so, reaches it as a number.
    let handed = "Native.DEMO.USE(func_000010(p0));";
    let project = project_of(&[listing(1, &functions(handed))]);
    assert_eq!(project.native_truths, BTreeSet::from([6]));
    let kept = "local_0 = func_000010(p0); Native.DEMO.USE(local_0);";
    let project = project_of(&[listing(1, &functions(kept))]);
    assert_eq!(project.native_truths, BTreeSet::from([6]));
}

/// The key of the invented type `n` of program 1, as a call through a slot names it.
fn invented(n: u64) -> TypeKey {
    TypeKey(0x0000_0001_0000_0000 | n)
}

#[test]
fn a_result_handed_on_from_a_slot_whose_whole_line_returns_a_truth_is_a_truth() {
    let through = |ty: &str| format!("return Virtual.type_00000001000000{ty}.Method_3(p0, p1);");
    let or_through = |ty: &str| format!("if (((as_u32(p0) == 0))) {{ return 0; }} {}", through(ty));
    let functions = [
        // The line of slot 3 of C1: a test, and an override that returns zero alone.
        function("VmValue", 0x10, 2, "return ((as_i32(p1) < as_i32(3)));"),
        function("VmValue", 0x20, 2, "return 0;"),
        function("VmValue", 0x30, 2, &or_through("C1")),
        function("VmValue", 0x40, 2, "return func_000030(p0, p1);"),
        // The line of slot 3 of D1 counts.
        function("VmValue", 0x50, 2, &or_through("D1")),
        function("VmValue", 0x60, 2, "return ((as_i32(p1) + as_i32(3)));"),
        // Zero, one, or what it returns itself.
        function(
            "VmValue",
            0x70,
            2,
            "if (((as_u32(p0) == 0))) { return 1; } return func_000070(p0, p1);",
        ),
        // A truth and what a function of the line that counts hands back.
        function(
            "VmValue",
            0x80,
            2,
            "if (((as_u32(p0) == 0))) { return ((as_i32(p1) < as_i32(3))); } return func_000050(p0, p1);",
        ),
    ];
    let mut project = parts_of(&[listing(1, &functions)]);
    project.types.insert(invented(0xC1), fills(None, 0x10));
    project
        .types
        .insert(invented(0xC2), fills(Some(invented(0xC1)), 0x20));
    project.types.insert(invented(0xD1), fills(None, 0x60));
    settle(&mut project);

    assert_eq!(result(&project, 0x20), Kind::Bool);
    assert_eq!(project.slot_result(invented(0xC1), 3), Some(&Kind::Bool));
    // Whichever type the object is, the call through the slot yields zero or one.
    assert_eq!(result(&project, 0x30), Kind::Bool);
    assert_eq!(result(&project, 0x40), Kind::Bool);
    assert_eq!(result(&project, 0x60), Kind::I32);
    assert_eq!(result(&project, 0x50), Kind::Unknown);
    assert_eq!(result(&project, 0x70), Kind::Bool);
    assert_eq!(result(&project, 0x80), Kind::Unknown);
}

/// A call of the function at `at` with a new object of the invented type `n`.
fn given_object(at: u32, n: u64) -> String {
    format!(
        "func_{at:06X}(Vm.NewObject(new VmType(\"{}\")));",
        invented(n)
    )
}

#[test]
fn objects_of_two_types_are_of_the_nearest_type_both_are() {
    let caller = [
        given_object(0x10, 0xA2),
        given_object(0x10, 0xA3),
        given_object(0x20, 0xA2),
        given_object(0x20, 0xB1),
        given_object(0x30, 0xA2),
        "func_000030(Vm.Address(locals, 0));".to_owned(),
        given_object(0x40, 0xA2),
        given_object(0x40, 0xA1),
        "return;".to_owned(),
    ];
    let new = |n: u64| format!("Vm.NewObject(new VmType(\"{}\"))", invented(n));
    let either = format!(
        "if (((as_u32(p0) == 0))) {{ return {}; }} return {};",
        new(0xA2),
        new(0xA3)
    );
    let functions = [
        function("void", 0x10, 1, TAKES),
        function("void", 0x20, 1, TAKES),
        function("void", 0x30, 1, TAKES),
        function("void", 0x40, 1, TAKES),
        function("VmValue", 0x50, 1, &either),
        function("void", 0x100, 0, &caller.join(" ")),
    ];
    let mut project = parts_of(&[listing(1, &functions)]);
    let under = |base: Option<u64>| TypeFacts {
        base: base.map(invented),
        ..TypeFacts::default()
    };
    project.types.insert(invented(0xA1), under(None));
    project.types.insert(invented(0xA2), under(Some(0xA1)));
    project.types.insert(invented(0xA3), under(Some(0xA1)));
    project.types.insert(invented(0xB1), under(None));
    settle(&mut project);

    let object = |n: u64| Kind::Object(invented(n).to_string());
    assert_eq!(param(&project, 0x10, 0), object(0xA1));
    // No type is both: a reference, of no type.
    assert_eq!(param(&project, 0x20, 0), Kind::Pointer);
    assert_eq!(param(&project, 0x30, 0), Kind::Pointer);
    // Never the type below: an object of the base is passed too.
    assert_eq!(param(&project, 0x40, 0), object(0xA1));
    assert_eq!(result(&project, 0x50), object(0xA1));
}

#[test]
fn a_parameter_some_call_hands_what_may_be_anything_is_of_no_one_objects_type() {
    let or_made = format!(
        "if (((as_u64(p0) == 0))) {{ p0 = Vm.NewObject(new VmType(\"{}\")); }} {TAKES}",
        invented(0xA2)
    );
    let copied = format!(
        "local_0 = p0; if (((as_u64(local_0) == 0))) {{ local_0 = Vm.NewObject(new VmType(\"{}\")); }} Native.DEMO.USE(local_0); return;",
        invented(0xA2)
    );
    let caller = [
        given_object(0x10, 0xA2),
        // What a native hands back, through a function whose result is of no kind.
        "func_000010(func_000040(1UL));".to_owned(),
        given_object(0x20, 0xA2),
        given_object(0x60, 0xA2),
        given_object(0x70, 0xA3),
        given_object(0x80, 0xA3),
        "return;".to_owned(),
    ];
    let functions = [
        function("void", 0x10, 1, TAKES),
        // Passes on what it assigned its parameter, not what its callers hand it.
        function(
            "void",
            0x20,
            1,
            "p0 = func_000040(p0); func_000050(p0); return;",
        ),
        function("VmValue", 0x40, 1, "return Native.DEMO.GET(p0);"),
        function("void", 0x50, 1, TAKES),
        function("void", 0x60, 1, TAKES),
        // Given an object of another type, and one of its own where it is given none.
        function("void", 0x70, 1, &or_made),
        // Copied into a variable that holds an object of its own where it is given none.
        function("void", 0x80, 1, &copied),
        function("void", 0x100, 0, &caller.join(" ")),
    ];
    let mut project = parts_of(&[listing(1, &functions)]);
    project.types.insert(invented(0xA2), TypeFacts::default());
    project.types.insert(invented(0xA3), TypeFacts::default());
    settle(&mut project);

    let object = Kind::Object(invented(0xA2).to_string());
    assert_eq!(param(&project, 0x60, 0), object);
    assert_ne!(param(&project, 0x10, 0), object);
    assert_ne!(param(&project, 0x50, 0), object);
    assert_ne!(param(&project, 0x70, 0), object);
    assert_ne!(param(&project, 0x80, 0), object);
}

#[test]
fn references_join_to_a_pointer_and_what_the_functions_given_one_read_it_as_does_not() {
    let object = |n: u64| Found::of(Kind::Object(invented(n).to_string()));
    let mut both = object(0xA2);
    assert!(both.join(&object(0xA3)));
    assert_eq!(both.kind, Some(Kind::Pointer));
    assert_eq!(both.objects.len(), 2);
    assert!(!both.join(&object(0xA3)));
    // A pointer to anything takes every type away.
    let mut any = both.clone();
    assert!(any.join(&Found::of(Kind::Pointer)));
    assert_eq!((any.kind, any.objects.len()), (Some(Kind::Pointer), 0));
    let mut same = object(0xA2);
    assert!(!same.join(&object(0xA2)));
    assert_eq!(same, object(0xA2));
    // A string, an integer: as before, no kind.
    let mut other = object(0xA2);
    other.join(&Found::of(Kind::String));
    assert!(other.dead);
    // Two types two callees read a cell as are no kind.
    let mut read = object(0xA2);
    assert!(read.join_exact(&object(0xA3)));
    assert!(read.dead);

    let mut project = Project::default();
    let facts = |base: Option<u64>, interfaces: &[u64]| TypeFacts {
        base: base.map(invented),
        interfaces: interfaces.iter().copied().map(invented).collect(),
        ..TypeFacts::default()
    };
    project
        .types
        .insert(invented(0xE2), facts(None, &[0xF1, 0xF2]));
    project.types.insert(invented(0xE3), facts(None, &[0xF1]));
    project.types.insert(invented(0xE4), facts(None, &[0xF2]));
    let names =
        |ns: &[u64]| -> BTreeSet<String> { ns.iter().map(|n| invented(*n).to_string()).collect() };
    // No base both derive from: the one interface both implement.
    assert_eq!(
        common_type(&project, &names(&[0xE2, 0xE3])),
        Some(invented(0xF1))
    );
    assert_eq!(common_type(&project, &names(&[0xE3, 0xE4])), None);
    let unnamed = BTreeSet::from(["System.Object".to_owned(), invented(0xE2).to_string()]);
    assert_eq!(common_type(&project, &unnamed), None);
}

#[test]
fn a_slots_parameters_are_what_every_function_of_its_line_that_mentions_them_states() {
    let functions = [
        function("VmValue", 0x30, 2, "return ((as_i32(p1) < 5));"),
        function("VmValue", 0x40, 2, "return 0;"),
        function("VmValue", 0x50, 2, "return ((as_i32(p1) < 5));"),
        function("VmValue", 0x60, 2, "Native.DEMO.USE(p1); return 0;"),
    ];
    let mut project = parts_of(&[listing(1, &functions)]);
    project.types.insert(invented(1), fills(None, 0x30));
    project
        .types
        .insert(invented(2), fills(Some(invented(1)), 0x40));
    project.types.insert(invented(3), fills(None, 0x50));
    project
        .types
        .insert(invented(4), fills(Some(invented(3)), 0x60));
    settle(&mut project);

    let of = |kind: Kind| Param { kind, cells: 1 };
    let line = [of(Kind::Unknown), of(Kind::I32)];
    // The override reads neither cell, so what it is passed there is the method's.
    assert_eq!(project.slot_parameters(invented(1), 3), Some(&line[..]));
    assert_eq!(project.slot_parameters(invented(2), 3), Some(&line[..]));
    assert_eq!(param(&project, 0x40, 1), Kind::Unknown);
    // One reads all of its second cell, as nothing types it.
    assert_eq!(project.slot_parameters(invented(3), 3), None);
}

#[test]
fn an_argument_as_first_read_is_an_int_widened_by_its_sign_by_what_makes_it() {
    let handed = [
        // A load of four bytes or of one, a reading of 32 bits, an operator at 32 bits.
        "load_i32(p0, 0x8)",
        "load_u8(p0, 0x1)",
        "as_i32(Native.DEMO.GET(1))",
        "((as_i32(Native.DEMO.GET(2)) + as_i32(1)))",
        // The nearest that are not: 32 bits widened by zeroes, and a load of the whole cell.
        "as_u32(Native.DEMO.GET(3))",
        "((as_u32(Native.DEMO.GET(4)) + as_u32(1)))",
        "load_u64(p0, 0x0)",
    ];
    let functions = [
        function("void", 0x10, 7, "return;"),
        function(
            "void",
            0x20,
            1,
            &format!("func_000010({}); return;", handed.join(", ")),
        ),
    ];
    let project = project_of(&[listing(1, &functions)]);
    let cells = project.widened.get(&key(0x10)).cloned().unwrap_or_default();
    assert_eq!(cells, BTreeSet::from([0, 1, 2, 3]));
}

#[test]
fn a_result_handed_on_is_an_int_widened_by_its_sign_where_its_signature_says_so() {
    let functions = [
        function("void", 0x10, 1, "return;"),
        function("VmValue", 0x20, 1, "return ((as_i32(p0) < 5));"),
        function("VmValue", 0x30, 1, "return Native.DEMO.GET(p0);"),
        function("void", 0x40, 1, "return;"),
        // Each result kept in a variable only that assignment writes, and handed on.
        function(
            "void",
            0x50,
            0,
            "v_000001_0 = func_000020(1); func_000010(v_000001_0); local_1 = func_000030(1); func_000040(local_1); return;",
        ),
    ];
    let project = project_of(&[listing(1, &functions)]);
    let cells = |at: u32| project.widened.get(&key(at)).cloned().unwrap_or_default();
    assert_eq!(result(&project, 0x20), Kind::Bool);
    assert_eq!(cells(0x10), BTreeSet::from([0]));
    assert!(
        cells(0x40).is_empty(),
        "a result of no kind may be any cell"
    );
}

#[test]
fn a_local_is_what_its_assignments_make_it_only_where_no_address_reaches_it() {
    let functions = [
        function("void", 0x10, 1, "return;"),
        function("void", 0x20, 1, "return;"),
        function("void", 0x30, 1, "return;"),
        function(
            "void",
            0x40,
            0,
            "local_0 = 0; local_0 = ((as_i32(local_0) + as_i32(1))); func_000010(local_0); return;",
        ),
        // Its own address taken: what is written through it is not said.
        function(
            "void",
            0x50,
            0,
            "local_0 = 0; Native.DEMO.USE(Vm.Address(locals, 0)); func_000020(local_0); return;",
        ),
        // A parameter's address taken: nothing bounds what it reaches.
        function(
            "void",
            0x60,
            1,
            "local_0 = 0; Native.DEMO.USE(Vm.Address(parameters, 0)); func_000030(local_0); return;",
        ),
    ];
    let project = project_of(&[listing(1, &functions)]);
    let cells = |at: u32| project.widened.get(&key(at)).cloned().unwrap_or_default();
    assert_eq!(cells(0x10), BTreeSet::from([0]));
    assert!(cells(0x20).is_empty() && cells(0x30).is_empty());
}

#[test]
fn a_parameter_past_one_whose_address_is_taken_is_not_what_its_callers_handed() {
    let functions = [
        function("void", 0x10, 1, "return;"),
        function(
            "void",
            0x20,
            2,
            "Native.DEMO.USE(Vm.Address(parameters, 0)); func_000010(p1); return;",
        ),
        function("void", 0x30, 0, "func_000020(1, 2); return;"),
    ];
    let project = project_of(&[listing(1, &functions)]);
    let cells = |at: u32| project.widened.get(&key(at)).cloned().unwrap_or_default();
    assert_eq!(cells(0x20), BTreeSet::from([0, 1]));
    assert!(cells(0x10).is_empty());
}

#[test]
fn a_function_code_no_part_read_may_call_takes_nothing_from_the_calls_that_name_it() {
    let functions = [
        function("void", 0x10, 1, "return;"),
        function("void", 0x20, 1, "return;"),
        function("void", 0x30, 0, "func_000010(1); func_000020(2); return;"),
    ];
    let listings = [listing(1, &functions)];
    let cells = |project: &Project, at: u32| {
        let held = project.widened.get(&key(at));
        held.cloned().unwrap_or_default()
    };
    let mut named = parts_of(&listings);
    unseen(&mut named, 1, &[], Some(BTreeSet::from([key(0x10)])));
    settle(&mut named);
    assert!(cells(&named, 0x10).is_empty());
    assert_eq!(cells(&named, 0x20), BTreeSet::from([0]));
    assert!(named.unseen.is_empty(), "a settled project keeps none");

    // Where the code's instructions were not read either, any function of the program and of those it imports.
    let mut any = parts_of(&listings);
    unseen(&mut any, 1, &[2], None);
    let marks = BTreeSet::from([FunctionKey::new(1, ANY), FunctionKey::new(2, ANY)]);
    assert_eq!(any.unseen, marks);
    settle(&mut any);
    assert!(cells(&any, 0x10).is_empty() && cells(&any, 0x20).is_empty());
}

#[test]
fn a_line_of_a_slot_whose_every_call_only_tests_its_result_of_zeroes_and_ones_returns_a_truth() {
    let tested = "if (((as_u32(Virtual.type_00000001000000C1.Method_3(p0, p1)) != 0))) { Native.DEMO.USE(1); } if (((as_u32(func_000020(p0, p1)) == 0))) { return; } return;";
    let line = |also: &str| {
        let functions = [
            function("VmValue", 0x10, 2, FLAG),
            function("VmValue", 0x20, 2, "return 0;"),
            function("void", 0x30, 2, tested),
            function("void", 0x40, 2, &format!("{also} return;")),
        ];
        let mut project = parts_of(&[listing(1, &functions)]);
        project.types.insert(invented(0xC1), fills(None, 0x10));
        project
            .types
            .insert(invented(0xC2), fills(Some(invented(0xC1)), 0x20));
        settle(&mut project);
        project
    };
    // Through the slot and by name, every call tests it.
    let project = line("");
    assert_eq!(
        (result(&project, 0x10), result(&project, 0x20)),
        (Kind::Bool, Kind::Bool)
    );
    assert_eq!(project.slot_result(invented(0xC1), 3), Some(&Kind::Bool));
    // One call counts with it: the number stands.
    let counted =
        "Native.DEMO.USE(((as_i32(Virtual.type_00000001000000C1.Method_3(p0, p1)) + as_i32(1))));";
    assert_eq!(result(&line(counted), 0x10), Kind::I32);
    // A call through a type no table resolves may reach any line of the slot.
    let elsewhere =
        "Native.DEMO.USE(((as_i32(Virtual.type_00000001000000D1.Method_3(p0, p1)) + as_i32(1))));";
    let project = line(elsewhere);
    assert_eq!(
        (result(&project, 0x10), result(&project, 0x20)),
        (Kind::Unknown, Kind::Unknown)
    );
    // Handed whole to a call, the result is read as anything there.
    let handed = "Native.DEMO.USE(Virtual.type_00000001000000C1.Method_3(p0, p1));";
    assert_eq!(result(&line(handed), 0x10), Kind::Unknown);
}

#[test]
fn what_a_function_of_a_slot_hands_on_is_read_wherever_the_function_is() {
    let hands = |also: &str| {
        let functions = [
            function("VmValue", 0x10, 2, "return func_000050(p0, p1);"),
            function("VmValue", 0x50, 2, FLAG),
            // By name, a test; through the slot, a number.
            function(
                "void",
                0x30,
                2,
                &format!("if (((as_u32(func_000010(p0, p1)) == 0))) {{ return; }} {also} return;"),
            ),
        ];
        let mut project = parts_of(&[listing(1, &functions)]);
        project.types.insert(invented(0xC1), fills(None, 0x10));
        settle(&mut project);
        project
    };
    assert_eq!(result(&hands(""), 0x50), Kind::Bool);
    let counted =
        "Native.DEMO.USE(((as_i32(Virtual.type_00000001000000C1.Method_3(p0, p1)) + as_i32(1))));";
    assert_eq!(result(&hands(counted), 0x50), Kind::I32);
    // A delegate of the function is called where no code says.
    let taken = "Native.DEMO.USE(Vm.Function(0x00000001U, 0x000010));";
    assert_eq!(result(&hands(taken), 0x50), Kind::Unknown);
}

#[test]
fn a_reading_still_on_its_way_down_a_chain_of_calls_reads_the_end_of_it_as_anything() {
    // Each function hands on the result of one at a lower offset, so a round carries a reading one step.
    let chain = |steps: u32| {
        let at = |n: u32| 0x1000 + 0x10 * n;
        let mut functions = vec![
            function("VmValue", at(0), 1, "return Native.DEMO.ASK(p0);"),
            function(
                "void",
                0x10,
                1,
                &format!(
                    "if (((as_u32(func_{:06X}(p0)) != 0))) {{ return; }} return;",
                    at(steps)
                ),
            ),
        ];
        for n in 1..=steps {
            let body = format!("return func_{:06X}(p0);", at(n - 1));
            functions.push(function("VmValue", at(n), 1, &body));
        }
        project_of(&[listing(1, &functions)])
    };
    assert!(chain(3).native_truths.contains(&5));
    let past = u32::try_from(ROUNDS).expect("the rounds are few") + 4;
    assert!(!chain(past).native_truths.contains(&5));
}

#[test]
fn a_slot_or_a_native_code_no_part_read_calls_is_read_as_anything_there() {
    let tested = "if (((as_u32(Virtual.type_00000001000000C1.Method_3(p0, p1)) != 0))) { Native.DEMO.USE(1); } return;";
    let asks = "if (((as_u32(Native.DEMO.ASK(p0)) != 0))) { Native.DEMO.USE(1); } return;";
    let line = |calls: Option<(BTreeSet<u32>, BTreeSet<u64>)>| {
        let functions = [
            function("VmValue", 0x10, 2, FLAG),
            function("void", 0x30, 2, tested),
            function("void", 0x40, 2, asks),
        ];
        let mut project = parts_of(&[listing(1, &functions)]);
        project.types.insert(invented(0xC1), fills(None, 0x10));
        if let Some(calls) = calls {
            unseen_calls(&mut project, Some(calls));
        }
        settle(&mut project);
        project
    };
    let read = line(None);
    assert_eq!(result(&read, 0x10), Kind::Bool);
    assert!(read.native_truths.contains(&5), "{:?}", read.native_truths);
    // A call through slot 3 and a call of the native, in code that did not lift.
    let unread = line(Some((BTreeSet::from([3]), BTreeSet::from([5]))));
    assert_eq!(result(&unread, 0x10), Kind::Unknown);
    assert!(!unread.native_truths.contains(&5));
    // Code whose instructions were not read may call anything.
    let mut blind = parts_of(&[listing(
        1,
        &[
            function("VmValue", 0x10, 2, FLAG),
            function("void", 0x30, 2, tested),
            function("void", 0x40, 2, asks),
        ],
    )]);
    blind.types.insert(invented(0xC1), fills(None, 0x10));
    unseen(&mut blind, 2, &[], None);
    settle(&mut blind);
    assert_eq!(result(&blind, 0x10), Kind::Unknown);
    assert!(blind.native_truths.is_empty());
}

#[test]
fn a_line_a_call_through_whose_slot_is_handed_on_is_read_as_the_function_that_hands_it_on() {
    let line = |read: &str| {
        let functions = [
            function("VmValue", 0x10, 2, FLAG),
            function(
                "VmValue",
                0x20,
                2,
                "return Virtual.type_00000001000000C1.Method_3(p0, p1);",
            ),
            function("void", 0x30, 2, &format!("{read} return;")),
        ];
        let mut project = parts_of(&[listing(1, &functions)]);
        project.types.insert(invented(0xC1), fills(None, 0x10));
        settle(&mut project);
        project
    };
    // Every call of the function that hands the slot's result on tests it.
    let project = line("if (((as_u32(func_000020(p0, p1)) == 0))) { return; }");
    assert_eq!(result(&project, 0x10), Kind::Bool);
    assert_eq!(project.slot_result(invented(0xC1), 3), Some(&Kind::Bool));
    assert_eq!(result(&project, 0x20), Kind::Bool);
    // One counts with it: the number stands for the line too.
    let counted = "Native.DEMO.USE(((as_i32(func_000020(p0, p1)) + as_i32(1))));";
    assert_eq!(result(&line(counted), 0x10), Kind::I32);
    // Handed whole to a call, it is read as anything, and so is the line.
    let handed = "Native.DEMO.USE(func_000020(p0, p1));";
    assert_eq!(result(&line(handed), 0x10), Kind::Unknown);
}

#[test]
fn a_call_through_a_class_whose_line_leaves_the_slot_empty_reaches_what_the_types_under_it_hold() {
    let tests_c1 = "if (((as_u32(Virtual.type_00000001000000C1.Method_3(p0, p1)) != 0))) { Native.DEMO.USE(1); }";
    let line = |base: Option<TypeKey>, also: &str| {
        let functions = [
            function("VmValue", 0x10, 2, FLAG),
            function("void", 0x30, 2, &format!("{tests_c1} {also} return;")),
        ];
        let mut project = parts_of(&[listing(1, &functions)]);
        // D1 holds nothing in slot 3, and C1, under it, fills it.
        let empty = TypeFacts {
            base,
            ..TypeFacts::default()
        };
        project.types.insert(invented(0xD1), empty);
        project
            .types
            .insert(invented(0xC1), fills(Some(invented(0xD1)), 0x10));
        settle(&mut project);
        project
    };
    let class = Some(invented(0xB1));
    let tested = "if (((as_u32(Virtual.type_00000001000000D1.Method_3(p0, p1)) == 0))) { return; }";
    let project = line(class, tested);
    assert_eq!(result(&project, 0x10), Kind::Bool);
    assert_eq!(project.slot_result(invented(0xC1), 3), Some(&Kind::Bool));
    // The call through the empty slot is a call of no one line, and states nothing itself.
    assert_eq!(project.slot_result(invented(0xD1), 3), None);
    let counted =
        "Native.DEMO.USE(((as_i32(Virtual.type_00000001000000D1.Method_3(p0, p1)) + as_i32(1))));";
    assert_eq!(result(&line(class, counted), 0x10), Kind::I32);
    // A type of no base is taken for an interface, whose slots the tables under it number apart:
    // the call may reach any line of the slot.
    assert_eq!(result(&line(None, tested), 0x10), Kind::Unknown);
}

#[test]
fn what_a_function_whose_signature_is_not_kept_calls_is_read_as_anything() {
    let tests = "if (((as_u32(func_000010(p0, p1)) == 0))) { return; } if (((as_u32(Native.DEMO.ASK(p0)) != 0))) { return; } return;";
    let reads = "Native.DEMO.USE(((as_i32(func_000010(p0, p1)) + as_i32(1)))); Native.DEMO.USE(Native.DEMO.ASK(p0));";
    let with = |returns: &str| {
        let functions = [
            function("VmValue", 0x10, 2, FLAG),
            function("void", 0x30, 2, tests),
            function("VmValues", 0x40, 2, &format!("{reads} {returns}")),
        ];
        project_of(&[listing(1, &functions)])
    };
    // Its returns state its cells: what it reads is counted.
    let counted = with("return new VmValues(1, 2);");
    assert_eq!(result(&counted, 0x10), Kind::I32);
    assert!(!counted.native_truths.contains(&5));
    // They state two counts, so it keeps no signature, and no read of it is counted.
    let lost = with(
        "if (((as_u32(p0) == 0))) { return new VmValues(1, 2); } return new VmValues(1, 2, 3);",
    );
    assert!(!lost.signatures.contains_key(&key(0x40)));
    assert_eq!(result(&lost, 0x10), Kind::Unknown);
    assert!(!lost.native_truths.contains(&5));
}

#[test]
fn a_reference_a_function_whose_signature_is_not_kept_took_before_the_passes_is_counted() {
    let tests = "if (((as_u32(func_000010(p0, p1)) == 0))) { return; } return;";
    // A reference the code takes and nothing reads: the passes take it off as dead.
    let taken = "v_000001_0 = Vm.Function(0x00000001U, 0x000010);";
    let with = |returns: &str| {
        let functions = [
            function("VmValue", 0x10, 2, FLAG),
            function("void", 0x30, 2, tests),
            function("VmValues", 0x40, 2, &format!("{taken} {returns}")),
        ];
        project_of(&[listing(1, &functions)])
    };
    // Its signature kept, or lost because its returns state two counts: the reference is counted either way.
    let kept = with("return new VmValues(1, 2);");
    assert!(kept.signatures.contains_key(&key(0x40)));
    let lost = with(
        "if (((as_u32(p0) == 0))) { return new VmValues(1, 2); } return new VmValues(1, 2, 3);",
    );
    assert!(!lost.signatures.contains_key(&key(0x40)));
    for project in [&kept, &lost] {
        assert_eq!(result(project, 0x10), Kind::Unknown);
    }
    // Without the reference, the one test of it says a truth.
    let functions = [
        function("VmValue", 0x10, 2, FLAG),
        function("void", 0x30, 2, tests),
    ];
    assert_eq!(
        result(&project_of(&[listing(1, &functions)]), 0x10),
        Kind::Bool
    );
}

#[test]
fn a_parameter_read_only_as_a_float_and_passed_on_where_it_is_read_so_is_a_float() {
    let read = format!("Native.DEMO.USE({FLOAT});");
    let and = |at: u32| format!("{read} func_{at:06X}(p0); return;");
    let functions = [
        function("void", 0x20, 1, &format!("{read} return;")),
        function("void", 0x10, 1, &and(0x20)),
        // Read whole by a native where it is passed on.
        function("void", 0x40, 1, TAKES),
        function("void", 0x30, 1, &and(0x40)),
        // Read only as a float all round a ring of calls.
        function("void", 0x60, 1, &and(0x70)),
        function("void", 0x70, 1, &and(0x60)),
        // Passed on to one that passes it on to the native.
        function("void", 0x80, 1, &and(0x90)),
        function("void", 0x90, 1, &and(0x40)),
        // Handed back whole, or its address taken.
        function("VmValue", 0xA0, 1, &format!("{read} return p0;")),
        function(
            "void",
            0xB0,
            1,
            &format!("{read} Native.DEMO.USE(Vm.Address(parameters, 0)); func_000020(p0); return;"),
        ),
        // Passed on to one that only passes it on to a reading of a float.
        function("void", 0xC0, 1, &and(0xD0)),
        function("void", 0xD0, 1, "func_000020(p0); return;"),
    ];
    let project = project_of(&[listing(1, &functions)]);
    for at in [0x20, 0x10, 0x60, 0x70, 0xC0, 0xD0] {
        assert_eq!(param(&project, at, 0), Kind::F32, "func_{at:06X}");
    }
    for at in [0x30, 0x80, 0x90, 0xA0, 0xB0] {
        assert_eq!(param(&project, at, 0), Kind::Unknown, "func_{at:06X}");
    }
}

#[test]
fn a_parameter_read_only_as_an_int_and_passed_on_where_it_is_read_so_is_an_int() {
    let read = "Native.DEMO.USE(((as_i32(p0) < 5)));";
    let functions = [
        function("void", 0x20, 1, &format!("{read} return;")),
        function("void", 0x10, 1, &format!("{read} func_000020(p0); return;")),
        // Read as a float where it is passed on: two kinds are none.
        function(
            "void",
            0x40,
            1,
            &format!("Native.DEMO.USE({FLOAT}); return;"),
        ),
        function("void", 0x30, 1, &format!("{read} func_000040(p0); return;")),
    ];
    let project = project_of(&[listing(1, &functions)]);
    assert_eq!(param(&project, 0x20, 0), Kind::I32);
    assert_eq!(param(&project, 0x10, 0), Kind::I32);
    assert_eq!(param(&project, 0x40, 0), Kind::F32);
    assert_eq!(param(&project, 0x30, 0), Kind::Unknown);
}
