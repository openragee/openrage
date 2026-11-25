# or-project

The project container: what a `.openrage-proj` holds and declares, the plan of a build as data, and the executor that writes it.

A project has no type. It holds modules, shown as Content, and declares outputs, shown as Builds: a server resource, a DLC pack or a package. Each kind of content hands an output a `Contribution`, `plan` merges them into what the output will write, `diff` says what changed since the last build, and `build` writes it. The editor and `openrage-cli project` share all of it.

⚠ A build never says a pack loads in game. It says every file it wrote equals what the plan named and reloads through the readers.

- Layer: domain.
- Start at [src/lib.rs](src/lib.rs).
- Tests: `just test-crate or-project`.
- In game: the sheet of what a person saw the game load is [src/acceptance.json](src/acceptance.json), and [In_Game_Acceptance.md](../../../docs/guides/In_Game_Acceptance.md) is its guide.
- Design: [Project_Model_Design.md](../../../docs/direction/Project_Model_Design.md); the plan's JSON is [Plan_Document.md](../../../docs/reference/Plan_Document.md).
- Reference: [Crate_Reference.md](../../../docs/reference/Crate_Reference.md#domain-crates).
