use super::*;
use crate::gta6::script::ir::TypeKey;
use crate::gta6::script::listing::Listing;
use crate::gta6::script::passes::{types, values};
use crate::gta6::script::program::FunctionRow;
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
            values::run(&mut f, &names, &mut stats);
            types::run(&mut f, &names, &mut stats);
            of_function(&mut project, &f);
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
    // The variable that keeps the result is tested, and handed to a native that says nothing.
    assert_eq!(result(&project, 0x40), Kind::Bool);
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
        // A 32-bit reading and a load say what the kinds pass reads, not what the cell holds.
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
    assert!(cells(0x60).is_empty() && cells(0x80).is_empty());
    // No call names the function at 0x20 or 0x50: their callers are not known.
    assert!(cells(0x20).is_empty() && cells(0x50).is_empty());
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
}
