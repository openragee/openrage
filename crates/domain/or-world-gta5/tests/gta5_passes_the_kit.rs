//! GTA V's title through the conformance kit, over an install built by hand and opened with no key.

#[path = "../../or-depends/tests/forest_fixture/mod.rs"]
mod forest_fixture;
mod hand_built;
#[path = "../../or-depends/tests/world_fixture/mod.rs"]
mod world_fixture;

or_world::conformance_kit!(hand_built::fixture());
