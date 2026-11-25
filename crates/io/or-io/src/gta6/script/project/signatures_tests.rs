use super::*;
use crate::gta6::script::listing::Listing;
use crate::gta6::script::passes::{types, values};
use crate::gta6::script::program::FunctionRow;
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
