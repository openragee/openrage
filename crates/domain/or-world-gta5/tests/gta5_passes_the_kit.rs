//! GTA V's title through the conformance kit, over an install built by hand and opened with no key.

mod forest_fixture;
mod hand_built;
mod world_fixture;

or_world::conformance_kit!(hand_built::fixture());
