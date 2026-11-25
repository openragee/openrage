# or-transport

The playback clock the editor's previews share: play, loop and scrub.

The particle preview and the animation preview both run on it, which is why neither `or-anim` nor `or-particle` depends on the other.

[src/clock.rs](src/clock.rs) is the frame clock, the game clock and the random source of what moves from frame to frame, which a shots run fixes so that two runs draw the same frames.

- Layer: foundation, which knows no file format and no game.
- Start at [src/lib.rs](src/lib.rs).
- Tests: `just test-crate or-transport`.
- Reference: [Crate_Reference.md](../../../docs/reference/Crate_Reference.md#foundation-crates).
