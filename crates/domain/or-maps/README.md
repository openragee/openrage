# or-maps

The maps engine: the map files a person streams into the game, as content of a project, with no UI.

`maps.json` is the map sets of a pack, each naming the sources it is made of. The crate reads what the map files place and the type files declare, says what is wrong as rules, edits an entity or an archetype in its source, says what a set places as boxes in the world, holds the manipulator that moves, turns and scales one, and finds the doors of a set with what a door script is told of them. `openrage-cli maps` runs the first part and the doors with no window.

The world model is [or-world](../or-world)'s and the typed roots are `or-io`'s: this crate holds no reader and no schema of its own.

- Layer: domain; a kind of content of [or-project](../or-project).
- Start at [src/lib.rs](src/lib.rs).
- Tests: `just test-crate or-maps`.
- Reference: [Crate_Reference.md](../../../docs/reference/Crate_Reference.md#domain-crates).
