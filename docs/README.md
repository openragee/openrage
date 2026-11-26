# Documentation

Every document in `docs/`, listed under the folder of its kind. The gitignored `docs/extra/` tier of RAGE's binary layouts is not part of the repository. [AGENTS.md](../AGENTS.md) says which to read for a task, and [Documentation_Standard.md](guides/Documentation_Standard.md#where-a-document-lives) says how they are written and where a new one goes.

## Direction

Records, plans and specifications: what is decided, what comes next, and the designs being built.

- [Project_Context.md](direction/Project_Context.md): what OpenRage is for, its vocabulary and the owner's dated decisions.
- [Roadmap.md](direction/Roadmap.md): everything planned, in dependency order, and where each item stands.
- [OR_IO_Specification.md](direction/OR_IO_Specification.md): the design of the `or-io` crate and the decisions it rests on.
- [Workbench_Plan.md](direction/Workbench_Plan.md): the order in which projects, sources, the clothing engine and their screens arrive.
- [Workbench_UX_Design.md](direction/Workbench_UX_Design.md): what a person sees and does around a project.
- [Project_Model_Design.md](direction/Project_Model_Design.md): how a project holds several kinds of content and ships it.
- [Workbench_Views_Design.md](direction/Workbench_Views_Design.md): a draft of the weapon, particle, map and other project views, and the order they could be built in; its first step, the table of task screens, is built.
- [World_Editor_Plan.md](direction/World_Editor_Plan.md): what OpenRage takes from NodeRunner Studio for GTA V, what that rests on, how it fits the tree, and the order and the briefs of the rows that build it.
- [World_Editor_Parity.md](direction/World_Editor_Parity.md): what NodeRunner Studio does for a GTA V map, capability by capability, and what OpenRage has, takes, adapts, defers, holds for a ruling or leaves out.
- [World_Editor_UX_Design.md](direction/World_Editor_UX_Design.md): what a mapper would meet in the GTA V world editor, flow by flow, each part tagged with the row that builds it.
- [Unified_World_Plan.md](direction/Unified_World_Plan.md): how the world viewer and editor become one title-neutral core with an adapter crate for each game, the contract between them, how a world is drawn, and the order and the briefs of the rows that build it.
- [Source_Format_Design.md](direction/Source_Format_Design.md): how a game file becomes an editable source and compiles back.
- [Clothing_Tool_Design.md](direction/Clothing_Tool_Design.md): where a clothing and ped-component tool lives.
- [Wardrobe_Parity.md](direction/Wardrobe_Parity.md): what each clothing tool does that the Wardrobe does, will do or leaves out, and what a project costs a clothing creator.
- [Shader_Pipeline_Design.md](direction/Shader_Pipeline_Design.md): how the original compiled shaders run natively on each OS.

## Reference

Contracts that code is written against, and the map of the tree.

- [Crate_Reference.md](reference/Crate_Reference.md): where each crate and part of the app sits, the rules between them and their traps.
- [CLI_Server_Protocol.md](reference/CLI_Server_Protocol.md): the wire contract of `openrage-cli serve`, with a Go client, [openrage_client.go](reference/openrage_client.go).
- [MCP_Server.md](reference/MCP_Server.md): `openrage-cli mcp`, the server an AI agent drives OpenRage through, how a client is given it and its tools.
- [Plan_Document.md](reference/Plan_Document.md): the JSON a build's plan is written as, which `openrage-cli wardrobe plan` prints.
- [Plugin_ABI.md](reference/Plugin_ABI.md): the contract between an in-app plugin and the host.
- [Particle_Shader_ABI.md](reference/Particle_Shader_ABI.md): the contract between the shader bake and the particle pipelines.

## Guides

Steps, checklists, traps and the policies a contributor follows.

- [Development_Workflow.md](guides/Development_Workflow.md): lanes, build output, the local checks, CI and how a change lands.
- [New_Format_Implementation_Guide.md](guides/New_Format_Implementation_Guide.md): bringing a GTA V format up from reconnaissance to the app.
- [Documentation_Standard.md](guides/Documentation_Standard.md): how documents are written, how figures stay true and which checks read them.
- [Oracle_Testimony.md](guides/Oracle_Testimony.md): what the compiled readers' testimony records and how a layout is checked against it.
- [Licence_And_Provenance_Policy.md](guides/Licence_And_Provenance_Policy.md): what the tree may take from the tools and sources around it.
- [Private_Tier.md](guides/Private_Tier.md): what a clone lacks, why, and how to rebuild it.
- [Interface_Languages.md](guides/Interface_Languages.md): how to make or change a language of the interface, and the words the French catalog uses.
- [Workbench_Guide.md](guides/Workbench_Guide.md): how a clothing creator opens a pack, fixes it, builds it, installs it and shares a project; the app shows it under Help.
- [In_Game_Acceptance.md](guides/In_Game_Acceptance.md): the sheet of what to look for in a running game, the recipe that makes a build to check from your own install, how to load it and how to write a result down.
- [GTA_VI_Modding.md](guides/GTA_VI_Modding.md): what a user does with a GTA VI install, from the keys of their executable to a copy of an archive, and why the game refuses what OpenRage writes.

## Features

How a subsystem works today and what it does not do.

- [File_Support.md](features/File_Support.md): what the editor and the CLI do with each kind of file, per game, and where each stops.
- [Animation_Preview.md](features/Animation_Preview.md): how the Animation workspace plays and edits `.ycd` clips.
- [YCD_Channel_Encoding.md](features/YCD_Channel_Encoding.md): how an edited `.ycd` has its channel data written back.
- [Particle_Preview.md](features/Particle_Preview.md): how the Particle workspace plays a `.ypt` effect.
- [Viewport_Shading_Modes.md](features/Viewport_Shading_Modes.md): what each shading mode of the viewport draws.
- [Mesh_Interchange.md](features/Mesh_Interchange.md): how a drawable goes out to glTF and edited geometry comes back.
- [BCn_Encoding_And_Mipmaps.md](features/BCn_Encoding_And_Mipmaps.md): how an RGBA image becomes a Gen8 texture.
- [Shader_Definition_Database.md](features/Shader_Definition_Database.md): what a material's parameters are, extracted from an install.
- [Dlc_Pack_Creation.md](features/Dlc_Pack_Creation.md): what a DLC pack states about itself and how OpenRage writes one.
- [GTA_VI_Xbox_Build.md](features/GTA_VI_Xbox_Build.md): what OpenRage reads, edits, saves and refuses of a GTA VI Xbox build, and how far each part was checked.
- [GTA_VI_World_Viewer.md](features/GTA_VI_World_Viewer.md): how the World screen mounts a GTA VI install, streams its maps and draws them.
- [GTA_VI_Script_Decompiler.md](features/GTA_VI_Script_Decompiler.md): what is read of a compiled GTA VI script, the levels and passes of the source it decompiles to, the names lists, and what cannot be recovered.
