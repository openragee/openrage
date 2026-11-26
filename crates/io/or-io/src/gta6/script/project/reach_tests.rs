use super::*;
use crate::gta6::script::listing::Listing;
use crate::gta6::script::passes::{types, values};
use crate::gta6::script::project::signatures::tests::{function, listing};
use crate::gta6::script::{Names, Stats};

/// The settled reaches of the functions of one listing of program 1, each read as the indexer reads it.
fn reaches(functions: &[String]) -> Project {
    let names = Names::default();
    let mut stats = Stats::default();
    let mut project = Project::default();
    let text = listing(1, functions);
    let listing = Listing::open(&text).expect("the listing opens");
    for read in listing.functions() {
        let mut f = cfg::lower(read.expect("the function reads")).expect("the function lowers");
        values::run(&mut f, &names, &mut stats);
        types::run(&mut f, &names, &mut stats);
        of_function(&mut project, &f);
    }
    settle(&mut project);
    project
}

fn key(at: u32) -> FunctionKey {
    FunctionKey::new(1, at)
}

#[test]
fn a_cell_reaches_what_the_functions_loads_and_stores_through_it_and_its_callees_reach() {
    let functions = [
        // Eight bytes at 8, and the four at 0x10 of the second cell.
        function(
            "void",
            0x10,
            2,
            "store_u64(p0, 0x8, 1UL); Native.DEMO.USE(load_i32(p1, 0x10)); return;",
        ),
        // Handed on moved by 0x20 bytes, and tested against zero.
        function(
            "void",
            0x20,
            1,
            "if (((as_u64(p0) != 0))) { func_000010(byte_offset(p0, 0x20), p0); } return;",
        ),
        // Kept, and handed to a native.
        function("void", 0x30, 1, "Native.DEMO.USE(p0); return;"),
        function("void", 0x40, 1, "func_000030(p0); return;"),
        // Handed to a function the project holds no reach of.
        function("void", 0x50, 1, "func_000090(p0); return;"),
        // A ring that moves the address on every round grows without end.
        function(
            "void",
            0x60,
            1,
            "func_000060(byte_offset(p0, 0x8)); return;",
        ),
        // A ring that does not is bounded by what it touches.
        function(
            "void",
            0x70,
            1,
            "store_i32(p0, 0x4, 1); func_000070(p0); return;",
        ),
    ];
    let project = reaches(&functions);
    let reach = |at: u32, cell: u32| project.reach(key(at), cell);
    assert_eq!((reach(0x10, 0), reach(0x10, 1)), (Some(16), Some(20)));
    // Its own load says nothing, and what it hands on reaches 0x20 + 16 bytes, or 20 as the second cell.
    assert_eq!(reach(0x20, 0), Some(0x30));
    assert_eq!(reach(0x30, 0), Some(UNBOUNDED));
    assert_eq!(reach(0x40, 0), Some(UNBOUNDED));
    assert_eq!(reach(0x50, 0), Some(UNBOUNDED));
    assert_eq!(reach(0x60, 0), Some(UNBOUNDED));
    assert_eq!(reach(0x70, 0), Some(8));
    assert_eq!(project.reach(key(0x90), 0), None);
}
