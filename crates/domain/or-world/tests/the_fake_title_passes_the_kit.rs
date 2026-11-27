//! The fake title through the conformance kit: every rule's test, written by the kit's macro,
//! over a world that names no game.

use or_world::fake::{fixture, FakeTitle};

or_world::conformance_kit!(fixture(FakeTitle::new()));
