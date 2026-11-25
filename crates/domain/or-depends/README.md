# or-depends

What an asset needs and where a model is placed, resolved over a mounted install.

Both are one graph read in opposite directions, and both are questions about an install rather than a file: a `.ymap` places an archetype by name and never says which `.ytyp` declares it. `Catalogue` turns a name into a file, `Resolver` an archetype into the files a streamer loads, and `PlacementIndex` reads placement backwards. `openrage-cli depends` runs it with no window. `WorldIndex` answers the maps and the bounds at a position, kept on disk and read again an archive at a time; `openrage-cli world index` and `world at` run it. `WorldCollision` reads the static bounds the index names near a place and answers the ground under it, and `openrage-cli world ground` runs it. `find` answers what a person typed, a place, a hash or a name, from those indexes, with a model's placements nearest a view first, and `openrage-cli world find` runs it.

⚠ What each format states, what only the install states and what is a filename convention is data, in `rules`, and its suite fails when a format has no ruling.

- Layer: domain; it takes a mount from `or-title`.
- Start at [src/lib.rs](src/lib.rs).
- Tests: `just test-crate or-depends`.
- Reference: [Crate_Reference.md](../../../docs/reference/Crate_Reference.md#domain-crates).
