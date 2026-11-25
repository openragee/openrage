# OpenRage roadmap

Everything OpenRage plans to do, in dependency order: a checkbox index by phase, the non-goals, and
a detail section per item.

OpenRage aims to replace CodeX, CodeWalker and OpenIV for GTA V Legacy and Enhanced, then RDR2 and
later GTA VI. The core generalises create, edit and delete across Rockstar formats; deep domain
workflows live on one of two plugin surfaces, a WebAssembly guest inside the app or a subprocess
outside it ([Phase 7](#phase-7--plugins-and-open-with)).

## How to read and maintain it

- Phases are ordered by dependency, and the items inside a phase roughly so. A checked box means the
  row is closed: done in the tree unless its line says replaced, obsolete, withdrawn or declined.
- The checkbox list is the index. Every box links to a detail section, and a number decomposed into
  lettered parts keeps its heading as a programme header, listed with its parts indented under it.
  `the_roadmap_is_an_index_of_itself` in `src/doc_counts.rs` holds both, so the number of items is
  written nowhere.
- A new item takes a letter suffix (`R0b`, `R25b`), never the next number, because other documents
  link to its anchor. An item decomposes into lettered parts only when each can be finished alone
  and the parts differ in what would finish them; unscoped parts are a reason to scope them.
- Cite by name (a document and heading, a constant, a test), never by line. A `file:NN` locator
  belongs only inside an audit finding taken at a stated revision.
- `U-nnn` names a row of the union list in [Inputs outside the tree](#inputs-outside-the-tree), and
  figures follow [Documentation_Standard.md](../guides/Documentation_Standard.md).

---

## The whole roadmap

### Phase 0 — Before anything else

Open on R2 alone: no check can read every count a document states.

- [x] [R0b Post-merge detection](#r0b-post-merge-detection)
- [x] [R1 Land the v2 overhaul branch on main](#r1-land-the-v2-overhaul-branch-on-main)
- [ ] [R2 Reconcile the documentation with the tree](#r2-reconcile-the-documentation-with-the-tree) — open on "no document states a count that no test asserts", a universal that `every_count_a_document_states_is_asserted_or_a_named_measurement` holds for every count it can read; a narrower Done-when is proposed in the row
- [x] [R3 Licence and provenance policy](#r3-licence-and-provenance-policy) — closed: superseded by D15 for layout declarations; publication is R66h; no provenance record is kept: the grammar is deleted, no `Provenance:` block is in code, and the provenance lint stays (owner, 2026-10-01)

### Phase 1 — The or-io crate (the critical path)

The descriptor-pack items, R4–R15, are done, and [R66](#r66-the-or-io-crate) replaced their
mechanism: D1 made the `or-io` declarations the layout authority, and D17 retired `openrage-packs`
([OR_IO_Specification.md](OR_IO_Specification.md), D1–D18). GTA V's structures are declared in
`or-io` under the engine's names (D15), every format `FORMATS` lists opens and saves as an owned
value (D16), and nothing in the tree builds, loads or discovers a descriptor pack. R66a–R66f are
done, and R66i: the owner archived and deleted the pack repository on 2026-10-01. What is next is
R66g's history rewrite, which the owner left for later; R66h's publication waits on it and on the
lawyer questions (D14), and both are parked. R65, the ruling of
2026-09-23 that OpenRage holds no RAGE resource structs, is superseded by R66, and a part R66
replaced is ticked as closed rather than done.

- [x] [R15 A conformance gate that runs where the code is](#r15-a-conformance-gate-that-runs-where-the-code-is)
- [x] [R4 Step 1 the pack is the layout authority](#r4-step-1-the-pack-is-the-layout-authority) — D1 made the `or-io` declarations the layout authority, and D17 retired the pack
- [x] [R5 Step 2 the pack learns to write](#r5-step-2-the-pack-learns-to-write) — the generic writer stands, and every owned value saves through it
- [x] [R6 Cross-field invariants before the oracle goes](#r6-cross-field-invariants-before-the-oracle-goes)
- [x] [R7 Step 3 make pack binding the only path](#r7-step-3-make-pack-binding-the-only-path) — binding replaced by D16's owned values
- [x] [R8 Step 3 strip the layout apparatus from rage types](#r8-step-3-strip-the-layout-apparatus-from-rage-types) — replaced by D15: `#[record]` computes every offset
- [x] [R9 Step 4 convert the four divergent leaves and relax the bind check](#r9-step-4-convert-the-four-divergent-leaves-and-relax-the-bind-check) — the per-target types stand; the bind check went with `or-asset`
- [x] [R10 Step 4 delete the Gen9 clone tree](#r10-step-4-delete-the-gen9-clone-tree) — the `G9` aliases are gone; the `gen9/` rename was withdrawn
- [x] [R11 Close the fixture-blocked layouts](#r11-close-the-fixture-blocked-layouts)
- [x] [R12 Step 5 open DocumentKind and hold an image not a payload](#r12-step-5-open-documentkind-and-hold-an-image-not-a-payload)
- [x] [R13 Step 5 shrink the shell](#r13-step-5-shrink-the-shell)
- [x] [R14 Step 5 finish the container seam](#r14-step-5-finish-the-container-seam)

- [ ] [R66 The or-io crate](#r66-the-or-io-crate) — R66a–R66f and R66i done; open: R66g, R66h
  - [x] [R66a Decide and record the direction](#r66a-decide-and-record-the-direction) — D1–D14 recorded in `Project_Context.md`, the standing documents point at the specification, and R66 is on this roadmap
  - [x] [R66b Clean what the crate will contain](#r66b-clean-what-the-crate-will-contain) — the key snapshot out of the tree (bundled again in `or-io` by D18), the version table measured, the leaked-source pattern lint fatal in every tracked file (#340)
  - [x] [R66c The crate and the macro on synthetic types](#r66c-the-crate-and-the-macro-on-synthetic-types) — `or-io` and `or-io-macros` exist; one declaration makes a struct, a view, a record and a description
  - [x] [R66d Declare GTA V format by format](#r66d-declare-gta-v-format-by-format) — every format declared under the engine's names with computed offsets and no unk fields (D15), and opened as an owned value (D16); metadata is `or_io::meta`. Carried past it: `.ysc` stays ordinary code until a corpus exists (OD-YSC), one site a save still loses (`census::STILL_LOST`), and GTA V declarations' bit ranges under `#[bits]` (OD-BITS)
  - [x] [R66e Move the consumers](#r66e-move-the-consumers) — animation, simulation, interchange, the editors, the CLI and the app onto the owned values of D16 and one resource payload
  - [x] [R66f Delete the old crates](#r66f-delete-the-old-crates) — `or-asset`, `or-format-api`, `or-ptx` and `or-macros` are deleted with the compiled readers, the old code generators and pack discovery, and the standing documents describe the result
  - [ ] [R66g Rewrite the history](#r66g-rewrite-the-history) — citations and documents about leaked code and the coverage test removed from every commit, verified by a scan shown failing first; not started, and the provenance lint's history mode, that scan, has named both leaks planted in a throwaway mirror
  - [x] [R66i Retire openrage-packs](#r66i-retire-openrage-packs) — nothing in the tree reads the repository, and the owner archived it as a private offline bundle and deleted it on 2026-10-01 (D17)
  - [ ] [R66h Publish the crate and the editor together](#r66h-publish-the-crate-and-the-editor-together) — this repository public and both crates on crates.io the same day; waits on R66g and the lawyer questions (D14), and is parked with them (owner, 2026-10-01)

- [x] [R65 No RAGE structs in OpenRage](#r65-no-rage-structs-in-openrage) — superseded on 2026-09-24 by R66; every part closed
  - [x] [R65a Type plans the pack builds](#r65a-type-plans-the-pack-builds) — landed (#195): the per-type plan a description builds, one bounds check per record; the record tree, owned declarations and testimony check read through it
  - [x] [R65b Records a tree whose scalars are bytes](#r65b-records-a-tree-whose-scalars-are-bytes) — landed (#195, #196): the editable record tree the inspector edits and the canonical writer saves
  - [x] [R65c Record the oracle's testimony](#r65c-record-the-oracles-testimony) — landed in #192 over the corpus and both installs; the record stays in `openrage-corpus/testimony` by the owner's exception, and every declared format's gates check it re-keyed by offset. Closed as done: the three Gen9 rows (`.ybn`, `.ycd`, `.yld`) are ruled not recordable, because neither install ships such a file (OD-TESTIMONY)
  - [x] [R65d Pointers at a member of their owner](#r65d-pointers-at-a-member-of-their-owner) — member pointers (`Pointee::Member`, owned `Member<T>`), which closed `.ypt`'s write row (#197)
  - [x] [R65e Generic types instantiated at export](#r65e-generic-types-instantiated-at-export) — generic layouts behind pointers, which closed `.ycd`'s write row (#197)
  - [x] [R65f The ypt write row](#r65f-the-ypt-write-row) — both targets through the canonical writer, to a fixed point (#197); `.ypt` opens owned since R66d
  - [x] [R65g The ycd write row](#r65g-the-ycd-write-row) — the Gen8 row through the canonical writer, to a fixed point (#197); the game ships no Gen9 `.ycd` to hold the other
  - [x] [R65h A ratchet that counts every RAGE struct](#r65h-a-ratchet-that-counts-every-rage-struct) — closed: replaced by [R66b](#r66b-clean-what-the-crate-will-contain), whose counts D15 then withdrew
  - [x] [R65i Animation on the pack](#r65i-animation-on-the-pack) — closed: replaced by [R66e](#r66e-move-the-consumers)
  - [x] [R65j The simulation and the particle model on the pack](#r65j-the-simulation-and-the-particle-model-on-the-pack) — closed: replaced by [R66e](#r66e-move-the-consumers)
  - [x] [R65k Interchange on the pack](#r65k-interchange-on-the-pack) — closed: replaced by [R66e](#r66e-move-the-consumers)
  - [x] [R65l The editors on records](#r65l-the-editors-on-records) — closed: replaced by [R66e](#r66e-move-the-consumers)
  - [x] [R65m The app holds one resource payload](#r65m-the-app-holds-one-resource-payload) — closed: replaced by [R66e](#r66e-move-the-consumers)
  - [x] [R65n The CLI on the pack](#r65n-the-cli-on-the-pack) — closed: replaced by [R66e](#r66e-move-the-consumers)
  - [x] [R65o Gen9 collapses into the bound pack](#r65o-gen9-collapses-into-the-bound-pack) — closed: obsolete; each target is declared as itself, the four changed types in per-target modules of `or_io::gta5`
  - [x] [R65p Delete the structs](#r65p-delete-the-structs) — closed: replaced by [R66f](#r66f-delete-the-old-crates)
  - [x] [R65q The RPF7 archive through the pack](#r65q-the-rpf7-archive-through-the-pack) — closed: replaced by [R66d](#r66d-declare-gta-v-format-by-format); RPF7's header and entry table are declared (`FiPackHeader`, `FiPackEntry`, #262), and mounting an install never needed a pack
  - [x] [R65r The RSC7 header through the pack](#r65r-the-rsc7-header-through-the-pack) — closed: replaced by [R66d](#r66d-declare-gta-v-format-by-format); the RSC7 header is declared as `DatResourceFileHeader` (#216)
  - [x] [R65s GXT2 and YPDB through the pack](#r65s-gxt2-and-ypdb-through-the-pack) — closed: replaced by [R66d](#r66d-declare-gta-v-format-by-format), GXT2 is declared and owned as `Gxt2` (#265); YPDB is ordinary code and not declared (OD-YPDB, [2026-09-26](Project_Context.md#the-pose-database-is-ordinary-code-2026-09-26))
  - [x] [R65t Data addressed by offset](#r65t-data-addressed-by-offset) — closed: obsolete; declarations compute their offsets, and RPF7 and GXT2 are declared, so the generic tree needs no offset repr
  - [x] [R65u Roles engine names out of source](#r65u-roles-engine-names-out-of-source) — closed: withdrawn; the declared field names, the engine's since D15, are the vocabulary, checked by the compiler
  - [x] [R65v Metadata containers](#r65v-metadata-containers) — closed: replaced by [R66d](#r66d-declare-gta-v-format-by-format); metadata is `or_io::meta` (D5, #267)

### Phase 2 — Proof: a check that could have failed

Done. The skip ledger stands, and `just test` ends with it. R17's instruments are replaced by R66's
per-format gates in `crates/io/or-io-verify/src/gates.rs`, and R18's answer, that nothing adequate
replaces the compiled oracle, leaves the recorded testimony and R66e's behaviour baselines as the
checks that outlived `or-asset` ([Oracle_Testimony.md](../guides/Oracle_Testimony.md)).

- [x] [R16 Make a skipped suite visible instead of green](#r16-make-a-skipped-suite-visible-instead-of-green)
- [x] [R17 Equivalence contracts and poison canvases for the resource writers](#r17-equivalence-contracts-and-poison-canvases-for-the-resource-writers) — its instruments replaced by R66's gates, and the poison canvas held again by `what_a_writer_inherits_from_its_canvas`
- [x] [R18 Decide what replaces the compiled oracle](#r18-decide-what-replaces-the-compiled-oracle)

### Phase 3 — The explorer core (OpenIV / CodeX / RPFKit parity floor)

A user can mount an install, walk it through one VFS, search it and be stopped before writing into
it (R19, R22–R25), and open any file it reaches, as text or as its bytes where no reader names it
(R25c). R20 is open on a DLC-shaped archive tree built in the app that the game loads,
and R25b on a person trying it. R21 is closed: extraction from an open archive answers it, and
install-wide jobs stay in `openrage-cli bulk` (owner, 2026-10-01).
[R26](#r26-index-dependencies-and-summaries) is closed: `openrage-cli depends export` copies an
asset with its textures, skeleton and collision, and a map's static bounds resolve where
`_manifest.ymf` names them; an app surface is beyond its Done-when. The UX review of 2026-10-03
adds the rows that make the browser trustworthy and central: nested archives that survive a save
(R20b, and R20c to R20j, found while closing R20b, R20c, R20e, R20f, R20g, R60f, R20h and R20i), go-to-file and search results that open (R24b, R24c), the Files view in the centre (R25d),
destructive operations that ask (R25e), copy and paste (R25f), an edit mode that is a visible
control (R25g), adds that land where the tree shows them (R25h, found while closing R20c), a
right-click that selects its row (R25i, found while closing R27k), a drag that carries every
selected row (R25j, found while closing R25i), a drag that copies and a change of nothing that
does nothing (R25k, found while closing R25j and R26c, and R25l, found while closing R25k), and
dependencies in the app (R26b, R26c found while closing it, R26d found while closing R26c, and R26e
found while closing R26d).

- [x] [R19 One virtual filesystem](#r19-one-virtual-filesystem) — the library, the CLI and the app glob through one VFS; the app's tree lists, reads, extracts and opens nested archives through `ArchiveDevice`, and browsing a drawable loads nothing into the renderer
- [ ] [R20 Archive lifecycle](#r20-archive-lifecycle) — the builder, the encryption chooser, the one-time `.bak`, `openrage-cli rpf defrag` and Defragment in the app's archive menu exist; open: a DLC-shaped archive tree built in the app that the game loads
- [x] [R20b Nested archives survive a save](#r20b-nested-archives-survive-a-save) — a rebuilt archive stores each untouched nested archive as the bytes it held and rebuilds an edited one under the encryption it was read with; an edit inside a nested archive saves through the archive on disk that holds it (#457)
- [x] [R20c Saving twice and into an archive not yet opened](#r20c-saving-twice-and-into-an-archive-not-yet-opened) — a saved archive is read back and the tree pointed at it; an archive row the tree has not opened is opened before anything is added to or moved into it; an entry that cannot be read refuses the save by name; `Ctrl+S` on an opened folder saves its archives; a renamed or moved archive saves where it now is (#462)
- [ ] [R20d A rebuilt NG archive the game can read](#r20d-a-rebuilt-ng-archive-the-game-can-read)
- [x] [R20e Two readers of one archive across a save](#r20e-two-readers-of-one-archive-across-a-save) — before a save replaces an archive's file, every file moved out of it is read into memory, so the archive it was moved into saves it whole whichever is saved first, and a nested archive moved out holds that save until the other is saved; a save of an archive an extraction reads is refused, naming the extraction (#473)
- [x] [R20f A moved or deleted archive row leaves nothing behind](#r20f-a-moved-or-deleted-archive-row-leaves-nothing-behind) — a row moved into another archive saves into it, asks the gate about it and is marked unsaved there alone; a move onto an archive row on disk lands inside it; one `Ctrl+S` saves every archive of a tree; a deleted archive leaves nothing to save; a defragment and an extraction of one archive refuse each other, an expansion begun across a save is listed again, and an extraction of an archive row not yet expanded opens it first (#482)
- [x] [R20g What archive rows still get wrong](#r20g-what-archive-rows-still-get-wrong) — an archive row with unsaved edits is marked until its save; a rename on disk leaves nothing to save; a delete of an archive an extraction reads is refused; an extraction opens every folder and archive under what it names before it plans; a folder dropped onto itself or one inside it is refused (#495)
- [x] [R20h What archive rows still get wrong after R20g](#r20h-what-archive-rows-still-get-wrong-after-r20g) — an extraction opens what it names on workers and the frame that starts it opens none; a rename on disk and a delete of a loose folder wait for the extraction reading them, as a move already did; dragging rows out and the counts of Only one type take every file under the selection, opened or not; a folder added inside an archive loses its New badge once a save writes it; an unscanned folder says it is not read; and a nested archive moved out of one still reads once that archive is renamed on disk (#535)
- [x] [R20i What archive rows still get wrong after R20h](#r20i-what-archive-rows-still-get-wrong-after-r20h) — a drag out opens nothing in the frame that starts it and says when to drag again; the counts of Only one type list what the tree holds and read no archive until Count types in archives… is clicked; a save under a folder an extraction is opening goes ahead unless the extraction is opening that archive; and an archive that would not open counts once it is mended (#565)
- [x] [R20j What archive rows still get wrong after R20i](#r20j-what-archive-rows-still-get-wrong-after-r20i) — a mended folder is opened by the next drag, a count lists a failed folder once and goes on with the menu closed, a drag out's opening is in the jobs list with a Cancel, and an undo and a mod's move wait for an extraction opening its rows (#890, #893)
- [x] [R21 Extraction and bulk jobs](#r21-extraction-and-bulk-jobs) — closed: the CLI's jobs, the shared job model `or-job` and extraction from the app's archive menu exist (#156); extraction from an open archive answers the row, and install-wide jobs stay in `openrage-cli bulk` (owner, 2026-10-01)
- [x] [R22 Keys](#r22-keys)
- [x] [R23 Names and hashes](#r23-names-and-hashes)
- [x] [R24 Search](#r24-search)
- [x] [R24b Go to file](#r24b-go-to-file) — `Ctrl+P` is VS Code's Quick Open over a file index of the install and of every folder and archive open in Files, nested archives included, walked off the UI thread: a name fuzzily or a hash finds a file, `Enter` opens it through R24c's walk and `Shift+Enter` reveals it in Files, a scope narrows to the base game or the mods overlay, and a `>` turns the box into the command palette; "Search every archive" reads the same index, so two expanded archives' searches both arrive (#487)
- [x] [R24c Search results that open](#r24c-search-results-that-open) — a row of the Search panel or of a search inside archives opens on a double click and offers Open, Reveal in tree and Extract…, each opening the archives on the hit's path first; a result list takes Ctrl and Shift clicks, and Extract… writes every selected result through the extraction job, its clash question included (#479)
- [x] [R25 The edit mode gate](#r25-the-edit-mode-gate)
- [ ] [R25b Explorer ergonomics](#r25b-explorer-ergonomics) — every clause of the What is in the tree (#181, #190, #198); open: the Done-when, which is a person trying it, and the shell halves no test reaches
- [x] [R25c A file no reader opens](#r25c-a-file-no-reader-opens) — a file no reader names opens read-only as its bytes, or as editable text when its bytes are text (#438, #445)
- [ ] [R25d The Files view in the centre](#r25d-the-files-view-in-the-centre) — every clause but Open with and Replace is in the tree (#489); open: the Done-when, a person's judgement
- [x] [R25e Destructive file operations ask and can be recovered](#r25e-destructive-file-operations-ask-and-can-be-recovered) — a delete asks first, naming how many files and how much, and sends a row on disk to the Recycle Bin; rename, move and import refuse a name already taken before they touch the disk; an extraction into a folder that holds some of its files asks Replace, Skip or Keep both once per job (#456)
- [x] [R25f Copy cut and paste](#r25f-copy-cut-and-paste) — Edit's Cut, Copy, Paste and Delete act on the focused panel's rows or ticked records, and a row's menu offers the first three; files copy and cut between archives and folders and to and from Explorer, a nested archive whole, through the edit gate and the refusal of a taken name; a grid row's field, a vector or a colour, pastes over a field of its parts; metadata records paste after the selection in another file; each paste undoes as one step (#491)
- [x] [R25g The edit mode visible and honest](#r25g-the-edit-mode-visible-and-honest) — the menu bar's mode is a menu of the three modes with a sentence each; any change of mode takes back a base-game Yes; Settings' Editing page holds the mode, the overlay, the archive encryption and the export fixes; a refused write offers the modes; Show in mods overlay selects an archive's overlay copy in the tree (#470)
- [x] [R25h Every add lands where the tree shows it](#r25h-every-add-lands-where-the-tree-shows-it) — New file, New folder and Copy into archive write into a folder on disk at once, refusing a name the disk already holds, and add inside the archive a row shows, on disk or nested, opened first and unsaved until a save writes it; a rebuilt archive keeps an empty folder (#472)
- [x] [R25i A right-click or a drag selects its row](#r25i-a-right-click-or-a-drag-selects-its-row) — a right click or the start of a drag on a row of Files or the Files view selects it alone when it is outside the selection and keeps the selection when it is inside; a drag or a drop from Explorer gives its panel focus, so F2, Delete, the clipboard keys and Undo act on that row (#520)
- [x] [R25j A drag carries every selected row](#r25j-a-drag-carries-every-selected-row) — a drag of a selected row onto a folder, in Files or the Files view, moves every row selected, asking the edit gate once and undoing as one step; a drop into a folder inside a row it carries is refused before the gate asks, and a row refused alone, by a name taken or a job reading it, is named while the rest move; the start of a drag cancels the rename a slow click scheduled (#525)
- [x] [R25k A drag copies, and a change of nothing does nothing](#r25k-a-drag-copies-and-a-change-of-nothing-does-nothing) — a drop onto a folder with Ctrl held copies the rows the drag carries, asking the edit gate once and, into an archive, undoing as one step; a selection holding the folder being viewed carries its other rows; a drop or a rename that changes nothing writes nothing, asks the gate nothing and records no step (#570)
- [x] [R25l What a drag copy still gets wrong after R25k](#r25l-what-a-drag-copy-still-gets-wrong-after-r25k) — a cut pasted in place stays held, a Shift click spans from the last row, a drag copy says Copied and is one step, a Ctrl drag shows the copy cursor, a drag out carries a tree-made selection's rows and a copy's staged files go with its answer (#887, #888, #889)
- [x] [R26 Index dependencies and summaries](#r26-index-dependencies-and-summaries) — `EntryFacts`, the dependency resolver and its body reads exist (`or-depends`, `openrage-cli depends`), measured over both installs, with collision through `physicsDictionary` and `.ybn`-only assets; `openrage-cli depends export` copies an asset's whole set with a manifest or refuses it by name; a `.ymap`'s static bounds resolve where `_manifest.ymf` names them (`openrage-cli depends of <map>.ymap`) and, for a map no imap group names, by filename stem as a labelled convention checked against the map's streaming box, and an interior archetype's bounds through its `Interiors`; beyond the Done-when: an app surface
- [x] [R26b Uses and used by in the app](#r26b-uses-and-used-by-in-the-app) — a model's or a map's row offers Uses, what `openrage-cli depends of` says its file uses, and a model's Placed by, the maps placing it; each item opens what it names or reveals it in Files, one the install lacks shows disabled with why, and the install is read once, as a job, when a menu first asks (#513)
- [x] [R26c Uses reaches every pack](#r26c-uses-reaches-every-pack) — an item of Uses or Placed by opens a file a DLC's sub-pack holds, because a sub-pack's files now say which archive holds them, and a map's Uses names each convention bound beside whether it sits inside, outside or not cross-checked against the map's streaming box, as `depends of` does (#530)
- [x] [R26d Uses over a retail install](#r26d-uses-over-a-retail-install) — a DLC's sub-pack answers to the pack's CRC device as to its plain one, a search hit inside a sub-pack nested in the title update opens, and a map's first Uses reads the front of each bound's file and not the whole, with the same box as the whole file for every `.ybn` of both retail installs (#561)
- [ ] [R26e What Uses over a retail install still gets wrong after R26d](#r26e-what-uses-over-a-retail-install-still-gets-wrong-after-r26d)

### Phase 4 — Generic create, edit, delete

R27 to R31 are done. The UX review of 2026-10-03 adds three rows on how editing feels: viewing a
file never edits it (R27b), every edit is undoable (R27c), and the inspector leads with a summary
in a modder's terms (R31b).
Closing them added R27d to R27t, R31c and R31d: an edit stays in the document it was made in
through every later save, a field that shapes the data or retypes its owner is not edited in
place, the viewport and every other view draw every edit, the grid and undo survive an edit of
the contents, the Animation preview draws what its sources hold, Undo from Files acts on what
Files shows and outlives a change on disk, an undo finishes or says why, an undo then reaches every tree it touched and asks as a write
does, the Animation preview follows every source it draws and can hide the skeleton file's own model, a bone's numbers are fields of the inspector, a pinned inspector shows, edits and undoes its own document, a skeleton is seen without an
animation, and what the Animation preview and a bone edit still leave is R27r's and R27s's.

- [x] [R27 Editing on the value tree](#r27-editing-on-the-value-tree)
- [x] [R27b Viewing never edits](#r27b-viewing-never-edits) — opening a file of every kind and drawing every panel changes no document: a panel writes a number only when the user changes it, and a value the file leaves out is shown as unset and added only by the user (#460)
- [x] [R27c Undo for every edit](#r27c-undo-for-every-edit) — every edit is a step of its own document's history, which the History panel lists and a click moves through; a save marks where the file stands, and undoing past it marks the document unsaved (#490)
- [x] [R27d An edit stays where it was made](#r27d-an-edit-stays-where-it-was-made) — an edit of a document's image is read back into its contents once its gesture ends, or sooner where a save or another edit needs them, so every later save writes a grid edit or paste, and the grid writes into the document its rows were read from, a pinned one included (#496)
- [x] [R27e A field that shapes the data is not edited in place](#r27e-a-field-that-shapes-the-data-is-not-edited-in-place) — a field whose value counts, sizes or lays out data elsewhere in the file (a texture's size, format and mip levels, a vertex declaration, an index size, an animation block's packing, the counts of runs elsewhere) draws read-only in the grid, its hover naming where that data changes; the Texture workspace's re-encode still changes a texture's format and size (#497)
- [x] [R27f The viewport draws every edit](#r27f-the-viewport-draws-every-edit) — the viewport's meshes, its wireframe and a document's thumbnail are built again whenever the document's contents change: each edit before a save, an undo or redo, a grid drag once it ends rather than on every frame of it, and a reload from disk (#501)
- [x] [R27g A field that retypes its owner is not edited in place](#r27g-a-field-that-retypes-its-owner-is-not-edited-in-place) — a texture's usage, Legacy's classification byte and Enhanced's usage word, and a collision primitive's type draw read-only in the grid with a reason, and a polygon slot holding a sphere, capsule, box or cylinder draws as that primitive (#501)
- [x] [R27h The grid and undo survive an edit of the contents](#r27h-the-grid-and-undo-survive-an-edit-of-the-contents) — an edit of the contents keeps the image a grid step before it was made on and puts it back when undone, a few images a document at most, and where an edit took the image the grid asks for one, which the next idle frame builds from the contents as a save writes them (#506)
- [x] [R27i Every view follows every edit](#r27i-every-view-follows-every-edit) — the viewport and the thumbnails upload a texture again once its pixels change, by a re-encode, an import or the undo of either, and no other texture; the Animation workspace builds its meshes again when any document its preview draws is edited, whichever document is active (#505)
- [x] [R27j The Animation preview draws what its sources hold](#r27j-the-animation-preview-draws-what-its-sources-hold) — the preview poses the skeleton source's bones again once an edit of them is read back, and each mesh source's meshes bind the textures their own drawable embeds before any dictionary's, whichever document is active; the plane a ped's `.yft` draws alone is that file's own geometry, not a fault (#527) (#547)
- [x] [R27k Undo from Files acts on what Files shows](#r27k-undo-from-files-acts-on-what-files-shows) — with Files focused, Undo, Redo and the History panel act on the archive or folder it shows, the only one it lists or else the one its selection is in, and never on the model active beside it; a change Files makes on disk at once is no step, so Undo does not take it back, but for a cut's delete, which undoes with its paste since R27m (#512)
- [x] [R27l A change on disk keeps the undo steps beside it](#r27l-a-change-on-disk-keeps-the-undo-steps-beside-it) — a rename, move or delete on disk leaves the install's or folder's history as it was, and a step still undoes in an archive renamed or moved on disk; only a delete of an opened archive, or a rename or move of one while a step holds rows taken out of an archive, empties it (#519)
- [x] [R27m An undo finishes or says why](#r27m-an-undo-finishes-or-says-why) — an undo or a redo that cannot finish every part of its step changes nothing and says why, in a toast and in the History panel; in a tree it alone leaves the history, the steps before it still undo and the saved mark stays true; undoing a cut and paste writes the cut file back where it was from its copy (#526)
- [x] [R27n What an undo still gets wrong after R27m](#r27n-what-an-undo-still-gets-wrong-after-r27m) — a paste of a row cut out of an archive onto disk is undone whole, a file cut from one tree into another's archive is listed again by the tree it left, an undo the edit gate would ask about asks, a part of a step that says false has changed nothing, and the Recycle Bin's stand-in takes two folders of one name
- [x] [R27o The Animation preview follows its sources fully](#r27o-the-animation-preview-follows-its-sources-fully) — a clip chosen while the preview is paused is posed; a mesh source's textures are ranked and pinned for it and not for the active clip dictionary; the level-of-detail switch lists the levels of the sources the preview draws; a closed mesh source leaves the list; a Looped edit made while a clip is skipped as a mismatch wins when it is allowed again. Editing a bone is [R27p](#r27p-a-bones-numbers-can-be-edited), which the owner asked for on 2026-10-06. A ped's grey plane stays drawn, and R27q offers a box that hides it (#559)
- [x] [R27p A bone's numbers can be edited](#r27p-a-bones-numbers-can-be-edited) — a bone's row in the Outline, or a bone picked in the viewport, shows its name, position, rotation in degrees (XYZ order) and scale in the Inspector as fields; an edit derives the default and inverse transforms of the bone and of every bone under it again, keeps every bone's id, the bone-id table, the order and the parents, redraws the preview, is one undo step and saves; adding or removing a bone, re-parenting and skin weights stay with glTF and a 3D tool (#574)
- [x] [R27q The skeleton file's own model can be hidden](#r27q-the-skeleton-files-own-model-can-be-hidden) — the Animation preview still draws a skeleton source's own model by default, and a box in Preview settings, "Draw the skeleton file's own model", hides it without hiding the skeleton or any mesh source; a restart keeps the choice (#566)
- [x] [R27r What the Animation preview still leaves after R27q](#r27r-what-the-animation-preview-still-leaves-after-r27q) — the texture box ticks only for a dictionary every drawn source pins, closing a texture dictionary builds the clip session no more, the level switch lists the drawn sources' levels with no document active, the mesh section and the corner box say nothing a skeleton that is not there or is chosen contradicts, and the box's test checks that a click saves to its own folder
- [x] [R27s What a bone edit still leaves after R27p](#r27s-what-a-bone-edit-still-leaves-after-r27p) — every Done-when holds: a bone of a drawable dictionary of each target edits, all three signatures of 985 staged skeletons are recomputed, each refusal test was seen failing with its check out, drawing a bone edits nothing, a tiny position reads; a skeleton step is counted whole and a `.ypt` model's bones edit too (#872, #873, #878, #881, #882); whether a fragment's physics children follow a moved bone is the owner's design question
- [x] [R27t A skeleton is seen without an animation](#r27t-a-skeleton-is-seen-without-an-animation) — the viewport has a Bones toggle that draws the skeleton of the drawable it draws at rest, the bone selected in the Outline highlighted and a click on a bone's line selecting its row; the Animation preview draws the skeleton chosen at rest before a clip is chosen or when it does not bind, and Show bones is on by default (#583)
- [x] [R28 Authoring and write-back targets](#r28-authoring-and-write-back-targets) — the authoring half, the export rule (owner, 2026-10-01) and the destination chooser are done: saving a document opened from an archive asks whether it goes back into the archive, to a loose file or into the mods overlay's copy, and writes only there, behind the edit gate
- [x] [R29 Kernel manners diagnostics probing and provenance](#r29-kernel-manners-diagnostics-probing-and-provenance)
- [x] [R30 Presets references and cross-record edits](#r30-presets-references-and-cross-record-edits) — a room-ID, entity-index, archetype-name or entity-set field, in an array or not, offers the names `or-world` reads from its document or, for a placed interior, from the install; a scenario-type, model-set or procedural-tag field offers the names the install's own tables declare; and a preset saved from one metadata record applies to a selection of records in one undoable edit
- [x] [R31 The structure analyser and the entry-metadata pairing](#r31-the-structure-analyser-and-the-entry-metadata-pairing)
- [x] [R31b The inspector leads with a summary](#r31b-the-inspector-leads-with-a-summary) — every kind's inspector opens on a summary in words and units, then the declared structure under the engine's names, where the fields an owned value keeps to itself wait behind "Show internals"; an enum field is a choice, a number is edited from what the file holds, a hash reads `0x` and eight digits, and a field written nowhere here says why (#493)
- [x] [R31c A pinned inspector shows its own document](#r31c-a-pinned-inspector-shows-its-own-document) — every kind's inspector body draws and edits the document the inspector is pinned to while another is active: the summary and grid at a file's root, a drawable's materials, a particle file's rules and the read-only bodies alike (#503)
- [x] [R31d Undo and the sliders follow the pinned inspector](#r31d-undo-and-the-sliders-follow-the-pinned-inspector) — Undo, Redo and the History panel act on the document the focused panel shows, a pinned inspector's included, and History opened or clicked from that inspector keeps its document; the Live Evolutions sliders are named for the effect the preview plays; a clip dictionary's inspector shows its declared structure under its summary (#510)

### Phase 5 — Formats OpenRage has never scoped

Read `openrage-cli formats` before picking a row up: per extension, an install's file count,
container, metadata root type and which of `or-io`'s readers parsed each sample. `.ynd`
and `.ynv` open as owned values in `or-io` and reach the app as document kinds of their own with
an outline, and the viewport draws a `.ynv`'s polygons and leaf sectors (R33a, R33b); the outline's
polygon menu disables a polygon or deletes one, refused with what still reaches it, as
`openrage-cli navmesh` does (R33e). The app also
opens `.awc`, `.rel` and the DLC `.datNN` files, `.bik`, `.gfx`, `.mrf`, `heightmap*.dat`,
`distantlights*.dat`, `waterheight.dat`, `worldblanket.dat` and `navprecalc.dat` as kinds of their
own, and the `*_cache_y.dat` cache files, `metadata.dat` and `audioworldsectorsinfo.dat` as one
(R32a, R32b, R32c, R34, R35b, R35c), and a `.ysc` as decompiled source in the text workspace (R63).
An `.awc` stream in any codec a shipped install uses, XMA included, plays on every platform and
exports to WAV, and is replaced from a WAV in the codec it has, PCM16, ADPCM or MP3 (R34a). A
`.bik` plays with its sound: Bink 1 and Bink 2 are decoded in OpenRage's own Rust, on every
platform (R34c). R37 is done:
the canonical form and `synth`'s placement landed in #176 and #185, a clean clone's
`hash_XXXXXXXX` spelling in #397, metadata XML no schema declares and the binary `.srl` in #439,
and its schema-mapping framework is declined (owner, 2026-10-02). Open: R33b's view, R33e's load
in the game, R34a's replacing an XMA stream and an MP3 channel, R34b's in-game
edit, R38's Blender and FBX writing, R62's signature word and game load, R63's natives database and R67's
corpus. R38b, from the UX review of
2026-10-03, puts R38's glTF export and import in the app.

- [x] [R32 World data ymap ytyp and interiors](#r32-world-data-ymap-ytyp-and-interiors) — closed on its Done-when: `or-world`'s domain layer, extents, refusals, cross-file LOD, manifest derivation, partitioning, extensions, archetype generation and XML (#176, #185), world-state groups, prefabs, and create and delete with their `_manifest.ymf` entries; whether the game loads the result is [R62](#r62-the-rbm-layout-synthesiser)'s open clause, and Menyoo import is [R67](#r67-menyoo-spooner-xml-import)
  - [x] [R32a Terrain height map and distant lights](#r32a-terrain-height-map-and-distant-lights) — `heightmap*.dat` and `distantlights*.dat` read and save byte-identical in `or-io` for every file both installs ship; `openrage-cli convert info` reports them, and the app opens them as their own kinds, the height map as an image with its grid extents and the distant lights as a list of groups
  - [x] [R32b Water height map, world blanket and nav precalc](#r32b-water-height-map-world-blanket-and-nav-precalc) — `waterheight.dat`, `worldblanket.dat` and `navprecalc.dat` read and save byte-identical in `or-io` for the one file of each both installs ship; `openrage-cli convert info` and the census report them, and the app opens each as its own kind with a picture in the texture workspace; the world blanket's cells and the nav precalc's entries keep their stated values, since nothing measured names them
  - [x] [R32c Cache files, metadata.dat and the audio sector grid](#r32c-cache-files-metadatadat-and-the-audio-sector-grid) — `gta5_cache_y.dat` and every DLC's `*_cache_y.dat`, `metadata.dat` and `audioworldsectorsinfo.dat` read and save byte-identical in `or-io` for every file both installs ship; `openrage-cli convert info` and the census name each reader, and the app opens them as one kind, `DocumentKind::DatTable`, with their sections and records as tables; `metadata.dat`'s body is kept as stated bytes, its layout unmeasured
- [ ] [R33 Paths navmesh scenarios and tracks](#r33-paths-navmesh-scenarios-and-tracks) — decomposed into R33a–R33e
  - [x] [R33a Path nodes ynd](#r33a-path-nodes-ynd) — declared in `or-io` and opened as an owned value (#208, #220); the app opens it as `DocumentKind::Ynd` with an outline of its nodes, links and junctions and a save (#351), which the owner ruled is "the editor opens it" (2026-10-01)
  - [ ] [R33b Navmesh ynv](#r33b-navmesh-ynv) — declared in `or-io` with a `packed(4)` quadtree record (#215) and opened as an owned `CNavMesh` (#287); the app opens it as `DocumentKind::Ynv` with an outline of its sectors and polygons, and the viewport draws its polygons filled and outlined and its leaf sectors' boxes; open: the owner's judgement of that view (2026-10-01)
  - [x] [R33c Recordings yvr and ywr](#r33c-recordings-yvr-and-ywr) — declared in `or-io` as `CVehicleRecording` and `FwWaypointRecordingRoute` and opened owned; every file both installs ship reads, saves and reaches its fixed point; the app opens both and `openrage-cli convert` reads them; the corpus stages a sample of each from both installs, and its gates run on a corpus run and pass
  - [x] [R33d Scenario points](#r33d-scenario-points) — `CScenarioPointRegion` is declared by hand in `or_io::meta` over the document's own type table (OD-META-TYPED, 2026-10-01); `or_world::scenario` names every point's type and model set against the install's manifests, and an edited region saves to its own layout; creating a region waits on R62
  - [x] [R33e Navmesh index stability](#r33e-navmesh-index-stability) — `CNavMesh::disable_polygon` cuts a polygon off by writing its edges, and the edges into it, as leading nowhere, since no shipped bit marks a disabled polygon, and `delete_polygon` removes one and renumbers what names the later ones, or refuses naming the edges, special links and cells that still reach it; `openrage-cli navmesh disable` and `delete` and the `.ynv` outline's polygon menu do both; whether the game skips a disabled polygon is the owner's to load
- [ ] [R34 Audio and media](#r34-audio-and-media) — decomposed into a codec, a schema family and a licensing question
  - [x] [R34a The audio wave container](#r34a-the-audio-wave-container) — `.awc` reads, writes back byte-identically and decodes PCM16, ADPCM and Enhanced's MP3, in `or-io` behind its `awc` feature, and de-interleaves multi-channel containers, checked by continuity and a hand-built container since none states a channel peak, and the app opens one, lists its streams, plays them on every platform, exports them to WAV and replaces a stream from WAV in the codec it has, PCM16, ADPCM or MP3, rebuilding a multi-channel stream's shared blocks, as `openrage-cli awc` does; the app and `openrage-cli awc export` decode every stream through `or-media`'s one interface and codec table; Legacy's eight codec byte 5 streams are XMA, identified by their packets and decoded by `or-media`'s port of FFmpeg's WMA Pro decoder to FFmpeg's own samples and to the peaks they state, so the app plays and exports them, while replacing an XMA stream, which no open encoder exists for, and replacing an MP3 channel of a multi-channel stream are open
  - [ ] [R34b The audio metadata schemas](#r34b-the-audio-metadata-schemas) — `.rel` and the `.datNNN` family; the container reads and saves byte-identically for every file both installs ship, in `or-io` behind its `rel` feature, and every item of every `relType` reads typed and writes back its bytes; `openrage-cli awc which` answers which container holds a sound, by container and item class, and 76,182 of 77,435 Legacy and 77,906 of 79,188 Enhanced wave references resolve; a DLC's `.datNN` is the same container as the `.datNN.rel` beside it, and the app opens both as `DocumentKind::Rel`, items typed and grouped by type, read-only, saving unchanged; an in-game edit is open
  - [x] [R34c Video preview](#r34c-video-preview) — the app opens a `.bik` as `DocumentKind::Bik` and plays it, picture and sound, with play, pause and seek through `or-media`'s interface on every platform: Bink 1 and Bink audio are decoded by `or-media`'s own port of FFmpeg's decoders, equal to FFmpeg 7.0 on every Bink 1 movie both installs ship, and Bink 2 by its own port of NihAV's decoder, equal to the install's `bink2w64.dll` plane for plane in all 175,065 frames both installs ship, its in-loop filter derived from that DLL; no external program runs and no library loads
- [x] [R35 The remaining formats](#r35-the-remaining-formats) — `.yed`, `.yfd`, `.ypdb`, `.fxc` and `.cut` are closed; `.ybd` reads version 43 and refuses 40 and 42 by naming the geometry fields their bytes leave unplaced; `.ysc` moved to [R61](#r61-script-containers)
- [x] [R35b Scaleform movies](#r35b-scaleform-movies) — `.gfx` is ordinary code in `or-io` behind its `gfx` feature: every movie both installs ship (1,160 Legacy, 1,163 Enhanced) lists its tags and saves byte-identical, also by way of the standard SWF it exports to; `openrage-cli gfx info|export|import` and the app's inspector list the tags, the textures and the imports, export to SWF and import one back, the census names its reader, the self-check draws it, and an LZMA SWF is refused by name
- [x] [R35c Move network definitions](#r35c-move-network-definitions) — `.mrf` is ordinary code in `or-io` (`or_io::mrf`): every network both installs ship (162 Legacy, 160 Enhanced, among them the owner's `taskexitvehicle.mrf`) reads with every state, transition target and initial node resolved and saves byte-identical; `openrage-cli convert info` lists node counts by kind and the state machines with their states, the census names its reader, and the app's inspector shows the network as a tree of state machines, states and transitions with their targets and conditions, and the nodes by kind; editing is open
- [x] [R36 Console and platform variants](#r36-console-and-platform-variants) — closed as declined, on the measurement it asked for
- [x] [R37 Metadata and XML at parity](#r37-metadata-and-xml-at-parity) — the published root-type list, the withholding rule, the app editor, the validator UX and the XML canonical form (#176, #185) landed over `or_io::meta`, and with no snapshot every name the build cannot resolve is spelled `hash_XXXXXXXX` and every corpus document converges on its own type table (owner, 2026-10-01); the schema-mapping framework is declined (owner, 2026-10-02)
- [ ] [R38 Mesh interchange](#r38-mesh-interchange) — glTF export and import of both targets, each written as itself, landed over the owned drawable and behind the CLI, and every owned save refuses an incoherent drawable, rewriting a count a file stated against its run only when asked and only losslessly; an FBX is read, through the same import and into a model source an edit recooks (`or-source`, [Source_Format_Design.md](Source_Format_Design.md)), and never written; from-scratch authoring is open, and "opens in Blender" is unverified
- [x] [R38b glTF in and out from the app](#r38b-gltf-in-and-out-from-the-app) — a `.ydr`, `.ydd` or `.yft` exports to glTF from its row's or its tab's menu, and a glTF goes back over the open model as one step to undo or save, each showing the report `openrage-cli mesh export|import` gives before anything is written or changed (#509)
- [x] [R61 Script containers](#r61-script-containers) — split out of R35; the container reads every script over two installs as ordinary code (OD-YSC) and reports a native by hash, and the app opens a `.ysc` read-only as decompiled source in the text workspace, with the listing `openrage-cli script disasm` prints in the inspector, saying why natives are hashes; the declared form is owed once a corpus exists, and the useful half is [R63](#r63-the-script-disassembler-and-the-natives-database)
- [ ] [R62 The RBM layout synthesiser](#r62-the-rbm-layout-synthesiser) — the synthesiser, the invented seal and the fallback landed and lay out every corpus RBM document; open: the `signature` word no tool can derive, and whether a running game loads the result
- [x] [R63 The script disassembler and the natives database](#r63-the-script-disassembler-and-the-natives-database) — split out of R61; a `.ysc` decompiles to structured source in the text workspace and through `openrage-cli script decompile` (owner, 2026-10-03), with a goto in 0.005% of retail functions; the decompiler and the listing name and type natives from alloc8or's lists through CSY0N's crossmap, downloaded only on the user's request and never bundled (owner, 2026-10-03), naming 7,341 of Legacy's 7,402 called natives and 11,460 of Enhanced's 11,504
- [x] [R64 Move the containing readers onto pack offsets format by format](#r64-move-the-containing-readers-onto-pack-offsets-format-by-format) — closed: stopped for R65, which R66 superseded, and its formats declared by R66d; `or-asset` and the compiled-layout ceiling went at R66f (#332)
- [ ] [R67 Menyoo spooner XML import](#r67-menyoo-spooner-xml-import) — split out of R32; unstarted, and unscheduled until a corpus of spooner files exists (owner, 2026-10-01)

### Phase 6 — Assets: textures, materials, geometry, the viewer

R40 and R41 are programmes scheduled as their parts, in the orders R40a → R40b → (R40c ∥ R40d ∥ R40e
∥ R40f) and R41a → R41b → (R41c ∥ R41e) → R41d. Every part of both is done. R42b, from the UX
review of 2026-10-03, adds what inspecting an asset in the viewport needs: picking, a level-of-detail
switch, collision and bones you can see. R42c and R42d, found while closing R59h and R42c, draw an
Enhanced file in the viewer as a Legacy one; R42e takes the panels around the viewer the same way, R42f, found while closing R42b, picks
every part the viewport draws, R42g, found while closing R42e, has the controls an Enhanced
file cannot use say so, R42h, found while closing R42f, takes a pick to its row and to the lines
it still misses, R42i, found while closing R42g, takes the Enhanced controls that still do nothing,
R42j, found while closing R42h, takes what a pick still misses, and R42k, found while trying to land
R27t, takes what drawing bones outside the Animation workspace leaves.

- [x] [R39 Textures](#r39-textures)
- [x] [R40 Materials and shaders](#r40-materials-and-shaders)
  - [x] [R40a The shader definition database as data](#r40a-the-shader-definition-database-as-data)
  - [x] [R40b The regeneration pipeline](#r40b-the-regeneration-pipeline)
  - [x] [R40c The material editor](#r40c-the-material-editor)
  - [x] [R40d The Gen8 Gen9 parameter remap](#r40d-the-gen8-gen9-parameter-remap)
  - [x] [R40e The Enhanced shader container](#r40e-the-enhanced-shader-container) — both containers read, and a Gen9 database built from them that names 99.5% of the staged Gen9 materials' entries
  - [x] [R40f The compiled-shader registry and the ABI slot table](#r40f-the-compiled-shader-registry-and-the-abi-slot-table) — `RDEF` reflection, a general ABI slot table beside the particle one, and a per-stage limit a shader is refused at
- [x] [R41 Geometry physics cloth and lights as core data](#r41-geometry-physics-cloth-and-lights-as-core-data)
  - [x] [R41a Vertex and index data as an editable model](#r41a-vertex-and-index-data-as-an-editable-model)
  - [x] [R41b Bounding volumes and BVH](#r41b-bounding-volumes-and-bvh)
  - [x] [R41c Bounds and physics construction](#r41c-bounds-and-physics-construction)
  - [x] [R41d Cloth as a model](#r41d-cloth-as-a-model)
  - [x] [R41e Lights shattermaps and bone identity](#r41e-lights-shattermaps-and-bone-identity) — bones addressed by tag wherever the addressing crosses a file
- [x] [R42 The viewer](#r42-the-viewer) — the per-channel debug modes and auto-skeleton resolution landed; the real-game-shader mode is cut and re-opens under [R40f](#r40f-the-compiled-shader-registry-and-the-abi-slot-table)
- [x] [R42b Viewport basics for inspecting assets](#r42b-viewport-basics-for-inspecting-assets) — a click selects the geometry under the cursor, posed as drawn, and the Outline reveals it; `F` frames the selection and `Home` everything; bones draw in front of the mesh; a level-of-detail switch with each level's distance and a Collision toggle; Material by default; a dictionary opens on its first entry; the overlays leave the axis gizmo clear and fit a narrow viewport (#515); picking a bone, a bound and a model is [R42f](#r42f-the-viewport-picks-what-it-draws)'s
- [x] [R42c The viewport draws what it is given](#r42c-the-viewport-draws-what-it-is-given) — an entry selected in an Enhanced `.ydd` draws, frames and is named in the status bar as a Legacy one is, and every mode but Channel drops the channel, so Solid and Material draw none of its colours (#477)
- [x] [R42d Every viewer path takes both targets](#r42d-every-viewer-path-takes-both-targets) — Material mode finds the embedded textures of an Enhanced drawable, dictionary entry, fragment and particle model, the Animation workspace offers Enhanced meshes and skeletons, and an Enhanced `.ypt` lists its models in the gallery, the one selected drawing and framing; a test holds every viewer path to both targets (#480)
- [x] [R42e The panels around the viewer take both targets](#r42e-the-panels-around-the-viewer-take-both-targets) — an Enhanced `.ypt` has its Outline, status-bar summary, Inspector and timeline, and an Enhanced `.yft`'s and `.ypt`'s textures are listed, each as a Legacy one's are; the viewer scan reads these panels and excepts only the particle simulation and the texture editor (#531)
- [x] [R42f The viewport picks what it draws](#r42f-the-viewport-picks-what-it-draws) — a click picks a bone, a bound and, with Alt, a model as well as a geometry, and in the Animation workspace reaches every mesh the preview draws, in the document it comes from; the box there names what the preview draws and what was picked, and the pick leaves the clip selected; the Outline and Inspector there follow it only once a double click on the box's Picked line opens it at its row ([R42h](#r42h-what-a-pick-still-misses-after-r42f)) (#549)
- [x] [R42g Enhanced controls that cannot work say so](#r42g-enhanced-controls-that-cannot-work-say-so) — an Enhanced `.ypt`'s Play, Rate, Loop and Drivers are greyed with the reason that nothing simulates it, and Replace from image…, Embed and Unembed over an Enhanced texture say the texture editor writes only the Legacy layout; simulating an Enhanced effect and editing its textures stay open (#560)
- [x] [R42h What a pick still misses after R42f](#r42h-what-a-pick-still-misses-after-r42f) — a click in the Animation preview keeps you on the clip list and a double click on the box's Picked line opens the pick at its row (owner, 2026-10-05); the bind-pose ghost and, in Wireframe, a fragment's physics bound and a `.ybn`'s bounds are picked, and so is a bone while no chosen mesh draws; `click X Y alt` scripts the Alt click that picks a model (#569)
- [x] [R42i Enhanced controls that still do nothing after R42g](#r42i-enhanced-controls-that-still-do-nothing-after-r42g) — an Enhanced `.ypt`'s pin rail and the Shaded bar's Exposure are greyed with the reason that nothing simulates it, a Legacy `.ypt`'s texture controls no longer blame an edition, and the format, quality, mip and sRGB inputs go grey with Apply wherever Apply cannot write (#865, #866)
- [x] [R42j What a pick still misses after R42h](#r42j-what-a-pick-still-misses-after-r42h) — Enter opens the pick, a pick opens the Physics section, a fragment lists the one physics bound it draws, a click picks a `.ybn`'s node by mesh and a `.ynv`'s polygon, the Outline reveal and the Alt click of a script are held or shown; the lines with no row of their own say so (#870, #875, #884)
- [x] [R42k What drawing bones outside the Animation workspace leaves](#r42k-what-drawing-bones-outside-the-animation-workspace-leaves) — the bones draw outside the Animation workspace behind their own flag, the bone picked draws as picked, the skeleton is built once, the row fits a narrow viewport, a `.ypt` model's bones draw and pick, and the strict corpus run passes; Bones stays two flags (#871)

### Phase 7 — Plugins and open with

Two surfaces, settled on 2026-09-21 in [Project_Context.md](Project_Context.md): in the app a plugin
is WebAssembly (untrusted, capability-controlled, many at once); outside it, a subprocess speaking
the CLI's NDJSON protocol (any language, any GPU, no place in the UI). Every row is filed under its
surface. `crates/domain/or-plugin` holds R44a–R44d: the capability model, the runtime, the guest ABI
and the host API; it and the app's tiered registries stay for this phase by the owner's ruling of
2026-09-30 (OD-AUDIT-KEEPS). R44e embeds the host in the app, behind consent and shown refusals,
R44f hands a guest the open file's bytes and takes bytes back through the edit gate, and R45 lets a
plugin that stays loaded add panels, commands, settings and viewport overlays. Open are R43's
stability commitment and writing routes, R46 and R47.
[R46](#r46-first-plugin-a-clothing-tool-on-openrage)'s cloth tool is the acceptance test, a rewrite
as a WASM plugin that links `or-io` itself over R44f's bytes (owner, 2026-10-05), because a WASM
guest cannot take the app's Rust types across; what it will still ask for is another archive's
textures, which R45 does not yet buy.
[R45](#r45-the-plugin-sdk-and-registration-surface)'s SDK mirrors the host ABI R44c fixed.

- [ ] [R43 The external surface and open with](#r43-the-external-surface-and-open-with) — every documented route reads except four that change the clothing pack of a project the caller names; open: the stability commitment and the convert routes
- [x] [R44 The plugin host](#r44-the-plugin-host) — R44a to R44f done; the editor runs a plugin on 4 billion instructions (owner, 2026-10-05)
  - [x] [R44a The capability model](#r44a-the-capability-model)
  - [x] [R44b The WASM runtime](#r44b-the-wasm-runtime)
  - [x] [R44c The guest ABI](#r44c-the-guest-abi)
  - [x] [R44d The host API in the pack vocabulary](#r44d-the-host-api-in-the-pack-vocabulary)
  - [x] [R44e Consent and refusal in the app](#r44e-consent-and-refusal-in-the-app) — the app embeds the host, asks before a plugin's first run, keeps and revokes the answer, and shows every refusal
  - [x] [R44f Payload and GPU access](#r44f-payload-and-gpu-access) — a guest reads a file's bytes and writes bytes back through the edit gate, measured; `or-io` parses in WASM
- [x] [R45 The plugin SDK and registration surface](#r45-the-plugin-sdk-and-registration-surface) — a plugin that stays loaded registers panels drawn from the widgets its guest describes, commands, settings with a page each, and viewport overlays with picks, each behind its own capability and the consent of R44e, and a sample guest registers each, with two panels, because an event belongs to the panel it came from; the format-and-title-as-data clause is withdrawn by OD-OTHER-TITLES
- [ ] [R46 First plugin a clothing tool on OpenRage](#r46-first-plugin-a-clothing-tool-on-openrage)
- [ ] [R47 The plugin slate after the first](#r47-the-plugin-slate-after-the-first)

### Phase 8 — Projects, packaging and mod management

No `.oiv` reader or installer exists yet, and [R52](#r52-mod-manager-and-load-order)'s Mods panel
lists an install's mods and their load order without one. [R49](#r49-the-mods-overlay) is the
keystone, scheduled as R49a–R49d, and all four landed: R49a's overlay-first resolution, so a `mods`
copy of an archive is what the VFS reads, R49b's copy into the overlay from the archive menu and from
a refused write, R49c's naming of a copy a title update has since outgrown, and R49d's line in
Settings on whether the game reads the overlay at all. R48's project manifest and R51c's DLC writers
with `openrage-cli dlc create` are built, and R51c needs nothing from `.oiv`. The UX review of 2026-10-03 adds R49e, comparing a file with another or
with the game's original (and R49f, found while closing it, R49g, found while closing R49f, and R49h and R49i, found while
closing R49g),
and R51d, R51c's pack built from the app.

- [ ] [R48 Projects](#r48-projects) — the manifest, its lifecycle, its menu, portable projects and the shadow mount landed (#189), membership is a list apart from the open tabs, and a member can be removed; the container (content and builds) and a FiveM resource build of a project's files landed in `openrage-cli project`, and so did the contract content contributes to a plan through, its merge and collision rules, budgets, split and diff; the Project workspace lists its assets, problems and builds (counted one way on the card, in the panel and in Properties, with a quick look that draws a texture, a filter that clears, Locate and Remove for a missing member, each a step of the project's History that Undo and Redo move along and a click goes back to (kept beside the project, out of Git) and a new id for two sources that share one), builds on Ctrl+B and recooks a saved source, reading again every tab open on the file it wrote, with a Build tab that says what the build did in sentences (a header line, the step running, plan, what changed and verification) and a Build button whose dot comes with a word; the start page, New project (one size at every step, Escape cancels it, a clothing pack in clothing words, made on a worker) and a launch that reopens the last project on the tab it was saved on landed, Close project asks when the project changed, and so did links to a pack or a whole project another project holds (its Files content and each pack, and none of its links; a project of links alone builds), a Builds window that declares and changes a build (content by builds, ceilings, split, file types) with the same edits in `openrage-cli project set-build`, adopting a folder grouped by what is recognised in it, templates as files and the project file as a dropped file and a file association, a Save that writes nothing when the file reads the same, states the workspace only when the person says the project opens there and names no path of this machine, and a Close project that never waits for a build; wave P36 polished it ([the project polish](Workbench_UX_Design.md#what-the-project-polish-builds)): a rule said in its sentence and never by its code, no toast for a missing file, one next action on an empty project, a Verify option on the build, the owner's choice of what a repository holds, a member followed by its id when its folder moves, a sample project and a tour, Redo that makes an import again, a second window that reads a project another holds, a plugin installed from a folder and a plugin write that is a step of Undo, and every control of the project screens in [an inventory](Workbench_UX_Design.md#control-inventory) that a test holds; wave P31 read it again against the owner's words ([what the re-verification changed](Workbench_UX_Design.md#what-the-re-verification-changed)): the rules read a drawables source, an import ends with a toast that has Undo import, Open in offers every workspace, and `openrage-cli project` applies a template and checks before sharing and against the game; open: a project file rendering live wherever the game's copy is open
- [ ] [R49 The mods overlay](#r49-the-mods-overlay)
  - [x] [R49a Overlay-first resolution](#r49a-overlay-first-resolution) — two of its Done-when clauses are withdrawn rather than met, and both belong to other rows
  - [x] [R49b The overlay as the place you write](#r49b-the-overlay-as-the-place-you-write) — the copy from the archive menu and from a refused write, and Show in mods overlay since R25g; open: moving the open document onto the new copy
  - [x] [R49c Staleness after a title update](#r49c-staleness-after-a-title-update) — a copy made from the app is recorded as the install's archive stood, so a title update that changed that archive is named, marked in the tree and listed in the Mods overlay settings, and Refresh lists what of the user's it would discard before it writes; open: re-applying those changes onto the patched archive
  - [x] [R49d The game-side loader](#r49d-the-game-side-loader) — Settings ▸ Editing ▸ Mods overlay says, per edition, whether the plugin and ASI loader beside the executable make the game read the overlay, and names what to add with the projects' own pages; a write into an overlay the game would not read warns once; open: a run of the game against a verdict
  - [x] [R49e Compare a file with another or with the original](#r49e-compare-a-file-with-another-or-with-the-original) — Select for compare, Compare with selected and Compare with original on a file's row compare binary metadata by record and field, a texture dictionary by texture and text side by side, on a worker, in a tab of its own that steps between differences; any other resource and two archives are [R49f](#r49f-compare-every-resource-and-archive)'s (#516)
  - [x] [R49f Compare every resource and archive](#r49f-compare-every-resource-and-archive) — a resource compares by part and field from the rows the inspector reads, two archives by entry (added, removed, changed by content), and a metadata map by key, with Compare with original for an archive in the overlay, encrypted ones included; open: an archive inside another has no original of its own (#558)
  - [x] [R49g What a comparison still misses after R49f](#r49g-what-a-comparison-still-misses-after-r49f) — a changed resource in an archive reads field by field, a comparison reads 24 levels of parts and an outline no longer repeats a part under itself, a blob behind a pointer or an array with no element type, a scalar array and an opaque field are compared in full, and a compare command says why it is off while a defragment holds its archive; open: the physical-segment layout assumption, data no row or blob reaches, and the elements limit (#567)
  - [ ] [R49h What a comparison still reads wrongly after R49g](#r49h-what-a-comparison-still-reads-wrongly-after-r49g)
  - [ ] [R49i What the checks of a comparison still leave after R49g](#r49i-what-the-checks-of-a-comparison-still-leave-after-r49g)
- [ ] [R50 Package installation](#r50-package-installation)
  - [ ] [R50a The OIV container and its manifests](#r50a-the-oiv-container-and-its-manifests)
  - [ ] [R50b The patch languages](#r50b-the-patch-languages)
  - [ ] [R50c The command engine the journal and the install](#r50c-the-command-engine-the-journal-and-the-install)
- [ ] [R51 Package authoring uninstall and DLC creation](#r51-package-authoring-uninstall-and-dlc-creation)
  - [ ] [R51a Package authoring](#r51a-package-authoring) — a project build writes an `.oiv` that reads back through this tree's own reader (a manifest written from the 2.2 shape, an add per file, and a clothing pack installed below its `dlc.rpf`); open: OpenIV installing one, and everything the reader of R50a is for
  - [ ] [R51b Package uninstall](#r51b-package-uninstall)
  - [ ] [R51c DLC pack creation](#r51c-dlc-pack-creation) — built (#148) and needing nothing from `.oiv`, and a project build writes one from its plan, with the types its files state and a path through an archive's name packed as that archive, a clothing pack laid out as the game's own packs are, and the entries of each archive counted against the 65,535 an archive holds; open: a generated pack loading in game
  - [x] [R51d A DLC pack from the app](#r51d-a-dlc-pack-from-the-app) — Create DLC pack… on a folder on disk asks what the command asks, shows what goes in, what is refused and the `dlclist.xml` line before anything is written, and builds as a job through the command's own call, never into a game install (#508)
- [x] [R52 Mod manager and load order](#r52-mod-manager-and-load-order) — the Mods panel lists the active install's mods by kind and status in a load order kept per install, and two enabled mods writing one path show the mod the VFS resolves it to, which a reorder changes; open: writing that order into the overlay the game reads, packages (R50), the stale and unreadable statuses and the mod-loader kind, which no longer wait on R49c and R49d, a DLC pack under `update/x64/dlcpacks`, and toggling an ASI plugin or a script

### Phase 9 — Titles

R54's Enhanced-parity half, [R54a](#r54a-enhanced-parity), is done: every format `FORMATS` in
`crates/io/or-io/src/gta5/formats.rs` lists opens and saves as an owned value for each target it
ships. R54b's crates, `or-io` and `or-io-macros`, wait on R66h's publication. R54c and R54d are
closed as obsolete: pack formats were deleted on purpose and the `.orpack` export goes (owner,
2026-10-01), and nothing loads a runtime pack for any title (OD-OTHER-TITLES). R55 converts a drawable, a
texture dictionary, a drawable dictionary and a fragment both ways with a loss report, and a folder
of them unattended. [R56](#r56-rdr2-the-second-container)
needs RDR2 files, and the seams it will use stay by OD-AUDIT-KEEPS. R55's loss report and R56's
numbers are population questions: an install census answers them.
[R57b](#r57b-earlier-rage-titles) holds GTA IV, EFLC, Max Payne 3 and RDR1, future goals since the
owner's ruling of 2026-10-01, and nothing schedules it. R55b, from the UX review of 2026-10-03,
puts R55's conversion in the app, closing it added R55c, closing R55c added R55d, and closing R55d added R55e.

- [x] [R53 GTA V the reference title](#r53-gta-v-the-reference-title)
- [ ] [R54 GTA V Enhanced and the portable open reader writer](#r54-gta-v-enhanced-and-the-portable-open-reader-writer)
  - [x] [R54a Enhanced parity](#r54a-enhanced-parity) — every format `FORMATS` lists opens and saves owned for each target it ships; R66d replaced the pack rows; the app's mount layers `update/update2.rpf` as the game does
  - [ ] [R54b The crates as a published SDK](#r54b-the-crates-as-a-published-sdk) — the crates to publish are `or-io` and `or-io-macros` (D2, D8), scheduled by R66h
  - [x] [R54c The orpack format and its compatibility rule](#r54c-the-orpack-format-and-its-compatibility-rule) — closed: obsolete, because pack formats were deleted on purpose; the leftover `.orpack` export goes (owner, 2026-10-01)
  - [x] [R54d A pack authored from outside this project](#r54d-a-pack-authored-from-outside-this-project) — closed: obsolete by OD-OTHER-TITLES
- [x] [R55 Retargeting between Gen8 and Gen9](#r55-retargeting-between-gen8-and-gen9) — a `.ydr`, `.ytd`, `.ydd` and `.yft` convert both ways with a loss report (`or_io::gta5::retarget`), and a folder of them unattended with one report (`openrage-cli convert retarget`)
- [x] [R55b Convert from the app](#r55b-convert-from-the-app) — a `.ydr`, `.ytd`, `.ydd` or `.yft` row in Files or the Files view, or a selection of them, converts to the other edition from its menu, the palette or a key, with the command's loss report shown before anything is written (#504)
- [x] [R55c Converting in a modder's words](#r55c-converting-in-a-modders-words) — the conversion's question names each value it drops in words, its job and its dialogs say Convert, a folder holding some of the files asks Replace, Skip or Keep both, and a document's tab offers Convert (#528) (#533)
- [x] [R55d What converting still gets wrong after R55c](#r55d-what-converting-still-gets-wrong-after-r55c) — a glTF export's write and Export as…'s are listed and cancelled as an Export, the palette and its key convert the open document, and the conversion's question wraps a long name inside its panel and selects line by line (#573)
- [x] [R55e What converting still gets wrong after R55d](#r55e-what-converting-still-gets-wrong-after-r55d) — a write under another job's slot names it, every question fits a narrow window, Convert says to select in Files, and the jobs list is held to every kind (#898)
- [ ] [R56 RDR2 the second container](#r56-rdr2-the-second-container) — the archive reads and writes (an RPF8 table in the clear, 2026-10-07); RSC8 and RDR2's structures are not started, and the other archives' tables need the title's TFIT keys; `PagingModel`, `ResourceEnvelope` and every `GameProfile` method are kept for it (OD-AUDIT-KEEPS)
- [ ] [R57 GTA VI readiness](#r57-gta-vi-readiness) — a property row; the paging model and the resource envelope are held behind their seams, and the archive is a version and a layout
- [ ] [R57b Earlier RAGE titles](#r57b-earlier-rage-titles) — unscheduled: GTA IV, EFLC, Max Payne 3 and RDR1, future goals rather than non-goals (owner, 2026-10-01); their archives read and write in `or-io`, and nothing above them does

### Phase 10 — The application

Appearance and scale (R59b), the persisted dock layout (R59d), a camera per document (R59e) and
readable diagnostics (R60b) have landed, and R59f's statistics panel shows the frame time by the
frame loop's phases and the caches against budgets the preferences keep, with every GPU class the
renderer allocates, egui's textures and each open document's bytes beside them, an owned value at
its heap, the app's other long-lived CPU state and wgpu's objects; localisation (R59c), the sizes
R59f lists as not counted, and onboarding (R60a) remain. The app builds, tests and runs on Linux,
finds a Steam install there and names a missing runtime library, and it builds, passes its
tests and draws its self-check on macOS, and its tooling no longer needs PowerShell off Windows,
while nothing packages it off Windows and a dozen Windows-only features are stubs elsewhere; the
owner decided on 2026-10-06 that both platforms are supported in full
([R58](#r58-cross-platform)).

The UX review of 2026-10-03 adds R59g to R59q and R60c to R60e; closing R60d added R60f, closing
R51d added R60g, closing R60g added R60h, closing R60h added R60i, closing R60i added R60j, closing R59j added R59r, closing the browser rows added R59s, closing R31b added R59t, closing R59s and R59r
added R59u, closing R26b added R59v, closing R59v added R59w, closing R59w added R59x, closing R59x, R59t and R27j added R59y, and
closing R59t and R59u added R59z; the family has no letter after z, so closing R59q and R59t added R68,
whose parts R68a and R68b take the shell findings from there, and closing R59q, R59y, R59z, R68a,
R68b and the app's test runs added R68c to R68g, trying to land R27t added R68h, and closing R68h
added R68i and R68j.
R59g's screenshots landed (#453), so every later row here is checked on `just shots` screens; the
rest make the shell trustworthy (R60d, R59h, R59i, R59n, R60f, R60g, R60h, R60i, R60j), open it on the install (R60c, R59m,
R59o), make it navigable (R59j, R59k, R59l, R59r, R60e) and finish it (R59p, R59q, R59s, R59t, R59u, R59v, R59w, R59x, R59y, R59z, R68a, R68b, R68c, R68d, R68e, R68f, R68g, R68h, R68i, R68j). The owner approved the review's plan and its order
([Project_Context.md](Project_Context.md#the-ux-reviews-plan-is-the-apps-next-work-2026-10-03)).

The review of the app's navigation of 2026-11-21 adds R69, whose parts replace the workspace strip
with tabs and a rail in the order the owner chose
([Project_Context.md](Project_Context.md#the-window-becomes-tabs-and-a-rail-in-ten-rows-2026-11-21)).

- [ ] [R58 Cross platform](#r58-cross-platform) — Linux and macOS supported in full (owner, 2026-10-06), in seven parts; the app builds, tests and runs on Linux and builds, tests and draws its self-check on macOS
  - [ ] [R58a One Rust implementation of the dev tooling](#r58a-one-rust-implementation-of-the-dev-tooling)
  - [ ] [R58b CI on macOS and Linux](#r58b-ci-on-macos-and-linux)
  - [ ] [R58c Correctness off Windows](#r58c-correctness-off-windows)
  - [ ] [R58d Install discovery off Windows](#r58d-install-discovery-off-windows)
  - [ ] [R58e Windows-only features elsewhere](#r58e-windows-only-features-elsewhere)
  - [ ] [R58f Shader baking off Windows](#r58f-shader-baking-off-windows)
  - [ ] [R58g Packages for Linux and macOS](#r58g-packages-for-linux-and-macos)
- [ ] [R59 Presentation input and workspaces](#r59-presentation-input-and-workspaces)
  - [x] [R59a The egui 0.34 deprecations](#r59a-the-egui-034-deprecations) — the one row here with a deadline it did not choose
  - [x] [R59b Appearance and scale](#r59b-appearance-and-scale) — landed in #182; how it looks is unchecked by a human
  - [ ] [R59c Localisation and the translation pipeline](#r59c-localisation-and-the-translation-pipeline) — doing it out of phase order was weighed against a measured string count and declined; `no_user_facing_literal_names_gen8_or_gen9` holds the app and `openrage-cli` to Legacy and Enhanced
  - [x] [R59d The layout that survives a restart](#r59d-the-layout-that-survives-a-restart)
  - [x] [R59e Workspaces that hold their own scene](#r59e-workspaces-that-hold-their-own-scene) — a camera per document, and a workspace stays a layout preset; the camera is session-only by decision
  - [ ] [R59f Performance and cache tuning](#r59f-performance-and-cache-tuning) — a statistics panel shows the frame time broken into the frame loop's phases, the texture cache and the viewport's GPU textures against budgets the preferences keep, and beside them every other GPU class the renderer allocates, egui's textures and each open document's bytes, an owned value at what it allocates (`Heap`), the app's other long-lived CPU state on a line each (the name index, the file index, search results, the archive trees and their tables of contents, the undo histories, the simulation, loaded HDRIs, what Uses and Placed by read) and wgpu's own objects by count; open: the sizes `or-io`, `or-cli` and `or-depends` keep private (the shader parameter remap, the indexes behind Uses and Placed by) and wgpu's sizes for its pipelines, bind groups, samplers and shader modules, which it does not expose
  - [x] [R59g Screens an agent can see](#r59g-screens-an-agent-can-see) — `just shots` draws the real shell from a script of steps and saves PNGs (#453)
  - [x] [R59h No command that does nothing](#r59h-no-command-that-does-nothing) — Maximize shows the focused panel alone until it runs again, Frame selection frames the selected model or geometry, Channel shading draws meshes, an archive entry's menu says Open, and Edit hid Cut, Copy, Paste and Delete until R25f brought them back working (#455, #461)
  - [x] [R59i Shortcuts by character live while typing](#r59i-shortcuts-by-character-live-while-typing) — a shortcut matches the character its key types, Ctrl and Alt combinations reach the app while a field has focus unless the field edits with them, the timeline's and the viewport's keys act only under the pointer, and `Ctrl+Y` redoes (#459)
  - [x] [R59j One command registry](#r59j-one-command-registry) — the menu bar, the palette, shortcuts, Settings ▸ Keyboard shortcuts and the plugins read one table of commands; the palette ranks fuzzy and recent-first, takes the arrow keys and shows each binding, and every hint names the key as bound; the context menus followed in R59r (#481)
  - [x] [R59k The shell's regions](#r59k-the-shells-regions) — Files and an Outline that follows the active tab on the left, the Inspector on the right in every workspace, every panel's title shown and the focused one marked, reveals that land from inside a panel, and moving through Files never switches the workspace (#483)
  - [x] [R59l Tabs that behave like an editor's](#r59l-tabs-that-behave-like-an-editors) — every document has a tab with a close button and one unsaved mark, an archive entry's names its archive, a file opened twice has one tab, a single click in Files opens a preview tab, and `Ctrl+Tab`, `Ctrl+Shift+T` and a list of the tabs that do not fit (#485)
  - [x] [R59m Every kind has a home](#r59m-every-kind-has-a-home) — Browse (the default), Model, Texture, Animation, Particle and Data; only a `.ypt` opens into Particle, the Document panel draws video, audio and every kind with no editor of its own in the centre, the gallery sits beside the preview and the viewport, and the workspace on screen is saved (#469)
  - [x] [R59n Glyphs and feedback by severity](#r59n-glyphs-and-feedback-by-severity) — every symbol the shell writes is in its fonts, a toast takes its level's colour, reads at 4.5:1 in both themes and stacks on its measured height, and the status bar counts only OpenRage's own warnings and errors (#463)
  - [x] [R59o Words empty states and the status bar](#r59o-words-empty-states-and-the-status-bar) — one name per action, archives called archives, no RSC7, PSC, TOC or WGSL in what the app writes, the New menu in words, one ellipsis and one separator, error windows that name what failed, OpenRage by its own name, every workspace's empty state offering actions, and the frame rate left to the Statistics panel (#478)
  - [x] [R59p Settings recents and small editor fixes](#r59p-settings-recents-and-small-editor-fixes) — the settings search finds every option and opening Settings again brings it forward; the viewport keeps its mode, background and exposure; recents say when a file was opened, skip what is gone and hold archive entries; Save As proposes the file's own name; the text editor scrolls sideways and goes to a line; the texture view keeps 100% and each texture's encode options and warns before a lossy re-encode; a question leaves the shell behind it inert (#488)
  - [x] [R59q An accessibility baseline](#r59q-an-accessibility-baseline) — every text colour the theme names reaches 4.5:1 on the panels it is drawn on in both themes and with any accent, no text on the pages the tests draw is set under 10 points (the small style is 10.5), a ring shows the widget the keyboard is on, no two kinds of icon share a glyph, and every control those pages draw has a name in the AccessKit tree; open: the viewport and the panels that need a file open are not scanned, and [R68a](#r68a-what-the-accessibility-baseline-still-leaves) hands the tree to the operating system (#555)
  - [x] [R59r Context menus from the command registry](#r59r-context-menus-from-the-command-registry) — every context menu offers its app commands from the registry, acting on what was clicked with one label, binding and enabled-when each; Rename is on `F2` in Files too; the edit mode's control and the workspace buttons are commands; a plugin's commands join them, as many as its guest registers since R45 (#500, #548)
  - [x] [R59s Nothing draws over what it belongs to](#r59s-nothing-draws-over-what-it-belongs-to) — the text editor's footer has a strip of its own and its find bar sits inside its lines, the texture view's header and encode bar wrap within a narrow panel, the viewport's overlay box fits its text, and toasts stack left of the open job list (#499)
  - [x] [R59t The default views in a modder's words](#r59t-the-default-views-in-a-modders-words) — a field typed `Hash32` reads as a hash whatever it is called; the metadata header's format code and schema path and the geometry view's vertex stride wait behind Show internals, which a restart keeps; an unopened archive shows its size and a file row is headed by what the file is; the glTF questions use no flags; and the Edition row, a comparison's path and Outline and a changed texture's pixels say what is so (#546)
  - [x] [R59u Small faults in the editor and the previews](#r59u-small-faults-in-the-editor-and-the-previews) — the find bar now searches a new query and says nothing until it has, Go to Line fits a narrow editor, no row of the texture view starts with a separator, a `.ytd` shows in the Particle workspace's texture preview, `Delete` in Files follows its binding, a DLC pack's folder is asked for by name and an Open Folder document's row offers Create DLC pack…, an open list closes with its document, each document's Outline starts at its top and Go to File keeps a name whole (#532)
  - [x] [R59v Screens never reach the desktop](#r59v-screens-never-reach-the-desktop) — from its first line a self-check sends Reveal in Windows Explorer, Open folder, the Recycle Bin, the clipboard, every file dialog and the registry to stand-ins that log what they were asked; a delete leaves the file where it is, and a `pick` step answers the next file dialog (#517)
  - [x] [R59w A self-check keeps to itself](#r59w-a-self-check-keeps-to-itself) — a self-check's shader pack and tools sit in its scratch configuration, Bake game shaders asks a stand-in that downloads and bakes nothing, a stream plays to a silent sink, a copy stages in the run's own folder, and each toast says what its stand-in did (#522)
  - [x] [R59x Self-checks and tests leave nothing behind](#r59x-self-checks-and-tests-leave-nothing-behind) — a test process removes its scratch configuration as it exits, a self-check's first-launch bake asks the stand-in as a rebake does, its silent sink holds only the stream sounding, and a script's open of a folder waits for its rows (#534)
  - [x] [R59y What a test run or a self-check still leaves behind](#r59y-what-a-test-run-or-a-self-check-still-leaves-behind) — a self-check removes its scratch folder however it ends, every crate's tests make their folders with `tempfile` and a test refuses a fixed one, a test process removes its scratch even when a worker is loading libraries as it ends, each test's preferences are its own, a copy stages in a folder of its own, and a script's `game` step waits for its rows (#568)
  - [x] [R59z Small faults left in the default views, menus and keys](#r59z-small-faults-left-in-the-default-views-menus-and-keys) — an Open Folder document's own row reveals the folder, copies its path and shows its properties, the palette packs the folder opened, a key a disabled command refuses says why (bar Cut, Copy and Paste), a menu of the shell stays open when a load makes another document active, no default view says "1 models", states an index count twice or names a path in the repository, the status bar's hover names the one error or warning it counts, and a typed address cannot panic; the other counts of one in the plural and the lists no menu names an owner for are left (#575)
- [ ] [R60 Onboarding permissions and crash reporting](#r60-onboarding-permissions-and-crash-reporting)
  - [ ] [R60a Onboarding and permissions](#r60a-onboarding-and-permissions)
  - [x] [R60b Diagnostics a user can read](#r60b-diagnostics-a-user-can-read) — landed in #180; how it looks is unchecked by a human
  - [x] [R60c The front door](#r60c-the-front-door) — a restart opens the remembered install active in Browse with no picker; with nothing open the Document panel is a start page (the installs found, Open file and folder, recents, the keys); the status bar switches the game through the close request, and New session and a project keep the install mounted (#474)
  - [x] [R60d Unsaved work survives closing and quitting](#r60d-unsaved-work-survives-closing-and-quitting) — every path that drops unsaved work asks Save, Don't save or Cancel, exiting lists every unsaved file, and a reload over unsaved edits asks first; New session lost its `Ctrl+N` with R59i (#455)
  - [x] [R60e One place for background jobs](#r60e-one-place-for-background-jobs) — the status bar counts the jobs running and opens one list of every kind, each with its progress, a Cancel and, once it ends, what it left; a job that cannot stop at once says it is stopping, and the bake no longer veils the window (#486)
  - [x] [R60f The unsaved marker tells the truth](#r60f-the-unsaved-marker-tells-the-truth) — an install or a folder is marked only while an archive in it is unwritten, an entry saved as a loose file is saved, a patch is an edit, detaching an edited dictionary and replacing the install from Settings or the Enhanced-open prompt ask, and leaving a project whose members changed offers to save it (#475)
  - [x] [R60g No save replaces a file a job reads](#r60g-no-save-replaces-a-file-a-job-reads) — a save of an archive, or of a file inside a folder, is refused naming the job while an extraction, a conversion, a comparison or a DLC pack's build reads it, and a delete or a defragment asks every job too (#523)
  - [x] [R60h Every write asks the jobs reading it](#r60h-every-write-asks-the-jobs-reading-it) — a conversion's and a glTF export's write ask the jobs reading what they would replace, an overlay save is not held back by the shipped archive it copies, and no save waits for the Uses read, which reads the install again after a file under it is replaced (#542)
  - [x] [R60i What a write still does not ask after R60h](#r60i-what-a-write-still-does-not-ask-after-r60h) — an extraction's, an Export as…'s, an import's, a New file's, a paste's and a move's write ask the jobs reading where they write, a destination the disk spells otherwise than a job's path is the same place, a file written under the install makes Uses read it again, and a read asked right after a save waits for the one the save dropped (#572)
  - [ ] [R60j What a write still does not ask after R60i](#r60j-what-a-write-still-does-not-ask-after-r60i)
- [ ] [R68 Shell faults after R59z](#r68-shell-faults-after-r59z)
  - [ ] [R68a What the accessibility baseline still leaves](#r68a-what-the-accessibility-baseline-still-leaves) — the window sends its AccessKit tree, every Tab stop in the dock has a name, the density graph and the curve handles are described, a disabled control's text is exempt by widget and not by alpha, the timeline's marks and the gizmo's axes read at 3:1, the focus ring stays inside a clipped widget's clip and an unsupported file carries a badge; open: the owner's check of the running editor with a screen reader, and the gaps this lane and R59q's found, the code editor's node among them (#563)
  - [x] [R68b A file nothing declares and a module at its gate](#r68b-a-file-nothing-declares-and-a-module-at-its-gate) — a test follows the `mod` declarations from both binaries and fails on a `.rs` file under `src/` that none reaches, the empty file src/ui/widgets/mod.rs is gone, and `src/state/mod.rs` measures 102 code lines under the gate after the edit gate's prompts moved to a file of their own (#554)
  - [ ] [R68c Tests that fail under load and errors a run logs](#r68c-tests-that-fail-under-load-and-errors-a-run-logs)
  - [ ] [R68d What the scan for leftover folders still misses after R59y](#r68d-what-the-scan-for-leftover-folders-still-misses-after-r59y)
  - [ ] [R68e Small faults left after R59z](#r68e-small-faults-left-after-r59z)
  - [ ] [R68f Comments and a module walk the UX lanes left](#r68f-comments-and-a-module-walk-the-ux-lanes-left)
  - [ ] [R68g What the accessibility checks still cannot see](#r68g-what-the-accessibility-checks-still-cannot-see)
  - [x] [R68h A test floor that a deletion crossed](#r68h-a-test-floor-that-a-deletion-crossed) — the suppression check holds with any number of suppressions, none included: its scan is held to having found a `.rs` file and no longer to having found suppressions, and fixture folders prove it finds a suppression that says why, names one that does not and passes with none (#581)
  - [ ] [R68i Count floors that a deletion could still cross](#r68i-count-floors-that-a-deletion-could-still-cross)
  - [ ] [R68j What the suppression scan still skips](#r68j-what-the-suppression-scan-still-skips)
- [ ] [R69 One window of tabs and a rail](#r69-one-window-of-tabs-and-a-rail) — the owner's choice of 2026-11-21, in ten parts in the order they are built; R69a to R69d change no model
  - [ ] [R69a The start page leads with what is installed and what was open](#r69a-the-start-page-leads-with-what-is-installed-and-what-was-open)
  - [ ] [R69b One menu bar with or without a project](#r69b-one-menu-bar-with-or-without-a-project)
  - [ ] [R69c One Build control and a save that offers what can happen](#r69c-one-build-control-and-a-save-that-offers-what-can-happen)
  - [ ] [R69d Every panel has a door and every tile a menu](#r69d-every-panel-has-a-door-and-every-tile-a-menu)
  - [ ] [R69e A tab for each thing and no workspace strip](#r69e-a-tab-for-each-thing-and-no-workspace-strip)
  - [ ] [R69f A rail of rooms and one primary action a tab](#r69f-a-rail-of-rooms-and-one-primary-action-a-tab)
  - [ ] [R69g One Details panel](#r69g-one-details-panel)
  - [ ] [R69h One undo for the tab on screen](#r69h-one-undo-for-the-tab-on-screen)
  - [ ] [R69i The Automate room](#r69i-the-automate-room)
  - [ ] [R69j The guide the tour and the design say the window as built](#r69j-the-guide-the-tour-and-the-design-say-the-window-as-built)

### Phase 11 — The GTA V world editor

The owner asked on 2026-11-25 for every capability of NodeRunner Studio, for GTA V, in OpenRage
([Project_Context.md](Project_Context.md#every-noderunner-studio-capability-for-gta-v-is-wanted-in-openrage-2026-11-25)),
and said the same day to build it in parallel lanes
([recorded](Project_Context.md#the-world-editor-plan-is-built-in-parallel-lanes-2026-11-25)).
[World_Editor_Plan.md](World_Editor_Plan.md) says what the programme rests on, how it fits the tree
and the order, and gives each row below its brief; [World_Editor_Parity.md](World_Editor_Parity.md)
weighs each capability; [World_Editor_UX_Design.md](World_Editor_UX_Design.md) says what a mapper
would meet. The rows are listed in the order of the plan's stages
([the order](World_Editor_Plan.md#the-order-it-is-built-in)): R70a, R70b, R70h and R70w stand on
nothing of this phase, R70g needs R70b only for the bounds at a position, and R70c, the GTA V map
drawn with its models, is what most screen rows wait for. A row takes the default beside its
question in [the rulings asked](World_Editor_Plan.md#rulings-asked-of-the-owner), except a part
whose ruling says "It waits". Parts of R70a, R70b, R70d, R70e, R70g, R70h, R70q, R70r, R70w and
R70y are built, with no screen, on installs and packs made by hand and on the staged corpus. No row
is closed, no install suite has run and nothing was loaded in a game: each row's Where it stands
says what is built and what is left. R70 has no letter left: a finding that fits no row opens the
next number.

- [ ] [R70 The GTA V world editor](#r70-the-gta-v-world-editor) — the plan of 2026-11-25, in the parts below, begun in lanes the same day; parts of R70a, R70b, R70d, R70e, R70g, R70h, R70q, R70r, R70w and R70y are built and no row is closed, and the Map screen of wave P44 and the World screen of GTA VI are what it starts from
  - [ ] [R70a What the game loads](#r70a-what-the-game-loads)
  - [ ] [R70b The GTA V world index](#r70b-the-gta-v-world-index)
  - [ ] [R70g Collision as a query](#r70g-collision-as-a-query)
  - [ ] [R70h One vocabulary of world edits](#r70h-one-vocabulary-of-world-edits)
  - [ ] [R70w A changed bound and a changed path region as sources](#r70w-a-changed-bound-and-a-changed-path-region-as-sources)
  - [ ] [R70c The GTA V world streamed and drawn](#r70c-the-gta-v-world-streamed-and-drawn)
  - [ ] [R70e Find anything and go there](#r70e-find-anything-and-go-there)
  - [ ] [R70i Selecting and moving in the world](#r70i-selecting-and-moving-in-the-world)
  - [ ] [R70q A build a server and a story install take](#r70q-a-build-a-server-and-a-story-install-take)
  - [ ] [R70d The day sky water and shadows of the world](#r70d-the-day-sky-water-and-shadows-of-the-world)
  - [ ] [R70f Layers of what the game hides](#r70f-layers-of-what-the-game-hides)
  - [ ] [R70j A library of props and what comes in](#r70j-a-library-of-props-and-what-comes-in)
  - [ ] [R70k Removing what the game placed](#r70k-removing-what-the-game-placed)
  - [ ] [R70r Other resources of the server](#r70r-other-resources-of-the-server)
  - [ ] [R70y What a project changed and putting it back](#r70y-what-a-project-changed-and-putting-it-back)
  - [ ] [R70l Editing collision](#r70l-editing-collision)
  - [ ] [R70m Interiors](#r70m-interiors)
  - [ ] [R70n Doors](#r70n-doors)
  - [ ] [R70o Parked vehicles zones lights and occluders](#r70o-parked-vehicles-zones-lights-and-occluders)
  - [ ] [R70p Roads scenario points and the navmesh](#r70p-roads-scenario-points-and-the-navmesh)
  - [ ] [R70s The model workshop](#r70s-the-model-workshop)
  - [ ] [R70t Peds cutscenes life and walking the map](#r70t-peds-cutscenes-life-and-walking-the-map)
  - [ ] [R70u Working together](#r70u-working-together)
  - [ ] [R70v The keys settings and guide of a mapper](#r70v-the-keys-settings-and-guide-of-a-mapper)
  - [ ] [R70x The audio of an interior](#r70x-the-audio-of-an-interior)
  - [ ] [R70z A browser of models and vehicles](#r70z-a-browser-of-models-and-vehicles)

---

## Inputs outside the tree

- `U-nnn` rows come from the 225-capability union list, `internal/tool-feature-inventory.md`, which
  is on the owner's machine and not in a clone ([Private_Tier.md](../guides/Private_Tier.md)); no gate checks
  a citation of it. [Appendix A](#appendix-a--union-coverage-index) places every row in an item or a
  non-goal.
- The inventory records CodeWalker as having no licence file, and
  [LICENSE-third-party.md](../../LICENSE-third-party.md) carries an MIT notice. Both hold: the MIT text
  is Neodymium's 2015 crypto ancestor, reproduced from CodeWalker's `Notice.txt`, and the ported code
  stays on the authors' permission (D7).
- The inventory marks audio playback, Bink video and glTF export as parity without a row of their
  own; they belong to [R34](#r34-audio-and-media) and [R38](#r38-mesh-interchange).

---

## Non-goals

Considered and declined, with the reason.

- **A live in-game link or an embedded view of the running game (U-148, U-149).** It injects into a
  running game and hands users online-ban exposure. A third party can build it as an R43 subprocess
  and only there, because a WASM guest can neither inject into a process nor hold a GPU resource.
- **A native in-process plugin ABI.** Rust has no stable ABI, and a `dylib` boundary buys crashes
  and leaked objects. In-app plugins are WebAssembly ([R44](#r44-the-plugin-host)), which crosses a
  boundary the host defines, versions and can refuse.
- **Mandatory telemetry, forced-update gates, monetised dialogs, closed distribution.** Anti-goals,
  not capabilities.
- **Guessing a layout to fill a gap.** ⚠ A borrowed or declared size is not a measurement:
  `fragGlassWindow` is `0x1A0` in CodeWalker and was declared `0x70` in this tree, both wrong, and a
  multi-pane carrier measured `0x40` ([R11](#r11-close-the-fixture-blocked-layouts)).

---

## Detail

### R0b Post-merge detection

[↑ checklist](#phase-0--before-anything-else)

**Done.** Two pull requests each merged green and together left `main` unable to compile, because
neither branch's run held both halves. `.github/workflows/main-tip.yml` checks every tip of `main`
after the merge, compile check first, and names the error in the step summary: detection, not a
gate, and it does not run on pull requests. After the compile check it runs the Windows suite with
no corpus, clippy, the optional-feature suites and `cargo hack`; since #170 that suite lives there,
so a newer tip cancels an older one by the owner's decision.
`crates/foundation/or-test-support/tests/what_looks_at_main_after_a_merge.rs` holds it
(`only_a_newer_tip_cancels_the_tip_check`, `the_deep_gate_still_covers_what_only_it_can`).

### R1 Land the v2 overhaul branch on main

[↑ checklist](#phase-0--before-anything-else)

**Done.** The v2 overhaul, with the descriptor-pack system (retired by D17) and the layered crates,
landed on `main`. `feat/ydd-clothing-preview` is unrelated, unmerged and kept; the archived cloth
tool is the tag `archive/ydd-clothing-preview` (`337ee1bb`), which
[R46](#r46-first-plugin-a-clothing-tool-on-openrage) rewrites rather than ports.

### R2 Reconcile the documentation with the tree

[↑ checklist](#phase-0--before-anything-else)

**What.** Keep every document's claims true of the tree, and extend the rule that a count a
document states is asserted by a test.

**Done when.** No document states a count that no test asserts; the `Generation`, `PACK_VERSION` and
Enhanced-`.yft`-write claims match the code, and no document counts retired read paths; the
`rage_layout_tests!` guidance is gone; and a Roadmap item over the Standard's budget has its
measurement logs and test narration moved to its suite and its pull request.

**Where it stands.** The named claims match the code: `PACK_VERSION` went with the `.orpack`
export by the owner's ruling of 2026-10-01, so no document states a live value for it, and the
read-retirement ledger went with `or-asset` in [R66f](#r66f-delete-the-old-crates), so no count of
it is left to state. The first clause stays open as a universal, and
`every_count_a_document_states_is_asserted_or_a_named_measurement` reads what a check can of it:
every numeral or number word from eleven up that is a percentage or is followed by `of` or a plural
noun, in `docs/`, the crate READMEs, `README.md` and `AGENTS.md`. Each is held by a test (`ASSERTED`
in `src/doc_counts/figures.rs`), by a revision, date, pull request, command or existing suite named
in its paragraph (its item, in the Roadmap and the Project context), or by a waiver that says why
(`WAIVED`: the Standard's own budgets and one illustration).
`the_count_check_can_tell_its_shapes_apart` and `the_anchor_check_can_tell_its_shapes_apart` hold the
reader, and `the_ledgers_excuse_only_counts_a_document_states` keeps the ledgers honest.

The check cannot close the universal because a count is a matter of meaning: "two archives" is a
pair as often as a count, so a number word below eleven, a bare number in a table cell, a decimal and
a count in a Rust comment go unread, as do `plugins/README.md`, `tools/` and `assets/`. It asks that
a figure name a source, not that the source be right or name both a suite and a revision.
**Proposed Done-when:** every count the gate reads is held and the ledgers waive only a rule or an
illustration; what a grammar cannot read stays a review rule. The rules this item wrote are in
[Documentation_Standard.md](../guides/Documentation_Standard.md).

Found while closing [R27p](#r27p-a-bones-numbers-can-be-edited) (#574),
[R59z](#r59z-small-faults-left-in-the-default-views-menus-and-keys) (#575),
[R60i](#r60i-what-a-write-still-does-not-ask-after-r60h) (#572),
[R59y](#r59y-what-a-test-run-or-a-self-check-still-leaves-behind) (#568),
[R27q](#r27q-the-skeleton-files-own-model-can-be-hidden) (#566),
[R27o](#r27o-the-animation-preview-follows-its-sources-fully) (#559) and
[R42g](#r42g-enhanced-controls-that-cannot-work-say-so) (#560): the Where it stands of each runs
from about 420 words to more than 1,100 against the Standard's 300 an item, narrates what failed
against `main` and how a test was made to fail, and carries measurement logs (run counts, entry
counts, a depth survey), which the Standard sends to the suite's doc comment and the pull request;
no gate checks the budget.

### R3 Licence and provenance policy

[↑ checklist](#phase-0--before-anything-else)

**Closed: superseded by D15 (2026-09-25) for layout declarations.** The four-tag provenance record
R3 asked for never became house style: layout work is an `or-io` declaration under the engine's
names, proved by the recorded testimony, the canonical fixed point and the censuses (D17), with a
comment only where something is unusual. Its 2026-09-20 scope is superseded too: publication is
[R66h](#r66h-publish-the-crate-and-the-editor-together) (D3, D8), the leaked-source pattern lint in
`scripts/lint_provenance.ps1` is fatal in every tracked file since #340, and R66b removed the path
citations from the tree. The owner ruled on 2026-10-01, in his words "no code provenance info block
or anything, delete it"
([recorded](Project_Context.md#no-provenance-record-is-kept-and-the-provenance-lint-stays-2026-10-01)).
[Licence_And_Provenance_Policy.md](../guides/Licence_And_Provenance_Policy.md) carries no tag grammar, no
module in the tree carries a `Provenance:` block, every ported module is credited in
`LICENSE-third-party.md`, and the provenance lint against leaked-source citations and committed game
content stays.

### R4 Step 1 the pack is the layout authority

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**Done, and replaced by D1 and D17.** Layouts were authored in `../openrage-packs`, exported with
`offset_of!` offsets, audited and discovered at startup. Since D1 the `or-io` declarations are the
layout authority ([R66](#r66-the-or-io-crate)), and D17 retired the pack: nothing in this tree
builds, discovers or compares one (OD-P61-SPLIT, #331, #332). What stands is `or_io::desc::Pack`,
the description `or-io` builds from its declarations per format and target
(`Format::description`), with its audit (`or_io::desc::export::audit`); its `.orpack` encoding
and `PACK_VERSION` went by the owner's ruling of 2026-10-01.

### R5 Step 2 the pack learns to write

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**Done.** The generic writer stands, and every owned value saves through it. `or_io::write::build`
rebuilds a resource from a value tree and the format's built-in description on a `ResourceHeap`,
which records the pointer fixups and cannot reach the RSC7 bucket packer, `value::empty` authors a tree from the description alone, and
`ImageMut::put_scalar` patches fixed-width scalars in place, refusing a value too wide for its
field. An allocation whose regions are computed takes a `BlobCodecs` reader and a `BlobWriters`
writer: the test is whether there is a constant to write down. R66 replaced the pack as the
writer's input and the pack-required write rows; a save writes OpenRage's canonical form, held by
`fixed_point` and `owned_saves_as_tree` in `crates/io/or-io-verify/src/gates.rs`.

### R6 Cross-field invariants before the oracle goes

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**Done** (#5, #7, #11). Invariant suites held relationships between fields of one read, each beside
a mutation-pinned companion showing it can fail. Checks of that kind stand in `or-io-verify`'s
per-format suites over owned values: `every_polygon_indexes_a_vertex_that_exists` in
`crates/io/or-io-verify/tests/bounds.rs`, and `every_clip_names_an_animation_its_dictionary_holds`
with `each_block_invariant_can_fail` in `crates/io/or-io-verify/tests/clip_dict.rs`. They compare
fields of one read with each other, never a declaration with the bytes, so they bound the risk
rather than replace the compiled oracle ([R18](#r18-decide-what-replaces-the-compiled-oracle)).

### R7 Step 3 make pack binding the only path

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**Done** (#12, #14), **and replaced by D16.** A `pack_only` type gave up the bind-time offset
comparison, the per-read differential and the compiled reader at once. R66 replaced binding with
declarations whose offsets `#[record]` computes, opened as owned values, and R66f deleted the
compiled readers; `pack_only` survives as a flag in the recorded testimony
(`crates/io/or-io-verify/src/testimony.rs`), naming the layouts read through the frozen key
(OD-TESTIMONY-KEY-FROZEN). ⚠ A description's coherence check (`PackType::incoherences`) catches a
typo and not a misreading: `a_description_shifted_wholesale_is_coherent_and_wrong` shifts every
field of a type and passes.

### R8 Step 3 strip the layout apparatus from rage types

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**Done** (#30), **and replaced by D15.** Hand-written `offset_of!` literals were deleted wherever
something else compared the field with the pack, and the survivors were held against a table of
what checked each type, which went with `or-asset` in #332. Under D15 no offset is written by hand:
`#[record]` computes offsets from `repr(C)` field order, padding is a declared field or
`#[pad_after(N)]`, and `#[at]` is an optional assertion
([declaring a structure](OR_IO_Specification.md#declaring-a-structure)). The recorded testimony
and the alignment census cross-check the result.

### R9 Step 4 convert the four divergent leaves and relax the bind check

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**Done** (#8, #79), **and replaced by R66d.** A declaration mirrored one target's raw struct and the
other target's description was checked for coherence alone; that mechanism went with `or-asset` in
#332. Its split stands: the types Gen9 changed are declared per target (`GrcVertexBufferG9`,
`GrcIndexBufferG9`, `GrcTextureG9` and the shader's `GrcInstanceDataG9`), and a type that differs
only in what it points at takes a target parameter, as `GtaDrawable<Gen9>` does (OD-10.8). ⚠ A
round trip is not the gate: reading and writing through one description cancels, so a coherent,
wrong Gen9 description saves to a fixed point; the typed reads and the recorded testimony are what
see it ([Oracle_Testimony.md](../guides/Oracle_Testimony.md)).

### R10 Step 4 delete the Gen9 clone tree

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**Done** (#38, #82). The `G9` aliases over the drawable spine are gone: call sites name
`GtaDrawable<Gen9>`, and `or-io` declares no `G9` alias (D15). The divergent leaves stay separate
types ([R9](#r9-step-4-convert-the-four-divergent-leaves-and-relax-the-bind-check)). The `gen9/`
rename was withdrawn, and `gen9::convert` was deleted with `or-asset` in #332; converting between
the targets is [R55](#r55-retargeting-between-gen8-and-gen9).

### R11 Close the fixture-blocked layouts

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**Done** (#44). ⚠ The staged corpus is the wrong denominator for a layout gap. `openrage-cli gaps`
over each retail install (every `.ydr`, `.ydd`, `.yft` and `.ybn`) found 433 glass panes across 246 carrier files,
and live cloth tunings on 212 of 1,832 Legacy and 204 of 1,838 Enhanced environment cloths, where
the staged corpus held none; the genuine zeros are tint payloads (0 of 190,715 Legacy and 188,903
Enhanced drawables) and `plane` bounds (0 of 666,416 and 667,105), all measured in #44.
`ClothInstanceTuning` is declared in `or-io`, and the pane, `BgPaneModelInfoBase`, is `0x40`, sized
on a multi-pane carrier: one pane says what a block holds, not where it ends.

### R12 Step 5 open DocumentKind and hold an image not a payload

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**Done** (#37). `DocumentKind` has an open end, `Registered`, and the app reads, patches and saves
through a `ResourceImage` beside its payload. `DocumentKind::NAMED` is the one list of named kinds,
and `a_named_kind_never_falls_through_these_matches` in `src/state/document_io.rs` drives each
through every match. [R66e](#r66e-move-the-consumers) replaced the per-format resource payloads with one,
`DocPayload::Resource` holding `or_io::Asset` (OD-PAYLOAD). A claim quoted from code is dated the
moment it is quoted: quote the test that holds it.

### R13 Step 5 shrink the shell

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**Done** (#40, #48). The shell passes its code-line budget (`src/shell_budget.rs`; quote its
`report_the_tree_by_code_lines`, since raw line counts and the gate disagree), and
`src/suppressions.rs` requires every surviving unused-surface suppression to say why. The name
index has a producer per category (`src/state/name_index.rs`), and `PRODUCER_OF` makes a category
without one fail to compile: the install walk supplies file names, and opened documents supply
`.ypt` rule names, which no listing can see. Mip fitting is in
`crates/io/or-io/src/authoring/fit.rs`, candidate ranking in `crates/runtime/or-anim/src/resolve.rs`.

### R14 Step 5 finish the container seam

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**Done** (#42, #51). The write half takes a `PagingModel`
(`crates/io/or-io/tests/a_second_paging_model_on_the_write_half.rs`), and the RSC7 envelope is
`ResourceEnvelope` in `crates/io/or-io/src/container/resource/envelope.rs`, with a private header
module and an opaque `Rsc7Seal`. ⚠ `author` invents a header and `reseal` reproduces one;
confusing the two once rewrote an Enhanced file at the Legacy version. OD-AUDIT-KEEPS keeps
`PagingModel` and `ResourceEnvelope` for [R56](#r56-rdr2-the-second-container) though each has one
caller today. `crates/io/or-io/tests/what_still_names_the_envelope.rs` holds every reference to the
header's writer, reader, read contents, magic and size outside
`crates/io/or-io/src/container/resource/` at zero, and holds those five to `pub(super)` inside it. DEFLATE stays outside the seam, because
RPF7 inflates entry bodies that are not resources.

### R15 A conformance gate that runs where the code is

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**Done** (#34). The `layouts` job (`.github/workflows/local-gate.yml`, a self-hosted runner on the
owner's desktop) runs what hosted CI cannot, for every push to a pull request from this repository
and to `main`: `or-io-verify`'s corpus gates, R66e's behaviour baselines with
`OPENRAGE_REQUIRE_FIXTURES` set, and the clean-checkout run. Its pack staging and `just pack-check`
went in #331 and its cross-pack comparison in #332, under D17 and OD-P61-SPLIT. ⚠ The runner
checks out with plain git, because `actions/checkout` fails on the busy desktop; its other
operations are in [Development_Workflow.md](../guides/Development_Workflow.md).

### R16 Make a skipped suite visible instead of green

[↑ checklist](#phase-2--proof-a-check-that-could-have-failed)

**Done.** The skip ledger (`crates/foundation/or-test-support/src/bin/skip-ledger.rs`) reports what
a run did not check, and `OPENRAGE_REQUIRE_FIXTURES` (`just test-strict`) turns a fixture skip into
a failure where a corpus exists; the `layouts` job sets it for the behaviour baselines.

### R17 Equivalence contracts and poison canvases for the resource writers

[↑ checklist](#phase-2--proof-a-check-that-could-have-failed)

**Done** (#39), **and its instruments replaced by R66.** It classed every pack-written format by
what a save may change, and filled each allocation with a non-zero byte before the writer ran so
that a field keeping the reservation's zero was named; both suites went with `or-asset` in #332.
What stands: the two-fill canvas as `or_io::write::coverage` (`crates/io/or-io/src/write/canvas.rs`),
which `crates/io/or-io-verify/tests/what_a_writer_inherits_from_its_canvas.rs` runs over every
staged file of each declared format and target, failing on a declared field nobody wrote and
catching one planted by moving an offset; `fixed_point` and `owned_saves_as_tree` in
`crates/io/or-io-verify/src/gates.rs`; and `what_a_save_drops_is_the_list` with
`census::STILL_LOST`, naming every site a canonical save still loses. ⚠ A save that agrees with a
read says the writer agrees with the reader, not with the file: both can be wrong the same way, and
only the recorded testimony compares a declaration with the bytes
([R18](#r18-decide-what-replaces-the-compiled-oracle)).

### R18 Decide what replaces the compiled oracle

[↑ checklist](#phase-2--proof-a-check-that-could-have-failed)

**Decided: nothing adequate replaces the compiled oracle** ([Oracle_Testimony.md](../guides/Oracle_Testimony.md)).
Every candidate compares a description with something other than the bytes. The recorded testimony
([R65c](#r65c-record-the-oracles-testimony)) stands in, read through the offset key frozen from the
version 12 pack and never retaken (OD-TESTIMONY, OD-TESTIMONY-KEY-FROZEN):
`crates/io/or-io-verify/tests/the_recorded_testimony_holds.rs` and each declared format's suite
hold it. R66f deleted the compiled readers once the declared formats held that record, and R66e's
behaviour baselines in `openrage-corpus/baselines` hold what the consumers produce.
`tools/cw-compare` is a compatibility diagnostic, not an arbiter: CodeWalker misreads `.ycd` in
places the local YCD specification lists; its README names two.

### R19 One virtual filesystem

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** One path type across disk and archive paths, into nested archives with no extract step and
with DLC precedence applied: `VfsPath` and `Vfs::glob` in `crates/title/or-title/src/vfs/`, under the
explorer, search, bulk jobs and plugins.

**Done when.** `glob("**/*.ydr")` works across archive boundaries from the library, the CLI and the
app; a nested archive four levels deep opens with no temp files; browsing a 3D format loads nothing
into the renderer.

**Where it stands.** Done. The library half holds (`one_path_across_archive_boundaries`), the CLI's
`Matcher::Glob` wraps the same `or_title::vfs::Pattern`, and the app mounts an open document as one
VFS and globs through it (`src/state/vfs_session.rs`, #155). The explorer tree is filled from
`ArchiveDevice::entries`, which lists a directory's original-case names, kinds and entry indices
and opens each nested archive it lists (`the_tree_is_filled_from_the_vfs_listing`); a node keeps its
entry index and override slot. `RpfEditSession::device` hands out an `ArchiveDevice` over a session
archive under the session's keys: entry bytes come from `ArchiveDevice::read_entry` by entry index,
so a renamed or moved entry still reads, extraction reads the same way in the form asked for
(`an_archive_entry_extracts_through_the_vfs_and_an_edit_wins`), and a nested archive the listing
could not open is opened by `ArchiveDevice::open_nested`, in memory. An edit stays in the node's
override slot and wins over the archive in a read, an extraction and a save. Selecting a drawable
in the tree previews its bytes and opens no document, builds no mesh and leaves the preview scene
empty (`browsing_a_drawable_loads_nothing_into_the_renderer`).
`the_app_still_walks_archives_without_this_vfs` asserts which app files reach the VFS.

**Risk.** A VFS that is a second copy of the RPF reader; it must be the path the app already uses.

**Covers.** U-001, U-002, U-003, U-004, U-008, U-013.

### R20 Archive lifecycle

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** Defragment in place with an exact size preview and progress, a one-time `.bak` before the
first write, and creating an archive nested inside an existing one, from the app.

**Done when.** A user can build a working DLC-shaped archive tree from inside OpenRage, shrink it,
and have the game load it, behind [R25](#r25-the-edit-mode-gate)'s edit-mode gate.

**Where it stands.** The builder (`crates/io/or-io/src/container/rpf/builder.rs`, driven by
`openrage-cli rpf create`), the encryption chooser (`RpfEncryption`, `RpfOutputMode`), import
classification from each body's own RSC7 header, the one-time `.bak` and `openrage-cli rpf defrag`
exist. The archive menu in `src/ui/panels/hierarchy/rpf.rs` can add an empty `.rpf` through New
File, and its Defragment… runs the CLI's rebuild (`or_io::container::rpf::Defrag`) as a job: it
reads every file with progress, states the size now and after, and writes only once the user
answers, behind the edit gate both times (`src/state/defrag_job.rs`,
`defragmenting_shrinks_the_archive_and_keeps_every_entry`, `defragmenting_asks_the_gate`). It
refuses an archive with unsaved edits or open tabs at the start and at the answer, nothing else
writes the archive meanwhile, and a file that changed since the preview is left alone
(`nothing_else_writes_an_archive_being_defragmented`,
`a_file_changed_after_the_preview_is_not_replaced`). Open: nobody has built a DLC-shaped tree in the app and had the game load it.

**Risk.** ⚠ The `.bak` must stay one-time: refreshed on every save, it holds the previous edit one
save later and has lost the shipped file two saves later.

**Struck: escrow detection (U-203).** `openrage-cli archives` found no unknown encryption mode and
no unexpected entry marker in 10,142 archives and 766,693 entries of two retail installs;
`RpfEncryption::Other` keeps the raw value if one appears. 9,801 of those archives are nested,
which every bulk operation has to plan for.

**Covers.** U-005, U-006, U-007, U-032, U-035, U-036, U-203 (struck), U-204, U-207.

### R20b Nested archives survive a save

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** Rebuilding an archive keeps every archive nested inside it, and an edit made inside a
nested archive saves back through the outermost archive that is on disk.

**Where it stands.** Done. Rebuilding an archive, whether an install's tree saves it or it is open
as a document of its own, writes each archive nested in it as one entry (`src/state/rpf_records.rs`):
the bytes its parent stores for it, or, once anything inside it was edited, a rebuild from its own
records, recursively, under the encryption it was read with. An edit marks every archive around it
unsaved, so the edit gate is asked about the archive on disk that holds it and that archive is the
one saved; saving it marks the archives nested in it saved. The save destination chooser offers
that archive on disk for an entry of a nested archive. `src/state/nested_archives_survive_a_save.rs`
holds both clauses: untouched nested archives, one browsed and one never opened, read back byte for
byte and still open, and an edit two archives deep reads back from disk whether it was saved from
its tab or with the install's tree. Two gaps found while closing it are
[R20c](#r20c-saving-twice-and-into-an-archive-not-yet-opened)'s and
[R20d](#r20d-a-rebuilt-ng-archive-the-game-can-read)'s.

**Done when.** A test rebuilds an archive that holds an untouched nested `.rpf` and reads that entry
back byte for byte, and a test saves an edit made two archives deep and reads it back from disk.
The first test is written before any fix.

### R20c Saving twice and into an archive not yet opened

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** After a save, the session reads the archive it wrote, so a second save in the same session
writes every entry from the right bytes; a file added to or moved into a nested archive the tree has
not opened reaches that archive; a rebuild that cannot read an entry refuses to save, naming it,
rather than writing it empty; and `Ctrl+S` on a folder opened with Open Folder saves the archives
edited in it.

**Where it stands.** Done. Once a save writes an archive of an install's or a folder's tree, the
session reads that file back and points every node under it at its entry there, through every
nested archive it has opened (`adopt_written_archive`), so a second save in the session reads what
the first wrote; an archive that cannot be read back is closed in the tree with an error. Adding a
file or folder to an archive row the tree has not opened, or moving one into it, opens the row
first as expanding it does, and one that cannot be opened refuses the edit, as the session refuses
anything added under an archive no listing has opened. A rebuild that cannot read an entry refuses
the save and names the entry. `Ctrl+S` on a folder opened with Open Folder saves the archives edited
in it, as an install's tree does. Renaming or moving an opened archive on disk, or a folder that
holds one, carries every row below it and every archive opened there to the new path with their
unsaved edits (`moved_on_disk`), since an opened archive reads and saves the path it holds.
`src/state/saving_twice_and_into_an_archive_not_yet_opened.rs` holds the four
clauses and the rename; each failed before its fix. A file moved between two archives, and an
extraction running across a save, read the replaced file until
[R20e](#r20e-two-readers-of-one-archive-across-a-save) closed both.

**Done when.** A test saves an install archive twice in one session and reads every entry back byte
for byte; a test adds a file to a nested archive the tree has not opened, saves, and finds it there;
a test makes an entry unreadable and finds the save refused with its name; a test edits an archive
inside an opened folder and saves it with `Ctrl+S`.

### R20d A rebuilt NG archive the game can read

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** A rebuilt archive encrypted NG decrypts with the key the game derives from its own file
name and size.

**Where it stands.** By reading, while closing [R20b](#r20b-nested-archives-survive-a-save): the
archive builder encrypts an NG table of contents with no archive name, so its key comes from an empty
name and the size, while a reader derives it from the file name. OpenRage reads such an archive back
only by trying every NG key, and or-io's `ng_archive_roundtrips_through_retail_provider` is ignored.
An edited NG nested archive is now rebuilt under NG, so this reaches every one of them.

**Done when.** `ng_archive_roundtrips_through_retail_provider` runs and passes, and a rebuilt NG
archive opens in the game, which is the owner's check.

### R20e Two readers of one archive across a save

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** A save never leaves anything else reading the archive it replaced at the old file's
offsets: a file moved between two archives saves whole whichever archive is saved first, and an
extraction running while its archive is saved extracts what it started from.

**Where it stands.** Done. Before a save replaces an archive's file, every file outside that
archive's tree still read through it, moved out of it or out of an archive nested in it, is read
into memory (`carry_readers_of`), so the archive it was moved into saves it whole whichever of the
two is saved first. A nested archive moved out has no bytes of its own to hold, so a save of the
archive it left is refused, naming it, until the archive now holding it is saved, and since
[R20f](#r20f-a-moved-or-deleted-archive-row-leaves-nothing-behind) `Ctrl+S` saves that one first, so
one press saves both. A save of an archive an
extraction reads is refused with a sentence naming the extraction, as a defragment's is, so the job
reads the file it was planned from to its end.
`src/state/two_readers_of_one_archive_across_a_save.rs` holds both clauses and the nested archive's
refusal; each failed before its fix.

**Done when.** A test moves a file between two archives, saves the one it left and then the other,
and reads the file back byte for byte; a test saves an archive while an extraction from it runs and
finds every extracted file whole.

### R20f A moved or deleted archive row leaves nothing behind

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** A row moved into another archive belongs to that archive at once: its tab saves into it,
the edit gate asks about it, and only that archive is marked unsaved. A move onto an archive row
that sits on disk lands in that archive, and `Ctrl+S` saves a tree's archives in an order that never
needs a second press. Deleting an opened archive leaves nothing to save. Neither a defragment nor
an expansion reads an archive across its save or while an extraction reads it. An extraction of an
archive row not yet expanded extracts its whole contents.

**Where it stands.** Done. A row belongs to the archive whose row sits above it in the tree, not to
the one it is still read from until that is saved (`archive_on_disk_around`), so a row moved into
another archive, on its own or in a folder, saves its tab there, asks the gate about it and marks it
alone unsaved, and a save carries each file moved out of a nested archive it reopens, one moved in
among them. A move onto an archive row on disk opens it and lands inside; a loose file is still
refused. `Ctrl+S` saves first an archive no nested archive moved out of it still reads, so one press
saves them all, except two archives that each hold a nested archive moved out of the other. A
deleted archive leaves the session with every archive opened in it, once each file moved out of
them is read into memory. A defragment and an extraction of one archive refuse each other, and an
expansion begun before a save or a defragment replaced its file is listed again. An extraction
opens an archive row first, as an add does, and since
[R20g](#r20g-what-archive-rows-still-get-wrong) every archive nested in it as well. The session's
unread unsaved flag is gone.
`src/state/a_moved_or_deleted_archive_row_leaves_nothing_behind.rs` holds each clause, and each
failed before its fix.

**Done when.** A test moves a row with an open tab between archives and finds its save destination,
the gate's question and the unsaved mark on the destination alone; a test moves a file onto an
archive row on disk and finds it there after a save; a test deletes an opened archive with unsaved
edits and finds nothing left to save; a test starts a defragment during an extraction and is
refused; and a test finds that an extraction of an archive row not yet expanded extracts its whole
contents.

### R20g What archive rows still get wrong

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** An archive row with unsaved edits shows it; renaming an opened archive on disk leaves
nothing to save; deleting an archive an extraction reads is refused; an extraction writes the
whole of what it names, a folder on disk the tree has not scanned and an archive nested in an
extracted one included; and a folder dropped onto a folder inside itself is refused.

**Where it stands.** Done. An archive row's unsaved badge, in Files and the Files view, reads its
archive's slot (`archive_unsaved`), the flag a save clears. A rename marks the archive holding the
row, never the row's own, so an opened archive renamed on disk leaves nothing to save, and a nested
archive renamed inside another is stored as the bytes it held. A delete of an archive an
extraction reads, or of a folder holding one, is refused, naming the extraction. An extraction
scans every folder and opens every archive under what it names before it plans, nested archives to
eight deep, on the UI thread ([R20h](#r20h-what-archive-rows-still-get-wrong-after-r20g) moved that
to the expansion workers). A folder dropped onto itself or a folder inside it is refused before
the edit gate asks, by the check a paste made, now shared and comparing paths as the disk names
them. Unloading an archive forgets its slots. Two archives that each hold a nested archive moved
out of the other still never save. `src/state/what_archive_rows_still_get_wrong.rs` holds each
clause, and each failed before its fix. What it left is
[R20h](#r20h-what-archive-rows-still-get-wrong-after-r20g)'s.

**Done when.** A test edits an archive and finds its row marked; a test drops a folder onto a folder
inside itself and is refused, with nothing written; a test renames an opened archive on
disk and finds nothing left to save; a test deletes an archive during its extraction and is refused;
and a test extracts an unscanned folder and an archive holding an unopened nested archive and finds
every file written.

### R20h What archive rows still get wrong after R20g

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** An extraction plans in the background; a loose folder an extraction reads cannot be
deleted, nor an archive it reads renamed on disk; dragging rows out to Explorer and
"Only one type" under More extract options take every file under what is selected, opened or not; a folder
added inside an archive loses its "New (in-memory)" badge once a save writes it; a folder on
disk the tree has not scanned does not read as holding nothing; and a nested archive moved into
another archive still reads after the archive it came from is renamed on disk.

**Where it stands.** Done. An extraction opens what it names on the expansion workers, four rows at
a time, and plans once the last has opened (`src/state/extract_planning.rs`): the frame that starts
it opens none, the job list shows it opening, its Cancel drops it before anything is written, and an
archive that will not open is told and left out. While it opens and while it runs it holds the files
on disk its rows sit in or are, loose files as well as archives, so a delete, a rename or a move on
disk of any of them is refused naming it. A rename on disk now asks the jobs reading it, as a
delete, a save and a move do. A drag out to Explorer opens every row under what is selected before
it hands over its files, and the counts of Only one type take every row under a folder, opened or
not, and are offered for every single folder
([R20i](#r20i-what-archive-rows-still-get-wrong-after-r20h) took the opening off the frame the drag
starts on, and put the counts' read behind a click). A folder added inside an archive stops showing
"New (in-memory)" once a save writes it. The inspector of a folder on disk the tree has not scanned
says it is not read until it is opened. `reparent` gives a new handle to every nested archive that
reads through the archive renamed, wherever its row now sits, so a nested archive moved out of one
archive still reads and saves once that one is renamed on disk. `register_archive` refuses with an
error once the ids are used, where it panicked.
`src/state/what_archive_rows_still_get_wrong_after_r20g.rs` holds each clause, and each of its tests
failed before its fix, or with the fix taken out, except two: the guard that an archive which will
not open ends the wait, which no earlier design failed, and the move half of the refused rename,
which [R25j](#r25j-a-drag-carries-every-selected-row) already held. What is left: a renamed
NG-encrypted nested archive is saved under its old name's key, which
[R20d](#r20d-a-rebuilt-ng-archive-the-game-can-read) holds.

**Done when.** A test extracts a folder holding unopened archives and finds the frame that starts
it opening none; a test deletes a loose folder, renames an archive and moves one, each while an
extraction reads it, and is refused each time; a test drags out a folder holding an unopened
archive and finds its files in what Explorer is handed; a test saves an archive with a new folder
in it and finds the badge gone; a test selects a folder on disk the tree has not scanned and finds
no count of nothing in its inspector; and a test moves a nested archive into another archive,
renames the first on disk, and reads and saves the moved archive's files.

### R20i What archive rows still get wrong after R20h

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** A drag out to Explorer does not hold the frame while it opens the rows under the
selection, the counts of Only one type do not read a whole install unasked, and an extraction that
opens rows holds back the saves of the files it reads and no others.

**Where it stands.** Done. A drag out to Explorer opens nothing on the frame that starts it: it asks
the workers for the rows under the selection the tree has not opened and is refused, naming how many
are left, where it held the window for three seconds before the size check (`DRAG_OUT_LIMIT`). The
rows go on opening over the frames that follow, archives nested in them included, and a toast says
when to drag again, which then hands over every file; a row that would not list or open is told
once, and every drag of the same rows after that goes ahead without it, still opening any other row
the tree has lost since, until other rows are dragged. The Only one type submenu lists the types the
tree already holds and reads nothing, by the owner's call of 2026-10-05; when rows under the folder
are unread it offers Count types in archives…, and only that click starts opening them. The submenu
stays open while it counts, shows how many rows are left, and the end of the count tells of an archive
that would not open. Each click starts a count of its own, so an archive that failed and was mended
(unloaded, as a save's read-back, a defragment and an overlay refresh unload one) counts the next
time, with no row remembered as failed and no toast each time the submenu is open. While an extraction
opens its rows, a save is refused only for the archives it has asked a worker to open
(`Planning::reads`), so a save of any other file under an install goes ahead; a delete, a rename, a
move on disk, an undo or redo, a defragment and an overlay refresh still ask `no_job_holds`, which
also holds the files its rows sit in or are, as
[R20h](#r20h-what-archive-rows-still-get-wrong-after-r20g) left them: the last two unload the
archive's row, so an archive already open under the rows is held as well.
`src/state/what_archive_rows_still_get_wrong_after_r20h.rs` holds the drag, the save, the defragment
and refresh, the count that reads nothing and the mended archive, and
`src/ui/the_counts_of_only_one_type_read_nothing_until_clicked.rs` drives the real menu; each failed
before its fix, or with the fix taken out. What is left: a drag out of a very large selection, an
install's root, now opens everything under it in the background before the size check refuses it,
and Extract takes it either way; the count of Only one type advances only while its submenu is
open, so closing it parks the walk until it opens again; and a folder that cannot be listed stays
unread, so the submenu offers Count types in archives… again and each click toasts the failure again.

**Done when.** A test drags out a folder holding an unopened archive and finds the frame that starts
the drag not held while the rows open; a test opens Only one type on an install's root and finds no
archive read before the cap or the click the owner chose; a test saves a file under an install while
an extraction opens rows elsewhere in it and finds the save done; and a test counts a folder holding
an archive that would not open, mends the archive, and finds it counted without the counts being
asked for another folder.

### R20j What archive rows still get wrong after R20i

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** A drag out and the counts of Only one type try again what was mended and say what they
left unread, and the rows a drag opens are listed and can be cancelled.

**Where it stands.** Found while closing [R20i](#r20i-what-archive-rows-still-get-wrong-after-r20h)
(#565): a row a drag told would not open is remembered by id for the same document and the same
ordered selection (`drag_told` in `src/state/extract_job.rs`, seeded into each fresh walk by
`open_rows_for_drag`), so every later drag of that selection goes ahead without it, and a folder
mended on disk since is not tried again until it is expanded in the tree or other rows are dragged;
a drag of other rows that ends with no failure forgets it, so dragging A (holding a folder that will
not list), then B, then A walks A afresh, refuses it again and toasts the failure again. Clearing
the memory on a timer or on a refresh of the tree would retry it, and nothing asked for that. A
folder that cannot be listed stays unread for good, so `census_of` keeps answering `Counting::Idle`
with rows unread, the submenu offers Count types in archives… again and each click asks the folder
afresh and toasts the same failure; the owner may want failed rows left out of the unread count. An
archive that would not open is neither counted as unread nor asked again by a fresh walk, so once
its file is mended only an unload (a defragment, a save's read-back, an overlay refresh) lets it
count. The count advances only while its submenu is open, since `census_of` is the tick, so closing
the submenu parks a walk with rows in flight. A drag out of an install's root opens everything under
it in the background with no bound and no row in the jobs list, and `request_cancel` in
`src/state/extract_job.rs` stops only an extraction or an export; the rows stay open in the tree
afterwards, the size check then refuses the drag, and nothing shows the opening but the first toast.
No test fails if the `no_job_holds` calls of an undo or a redo (`src/state/undo.rs`) or of a mod's
move (`src/state/mod_manager.rs`) go back to `no_job_reads`, which would let either run while an
extraction opens its rows.

**Done when.** A test drags a selection holding a folder that will not list, mends it, drags again
and finds it opened; a test counts a folder that cannot be listed and finds the count not offered
again, or told once, as the owner decides; a test drags an install's root and finds the opening in
the jobs list with a Cancel that stops it; a test closes the submenu mid-count and finds the count
go on or say it has stopped; and a test puts `no_job_reads` back in an undo and in a mod's move and
finds it fail.

**Partly closed.** `no_job_holds` is now held in an undo
(`an_undo_writing_under_an_extraction_still_opening_its_rows_waits_for_it`) and in a mod's move
(`a_mod_does_not_move_under_an_extraction_still_opening_its_rows`): each fails with `no_job_reads`
put back in its place, the second over a mod of loose files, which the extraction has asked for no
archive of. A folder mended after a drag told it would not list is opened by the next drag (`a_folder_mended_after_a_drag_told_it_would_not_list_is_opened_by_the_next_drag`), and a folder a count could not list is told once and left out of what is unread, as the owner's delegated choice, until it lists or is unloaded (`a_folder_the_count_could_not_list_is_told_once_and_left_out_of_the_unread_count`), each seen failing first. A count goes on while the submenu that started it is closed, ticked by the frame as a drag's opening is (`a_count_goes_on_while_the_submenu_that_started_it_is_closed`; the owner's choice, delegated, was to go on rather than say it stopped), and a drag out's opening of rows is in the jobs list as "Opening what the drag out holds" with a Cancel that drops it (`a_drag_out_opening_its_rows_is_in_the_jobs_list_and_its_cancel_stops_it`), each seen failing first. Every clause of R20j's Done-when holds.

### R21 Extraction and bulk jobs

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** Whole-install bulk jobs (every texture, every file matching a filter, every script, every
shader) with per-type selection, progress and abort, in the app as well as the CLI.

**Done when.** Both the app and the CLI can run the same job, report progress, and be cancelled.

**Where it stands.** The CLI's filter, dry run, progress and abort, the shared job model `or-job`,
and extraction from the app's archive menu behind the edit gate (#156) exist: `openrage-cli bulk`
extracts the winning copy of each logical path in DLC order across an install, and the app extracts
from an open archive. **Closed** by the owner on 2026-10-01: extraction from an open archive answers
the row, and install-wide jobs stay in `openrage-cli bulk`.

**Traps.** ⚠ A resource's on-disk header slot holds filler, so a raw-extracted `.ydr` loads nowhere;
`BodyForm::is_loadable_for` decides which body forms are safe. A per-item failure is counted, never
propagated, and an interrupt stops the run with the count so far. `bulk` works above the VFS and
`rpf extract` beside it (`which_side_of_the_vfs_each_job_is_on`); `--every-copy` is deliberately
absent.

**Covers.** U-033, U-034.

### R22 Keys

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**Done.** A SHA-1 fingerprint search finds the AES key, the hash table and the AWC key in the user's
own executable. NG material comes from a key snapshot: the user's own (`magic.dat` beside the keys
cache, `--magic` or `OPENRAGE_KEY_SNAPSHOT`), else the one `or-io` bundles behind its default
`bundled-keys` feature (D18), which decodes only with the AES key from the user's executable. A key
set records and reports where each class came from, and a snapshot that disagrees with the
executable loses its NG keys. A fingerprint search survives a payload moving between builds, not
changing. The figures are in [Keys](OR_IO_Specification.md#keys-d4).

### R23 Names and hashes

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**Done.** A per-title reverse index over both hash functions, with collisions recorded, `.nametable`
read and written, and `openrage-cli hash` and `names`. `string_hash` is pinned against names the
game writes. `crates/domain/or-particle/src/rage/mod.rs` re-exports `or_io::hash`, so there is one
hash; the literal-hash schema index of `or_io::meta`
(`crates/io/or-io/src/meta/binary/hash_index.rs`) stays separate. Persistence waits on
[R26](#r26-index-dependencies-and-summaries).

The app opens a `.nametable` as `DocumentKind::NameTable` (#436): its names one per line in the
text editor, saved through `or_io::gta5::data::nametable`, with a line that holds a control
character refused at save by its line number (`an_edited_nametable_saves_and_reopens_equal`,
`a_line_that_cannot_be_a_name_is_refused_by_name_at_save`). An entry the reader skips is lost on
save, a [known gap](#deferred-and-known-gaps).

### R24 Search

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**Done.** Content search over an install runs on a worker, streams, stops on request and exports in
bulk; `openrage-cli search --verify` parses every file and groups refusals by message shape.
Overlay scope is open work, unblocked by [R49a](#r49a-overlay-first-resolution).

### R24b Go to file

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** `Ctrl+P` finds any file of the active install and of the open folders by name, fuzzily,
or by hash, and opens it; another key reveals it in the tree instead. A scope narrows the search to
the base game or the mods overlay.

**Where it stands.** Done. `Ctrl+P` opens Go to File, VS Code's Quick Open
(`src/ui/panels/command_palette.rs`): one box that matches files by name, fuzzily and the ones
opened from it first, or by a hex or decimal hash of a name or of its stem under the case-folding
`string_hash`, and that becomes the command palette after a `>`, which `Ctrl+Shift+P` opens it on.
`Enter` opens the highlighted file through R24c's `walk_to`, opening every archive on its path as
the tree does, and `Shift+Enter` or a Shift-click reveals it in Files instead. All, Base game and
Mods overlay narrow the list to everything, to the install outside its `mods` folder, or to inside
it. The box reads the file index (`src/state/file_index.rs`): every file of the active install and
of each install, folder and archive open in Files, nested archives included, walked off the UI
thread through `or_title::install::crawl` and streamed, so a file is found before the walk ends,
with the walk's progress where the results are listed; a first walk is the File index job of the
background jobs, and stops from there. A tree edit, a save, and each opening of the box walk a root
again, reopening only the archives on disk whose size or time changed. "Search every archive" reads
the same index, per document, so two expanded archives' searches both arrive and an install's
reaches inside its archives; it draws its first thousand matches, and is no longer a job of its
own. A file already open focuses its tab, as R59l's opens do. `src/state/going_to_a_file.rs` holds
the three clauses on a temporary install opened cold, the adder two archives deep, with the fuzzy
ranking, the scope, both searches and a file found once its save lands; each failed before its fix.
An edit inside an archive reaches the index once it is saved. Seen on `just shots` screens, not in
a window.

**Done when.** From a cold start on an install, `Ctrl+P`, "adder" and `Enter` open `adder.yft`, a
hash finds the file it names, and a result can be revealed in the tree.

### R24c Search results that open

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** A row of the Search panel, or of a search inside archives, opens its file, reveals it in
the tree and extracts with the rest of the selection.

**Where it stands.** Done. A row of the Search panel, or of the list "Search every archive" turns
the tree's filter into, opens on a double click, and its menu offers Open, Reveal in tree, Extract…
and Copy path. Each goes through `AppState::walk_to` in `src/state/navigation.rs`, the walk a
favourite takes: it opens the install or archive document the hit is in, then every folder and
archive on its path as expanding them in the tree does, a step a frame, and then reveals the row,
opens it through the tree's own open (`open_rpf_entry`), or extracts it. A result list takes Ctrl and Shift
clicks as the tree does, and Extract… writes every selected result in its decoded form through the
extraction job, which asks once whether to replace files already in the folder. Reveal in tree
clears the tree's filter and brings the Hierarchy panel forward, so the row it selects is drawn. The
cases in `src/state/search_results_that_open.rs` click the rows through real frames, each hit
inside an archive the tree had not opened.

**Done when.** Every result row offers Open, Reveal and Extract, and a test opens a result through
the call the row makes.

### R25 The edit mode gate

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**Done.** Every write inside a game install asks `EditMode` (`ReadOnly` by default, `Overlay`,
`Everything`), shown in Settings and the menu bar; `the_gate_stands_in_front_of_every_write` derives
the write paths from the source. The gate asks install membership, with the `mods` overlay carved
out; writing outside an install is always allowed, and there is no persisted "never ask again".

### R25b Explorer ergonomics

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** View modes, independent sort and group-by, a live filter escalating into global search,
history, breadcrumbs, recents and favourites, a preview pane, Windows file associations, a path
argument, and drag in and out with Explorer. The file associations and the path argument are this
row's, not [R43](#r43-the-external-surface-and-open-with)'s.

**Done when.** Someone who used OpenIV yesterday can navigate without instructions.

**Where it stands.** Every clause of the What is in the tree (#181, #190, #198), each held by a test
that drives real egui frames and reads what they wrote, and `just self-check` draws each view and
the preview pane. File associations write only under the current user's classes key, only when
Register is pressed, and journal every value so Undo restores it. Open: the Done-when, a person's
judgement, and the shell halves nothing headless reaches (`SHDoDragDrop` to Explorer, the cursor
during a real drop, Open With after a real registration, the mouse side buttons, the command-line
path).

**Covers.** U-009, U-028, U-029, U-030, U-031.

### R25c A file no reader opens

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** What the explorer does with a file none of OpenRage's readers names: a binary `.dat` with
no reader, an extension nothing knows, an archive entry the tree greys. Refusing it left a user no
way to look inside, and a text file with an unfamiliar extension, such as `preload.list`, no way to
read it.

**Done when.** Every file the explorer reaches opens: one no reader names opens read-only as its
bytes, and one whose bytes are text opens as editable text.

**Where it stands.** Done. A file `DocumentKind::from_path` names no kind for, or a binary `.dat`
that `TextDocument::open` refuses, opens read-only as `DocumentKind::Hex`
(`src/state/document/hex.rs`, #438): a banner saying why no reader took it, the file's size, its
leading magic where that is known, and offset, hex and ASCII rows. A save is refused and the New
menu never offers the kind. It opens so from disk, from a drop, from history and from an archive
entry's double-click or Open (`a_binary_dat_with_no_reader_opens_as_hex_under_its_refusal`,
`an_archive_entry_no_reader_opens_opens_as_hex` in `src/state/opening_an_unknown_file.rs`). A text
`.dat` opens as text (#421), and a file of any unknown extension whose bytes start with a UTF-16
byte order mark, or are valid UTF-8 with no NUL in their first 8 KiB, opens as editable text and
saves its bytes back (#445, `an_unknown_extension_holding_text_opens_as_editable_text`).

- Latin-1 text with an unknown extension still opens as bytes: the text editor's Latin-1 fallback
  decodes any bytes at all, so it cannot tell such text from binary.
- The archive tree still greys an entry no reader names; only opening one is no longer refused.

### R25d The Files view in the centre

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** Browsing gets the main area of the window: an editable address bar with breadcrumbs;
Back, Forward, Up and Refresh; Details columns for the name, the type in words, the size, the stored
size, compression, encryption, the edition and the resource version; Icons with real thumbnails; a
preview pane that draws a file the way its own view does; a status line of items, selection and
size; full keyboard operation (the arrows, `Enter`, `Space`, `F2`, `Delete`, `Ctrl+A` and
type-ahead); and a context menu with Open, Open with, Reveal in Windows Explorer, Copy name or hash,
Replace, Export as and Properties. New folder asks for its name.

**Where it stands.** Every clause but Open with and Replace is in the tree; the owner's session
remains. Browse's centre is the Files view (`src/ui/panels/hierarchy/files_view.rs`) of the folder
last gone to in Files, which stays the tree: an address bar of steps that takes a typed path
(`Ctrl+L`); Back, Forward, Up, and Refresh for a folder on disk; Details with the eight columns,
each sorting from its header; Icons with each name wrapped whole and a picture made off the UI
thread, a dictionary's first texture, an image or a model's render; a status line; and a preview
pane that draws a texture, a model and text as their views do, and hex for the rest. The arrows,
`Enter`, `Space` (the preview pane), `Ctrl+A` and type-ahead act under the pointer, as every
panel's keys do; `F2` and `Delete` run through R25e on the keys the user bound them to, the key
dispatcher's, and the panel reads neither ([R59u](#r59u-small-faults-in-the-editor-and-the-previews)). A row's menu adds Reveal in Windows Explorer, Copy
name, Copy hash, Export as… and Properties, and New folder edits its name in place
(`src/state/the_files_view_in_the_centre/`). Nothing opens a row in another program or replaces an
entry's bytes yet, so Open with and Replace are not offered; a model's render is untextured. Seen
on `just shots` screens, not in a window.

**Depends on.** [R59k](#r59k-the-shells-regions), for the layout it sits in.

**Done when.** [R25b](#r25b-explorer-ergonomics)'s: someone who used OpenIV yesterday navigates
without instructions, judged on `just shots` screens and one session of the owner's.

### R25e Destructive file operations ask and can be recovered

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** Deleting a file or folder on disk sends it to the Recycle Bin after a confirmation that
says how many items and how much; rename, move and import check for a clashing name before they
touch the disk; extraction into a folder that holds the same names asks Replace, Skip or Keep both,
once per job.

**Where it stands.** Delete… in the archive tree's menu asks first, saying how many files and
folders go and how much they hold, the files of a folder the tree has not scanned included. A row
on disk goes to the Recycle Bin through `SHFileOperationW` (`src/platform/recycle_bin.rs`), and
outside Windows the question says the delete cannot be undone; a row inside an archive leaves the
session at once and the file on disk when the archive is saved, and the question says so. The edit
gate still allows every write outside an install, so for a mod folder opened with Open Folder the
question and the bin are the protection. Rename, move and import ask the tree and the disk whether
the name is taken before anything touches the disk, and leave both files as they were when it is.
An extraction into a folder that already holds some of its files asks once whether to replace
them, skip them or keep both (the new copy as `name (2).ext`), and the answer holds for every
clash in that job. `destructive_file_operations_ask` holds all three; its delete lands in a
stand-in for the bin, so no test reaches the user's, and the real call was checked by hand on a
folder under the temporary directory. A selection of several rows still cannot be deleted at once.

**Done when.** A test deletes a loose folder through the tree and finds it in the platform's recycle
bin, a rename onto an existing name leaves both files as they were, and extracting over an existing
file asks.

### R25f Copy cut and paste

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** Copy, cut and paste for files between archives and folders and with Windows Explorer, for
values between fields of the same type, and for records and items between documents.

**Where it stands.** Done. Edit ▸ Cut, Copy, Paste and Delete are registry commands again, on
`Ctrl+X`, `Ctrl+C`, `Ctrl+V` and `Delete`, which a text field with focus keeps
([R59i](#r59i-shortcuts-by-character-live-while-typing)). They act on what the focused panel has
selected: the rows of Files or of the Files view in the centre, or the records ticked in a metadata
file's form; with nothing to act on each is disabled with its reason
(`the_edit_menu_offers_no_command_that_does_nothing`). A row's menu in Files and in the Files view,
a selection's menu and the Files view's empty space offer Cut, Copy and Paste with their keys.
Copied rows are held as files (`src/state/clipboard/mod.rs`): a row on disk as its own file, a row
inside an archive written out under the temporary folder, a nested archive as one `.rpf` with what
it holds. On Windows they go on the clipboard as a file list Explorer pastes, and Paste takes the
file list Explorer copied after OpenRage did (`src/platform/clipboard.rs`, which a test build
replaces with a stand-in, so no test touches the desktop's clipboard). A paste is an import: it asks
the edit gate about the destination and, for a cut, the source, refuses a name already taken, and
undoes as one step. A cut row moves where the tree can move it, and otherwise leaves its archive or
folder once its copy has landed, a row on disk into the Recycle Bin. A row of the Inspector's grid
offers Copy and Paste of its field, a vector or a colour whole, over a field of the same parts,
refusing another with a sentence, as one step of the grid's own writes (`AppState::patch_scalars`).
Ticked records copy from the form or Edit and paste after the last record ticked in another file,
refusing a type the array does not hold, and Edit's Delete takes ticked records out.
`src/state/copy_cut_and_paste/` holds the Done-when's three tests
(`a_file_copied_in_one_archive_pastes_into_another_and_into_explorer`,
`a_vector_copied_from_one_field_pastes_into_another`,
`a_metadata_record_copied_from_one_file_pastes_into_another`) and the cut, gate, nested archive,
Explorer, menu, key and undo tests; each failed before its fix. The desktop's clipboard has not been
tried by hand, a copy of more than 512 MiB of rows inside archives is refused, an archive with
unsaved edits is saved before it can be copied, and records are copied rather than cut. Seen on
`just shots` screens, not in a window.

**Done when.** A file copied in one archive pastes into another and into Explorer, a vector copied
from one field pastes into another, and a metadata record copied from one file pastes into another.

### R25g The edit mode visible and honest

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** The edit mode in the menu bar is a control that offers the three modes with a sentence
each. Choosing Read only takes back a base-game "Yes" given earlier in the session. The mode has a
Settings page of its own, with the overlay, the archive encryption on save and the export fixes,
instead of sitting under Keys & Encryption. A refused write offers to change the mode, and "Show in
mods overlay" joins "Copy to mods overlay", [R49b](#r49b-the-overlay-as-the-place-you-write)'s
unbuilt half. R25 persists no "never ask again", and neither does this.

**Where it stands.** Done. The menu bar's mode is a menu (`edit_mode_banner`) offering the three
modes, each with its sentence and the current one marked (`src/ui/panels/edit_mode.rs`). The menu
bar, Settings and a refusal's offer all change the mode through `set_edit_mode`, which takes back
the session's base-game Yes and withdraws a base-game question still waiting, and the gate consults
the Yes only once the mode permits the write
(`a_base_game_yes_ends_when_the_menu_bar_switches_to_read_only`,
`a_session_yes_never_outlives_the_mode_it_was_given_in`). Settings' Editing page holds the mode,
the overlay, and the archive encryption and export fixes under "When saving and exporting"; Keys &
Encryption holds only keys. A refused write also raises a prompt under the menu bar's control that
offers the modes (`a_refused_write_offers_to_change_the_mode`); choosing one does not retry the
write. An archive whose overlay copy exists offers Show in mods overlay in place of Copy, which
selects the copy in the tree (`a_file_on_disk_is_revealed_in_the_tree_that_holds_it`).

**Done when.** A test grants a base-game write, switches to Read only and is refused, and the mode
changes from the menu bar.

### R25h Every add lands where the tree shows it

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** New file, New folder and Copy into archive add to the folder or archive the row shows, whether
it is a folder on disk, an archive file on disk or an archive inside one, and what they add is
written at once or counted as unsaved until a save writes it.

**Where it stands.** Done. A row adds into a folder on disk only when it is one: an archive on disk
has a path too, its own file's. New file, New folder and Copy into archive on a folder on disk, in an
Open Folder document or an install's tree, write at once, through the edit gate and
[R25e](#r25e-destructive-file-operations-ask-and-can-be-recovered)'s refusal of a name the tree or
the disk already holds (`claim_name`), and New file names its blank `untitled` with a number when
either holds that name, the folder's unscanned files included. On an archive row, whether its
`.rpf` sits on disk or inside another archive, the three open it first, as
[R20c](#r20c-saving-twice-and-into-an-archive-not-yet-opened) does, and what they add stays in the
session, counted unsaved, until a save writes it. A rebuilt archive keeps an empty folder, which it
dropped before because no file's path named it (`RpfBuilder::mkdir`).
`src/state/every_add_lands_where_the_tree_shows_it.rs` runs the three commands on a folder on disk,
an archive on disk and an archive nested in one; four of the nine failed before the fix: New file
in the folder stayed in memory, New folder and Copy into archive on the archive on disk failed with OS
errors 183 and 3, and the save dropped New folder in the nested archive. A move onto an archive row
on disk lands inside it since [R20f](#r20f-a-moved-or-deleted-archive-row-leaves-nothing-behind).

**Done when.** A test runs New file, New folder and Copy into archive on a folder row on disk, an archive
row on disk and a nested archive row, and finds each result on disk or, after a save, in its
archive.

### R25i A right-click or a drag selects its row

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** A right-click or the start of a drag in Files or the Files view selects the row it lands
on, as Explorer does, and a drag or a drop gives Files focus, so F2, Delete, Ctrl+C, Ctrl+X, Ctrl+V
and Undo afterwards act on that row.

**Where it stands.** Done. A right click on a row of Files or of the Files view that is outside the
selection selects that row alone before its menu reads the selection, as Explorer's does, and one
inside it keeps the whole selection, so a menu on several rows still acts on them all
(`select_unless_selected` in `src/ui/panels/hierarchy/rows.rs`, from `rpf_context_menu`). The start
of a drag selects its row the same way and gives the panel it is in focus, Files for the tree and
the Document panel for the Files view, and files dropped from Explorer onto either give it focus, as
a click there does (`start_drag`, `focus_on_drop`). So F2, Delete, Ctrl+C, Ctrl+X, Ctrl+V and Undo
afterwards act on that row: with two trees listed and the selection in the other,
`a_right_click_on_a_row_not_selected_selects_it_and_f2_renames_it` right-clicks a row in Files,
renames it with F2 and finds Ctrl+Z undoing that rename and not the other tree's;
`a_file_dropped_onto_files_gives_files_focus` and
`the_start_of_a_drag_selects_its_row_and_gives_its_panel_focus` hold the rest
(`src/state/a_right_click_or_a_drag_selects_its_row.rs`). Each of the three failed before the
change. `promote_pending_rename` takes its rename without an `unwrap`. A document's own row in Files
and the Files view's empty space still select nothing on a right click. Seen on `just shots`
screens, not in a window.

**Done when.** A test right-clicks a row that is not selected, finds it selected, presses F2 and
renames it; and a test drops a file onto Files and finds Files focused.

### R25j A drag carries every selected row

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** A drag of a selected row onto a folder moves or copies every selected row, as a drag out to
Explorer already takes them all, and a drag never opens a rename.

**Where it stands.** Done. A drop onto a folder row in Files or the Files view moves the rows the
drag carries (`maybe_drop_into_folder`): the selection when it holds the row dragged, the rule the
drag out to Explorer now shares (`Selection::dragged`); let go over one of those rows, the drag
went nowhere. They move through `src/state/rpf_moves.rs`, which a single move (Move to, a cut's
paste) now takes too: a row inside another goes with it; a drop into a folder inside a row it
carries is refused, naming the row, before the edit gate asks; the gate is asked once for the
folder and every archive or folder the rows leave, so one Yes moves them all and a refusal moves
none; each row then moves as a single move does,
[R20f](#r20f-a-moved-or-deleted-archive-row-leaves-nothing-behind)'s bookkeeping included, and the
moves inside archives undo as one step. A row refused on its own, by a name the folder holds or, on
disk, by a job reading it (`no_job_holds`, since
[R20i](#r20i-what-archive-rows-still-get-wrong-after-r20h)), or on disk by a job reading the folder
it would land in (`no_job_reads`, since [R60i](#r60i-what-a-write-still-does-not-ask-after-r60h)),
stays where it was and is named with why ("Moved 2
item(s); 1 failed: …"), and the rest move, as a paste's rows do. The start of a drag cancels the
rename a slow click on its row scheduled (`start_drag`).
`src/state/a_drag_carries_every_selected_row.rs` holds the Done-when's two tests,
`three_rows_selected_and_one_dragged_onto_a_folder_all_move_and_undo_together`, in Files and in
the Files view, and `a_row_slow_clicked_and_then_dragged_opens_no_name_editor`, and one each for
the gate, a folder dropped into itself, a name taken and a job reading a row; each failed before
its fix. A drag moves; with Ctrl held at the drop it copies
([R25k](#r25k-a-drag-copies-and-a-change-of-nothing-does-nothing)). Not seen on `just shots` screens, whose scripts
have no drag.

**Done when.** A test selects three rows, drags one onto a folder and finds all three moved, and a
test slow-clicks a row, drags it and finds no name editor open.

### R25k A drag copies, and a change of nothing does nothing

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** A drag onto a folder copies when a modifier says so, as R25j's What says, a selection that
includes the folder being viewed still carries its other rows, and a drop or a rename that changes
nothing writes nothing, asks the edit gate nothing and records no step.

**Where it stands.** Done. A drop onto a folder row in Files or the Files view copies the rows the
drag carries when Ctrl is held at the drop, and moves them otherwise (`maybe_drop_into_folder`,
`rpf_copy_nodes`). A copy stages each row as a file as Copy does, leaves the clipboard alone and
goes in through the import a drop from Explorer takes, so the edit gate is asked once for the folder
and the copies into an archive undo as one step, and into a folder on disk they are written at once
and are no step. A move or a copy of rows onto the folder they already sit in leaves those rows out
before anything is asked (`rows_to_place`): it writes nothing, asks nothing and records no step,
which a Move to into the same folder now shares. A selection that includes the folder being viewed
carries its other rows: `top_rows` takes the root out before it nests the rest, where the root
swallowed every row beside it, and a click on a row in the Files view takes the folder it shows out
of the selection (`leave_folder_shown`), where going into the folder had selected it and a drag or
Copy carried it with everything in it. `rpf_rename_node` returns before the gate when the name is
the row's own, as the editor's own commit of an unchanged name already did.
`src/state/a_drag_copies_and_a_change_of_nothing_does_nothing.rs` holds the Done-when's tests,
`a_row_dropped_onto_a_folder_with_ctrl_held_is_copied_and_one_ctrl_z_takes_the_copy_away` in Files
and in the Files view,
`a_selection_holding_the_folder_being_viewed_copies_and_drags_its_other_rows`,
`a_row_dropped_onto_the_folder_it_sits_in_records_no_step_whether_it_moves_or_copies` and
`a_move_or_a_rename_that_changes_nothing_asks_the_edit_gate_nothing_and_writes_nothing`, and three
more for the root, a viewed folder inside the archive and the gate on a copy; each failed before its
fix, the last with the gate taken out of the copy. Not seen on `just shots` screens, whose scripts
have no Ctrl-click or drag. What is left: nothing on screen says that Ctrl is held during the drag,
as Explorer's copy cursor does; a copy onto the folder it sits in does nothing where Explorer makes
a second file; a drag copy's toast reads "Imported N file(s)" and its history step "Import N
file(s)"; a cut's paste into the folder its rows sit in still asks the edit gate first
(`paste_rows`) and says "Pasted N item(s)", though it moves nothing; and what raised "The edit mode
refused a write" in the two [R26c](#r26c-uses-reaches-every-pack) runs is not found, since the
editor's commit of an unchanged name already did nothing.

**Done when.** A test drags a row onto a folder with the copy modifier held and finds the row in
both places; a test selects the folder being viewed with rows in it, copies, and finds no "Select a
file or folder"; a test drops a row onto the folder it sits in and finds no step in the history; and
a test renames a row to its own name and finds nothing written and the gate not asked.

### R25l What a drag copy still gets wrong after R25k

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** A cut pasted where it already sits, a drag out of a selection holding the viewed folder
and a drag copy's staged files leave nothing wrong behind, and a Ctrl drag says it copies.

**Where it stands.** Found while closing
[R25k](#r25k-a-drag-copies-and-a-change-of-nothing-does-nothing) (#570): a cut pasted into the
folder its rows sit in returns Ok from `rpf_move_node`, so the rows count as landed, the cut leaves
the clipboard and the toast says "Pasted N item(s)" with an empty undo batch though nothing moved,
and `paste_rows` asks the edit gate for the folder and the cut rows' archives before
`rpf_move_nodes` and `rows_to_place` are reached; the drag out to Explorer (`drag_out` in
`src/ui/os_drop.rs`) takes `Selection::dragged` as it is, so a selection holding the viewed folder,
made in the tree and not by a Ctrl click in the Files view, still carries the folder out with
everything in it; `Selection::toggle` sets the anchor to the folder it takes out, which is not in
the listed order, so the next Shift click selects one row where it should span from the last row
clicked; a copy whose gate question is left waiting, or is answered, keeps its staged files under
`openrage-clipboard` in the temporary folder until `staging_dir` sweeps them after a day, as Copy's
staged files do, and the cleanup test's `pending_retail_edit.is_none()` misses the refused case when
an unrelated question was already pending; a drag copy goes through `import_paths_into_folder`, so
its toast and its step read Imported and Import where a verb on `import_paths_into_folder_inner` and
`PendingRetailEdit::ImportPaths` would say Copied; nothing on screen says Ctrl is held, as
Explorer's copy cursor does, which `ctx.set_cursor_icon(egui::CursorIcon::Copy)` in the held branch
of `drag_out` would; a Ctrl drag onto the folder the row sits in does nothing where Explorer makes a
second file, which needs a naming rule the app has none of for archives; and what raised "The edit
mode refused a write" in the two [R26c](#r26c-uses-reaches-every-pack) runs is not found, since a
rename that changes nothing already did not ask the gate. The test of a selection holding the viewed
folder fails before the fix through `leave_folder_shown` and not `top_rows`, whose half a direct
call to `copy_rows` holds; no test undoes a copy of several rows as one step; and `rpf_copy_nodes`
is not among the writes `src/state/the_gate_in_front_of_every_write.rs` lists, so its gate stands
only because it calls `import_paths_into_folder`.

**Done when.** A test pastes a cut into the folder its rows sit in and finds the cut still on the
clipboard and no "Pasted"; a test drags out a selection holding the viewed folder, made in the tree,
and finds the folder not carried; a test Shift clicks after a click that took the viewed folder out
and finds the span from the last row; a test answers a copy's gate question and finds no staged file
left; a test Ctrl drags a row and finds a copy cursor and a toast saying Copied; and a test undoes a
copy of three rows and finds one step.

**Partly closed.** A cut pasted into the folder its rows sit in stays held, says "Already in this
folder" and makes no step
(`a_cut_pasted_into_the_folder_its_rows_sit_in_stays_held_and_says_nothing_moved`), and a Shift click
after the viewed folder, which the listing does not list, was taken out of the selection spans from
the row clicked last
(`a_range_after_the_off_screen_anchor_was_taken_out_spans_from_the_last_row_clicked`), each seen
failing first. A drag copy says Copied and a copy of three rows is one step named for a copy
(`a_drag_copy_says_copied_and_three_rows_undo_as_one_step`), and a drag with Ctrl held shows the copy
cursor while it is on (`a_drag_with_ctrl_held_shows_the_copy_cursor_and_one_without_does_not`), each seen
failing first. A drag out of the window of a selection made in the tree that holds the folder shown
carries its rows and not the folder with all it holds
(`a_drag_out_of_a_selection_made_in_the_tree_that_holds_the_folder_being_viewed_carries_its_rows`: it
carried the one folder before the fix), and a copy's staged files are gone once its gate question is
answered, Yes or No (`a_copys_staged_files_are_gone_once_its_gate_question_is_answered`), each seen
failing first. Every clause of R25l's Done-when holds, and the staged files of a copy are a `Staged`
guard that removes its folder when the last hold on it goes, so a copy the gate refuses outright and a
copy that asks while another question waits leave nothing for `staging_dir`'s sweep
(`a_copy_the_gate_refuses_leaves_no_staged_file`,
`a_copy_that_asks_while_another_question_waits_takes_the_staged_files_of_the_one_it_replaces`, each
seen failing with the guard's removal taken out; the replaced question's files were the one left
behind before the guard).

### R26 Index dependencies and summaries

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** Resolve an asset's full dependency set (textures, TXD parent chain, skeleton, collision) so
"export this" pulls what it needs; resolve a selection back to its file, archive entry and in-game
streaming path; find every world placement of a model by name or hash; build a reusable asset
library from an install.

**Done when.** "Export this" pulls an asset's textures, skeleton and collision; "find every
placement" answers for a model name.

**Where it stands.** `EntryFacts` read the RSC7 version from the entry table, so
`openrage-cli describe` summarises an install in 2.16 s against 370.9 s for reading every header
(#56). A mount plus full name enumeration takes 6.8 s cold and 1.05–1.24 s warm (release build,
both installs), which struck the startup cache. `describe` counts entries and `versions` distinct
logical paths, so on Legacy `**/*.ydr` they report 83,934 and 83,746: 188 paths sit in more than one
DLC pack. An index must descend: 9,801 of 10,142 archives across both installs are nested. The
resolver is `crates/domain/or-depends` and `openrage-cli depends` (#158): over a retail Legacy
install it resolves 663,100 of 695,838 edges, and the file's own `assetType` agrees on 158,665 of
158,665 comparable archetypes (`what_a_dependency_index_derives`); `rules::RULES` states per format
what a file cannot say, and `ModelContents` reads what only a body says. Over the retail Enhanced
install, `openrage-cli depends census` (release build) resolves 664,162 of 696,623 edges from 159,358
archetypes in 1,707 `.ytyp` from entry tables alone, with `assetType` agreeing on 158,953 of 158,953,
at `e29ecb87`. At `3d510dc5` it also reads 7,682 `.ydd` and 13,767 `.ydr` bodies (23 s) and resolves
677,750 of 696,623 edges; over Legacy, 676,601 of 695,838 (58 s). A `physicsDictionary` names a
`.ybn` or `.ybd` in only 334 of 14,159 Enhanced edges; the rest mark a model carrying its own bound
(13,748 of the 13,767 `.ydr` behind them carry one, against 0 of 20,000 sampled behind archetypes
stating none, a one-off count at `e29ecb87`), and no archetype stating one declares a `.ydd`. The
edge resolves to that bound: 13,977 of 14,159 on Enhanced, 13,893 of 14,086 on Legacy. Reading the
declared `.ydd` confirms 76,846 of 76,901 `Found::Entry` edges on Enhanced; the other 55 name
entries missing from the patch-pack copy of three `.ydd` the catalogue resolves to, so their
`assetType` now disagrees, as it does for 55 on Legacy
(`a_model_body_settles_its_collision_and_its_dictionary_entries`). An archetype whose `assetName`
the install holds only as a `.ybn` resolves to that `.ybn`: 391 on Enhanced and 388 on Legacy,
every one assetless and stating a `physicsDictionary`
(`an_archetype_whose_asset_is_only_a_bound_resolves_to_that_bound`). Of the 182 Enhanced collision
edges that resolved to nothing, 162 were such archetypes, whose `physicsDictionary` names no file
and whose collision is that `.ybn`, so the edge now resolves to it; 19 are a `.ydr` carrying no
bound with no `.ybn` or `.ybd` of the name (two name the model itself), and one is an assetless
archetype whose name no family holds. The edge resolves on 14,139 of 14,159 on Enhanced and 14,052
of 14,086 on Legacy, whose 34 are the same 20, 13 naming their declaring `.ytyp` and one more
naming nothing. `openrage-cli depends census` (release build) then resolves 678,303 of 696,623
edges on Enhanced and 677,148 of 695,838 on Legacy, at `57e4a325`. See
[Asset_Dependency_Resolution_Specification.md](../extra/Asset_Dependency_Resolution_Specification.md).
`openrage-cli depends export` takes an archetype name or an asset path and copies the asset and
every file of its set (the declaring `.ytyp`, the texture dictionaries and their parents, the
collision, the clip dictionary and any `_hi` companion) into a folder, with a manifest giving each
file's edge, the name it was asked for and why it is there. A model's skeleton is inside the model,
so the asset file carries it. A set with an edge the install does not hold, or a file that will
not open, writes nothing and names each missing edge
(`an_export_writes_the_whole_set_with_a_manifest`,
`an_export_missing_one_piece_refuses_and_names_it`, over a synthetic install).
A map's static bounds are named by no archetype, and the only file naming them is
`_manifest.ymf`, whose `MapDataGroups` gives an imap group's `.ybn`: on Legacy 409 groups (388
names; the 21 repeats are identical copies) name 2,549 bounds, and on Enhanced 401 groups name
2,114; every group name is a `.ymap` and every bound a `.ybn` the install holds. A `.ymap`'s own
`physicsDictionaries` lists the physics dictionaries of the archetypes it places, not its bounds:
it names a `.ybn` in 583 of 46,696 entries on Legacy (583 of 46,661 on Enhanced), never one of
the map's own. Every other map's `.ybn` streams by its own extents with nothing tying it to the
map: not the archive (714 of 13,914 Legacy `.ybn` share one with any `.ymap`, whose maps sit in a
separate metadata archive) and not a reference, only the stem (`<map>_N`, `hi@<map>_N`,
`ma@<map>_N`: 12,371 of 13,914 Legacy `.ybn` name a `.ymap` that way), and for the group maps the
stem and the manifest agree on 2,501 of 2,501 Legacy `.ybn` (2,066 of 2,066 on Enhanced).
`Resolver::of_map` resolves a map's static bounds through the manifest
(`a_maps_static_bounds_resolve_through_its_manifest_group`); `openrage-cli depends of
<map>.ymap` reports them, 63 of 63 for `prologue01.ymap` on Legacy. Measured over both retail
installs by a scratch probe at `94ff4041`. A map no group names takes the `.ybn` its stem names,
by the owner's choice of 2026-10-02
([Project_Context.md](Project_Context.md#a-map-no-imap-group-names-takes-its-static-bounds-by-filename-2026-10-02)),
as `MapStemConvention` edges, which `Via::is_stated` refuses, each carrying whether the bound's
header box lies inside the map's `streamingExtents`; one outside is reported and flagged, and a
group map ignores its stem (`a_non_group_maps_static_bounds_resolve_by_filename_stem`).
`openrage-cli depends of <map>.ymap` labels each such edge a convention with that verdict, and
`depends census` (release build, at `bc1df2b0`) counts them: on Legacy, of 11,076 `.ymap`, 388
are groups and 696 others take 10,772 `.ybn` by stem, 10,688 inside the map's streaming box and
84 outside; on Enhanced, of 11,052, 380 are groups and 696 others take 10,770, 10,686 inside and
84 outside. The archetype edges are unchanged at that revision.
No field of an interior archetype names its collision either: `_manifest.ymf`'s `Interiors`
names each interior archetype's `.ybn`, 639 bounds for 388 interiors on Legacy and 644 for 391 on
Enhanced, every interior an archetype the install declares and every bound a `.ybn` it holds;
the 14 interiors two manifests name carry the same list in both. `Resolver::of_archetype` adds
them as `ManifestInterior` edges
(`an_interior_archetypes_bounds_resolve_through_the_manifest`), and `openrage-cli depends of` and
`depends census` report them. Each list holds the interior's own `.ybn`, which its `assetName`
already resolves to, and 251 more on Legacy (253 on Enhanced) that nothing else names, such as
`hi@v_michael.ybn` beside `v_michael.ybn`, so an exported interior now carries them. The census
(release build, at `9a74dca2` with this edge) resolves 639 of 639 interior edges on Legacy and
644 of 644 on Enhanced, and 677,787 of 696,477 edges on Legacy and 678,947 of 697,267 on
Enhanced.

**Open.** Collision edges naming nothing the install holds (20 on
Enhanced, 34 on Legacy) stay unresolved.
Whether the 55 missing `.ydd` entries are the catalogue's shadow order or the install's is not
settled. Since [R26b](#r26b-uses-and-used-by-in-the-app) a row's Uses and Placed by show dependency sets
and placements in the app as well; vehicles and peds, declared in `.meta`, are outside the resolver.

**Risk.** ⚠ A dependency set that is nearly right is worse than none: an export missing one texture
looks like it worked.

### R26b Uses and used by in the app

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** A file's context menu lists what it uses (its texture dictionaries, skeleton and
collision) from the resolver behind `openrage-cli depends`, and, where the CLI's placements answer
it, what places it; either opens.

**Where it stands.** Done. A row of Files or of the Files view whose file is a model (`.ydr`,
`.yft`) or a map (`.ymap`) offers Uses, and a model's row Placed by
(`src/ui/panels/hierarchy/uses.rs`). Uses lists what `or_cli::DependencyIndex::uses` resolves the
file to use, the call `openrage-cli depends of` makes, which now takes an asset path as `export`
does: the declaring `.ytyp`, the texture dictionaries and the ones they fall through to, the
collision, the clip dictionary and the high-detail companions, the file itself aside, each beside
what it is to the file in the words of `depends export`'s manifest. The app calls `uses_checked`,
which `uses` calls after reading a map's boxes, and a map's Uses says beside each bound where it
sits against the map's streaming box ([R26c](#r26c-uses-reaches-every-pack)). A model's skeleton is
inside the model, as R26 found. Placed by lists each map placing the archetype that names the model, with
how often, its first 25 by name, folded through `DependencyIndex::place`, the loop
`depends placements` runs. An item opens its file as Go to File does, through `walk_to`, and Reveal
in tree beside it selects it in Files; a dependency the install does not hold is listed disabled
under its hash, saying it is not in this install. The active install is read once, off the UI
thread, as the Uses and Placed by job of the jobs list (`src/state/dependencies.rs`), the first time
a Uses menu asks, and its maps the first time Placed by asks, and a map's Uses reads that map's
bounds' boxes on a worker, a third read of the same job, the first time it asks
([R26c](#r26c-uses-reaches-every-pack)); until then the menu says it is looking, and another
install, mount or set of keys reads it again, as does a file replaced under the install by a
save, a rename, a move, a delete, a defragment, an import, a New file, a paste, an extraction, an
Export as…, a conversion's or a glTF export's write, or an undo or a redo that writes one
([R60h](#r60h-every-write-asks-the-jobs-reading-it), [R60i](#r60i-what-a-write-still-does-not-ask-after-r60h)).
Over the retail Legacy install a
dev build answered Uses 75 to 90 s after the first menu asked, 66 to 80 s of it reading 7,680
`.ydd` and 13,683 `.ydr` bodies, and Placed by under two minutes after that, listing 31 maps for
the pine tree prop_tree_pine_01 (`just shots`, at `a69b468e` with this change). A file outside the
active install, such as a drawable in a loose folder, has no archetype tables to answer from, and
Uses says so in words. The file index cannot stand in for the catalogue: it lists files in the
order a folder walk meets them, not the mount's, where `update.rpf` and the DLC packs override the
base game and the mods overlay comes first, so a name a later pack ships would resolve to a copy
the game never loads; and the type files, manifests and model bodies are read through the mount
anyway. `src/state/uses_and_used_by_in_the_app.rs` builds an install whose `.ytyp` and `.ymap` are
PSO written from a schema it declares, so it runs with no schema snapshot:
`a_drawables_uses_lists_its_texture_dictionary_and_opens_it` finds the menu looking, then the
texture dictionary listed among exactly the files `openrage-cli depends export` copies with the
drawable, and opens it; `a_dependency_the_install_does_not_hold_shows_disabled_with_the_reason`,
`a_drawable_in_a_loose_folder_says_why_uses_cannot_answer` and
`placed_by_lists_the_map_placing_a_drawable_and_opens_it` hold the rest. Each failed while the
menus offered no Uses. Seen on `just shots` screens, not in a window.

**Done when.** A drawable's "Uses" lists its texture dictionary and opens it.

### R26c Uses reaches every pack

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** An item of Uses or Placed by opens a file a DLC's sub-pack holds, and a map's Uses says
which bounds its streaming box takes in.

**Where it stands.** Done. or-title mounted a DLC's `dlc1.rpf` and its later sub-packs on the
pack's device with no label, so `Vfs::locate` answered a file in one with its path inside the
sub-pack alone, which no walk from the install root finds: Uses and Placed by listed the file, and
its click said it no longer existed. A sub-pack's device now carries the container it came from,
`x64/dlcpacks/<pack>/dlc1.rpf` on disk or `update/update.rpf/dlcpacks/<pack>/dlc1.rpf` for a pack
nested in the title update, and a nested sub-pack's `MountRoot` names that whole chain where it
named only the part inside its holder
(`crates/title/or-title/tests/a_dlc_sub_packs_files_say_where_they_sit.rs`).
`src/state/uses_reaches_every_pack.rs` builds an install whose `dlc1.rpf` holds a texture
dictionary and a map: `a_file_a_dlc_sub_pack_holds_opens_from_uses` opens the dictionary from the
drawable's Uses, beside the files `openrage-cli depends export` copies, and
`a_map_a_dlc_sub_pack_holds_opens_from_placed_by` opens the map from Placed by. A map's Uses now
says beside each convention bound where it sits against the map's streaming box, in the words
`depends of` prints (`or_depends::export::verdict`): inside it, OUTSIDE it, or not cross-checked
where a body did not read. `DependencyIndex::uses` reads the map's streaming box and each bound's
header box itself, which `depends of` did in a step the app lacked, and `uses_checked` with
`map_boxes_of` split the two so that the menu reads them on a worker, off the UI thread: a third
read of the Uses job, in the jobs list, begun the first
time a map's Uses asks, again for another map, and again after a file under the install is
replaced or written ([R60h](#r60h-every-write-asks-the-jobs-reading-it),
[R60i](#r60i-what-a-write-still-does-not-ask-after-r60h)).
`a_maps_uses_names_each_bound_with_where_it_sits_against_its_streaming_box` finds a bound inside,
one outside and one whose body does not read, each named with its verdict. Each of the five tests
failed before the fix: the sub-pack's file located to its bare path, an item of either menu
toasted that its file no longer existed and opened nothing, and the map's Uses listed the bounds
with no verdict. Seen on `just shots` screens over those installs, before and after, not in a
window: the item's tooltip read the bare name and now reads
`x64/dlcpacks/testpack/dlc1.rpf/props_txd.ytd`, and its click opens the dictionary in Texture.
A map's first Uses read each of its bounds' bodies whole when this row was done, which was a pause for a map
with many; [R26d](#r26d-uses-over-a-retail-install) reads the front of each bound's file and measures it over both retail installs.

**Done when.** A test opens a file a DLC sub-pack holds from Uses, and a test finds a map's Uses
naming each bound with its verdict.

### R26d Uses over a retail install

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** Uses and Placed by open a file in any sub-pack of a retail DLC, whichever device a path
was written against, and a map's first Uses reads only the headers of its bounds.

**Where it stands.** Done. The DLC sub-pack loop in `crates/title/or-title/src/vfs/game_mount.rs`
mounted each `dlcN.rpf` on `{device}:` only, while `dlc.rpf` is mounted on `{device}:` and
`{device}CRC:`, so a path a content file writes against the CRC device found nothing a sub-pack
holds. A sub-pack now answers to both, and `locate` names the same archive by either
(`a_file_in_a_sub_pack_opens_by_a_path_written_against_the_crc_device`, for a pack on disk and one
inside the title update, which failed first with the sub-pack's file not found by the CRC device).
Over the retail installs on this machine, at `5cd44191` with this change,
`every_file_a_real_dlc_sub_pack_holds_answers_to_the_crc_device` finds each of the 483 files in the
`dlcN.rpf` of 4 packs of Legacy and each of the 468 in 4 packs of Enhanced by the CRC device, the
same archive as by the plain one. A search hit inside a sub-pack nested in the title update opens,
and needed no change: the Search panel's hits come from the crawl of the install (`crawl`), which
never read `MountRoot.container` and names the whole chain of archives, so [R26c](#r26c-uses-reaches-every-pack)'s
change to it reached `openrage-cli describe` and `search --mounted`, whose containers
`crates/title/or-title/tests/a_dlc_sub_packs_files_say_where_they_sit.rs` holds, and not the panel's results;
`a_search_panel_result_inside_a_dlc_sub_pack_in_the_title_update_opens_from_its_rows_menu` finds the
hit naming that chain and opens it from the row's menu. There was no defect for it to fail on, so
it was seen failing under two changes to its fixture, each put back: with `notes.txt` in the pack's
`dlc.rpf` and not its `dlc1.rpf`, the hit names `dlc.rpf` and the assertion on the chain fails, and
with `update.rpf` replaced on disk after the search by an archive that lacks the pack, the row's Open
opens nothing and the app says the file no longer exists. A map's first Uses now
reads the front of each bound's file and not the whole: `Vfs::open_head` reads no more than a limit
of a file where its device can, `ArchiveDevice` reads that much of a resource's stored body
(`RpfArchive::read_prefix`), a file cut short opens to the front of its payload as
`Rsc7::open` always kept what a cut stream inflated to, and `ModelContents::read_bound` takes the
box from the first 4,096 bytes. `a_maps_first_uses_reads_each_of_its_bounds_for_its_header_alone`
(`src/state/uses_over_a_retail_install.rs`) mounts a recording device over the install's archive,
asks `DependencyIndex::map_boxes_of` for a map whose two `.ybn` hold 100,000 bytes past their
header, and finds each read once, for under a tenth of the file, with their inside and OUTSIDE
verdicts unchanged; it failed first with `plain_map_0.ybn was read for 105801 bytes`. Over the retail
installs, at `5cd44191` with this change,
`every_bound_of_an_install_has_the_same_root_from_its_front_as_whole` reads every `.ybn` both ways
and finds the same root in each: 15,130 of 15,130 on Legacy, 57.8 MiB read from the front against
2,503.9 MiB whole, in 11.4 s against 83.0 s, and 14,695 of 14,695 on Enhanced, 56.1 MiB against
2,424.6 MiB, in 10.7 s against 64.8 s (a dev build on a machine other builds shared, so the seconds
are indicative, and the bytes are lengths returned, a whole file inflated against at most 4,096
bytes of a stored front, so their ratio is no measure of what a whole read costs from the archive).
The front is read from the start of the deflate stream, where
`dat_decompress_expecting` also tries a payload behind eight opaque bytes and a stored one, and
that measure is what shows no `.ybn` of either install is one of those. Not measured: a retail
map's whole first Uses, which also reads the install first.

**Done when.** A test mounts a sub-pack and finds a file in it by a path written against the CRC
device; a test opens a search hit inside a nested sub-pack; and a test asks a map's Uses and finds
each bound's file read for its header alone.

### R26e What Uses over a retail install still gets wrong after R26d

[↑ checklist](#phase-3--the-explorer-core-openiv--codex--rpfkit-parity-floor)

**What.** A map's Uses reads the front of a bound wherever its device can, falls back to the whole
file where the front gives no root, and what R26d measured is held where a merge reaches it.

**Where it stands.** Found while closing [R26d](#r26d-uses-over-a-retail-install) (#561):
`FileDevice::open_head`'s default opens the whole file and cuts it, so only `ArchiveDevice` reads
less, and a loose or overlay `.ybn` on a folder device is read whole for its header and saves
nothing; where the front gives no root, a `.ybn` cut at 4,096 bytes whose stored payload, or payload
behind eight opaque bytes, `dat_decompress_expecting` accepts only at the exact length opens to
neither, the read ends in an error and `add_bound` counts the bound unreadable, so a bound in a
mod's archive silently becomes not cross-checked where a whole read gave inside or OUTSIDE; the row
shows only that no `.ybn` of either retail install is one of those. The doc of `open_head` says it
gives the bytes cut to `limit`, but a resource read from an archive returns at least the 16 bytes of
its header (`read_prefix` takes `limit.max(Rsc7::HEADER_LEN)`) and an entry
decrypted whole, a `.ysc` in an encrypted archive, in full whatever the limit, which cannot matter
for a `.ybn`. The retail equivalence test declares its own `FRONT` of 4,096 beside `BOUND_FRONT` in
`crates/domain/or-depends/src/contents.rs`, which is private and which `ModelContents::read_bound`
reads, so lowering one leaves the evidence on the other; every "reads only the header" assertion
measures the length of the bytes returned and not what the archive was asked for, so a read of the
whole body that is then cut passes them, and nothing pins `read_at` bounded. The two retail-install
suites skip under `just test` and `just ready-full` on a machine with no `OPENRAGE_GAME_ROOT`, as
the skip ledger says, so a run at a merge holds the retail figures only as this row's recorded
numbers.

**Done when.** A test opens a loose `.ybn` through a folder device and finds the bound given its
verdict; a test gives a mod archive a `.ybn` whose payload is stored and finds its bound still given
inside or OUTSIDE; a test asks a recording device what `read_at` was asked for and finds it bounded;
and a test finds `FRONT` and `BOUND_FRONT` one constant.

**Partly closed.** `ModelContents::read_bound_from` reads the whole file where the front gives no
root, and counts a bound unreadable only when bytes were read and hold none
(`a_bound_whose_front_gives_no_root_is_read_whole`, seen failing with the fallback taken out);
`a_loose_bound_is_read_through_a_folder_device_and_given_its_verdict` holds the folder device's
verdict; and `Vfs::BOUND_FRONT` is the one constant `ModelContents` and the retail equivalence test
read. `a_resources_front_is_asked_of_the_file_and_no_more` measures what `read_prefix` asks of the file by what the file can still answer: with it cut short after the front the whole body fails to read and the front goes on (seen failing with `read_prefix` taking the whole body), which needs no recording seam in the reader. Still open: the stored-payload mod archive end to end, which the whole-file fallback serves in principle and no builder here can make.

### R27 Editing on the value tree

[↑ checklist](#phase-4--generic-create-edit-delete)

**Done** (#57): `or_io::write::edit` (`crates/io/or-io/src/write/edit.rs`) grows and shrinks runs of
a described tree and adds and removes children, and a declared record's generated run accessors
insert and remove through it (`crates/io/or-io/src/decl/record.rs`). It maintains `Field`,
`CountAndCapacity` and `MaskedField` counts and refuses `Fixed`, `Inline`, `Unknown`, `Product`,
`PrefixedAboveOne` and `Sum`, whose length the description fixes or computes, and `PtrSlots`,
whose index is an identity. ⚠ `build` refuses a count that disagrees with its run
(`CountDisagrees`) but cannot see a whole-field write to a `MaskedField` count destroying the flag
bits beside it, or a run grown past a `CountAndCapacity` capacity; both files load and misbehave. A
capacity is never lowered, because it sizes the allocation the game expects; an insert preserves
order, because an index into a run may be stored elsewhere in the file; and `remove` and
`insert_at` are exact inverses, so undo needs no command type
(`a_remove_and_an_insert_at_undo_each_other_exactly`). The editing surface is
[R31](#r31-the-structure-analyser-and-the-entry-metadata-pairing)'s grid.

### R27b Viewing never edits

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** Opening a file or drawing any panel never changes a document. A default a panel would fill
in is applied by an action the user takes, and that action is undoable.

**Where it stands.** Done.
`opening_a_file_of_every_kind_and_drawing_every_panel_modifies_no_document` opens the smallest
staged file of each extension the app opens, per target, from the shared and the metadata corpus,
through `AppState::load_file`, draws its workspace and then each registered panel through the
editor's frame with no window, and fails on a document marked modified or one whose save would then
write other bytes; the self-check's `--open` row fails on a modified document too. The two files
the review saw were the inspector's described grid writing back a NaN: they hold NaN in the W lane
of the drawable's bounding box and of the navmesh's matrix and extents, and an egui `DragValue`
turns a NaN into an infinity, and clamps a value past its range, reporting either as an edit with
no input. A panel draws a document's number through `ui::panels::drag_value`, which writes only
what the user changes (`a_value_drawn_as_the_file_holds_it_is_not_written_back`,
`viewing_a_clip_whose_rate_is_past_its_fields_range_edits_nothing`). The particle inspector's empty
keyframe lists and the metadata form's unset fields show as such, and an Add the user clicks writes
the default (`drawing_a_keyframe_list_the_file_leaves_empty_adds_no_keyframe`,
`drawing_the_form_leaves_the_instance_as_the_file_holds_it`). Both panels' edits undo since
[R27c](#r27c-undo-for-every-edit).

**Done when.** The self-check opens a corpus file of every kind, draws every panel, and finds no
document modified.

### R27c Undo for every edit

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** Every edit is undoable, in a history per document that a panel lists: the inspector's
grid, materials, the particle outline and its curves, texture re-encodes, stream replaces, metadata
fields, and an archive session's import, rename, move and delete.

**Where it stands.** Done. Each document holds its own `EditHistory`
(`src/state/edit_history.rs`), and Undo, Redo and the History panel (`src/ui/panels/history.rs`,
one of the registry's panels) act on the active document, or since
[R31d](#r31d-undo-and-the-sliders-follow-the-pinned-inspector) on a focused pinned inspector's and
since [R27k](#r27k-undo-from-files-acts-on-what-files-shows) on what a focused Files shows; a
click on a row of the panel goes
back or forward to it. A step holds only what its edit changed, the side the document does not
hold now, and undoing or redoing swaps the two: a field's old value, a grid scalar's old bytes, a
texture's old pixels, a stream's old samples with the container's old layout and no other
stream's samples, rows taken out of an archive; since
[R27h](#r27h-the-grid-and-undo-survive-an-edit-of-the-contents) an edit of the contents made over a
grid step also holds the image that grid step patched. A metadata edit is a path into the value tree, and
an array's element taken out is put back where it was, the inverse R27 found exact. An edit records
its step through `AppState::record`; `Document::mark_edited` stands for an edit no step holds and
empties the history, which is what typing in the text editor does, since that keeps its own undo.
A save keeps the history and marks the step the file holds, so undoing past it marks the document
unsaved and redoing to it clears the mark; a drag, or typing into one field, is one step until the
pointer lets go and nothing holds the keyboard, and a drop of files into an archive is one step.
Recorded: the inspector's described grid; metadata fields and presets; clip fields and pose keys;
Legacy and Enhanced materials and moving one; the particle inspector, its outline's adds, deletes,
duplicates and binds, and its curves; a texture's re-encode, import, embed and unembed, in a `.ytd`,
a drawable or a `.dds`; a stream's replace; a text table's entries; a movie's import; and an archive
session's added folders and files, imports, renames, moves and deletes while unsaved. ⚠ A write of
an archive empties a history holding rows taken out of one, whose entries the write renumbers; a
rename, move or delete on disk is done at once and is not undone, but for the delete a paste makes
of a cut row, which since [R27m](#r27m-an-undo-finishes-or-says-why) is part of the paste's step. `src/state/every_edit_undoes.rs`
and a test beside each editor's own edit, undo and find the document as it opened, and undoing in
one document leaves another untouched; each failed with the recording, the folding or the saved
mark taken out. A plugin's patch of the resource image is one step of Undo. Not undone: an entry
saved into its archive, which empties the history. Seen on `just shots` screens, not in a window.

**Done when.** For each kind with an editor, a test edits, undoes and finds the document equal to
what it opened as, and undoing in one document leaves another untouched.

### R27d An edit stays where it was made

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** A grid edit writes into the document the inspector shows, and every later save keeps it:
the document's contents hold what the grid changed, so a second save, or a save after another edit,
writes it too.

**Where it stands.** Done. An edit of a document's resource image (the grid's, a pasted field, a
plugin's) is read back into its contents through the format's reader once its gesture ends: a
drag, or typing into a field, writes the image each frame and is read once the pointer lets go and
nothing holds the keyboard (`AppState::seal_edits` in `src/state/undo.rs`); an undo or redo reads
at once. A read parses the whole file, so a drag on a large file pauses once, as it ends, and it
clears the document's texture previews. Whatever needs the contents sooner reads them first
(`Document::catch_up` in `src/state/document/history.rs`): a save, a close's included,
`mark_edited`, baking pose keys, and a payload edit made while an image edit is unread, which is
taken back out, the image read, and made again (`AppState::record`). A save while the image holds
an unsaved edit writes the image, moving only the edited bytes. The grid and its row menu use the
document their rows were read from (`draw_editable_summary` in
`src/ui/panels/inspector/structure.rs`), so a pinned inspector edits the pinned document. ⚠ An edit
that leaves an image its reader refuses stays in the image alone, so only the next save writes it.
`src/state/an_edit_stays_where_it_was_made.rs` saves and reopens a staged Legacy drawable after
grid, material and step-less edits in each order and after an undo, drags and pastes in an
inspector pinned to an Enhanced drawable, and finds a twelve-frame drag read back once. The two
Done-when tests failed before the change, and each other fails with the read, merge, clearing or
choice of document it guards taken out. Not done, and [R31c](#r31c-a-pinned-inspector-shows-its-own-document)'s: a Legacy kind with an
inspector of its own, such as a drawable, draws the active document's grid while the inspector is
pinned to another. The viewport draws every edit made before a save since
[R27f](#r27f-the-viewport-draws-every-edit).

**Done when.** A test edits a field in the grid, saves, edits a material, saves again, and reopens
the file to find both edits; and a test pins the inspector to one document, edits a field while
another is active, and finds the pinned document changed and the active one untouched.

### R27e A field that shapes the data is not edited in place

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** A field whose value describes the shape of data elsewhere in the file (a texture's width,
height, depth, format and mip levels, a buffer's element count and stride, a vertex declaration)
draws read-only in the inspector's grid, with a reason naming where it can be changed, so no edit
leaves the data out of step with what describes it.

**Where it stands.** Done. `or_editor::shapes_data` (`crates/domain/or-editor/src/shapes.rs`)
lists by type and field each declared field the grid would otherwise write that counts, sizes or
lays out data elsewhere in the file: both targets' texture size, depth, format, mip levels and
layout; the Legacy vertex declaration, the Enhanced one's packed word, an Enhanced index size, and a
geometry's primitive type and extra vertices; an animation block's packing and an animation's
frames; a material's value counts, ranges and copies; and the counts and sizes of runs in drawables,
fragments, path regions, cloth, skeletons, bounds and expressions. Each draws as a value whose hover
names where its data changes: the Texture workspace's re-encode or import for a Legacy texture,
nowhere for an Enhanced one, `openrage-cli mesh import` for a geometry's buffers, a save after an
edit in the Animation workspace for an animation.
`every_listed_field_is_one_the_grid_would_otherwise_write` holds the list to such fields;
`every_field_that_shapes_data_draws_read_only_with_a_reason_in_every_corpus_kind` opens the
smallest described file of each declared kind per target and checks the grid of each type its
outline reaches; `the_texture_views_own_reencode_still_changes_a_textures_format_and_size` reads
both edits back from a save. Seen on `just shots` screens, not in a window.

**Done when.** A test walks every corpus kind of both targets and finds every field that shapes data
drawn read-only with a reason, and a test finds that the texture editor's own re-encode still changes
a texture's format and size.

### R27f The viewport draws every edit

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** The viewport, its wireframe and a document's thumbnail draw the document as it stands
after each edit.

**Where it stands.** Done. The viewport's mesh and line caches and the thumbnails
(`MeshCacheKey`, `LinesCacheKey` and `ThumbKey`, in `src/render/viewport/mod.rs` and
`src/render/thumbnails.rs`) are keyed on `Document::payload_epoch`, which counts each change of the
document's contents: an edit of them, an undo or a redo, an image edit once it is read back, and a
new image with the contents decoded from it, as a live reload of a `.ypt` brings. ⚠ An image edit
counts only once read back: a drag in the grid writes the image every frame and the contents once it
ends, so a key on the edit count would build the unchanged contents every frame and then miss the
read. `two_edits_before_a_save_each_draw_the_drawable_again`
(`src/state/the_viewport_draws_every_edit.rs`) computes the three keys as the viewport compares them
for a staged Legacy drawable: unchanged by a frame of a grid drag, changed by its read at the
gesture's end, by a material edit and by undoing it. With the keys on the unsaved mark it failed at
the read, at the second edit and at the undo, and
`a_particle_file_reloaded_from_disk_over_its_edits_is_drawn_again` failed without the reload's
count.

**Done when.** A test makes two edits to a drawable without saving and finds what the viewport
draws rebuilt after each, and its wireframe and its thumbnail as well.

### R27g A field that retypes its owner is not edited in place

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** A field whose value says what kind of thing its owner is, so that an edit makes it
another kind, draws read-only in the grid with a reason: a texture's usage byte and a collision
primitive's type.

**Where it stands.** Done. `or_editor::shapes_data` (`crates/domain/or-editor/src/shapes.rs`) also
lists, each with a reason of its own, the fields that say what their owner is: a Legacy texture's
and texture reference's `resource_type_and_conversion_flags`, which [R39](#r39-textures) keeps
untouched because setting it can make the entry a render target in game; an Enhanced texture's and
texture reference's `usage_data`, which the Enhanced texture's own view already shows as its usage
and keeps read-only; and a sphere's, capsule's, box's and cylinder's `prim_type`, the low byte of
its polygon slot's tag. The grid had drawn no primitive: it read a tag only from an integer field,
so a slot holding a primitive drew as a polygon, a sphere's radius as two vertex indices; it now
reads a tag as `or-io` does, the bits of a float included
(`a_polygon_slot_holding_a_primitive_draws_as_that_primitive_with_its_type_read_only`), so each
primitive draws with its own fields, and its type read-only.
`a_textures_usage_and_every_primitives_type_draw_read_only_with_a_reason_in_every_corpus_kind` walks
the smallest described file of every corpus kind of both targets, and every staged file of a kind
that can hold a primitive, and finds both targets' texture usage and all four primitives' types
drawn read-only with their reason. Before the change the walk found none of the six, and the
`or-editor` test failed at the slot's type, then, with the tag read, at the editable `prim_type`.

**Done when.** A test walks every corpus kind of both targets and finds a texture's usage byte and
every primitive's type drawn read-only with a reason.

### R27h The grid and undo survive an edit of the contents

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** After a grid edit and then an edit of the document's contents (a material, a list, a
bake), the grid's step still undoes, and the declared grid stays drawn and editable.

**Where it stands.** Done. An edit of the contents takes the document's image into its step, kept
only when the step straight beneath was made on it (`made_on_the_image` in
`src/state/edit_history.rs`), and undoing or redoing the edit swaps the image back
(`Document::stepped`), so each grid step meets the image its offsets point into. ⚠ An image built
again from the contents is laid out anew, so a grid step's offsets would point at other fields in
it. A history keeps at most `EditHistory::IMAGES` such images, each a whole file, and forgets its
oldest steps past that. Where an edit took the image, `described_root`, which every grid and
described outline reads, asks for one, and `AppState::seal_edits` builds it from the contents as a
save writes them (`src/state/document/image.rs`); on a large file that build pauses once. Every
recorded edit of the contents takes this path, a material, a texture's re-encode, a particle list's
adds and deletes, and their undos; a bake of pose keys empties the history
([R27c](#r27c-undo-for-every-edit)) and drops the image, so a clip dictionary's grid, drawn since
[R31d](#r31d-undo-and-the-sliders-follow-the-pinned-inspector), asks for a fresh one. `a_grid_edit_and_the_material_edit_after_it_both_undo` and
`the_declared_grid_is_drawn_and_editable_after_a_material_edit`
(`src/state/the_grid_and_undo_survive_an_edit_of_the_contents.rs`) failed before the change; two
tests with no corpus fail with the swap, the keeping or the cap taken out. Seen on `just shots`
screens, not in a window.

**Done when.** A test edits a field in the grid, then a material, undoes twice and finds both
undone; and a test finds the declared grid drawn and editable after a material edit.

### R27i Every view follows every edit

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** The viewport and the thumbnails draw an embedded texture as it stands after a re-encode or
an import, and the Animation workspace draws an edit to any document its preview draws.

**Where it stands.** Done. Each of the viewport's GPU textures (`GpuTextures` in
`src/render/gpu_budget.rs`) keeps the `TextureStamp` of what it was uploaded from: the texture's
format, size and levels and every byte it stores. As the viewport or a thumbnail builds its meshes,
it resolves each texture name once (`bound_texture` in `src/render/viewport/texture_upload.rs`) and
compares the stamp, so a re-encode, an import or the undo of either uploads that texture again and
no other, and the copy it replaces leaves the budget's count, as does one whose name no longer
resolves. In the Animation workspace the mesh key also counts each mesh source's and the skeleton
source's changes (`anim_scene_signature` in `src/render/viewport/retarget.rs`), since the active
document is often none of them.
`a_reencoded_or_imported_embedded_texture_is_uploaded_again_and_no_other`
(`src/state/every_view_follows_every_edit.rs`) re-encodes the embedded texture a staged Legacy
drawable's meshes bind, imports two images of one size over it and undoes each, and finds that
texture's copies for the viewport and a thumbnail uploaded again, its neighbour's kept and the
count true; with the stamp ignored it failed at the re-encode, and with the stored bytes left out
of the stamp at the second import.
`an_edit_to_a_document_the_animation_preview_draws_builds_its_meshes_again` edits a drawable the
preview draws as a mesh source, then as the skeleton's document, while a clip dictionary is active;
it failed at the first edit before the change, and at the last with the skeleton source's count
left out. [R27j](#r27j-the-animation-preview-draws-what-its-sources-hold) then did what this row left:
the preview bound a skeleton's bones only when the clip or the skeleton was chosen (`AnimSession`),
so an edit of the bones was not drawn until that choice changed.

**Done when.** A test re-encodes an embedded texture and finds the viewport's copy built again, and
a test edits a document the Animation preview draws while another is active and finds its meshes
built again.

### R27j The Animation preview draws what its sources hold

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** The Animation preview draws a skeleton source's bones as edited, and a mesh source's
meshes with its own embedded textures, whichever document is active.

**Where it stands.** Done. `AnimSession` also remembers the count of changes of the skeleton
source's document it was built from (`skeleton_epoch` in `src/state/animation_session.rs`, that
document's `payload_epoch`), so an edit of a bone binds the clip to the edited skeleton and poses it
again. ⚠ A drag in the grid writes the image every frame and the contents once it ends, so the
session is built again once, never per frame. The transport belongs to a choice of clip and
skeleton source (`AnimSession::holds_the_transport_for`), and every rebuild for the choice it was
set for leaves the working range and Loop as the user set them, whatever asked for it: an edit of a
bone, Insert key, Clear key, Discard edits, the root-motion toggle, Allow mismatched skeleton, a
mesh source added or removed, a clip field edit, the undo of a pose or a clip field edit, a bake,
the same clip chosen again, Choose automatically landing on the same skeleton. Only the clip's
length and frame rate follow the clip (a range never trimmed follows its length, a trimmed one is
kept inside it), and choosing another clip or skeleton source hands all of it back to the clip's
own. The one thing a rebuild for the same choice does not keep is Loop when it changed the clip's
own Looped flag, the Loop box in Preview settings or its undo or redo: the flag is the newer word,
and the transport takes it (`follow_the_clip` is told `looped_changed`, the clip's flag compared
with the one the transport last took; a clip skipped as a mismatch keeps that one, so a flag edited
meanwhile wins when the clip is allowed again, and a flag ticked and cleared again meanwhile is no
change and the user's Loop stays). `AnimSession::invalidate` asks for a build and nothing more: it leaves where they were the
clip and skeleton source the session was built for, and the choice the transport belongs to. The
mesh key ([R27i](#r27i-every-view-follows-every-edit)) holds the skeleton source and the count the session
was built from (`AnimSession::skeleton_built_from`) as well as the document's current count, which
the meshes themselves are built from. A frame ticks the animation first and draws last, so an undo
in the menu moves the count after the session was built: the meshes of that frame are retargeted
onto the old bones, and are built again the frame the session is, which is the only thing that
moves the key then. Each mesh the preview builds carries the mesh
source it was built for (`MeshUpload::source`), and its texture is resolved from that source's own
embedded textures first (`texture_embedded_in` in `src/state/source_textures.rs`: a dictionary's
chosen entry, else any entry), then from the open `.ytd` files, ranked against that source (its
stem, folder, archive and pins) and not against the active document, which is the subject only
where no mesh source is named ([R27o](#r27o-the-animation-preview-follows-its-sources-fully));
the badge and the status bar report the same resolution. ⚠ The viewport keeps one GPU copy for each name, so two sources that embed one name
with other pixels are uploaded again on every build, and each draw still binds its own.
`an_edit_to_a_bone_of_the_skeleton_source_poses_the_skeleton_again`
(`src/state/the_animation_preview_draws_what_its_sources_hold.rs`) writes a rigged drawable of each
target, edits a bone in the grid while a clip dictionary is active, and finds the bone and its world
matrix posed again once the edit is read back and again at its undo, and the session not built again
for a frame of the drag; it failed at the bone's new place before the change, and at the drag with
the count taken at the patch.
`the_meshes_are_built_again_once_the_session_that_poses_them_is` edits a bone, then undoes it, each
between a tick and the viewport's draw, and finds the mesh key moved by the next tick and by
nothing after it; it failed at the first edit with the key holding only the document's current
count. `an_edit_to_the_skeleton_source_keeps_the_transports_working_range_and_loop` trims the range
and turns Loop on, edits a bone and finds both kept; it failed at the range before the change.
`a_rebuild_that_chooses_nothing_new_keeps_the_transports_working_range_and_loop` does the same for
each rebuild listed above, doing what its panel or command does to the state and finding the
session built again with both kept; with `invalidate` making the choice look new it failed at the
first, Insert key, with the range back at the clip's own.
`choosing_another_clip_or_skeleton_source_hands_the_transport_back_to_the_clips_own` finds both
handed back by a second clip and by a second copy of the rig as the skeleton source, and kept by
the rebuild after it. `a_clip_field_edit_moves_the_transports_length_and_keeps_the_users_range_and_loop`
shortens and lengthens the clip with Loop on, and finds an untrimmed range following its length and
a trimmed one kept inside it; with the rebuild leaving the transport alone it failed at the
shortened clip's range, and with every range counted as trimmed at the lengthened one's.
`the_clips_looped_field_sets_the_transports_loop_when_it_changes_and_not_otherwise` ticks the
field with the Loop button off and finds Loop on, finds a rename leave the button as it was set,
the field cleared turn Loop off over the button, and its undo and its redo each move it; with the
rebuild always putting the button back it failed at the ticked field, with Loop still off. The
unit tests `the_transport_belongs_to_the_choice_it_was_last_set_from` and
`invalidating_a_session_builds_it_again_and_moves_nothing_it_was_built_from`
(`src/state/preview_scene.rs`) hold the session's side of it.
`a_mesh_source_is_drawn_with_the_textures_it_embeds_while_a_clip_dictionary_is_active`
previews the smallest staged `.ydr` of each target whose meshes ask for a texture it embeds, with a
clip dictionary active, and finds each such mesh textured and the badge naming the drawable; it
failed at the first mesh. `two_mesh_sources_embedding_one_name_are_each_drawn_with_their_own_pixels`
re-encodes one copy of a Legacy drawable's texture and fails when the first source answers for every
mesh. **The grey plane** is no mesh drawn without its skin and no bound: a ped's `.yft` holds it, the
one geometry of its first level of detail, a grid of 25 vertices 25 metres on a side at z = 0 skinned
to the root bone alone, and the file's own bounds state the same extent; with no mesh added the
preview draws the skeleton source's own drawable, as its panel says
(`a_peds_fragment_chosen_alone_draws_the_one_plane_its_own_bounds_state` on the two staged Legacy
peds). It is left as drawn by default, since nothing tells such a stand-in from a drawable, and
[R27q](#r27q-the-skeleton-files-own-model-can-be-hidden) offers a box that hides it. When this row was
done no outline or inspector edited a bone (a drawable's bones were listed read-only, and the grid
reached one only through the described outline, which kinds with an outline of their own do not
draw), so a bone edit reached the preview through the paths that write the image, which the test
uses; [R27p](#r27p-a-bones-numbers-can-be-edited) edits a bone in the Inspector now. Seen on
`just shots` screens, not in a window: a prop's barrier previewed beside a clip dictionary drew grey
with its badge reading "0 of 1 · no source", and draws its own texture with "1 of 1"; a ped's
fragment alone draws its plane as it did. No screen showed a bone edit then; R27p's screens show one.

**Done when.** A test edits a bone of the preview's skeleton source and finds the posed skeleton
built again, and a test previews a drawable with embedded textures while a clip dictionary is
active and finds its meshes textured.

### R27k Undo from Files acts on what Files shows

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** With Files or the Files view focused, Undo, Redo and the History panel act on the archive
or folder those panels show, so `Ctrl+Z` after a rename, move, delete or import there undoes that
change and nothing in another document.

**Where it stands.** Done. While Files has focus, Undo, Redo and the History panel act on the
install, folder or archive Files shows (`focused_document` in `src/ui/commands/target.rs`): the
only one it lists, else the one its selection is in, else none. The only one came first because a
row's menu acted on its row without selecting it; since
[R25i](#r25i-a-right-click-or-a-drag-selects-its-row) a right-click selects its row too. The Files view draws the active document, which
they acted on already.
`ctrl_z_in_files_undoes_the_rename_made_there_and_keeps_the_active_models_edit` clicks a row in
Files beside a drawable with an edit active in Model, with another archive holding an edit listed
above it, renames the row with F2, and finds `Ctrl+Z` undoing the rename and `Ctrl+Y` redoing it,
with both other edits kept;
`ctrl_z_in_files_undoes_a_rename_from_a_rows_menu` renames from the row's
menu instead; `the_history_panel_opened_from_files_lists_and_undoes_the_rename_made_there` finds
History opened from Files listing the rename and not the drawable's edit
(`src/state/undo_from_files_acts_on_what_files_shows.rs`). A folder on disk opens as a document of
its own, and an archive in it is part of that document, so a change inside the archive undoes
there; a rename, move, delete or import of a file on disk is made at once and is no step, so Undo
does not take it back and reaches no other document; since
[R27m](#r27m-an-undo-finishes-or-says-why), the delete a paste makes of a cut row is part of the
paste's step and undoes with it
(`a_rename_in_a_folder_on_disk_is_no_step_and_ctrl_z_in_files_leaves_the_models_edit`). Each of
the four failed before the change, and the first two each fail with the other half of the rule
taken out. Seen on `just shots` screens, not in a window.

**Done when.** A test renames a row in Files while a model with an edit is active, presses `Ctrl+Z`
with Files focused, and finds the rename undone and the model's edit kept.

### R27l A change on disk keeps the undo steps beside it

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** A rename, move or delete of a file on disk leaves the history of the install or folder it
is in as it was, as an import or a new folder already does, so the steps recorded inside its
archives still undo.

**Where it stands.** Done. A rename, move or delete on disk is counted by
`Document::changed_on_disk` (`src/state/document/history.rs`), which marks the document from its
tree as `mark_edited` does and leaves the history as it was. A step names its rows by id, never by
path, and only rows at or under an archive the tree has opened, so a change that reaches no such
archive leaves every step finding what it changed. A change whose row is an opened archive, or a
folder holding one (`archives_opened_under`), empties the history where a step could no longer undo
in place: a delete takes the rows its steps name, and a rename or move, which keeps every id while
`moved_on_disk` points the tree's archives at their new files, empties it only while a step holds
rows taken out of an archive, which are outside the tree and still read the old file, as after a
write of the archive. Dropping only the steps that lost their rows would let undoing the rest reach
the saved mark and mark an archive saved that still holds a dropped step's change, and refusing at
Undo would leave such a step above every other.
`a_rename_move_or_delete_on_disk_beside_an_archive_leaves_the_steps_inside_it_to_undo` and
`an_archive_renamed_and_moved_on_disk_undoes_and_redoes_the_rename_inside_it_where_it_now_is`
(`src/state/a_change_on_disk_keeps_the_undo_steps_beside_it.rs`) failed before the change, the
first fails again with the reach check taken out, and the second with any reach emptying the
history;
`a_delete_on_disk_of_a_folder_holding_an_opened_archive_leaves_no_step_that_cannot_undo` fails with
the delete's half taken out or the walk kept to the row itself, and
`an_archive_renamed_on_disk_never_has_a_row_put_back_that_reads_its_old_file` with the taken rows'
half taken out, a nested archive put back then reading the renamed archive's old file.

**Done when.** A test renames a file inside an archive, then renames a file on disk in the same
folder, and undoes the archive's rename.

### R27m An undo finishes or says why

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** An undo or a redo either finishes every part of its step or changes nothing and says
why, keeping the history it can still undo; and undoing a cut and paste puts the cut file back
where it was.

**Where it stands.** Done. A step's parts flip whole or not at all (`flip_whole` in
`src/state/edit_history.rs`): where one cannot, those flipped before it flip back and the archives'
unsaved marks are put back, so the document is as it was. The undo or redo then says why, in a
toast and in a row of the History panel, naming the step and its part's reason, such as a row no
longer in the tree after a Refresh or a defragment (`Swap::why_not`). In an install's, a folder's
or an archive's tree, whose steps name rows by id, that step alone leaves the history
(`EditHistory::dropped`), a redo's with the steps made after it, and the steps before it still
undo. The saved mark stays true: on the dropped step it moves to the step under it, which with the
change kept is what the file holds; under it, it is gone, since no position holds the file once the
change cannot go; above it, it stays. Dropping the step without moving the mark is what
[R27l](#r27l-a-change-on-disk-keeps-the-undo-steps-beside-it) rejected, since the undos left could
reach a saved mark the kept change made false. Any other document empties its history as before:
its steps name places by path or index, which the kept change may have moved. A row on disk cut and
pasted into an archive is part of the paste's step (`Cut` in `src/state/rpf_undo.rs`): undone, its
files are written back where they were from the pasted copy and its rows go back into the tree;
redone, it goes to the Recycle Bin again. Written back, not restored from the bin, which no call
restores from and which outside Windows deletes for good; the copy holds the bytes, in memory until
its archive is saved and in the archive after, so the step holds none. A step that writes on disk
first asks the jobs that read files and the edit gate, and a refusal leaves it in place; the bin's
stand-in under a self-check leaves the file, so nothing is written back.
`an_undo_whose_row_a_refresh_dropped_changes_nothing_says_why_and_the_older_steps_still_undo` and
`undoing_a_cut_and_paste_into_an_archive_puts_the_cut_file_back_where_it_was`
(`src/state/an_undo_finishes_or_says_why.rs`) failed before the change, the first half undoing the
paste, the second leaving the file in the bin; the first fails again with the flip back, the marks
put back, the History panel's row, the step left alone or the saved mark's rule taken out. A cut
folder, a cut into another document's archive, the edit gate, a job reading the folder and the
bin's stand-in each have a test that fails with its part taken out, and
`a_step_that_cannot_finish_leaves_the_saved_mark_where_the_file_stands` holds the mark's rule. Not
done: a file cut from one document's tree into another's archive is written back on undo, but the
first tree lists it only after a Refresh.

**Done when.** A test undoes a step of several parts after a Refresh dropped one of its rows and
finds nothing changed, a sentence naming why, and the older steps still undoable; and a test cuts
a loose file, pastes it into an archive, undoes, and finds the file back where it was.

### R27n What an undo still gets wrong after R27m

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** An undo of a cut and paste puts the file back where it was and lists it in every tree it
left, an undo the edit gate would ask about asks instead of refusing, and a part of a step that
reports it flipped did.

**Where it stands.** Found while closing [R27m](#r27m-an-undo-finishes-or-says-why) (#526): a row
cut inside an archive and pasted into a folder on disk is not undone whole, since Undo puts the row
back in its archive and leaves the pasted file on disk; a file cut from one document's tree into
another's archive is written back on undo, but the tree it left lists it only after a Refresh; an
undo that the edit gate would ask about refuses with a sentence instead of asking;
`SpawnerNames::swap` and `NewEntry::swap` can change something and still return false, which breaks
the contract `flip_whole` relies on to flip a step's other parts back; and the Recycle Bin's test
stand-in renames into `bin/<name>`, so recycling two folders of one name fails in a test.

**Done when.** A test cuts a row inside an archive, pastes it into a folder on disk, undoes, and
finds the row back in the archive and no file left on disk; a test cuts a file from one document's
tree into another's archive, undoes, and finds the first tree listing it without a Refresh; a test
undoes a step the edit gate would ask about and finds the question asked; a test flips a
spawner-name step and a new-entry step and finds false only where nothing changed; and a test
recycles two folders of one name through the stand-in. The clauses hold in the tree, each seen failing first: the two swaps that could change something and say false now say it only where nothing changed (`a_new_entry_that_cannot_come_back_because_its_name_is_taken_changes_nothing`: `NewEntry` put its texture over the entry of that name and said false; `spawner_names_that_cannot_all_go_back_change_none_of_them`: `SpawnerNames` put the names back that it reached before the place that was gone); the Recycle Bin's stand-in numbers a second item of one name as a real bin does (`two_folders_of_one_name_are_both_taken_as_a_real_bin_takes_them`); and an undo or a redo the gate would ask about asks, holding the step, and the answer takes it through (`PendingRetailEdit::Step`, in `undoing_a_cut_asks_the_edit_gate_before_it_writes_the_file_back`); and a cut of an archive row pasted into a folder on disk is undone whole, since a file written into a folder on disk now records a step that takes it back (`Written` in `src/state/rpf_undo.rs`, held by `undoing_a_cut_of_an_archive_row_pasted_into_a_folder_on_disk_puts_it_back_whole`, which also redoes it and finds a file changed since not deleted); and a file cut from one document's tree into another's archive is listed again by the tree it left once the undo has written it back, since the step names that document and the undo has it read the folder again (`Swap::leaves_a_tree_stale`, held by `undoing_a_cut_between_two_trees_lists_the_file_in_the_tree_it_left_without_a_refresh`, which fails by timeout when the read is taken out). All five clauses hold.

### R27o The Animation preview follows its sources fully

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** Choosing another clip while the preview is paused poses it, the texture ranking and the
badge's Pin, Unpin and Attach actions follow the mesh source the preview draws and not the active
document, the level-of-detail switch lists that source's levels, a mesh source that closes
leaves the preview's list, and a Looped edit made while a clip is skipped as a mismatch wins when
it is allowed again.

**Where it stands.** Done, but for one question that is the owner's. The viewport poses its
skinned meshes again whenever `AnimSession::skin_revision` moves (`pose_uploads` in
`src/render/viewport/posing.rs`), where it compared the transport's time, which another clip chosen
or an edit made while the preview is paused leaves where it was. The texture resolution takes its
subject from the mesh source a mesh came from, else from the active document (`subject` in
`src/state/texture_resolution.rs`): the ranking (same stem, detail variant, same folder, same
archive), the pins and the embedded textures are the source's. The badge's Pin, Back to Auto and
its checkboxes act on the documents the preview draws, and show a pin on any of them
(`AppState::textured_documents`); Attach was the session's already and needed nothing.
`tick_animation` forgets a closed mesh source with a closed clip or skeleton source
(`PreviewScene::documents`). `levels` in `src/render/viewport/pick.rs` lists the levels any
drawable the viewport draws has models at, each with the distance of the first that has it, so the
level-of-detail switch shows in the Animation workspace, where the active document is a clip
dictionary. The transport takes the clip's own Looped flag when it differs from what the transport
last took (`AnimSession::looped_changed`), which a clip skipped as a mismatch keeps: a Looped edit
made while it is skipped wins when it is allowed again, as it does while the clip plays (owner,
2026-10-05). No panel edited a bone when this row was done (the Inspector listed a drawable's bones
read-only, and the grid reached one only through the described outline, which `draw_doc_body` skips
for `.ydr`, `.ydd` and `.yft`), so R27j's first Done-when clause was reachable only through paths
that write the resource image, plugins and scripted edits; the owner asked for editing a bone on
2026-10-06, and [R27p](#r27p-a-bones-numbers-can-be-edited) is that row. The grey plane a ped's `.yft` draws is that file's own geometry, which nothing
tells from a drawable, so it stays drawn and [R27q](#r27q-the-skeleton-files-own-model-can-be-hidden)
offers a box that hides it (owner, 2026-10-06). Not part of this row: the viewport's GPU
texture cache key stays the entry and the name, so two mesh sources embedding one name with other
pixels are uploaded again on every build, though each draw keeps its own bind group; and
`try_auto_open_texture_sibling`, `try_open_texture_from_search_paths` and `stem_missing_textures`
still read the active document, so in the Animation workspace a mesh source's own sibling `.ytd` is
not opened for it, while `embedded_bound` and `build_lines_for_active_document` still read it too,
so the drawable row there shows the level-of-detail switch but never Collision, and Wireframe draws
the clip dictionary's lines; and when the mesh sources chosen draw nothing, the viewport draws the
skeleton source's own drawable (unless R27q's box is unticked), which `drawn_documents` leaves out while a mesh source is chosen, so the
badge's Pin and Unpin then reach documents that draw nothing and `levels` lists none of that
drawable's levels.

**Done when.** A test chooses another clip over the same skeleton while paused and finds the new
clip's pose drawn; a test ranks a mesh source's texture with a clip dictionary active and finds the
source's own folder and archive counted; a test closes a mesh-only source and finds it gone from
the preview's list; a test previews a mesh source of several levels of detail while a clip
dictionary is active and finds the level-of-detail switch listing them; and a test ticks a clip's
Looped field while the clip is skipped as a mismatch, allows it again and finds Loop following the
field.

### R27p A bone's numbers can be edited

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** A bone's name, position, rotation and scale can be edited. Selecting a bone (its row under
a drawable's or fragment's skeleton in the Outline, or a bone picked in the viewport) shows them in
the Inspector as fields, the rotation in degrees in the XYZ order: a turn about the parent's X axis,
then its Y, then its Z. An edit derives the default and inverse transforms of that bone and of every
bone under it again, keeps every bone's id, the bone-id table, the bone order and the parents,
redraws the preview at once, is one undo step, and saves. Adding or removing a bone, re-parenting
and skin weights are not part of it: they go through glTF and a 3D tool
([R38](#r38-mesh-interchange)).

**Where it stands.** Done, for the numbers only, which the owner asked for on 2026-10-06; it is the
row [R27o](#r27o-the-animation-preview-follows-its-sources-fully) left to the owner. Before it,
the Inspector listed a drawable's bones read-only and showed no bone of a fragment at all.
`draw_bone` (`src/ui/panels/inspector/bone.rs`) keeps the index, bone id, parent, next sibling and
mirror as read-only rows and makes the name, position, rotation and scale fields. `bone_selected`
(`src/state/bone_numbers.rs`) reads the selection every Outline row and viewport pick of a bone
already makes, so a bone picked in the viewport shows the same fields; `draw_ydr` and `draw_yft`
call `draw_selected_bone`, `draw_ydd` calls `draw_bone` with the entry's index, and the Enhanced
path calls `draw_selected_bone` too, its Inspector body being the described root. A fragment's fields lead the panel, as a drawable's part does, and do not wait below
the fragment's structure. An edit goes through `AppState::edit_bone`: it works on a copy of the
skeleton, derives the rest pose again, swaps the copy in and records the old skeleton as one step
keyed by the bone and the field, so a drag folds into one step, and an undo puts the old skeleton
back whole: the next save writes the bytes it wrote before the edit. The payload edit moves the
document's `payload_epoch`, which [R27j](#r27j-the-animation-preview-draws-what-its-sources-hold)
poses the preview again from. ⚠ **A fragment's other skeletons.** A drawable of a fragment very often
holds the common drawable's skeleton again (the `.yft` lane measured 817 Legacy and 73 Enhanced
of them, and a cloth's own drawable is the common drawable again in 16 and 1), and the file shares
one allocation between them: they read as copies of the common drawable's, and a save writes one as
that alias only while it equals the owner, and as an allocation of its own once an edit makes them
differ (OD-ALIAS-BY-VALUE). An edit of the common drawable's bone alone would have saved each of
them with the old numbers, so a fragment ended up with skeletons that disagree. `edit_bone` edits
every skeleton of the fragment, in each drawable and in the drawable each environment cloth names,
that is equal to the one edited, the same way and as parts of the same step, so one undo puts all of
them back; a skeleton that differs, a child drawable's own rig for one, is left as it was. A
dictionary's entries are edited one at a time: a scan of the staged corpus found no `.ydd` holding
two equal skeletons (12 of the 48 Legacy ones hold any; the 2 Enhanced hold none).
⚠ The rotation fields are Euler angles because nobody types a
quaternion: the XYZ order is the one RAGE names, a turn about X first, then Y, then Z, each about
the parent's axes, and what is stored is the unit quaternion of it (`euler_degrees` reads one back).
Angles are not unique, so at Y = ±90° the X and Z fields trade what they show.
`CrSkeletonData::rebuild_rest_pose` (`crates/io/or-io/src/gta5/skeleton.rs`) is the derivation, and
`add_bone` does not already do it: that takes both transforms as arguments, and nothing derived them
before this row. A bone's default transform is its rotation's three axes, each stretched along the
bone's scale (rotation times scale, so the bone stretches along its own axes, as
`or_anim::Bone::bind_local` composes them), and its translation; its inverse transform is the exact
inverse of the transforms laid down every bone above it and the bone itself, scale included, as
`or_anim::Skeleton` builds it. A chain that scales to nothing along an axis has no inverse, and the
edit is refused with the skeleton as it was. It derives the bone and every bone under it, and leaves
the fourth lane of each column as the file held it, so the default transforms laid down a chain,
times the inverse of the bone at its end, are the identity (the default alone times the inverse is
the identity only for a root). **The three signatures.**
`signature` is a CRC-32 over each bone's degrees of freedom and id in order, `signature_non_chiral`
the same over each bone's id and which of translate, rotate and scale it has, and
`signature_comprehensive` the first one's input with each bone's translation, rotation and scale
lanes after its own. An edit of a number keeps the first two and the bone-id table, which come from ids and
flags alone, and changes the third, which `rebuild_rest_pose` writes again; a rename changes none of
them. `a_skeletons_rest_pose_is_what_its_bones_derive`
(`crates/io/or-io-verify/tests/skeleton_rest_pose.rs`) checks the derivation against the shipped
files: of the 985 skeletons the corpus stages (902 Legacy, 83 Enhanced, 45,689 bones) it wipes each
one's transforms and comprehensive signature, derives them again from the bones and finds every
transform within 1.6e-5 of the file's (the worst, in a jet's skeleton, is one step of a float of
that size) and every signature equal; with the descendants and the signature left out it failed at
132 off, with all 985 signatures differing. ⚠ No staged bone has a scale other than 1, so no
shipped file measures the order scale and rotation compose in, nor whether the engine's inverse
keeps the scale: both are the animation skeleton's, and
`a_scaled_and_turned_bones_rest_pose_is_what_the_animation_skeleton_builds` holds a bone both
scaled and turned to that skeleton's forward kinematics, with a unit test worked by hand beside it
(`a_scaled_bone_that_is_turned_stretches_its_own_axes`); they failed with the scale applied along
the parent's axes and left out of the inverse, the first at 0.40 off. `signature_non_chiral` is kept
by reading what it hashes, not checked against the files.
**The tests** are in `src/state/a_bones_numbers_can_be_edited.rs`, over a four-bone drawable and
fragment of each target and the smallest staged `.ydr`, `.ydd` and `.yft` of each that holds a bone
with bones under it (every Legacy kind and the Enhanced `.yft`; the Enhanced `.ydr` and `.ydd`
staged have one bone or none). `a_bones_position_dragged_in_the_inspector_is_saved_with_its_rest_pose_derived_again`
selects the bone's Outline row, drags the Position field of the drawn Inspector, saves, reopens, and
finds the new position, the whole skeleton within 1e-4 of what the preview's forward kinematics
derive from the bones (every bone under the one moved included), every other bone's numbers equal
to before, every bone's name, id, place in the order and parents kept, the two signatures kept and
the third changed, and the fields within the panel's first screen, where a fragment's structure used
to push them; it failed at the forward kinematics, 0.35 off, with the derivation left out of the
edit, at its one step with no step recorded, and at the fields' place with a fragment's structure
drawn above them.
`renaming_a_bone_keeps_its_id_and_the_bone_tag_table` finds the skeleton saved after a rename equal
to the one saved before but for the name, hidden table included; it failed when a rename moved the
id. `a_rotation_in_degrees_is_a_unit_quaternion_turned_about_x_then_y_then_z` edits 90°, 90°, 0° and
finds the quaternion (0.5, 0.5, -0.5, 0.5) and the axes X then Y puts them at, then drags the field
and finds a unit quaternion; it failed with the angles applied the other way round.
`a_rotation_edited_in_a_bone_poses_the_previews_skeleton_again` turns the root of the rig R27j's
tests preview a quarter about Z and finds the session asked for, the bone posed as a unit
quaternion and its child at (-1, 1); it failed with no step recorded.
`an_undone_edit_saves_the_bytes_the_file_had_before_it` edits each of the four fields, undoes and
saves, and finds the bytes of the save before; it failed with no step recorded.
`a_scaled_and_turned_bones_rest_pose_is_what_the_animation_skeleton_builds` turns a bone about all
three axes, scales it differently along each, saves, reopens, and finds the default's columns as long
as the scales and the whole skeleton within 1e-4 of the animation skeleton's forward kinematics.
`an_edit_a_skeleton_cannot_take_changes_nothing` holds that a number that is not finite, a scale of
zero, a name with a NUL and a bone that is not there change nothing.
`an_edit_of_a_fragments_bone_reaches_each_skeleton_equal_to_the_one_edited_and_no_other` edits a
bone of a fragment of each target whose extra drawable and cloth drawable hold the common
skeleton again and whose other extra drawable holds one of its own, undoes and finds the bytes of
the save before, edits again, saves, reopens, and finds the two copies equal to the new common
skeleton and the third as it was.
`an_edit_of_a_staged_fragments_bone_reaches_the_skeletons_its_drawables_save_as_copies_of_it` does
the same to the smallest staged fragment of each target that holds its common skeleton again
(`slod_human.yft` of Legacy, a prop of Enhanced). Both failed with the edit made to the common
drawable's skeleton alone, at the first copy, and the first failed at the cloth's with the cloth
left out of the edit. Not done: a `.ypt` model's bones (`draw_ypt_model_bone`) are still listed
read-only; the physics children and bounds of a fragment sit by their own matrices and do not follow
a bone moved; a skeleton of a fragment's other drawable that differs from the common drawable's is
neither listed by the Outline nor reached by this edit; and no bone is edited in a real window.
Seen on `just shots` screens, not in a window: a ped's fragment with its Pelvis row selected shows
the Bone fields where it showed the fragment's structure and no bone, typing into the Position and
Name fields moves and renames it and the Outline row follows, and an Enhanced fragment's bone shows
the same fields.

**Done when.** A test edits a bone's position in a staged drawable or fragment of each target where
one is staged, saves, reopens, finds the new position, finds its and every descendant's default and
inverse transforms equal to those recomputed from the bone data (the default transforms down its
chain times its inverse are the identity), and every other bone unchanged; a test renames a bone and
finds its id and the bone-id table unchanged; a test edits a rotation in degrees and finds a unit
quaternion and the preview's posed skeleton built again; and a test undoes an edit and finds the
saved bytes equal to those before it.

### R27q The skeleton file's own model can be hidden

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** The Animation preview draws a skeleton source's own model by default, and a box in the
preview's skeleton section, "Draw the skeleton file's own model", hides that model without hiding
the skeleton or any mesh source. A ped's `.yft` holds a 25 metre plane at z = 0 as its only
geometry, skinned to the root bone, so a ped previewed with no mesh added draws that plane and
nothing else. The choice is kept across a restart.

**Where it stands.** Done. The owner decided on 2026-10-06 to keep the plane drawn by default and
offer a box to hide it, which settles one of
[R27o](#r27o-the-animation-preview-follows-its-sources-fully)'s two questions;
`a_peds_fragment_chosen_alone_draws_the_one_plane_its_own_bounds_state` holds that the preview
draws it. `PreviewScene::draw_skeleton_model` is ticked unless the user unticked it. The box is
drawn after Allow mismatched skeleton (`draw_skeleton_section` in
`src/ui/panels/preview_settings.rs`), and a click goes through `AppState::set_draw_skeleton_model`,
which also writes `Prefs::draw_skeleton_model`; `PreviewScene::restored` opens the next run on it,
and a preferences file without the field reads as ticked. Unticked,
`build_meshes_for_preview_scene` (`src/render/meshes.rs`) no longer falls back to the skeleton
source's own drawable when no mesh source draws anything, and `PreviewScene::drawn_sources` leaves
it out, so what a click picks, the texture badge, the level-of-detail switch and the box at the
viewport's corner follow what is drawn. ⚠ The mesh key holds the choice (`anim_scene_signature` in
`src/render/viewport/retarget.rs`): without it the box changes nothing on screen until something
else builds the meshes again. A mesh source added is drawn with the box either way, and so are the
skeleton and its bones. The mesh section says "None added — the skeleton file's own model is
hidden." while the box is unticked. The preview's other settings (Show bones, Bind-pose ghost,
Apply root motion) are `PreviewScene` fields that start at their default each run, so this is the
first the preferences keep, and keeping the others is
the owner's to ask for. The tests are in `src/state/the_skeleton_files_own_model_can_be_hidden.rs`:
they draw Preview settings and click the box, on a Legacy fragment, drawable and dictionary entry
holding one triangle skinned to the root bone of a two-bone rig, and on the two staged Legacy
peds (`a_ped_chosen_alone_finds_the_box_ticked_and_the_plane_drawn_until_it_is_unticked`,
`unticking_the_box_draws_no_mesh_but_the_skeleton_and_a_mesh_source_added_is_still_drawn` and
`the_choice_to_hide_the_skeleton_files_model_is_kept_across_a_restart`). Seen on `just shots`
screens, not in a window: a ped's `.yft` chosen with a dance `.ycd` draws its grey plane with the
box ticked, and with it unticked draws only its bones, the box at the viewport's corner reading
"Bones of a_m_y_skater_01.yft" and the level-of-detail switch gone, since nothing drawn has levels.
Not done: with a skeleton chosen, the box unticked and Show bones off, the viewport's box still says
"Nothing to draw · choose a mesh or a skeleton in Preview", and the tests draw Legacy fixtures
only, though nothing the change touches names a target.

**Done when.** A test chooses a ped `.yft` alone and finds the box ticked and the plane drawn; a
test unticks it and finds no mesh drawn but the skeleton, with a mesh source added still drawn;
and a test finds the choice kept across a restart.

### R27r What the Animation preview still leaves after R27q

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** The preview's texture badge, level-of-detail switch and closing follow every source it
draws, and the box that hides the skeleton file's own model says nothing it cannot back.

**Where it stands.** Found while closing
[R27o](#r27o-the-animation-preview-follows-its-sources-fully) (#559): the badge shows a dictionary
pinned when any drawn source pins it, but Pin writes to every drawn source and a ticked box can only
Unpin, so with source A pinned to X and a second mesh source B added X shows ticked while B is not
pinned, and the user unticks and ticks it to pin B; `PreviewScene::documents` also yields the
texture sources and the pick, which the row does not name and no test covers, and `forget_document`
answers true when a texture source goes, so `tick_animation` invalidates the clip session and builds
it again for a closed texture dictionary as for a closed mesh-only source; `levels` in
`src/render/viewport/pick.rs` lists none whenever `active_document` is none, though the Animation
workspace draws sources then; the buffer writes of `refresh_skinning` need a GPU device and no test
holds them, and the badge's mapping to the `*_in_view` functions and its read of
`texture_pins_in_view` are untested for the same reason; and the Attach step of the badge test, like
the test that keeps the user's Loop where the Looped field did not change, passes with or without
the change. Found while closing [R27q](#r27q-the-skeleton-files-own-model-can-be-hidden) (#566):
with the box unticked and no skeleton file open or chosen the mesh section still says "None added —
the skeleton file's own model is hidden.", for a box the panel does not draw then
(`draw_skeleton_section` returns early); with a skeleton chosen, the box unticked and Show bones off
the corner box says "Nothing to draw · choose a mesh or a skeleton in Preview" (`preview_lines` in
`src/ui/panels/viewport_layout.rs`), which tells the user to choose what is chosen, and
`docs/features/Animation_Preview.md` says the viewport draws only the bones there, which holds with Show
bones on alone ([R27t](#r27t-a-skeleton-is-seen-without-an-animation) turns Show bones on by
default, so the sentence now takes the user unticking it as well as the box, and holds then; while
it is ticked and a skeleton is chosen, the corner box says "Bones of" the file, with or without a
clip); the field doc of `PreviewScene::draw_skeleton_model` and the box's hover say the
model is drawn while no mesh source is chosen, where it is drawn too when the chosen sources draw
nothing; and `other_tests_find_the_box_ticked` reads the preferences from a thread of its own, which
since [R59y](#r59y-what-a-test-run-or-a-self-check-still-leaves-behind) (#568) has a folder of its
own, so it finds the default whatever a click leaked and its comment is false.

**Done when.** A test adds a second mesh source to a pinned dictionary and finds the box not ticked
for it; a test closes a texture dictionary and finds the clip session kept; a test finds the
level-of-detail switch listing the drawn sources' levels with no active document; a test draws the
mesh section with no skeleton and finds no sentence about a hidden model; a test unticks the box
with a skeleton chosen and Show bones off and finds the corner box not telling the user to choose
one; and the leak check of the box's test fails when a click writes the preferences another test
reads. All six hold in the tree, each with a test seen failing first except the last, which could not: `reached_documents` takes the Animation workspace's sources whether or not a document is active; `forget_document` counts the clip, the skeleton source and the meshes as the scene's changes and no texture source; `is_pinned_in_view` ticks the box only for a dictionary every drawn source pins; the mesh section says "None added." where no skeleton is offered; the corner box says the skeleton file's model is hidden and Show bones is off, where a skeleton is chosen; and the leak check was vacuous since each test has a preferences folder of its own, so the box's test now also asks that its own folder holds the box unticked after the click, which fails where the click does not save.

### R27s What a bone edit still leaves after R27p

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** A bone edit is held on a drawable dictionary of each target as well as a drawable and a
fragment, what the row says of the signatures and the corpus is what its checks run, and an edit
reads cleanly at the gimbal and at small numbers.

**Where it stands.** Found while closing [R27p](#r27p-a-bones-numbers-can-be-edited) (#574): no
synthetic drawable dictionary of either target is built, so the dictionary route (`skeleton_of`,
`skeleton_mut` and `bone_selected` in `src/state/bone_numbers.rs`) has no test on a clean checkout
and the Enhanced one none anywhere; the corpus test recomputes only `signature_comprehensive` of the
three signatures, so the row's formulas for `signature` and `signature_non_chiral` and its "every
signature equal" rest on the code and on a "two signatures kept" assertion that cannot fail; its 985
skeletons count the file's skeletons, a shared one many times, and not how many differ, and the scan
that found no `.ydd` holding two equal skeletons was a throwaway whose figure cannot be run again;
the refusal tests `a_rebuild_that_cannot_finish_changes_nothing` and
`an_edit_a_skeleton_cannot_take_changes_nothing` were never seen failing, and one half of the second
cannot fail on the check it names; and nothing checks that drawing the bone fields on a bone nobody
edited edits nothing, the rule of [R27b](#r27b-viewing-never-edits). `part_bytes` in
`src/state/edit_history.rs` counts a `Change::part` as its size plus the heap of a texture or a
payload, so a skeleton step of about 20 KB for a ped is under-reported against the 200-step cap, and
a fragment's step with copies holds one skeleton more for each. The Position and Scale fields show
two decimals, so a ped's pelvis reads "x -0.00" where a minimum number of decimals would show it,
and at the gimbal, a ped's pelvis resting at about Y = 89.98 degrees, an edit of the X or Z field
keeps the rotation but may show the others differently afterwards. A fragment's physics children and
bounds sit by their own matrices and do not follow a bone moved, a `.ypt` model's bones
(`draw_ypt_model_bone`) are still read-only, and no bone is edited in a real window. `fk_golden` of
`or-anim` reports skipped checks under `just test` though a corpus is staged beside the lane, which
was not looked at, and `docs/guides/Development_Workflow.md` does not say that opening an Enhanced file in
a `just shots` script needs a click on "Yes, switch to GTA V (Enhanced)" after the open.

**Done when.** A test edits a bone of a drawable dictionary of each target and finds it saved with
its rest pose derived again; a test recomputes all three signatures of every staged skeleton; a test
sees each refusal test fail with its check taken out; a test draws the bone fields on an unedited
bone and finds no step; and a test sets a bone's position to a tiny number and finds it shown.

**Partly closed.** `drawing_the_fields_of_a_bone_nobody_edited_edits_nothing` draws the Inspector over
a bone at both targets and held three ways and finds no step and no dirty document (seen failing with
an edit forced in `draw_bone`). `a_bones_position_of_a_hundred_thousandth_of_a_metre_is_read_as_that`
reads a position of 1e-5 as "0.00001" at both targets, where the fields showed "0.0000" at four
decimals (seen failing first; they now show up to six). The refusal tests were each seen failing with
a check taken out: `an_edit_a_skeleton_cannot_take_changes_nothing` without the NUL check in
`apply` and without the bone-bound check, and `a_rebuild_that_cannot_finish_changes_nothing` with
`rebuild_rest_pose` writing each bone before the last was checked. The explicit finite check in
`apply` is not what that test holds: with it taken out the test still passes, because
`rebuild_rest_pose` refuses a non-finite pose itself, so the check is a second layer and not a gap.
`viewed::<T>(Holder::Entry)` already edits a bone of a drawable dictionary of each target, in
`editing_a_bone`. `part_bytes` now counts a skeleton step's heap: a skeleton of 300 bones was counted as 449 bytes against the 636,416 it carries (`a_step_that_swaps_a_skeleton_counts_the_bones_it_holds`, seen failing first). `a_skeletons_other_two_signatures_are_what_its_bones_state` recomputes `signature` and `signature_non_chiral` of all 985 staged skeletons from the engine's formulas (the bone-id table order for the second) and finds none differ; with the dofs word masked it finds 933 differ, but a wrong mask for translation about Z alone is not caught, since no staged bone translates about Z alone. Still open:
the physics children of a fragment following a bone (they do not: the fields of a fragment's bone now
say so, `the_fields_of_a_fragments_bone_say_its_physics_bounds_stay_where_they_are`, seen failing
first; whether they should follow is a design question for the owner). `fk_golden` no longer skips: its
checks run over 68 shipped skeletons (#879), and the `just shots` note about the Enhanced switch prompt
is in the `open` row of Development_Workflow.md. A `.ypt` model's bones are edited as a drawable's are, in the Inspector,
with undo and a save that keeps the edit (`the_bone_of_a_particle_model_is_edited_from_the_inspector_and_undone`
and `an_edited_particle_model_bone_reaches_the_saved_file`, at both targets, seen failing first).

### R27t A skeleton is seen without an animation

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** A skeleton can be seen without an animation. The viewport's controls, beside the shading
modes and the level-of-detail and Collision toggles, gain a Bones toggle that draws the skeleton of
the drawable the viewport draws (a drawable's, a fragment's, the entry a drawable dictionary has
selected) at its rest pose, in the Model workspace and wherever a drawable is viewed. The bone
selected in the Outline, whose numbers the Inspector edits
([R27p](#r27p-a-bones-numbers-can-be-edited)), is drawn highlighted, and a click on a bone's line
selects its row, as [R42f](#r42f-the-viewport-picks-what-it-draws)'s picking does in the Animation
workspace. There the preview draws the chosen skeleton source's bones at rest as soon as it is
chosen, before any clip is chosen or when the clip does not bind, and Show bones is on by default.

**Where it stands.** Done, which the owner asked for on 2026-10-06 after opening a `.yft` and
finding no way to see its skeleton, with the toggle, the highlight, the click and the bones before a
clip is chosen, and Show bones on by default (owner, 2026-10-06). Before it, `append_skeleton_lines`
in `src/render/viewport/mod.rs` returned unless the workspace was Animation, Show bones was ticked
and `state.anim.skeleton()` had a binding, which `rebuild_anim_session` clears when no clip is
chosen, and `bone_under` in `src/render/viewport/pick_lines.rs` was gated the same way. Now
`append_skeleton_lines` asks `bones::lines` (`src/render/viewport/bones.rs`), which `bone_under` and
the box at the viewport's corner ask too, so the lines drawn and the lines clicked are one set.
Outside the Animation workspace it is the skeleton of each drawable `skeletons` in
`src/render/viewport/pick.rs` finds in what the viewport draws for the active document, built from
the document's bones each frame, so an edit of a bone ([R27p](#r27p-a-bones-numbers-can-be-edited))
moves its line with nothing to invalidate, while the Bones box is ticked
(`PreviewScene::show_rest_bones`, off by default as Collision is). In the Animation workspace it is
the skeleton source's, posed by the clip where one binds and else at rest, built the same way: a
session with no clip, or cleared because the clip does not fit, has no pose, and the bones of the
source's document are drawn as they stand. `PreviewScene::show_bones`, the box Show bones, starts
ticked. The Bones box in the viewport's row, which `src/ui/panels/viewport_drawable_row.rs` draws
whenever there is a skeleton to draw, beside the switch and Collision, is that flag in the Animation
workspace and `show_rest_bones` elsewhere: ⚠ the two keep the choice apart, so a drawable opened in
Model does not show its bones until Bones is ticked there. The bone whose Outline row (`Named::bone`) is the
document's selection is drawn in `SELECTED` (`src/render/skeleton_lines.rs`), its line and its
joint, and a click picks the nearest line within 5 pixels before any bound or mesh, as that row: in
the Model workspace it is selected and the Outline scrolls to it and shows the bone's fields in the
Inspector, and in the Animation workspace it is held in `PreviewScene::picked`, as R42f says.
The tests are in `src/state/a_skeleton_is_seen_without_an_animation.rs`, over a two-bone rig of each
target held as a drawable, a dictionary's entry and a fragment, and the smallest staged `.ydr` and
`.yft` of each target that ships a rest pose for its bones (Legacy's `.ydr` and `.yft` and
Enhanced's `.yft`; no staged Enhanced `.ydr` holds a skeleton with a bone above another and a rest
pose of its own).
`the_bones_toggle_draws_one_line_for_each_bone_with_a_parent_from_its_parents_rest_position` finds
one line for each bone with a parent, from where the parent rests to where the bone does, each
place the one the file's own inverse transform gives, which no derivation of ours made, within a
millimetre; `the_bone_selected_in_the_outline_is_drawn_in_the_highlight_colour` finds the line and
the joint of the bone selected in the highlight colour, the others as they were, and none with a
row that is no bone's; `a_click_on_a_bones_line_in_the_model_workspace_selects_its_row` finds the
bone's row selected and shown in the Outline, and not at thirty pixels from its line or with Bones
off; `a_bones_position_edited_in_the_inspector_moves_its_line` finds the line move with the edit
and back with its undo; `the_viewport_offers_a_bones_toggle_beside_a_skeleton_and_not_beside_none`
clicks the box in the viewport's row;
`a_skeleton_chosen_in_the_animation_workspace_draws_its_bones_with_no_clip_chosen` finds the bones
drawn at rest, named in the corner box and clicked as a pick, and hidden with Show bones off;
`a_clip_that_does_not_bind_leaves_the_skeleton_chosen_drawn_at_rest` does the same for a clip over
other bones; and `show_bones_is_on_for_a_fresh_preference_file` finds Preview settings drawing the
box ticked. All eight failed before the change, run against a `bones::lines` that did what
`append_skeleton_lines` did before it: the first and the second at the missing lines, the third
at the row not selected, the fourth at the line not moved, the fifth at no toggle drawn, the sixth
and seventh at no line, and the last at Show bones off. Seen on `just shots` screens, not in a
window: a car's `.yft` in the Model workspace with Bones ticked draws its bones in front of the
model, and a click on a line selects its row, shows the bone's fields in the Inspector and draws the
line in the highlight colour; a ped's `.yft` chosen as the skeleton in the Animation workspace with
a clip dictionary and no clip draws its bones at rest, the box at the corner saying "Bones of" its
file, and with Show bones unticked draws only the model, as it did before. Not done, and taken up by
[R42k](#r42k-what-drawing-bones-outside-the-animation-workspace-leaves): a drawable
dictionary draws the bones of the entry the viewport draws and not of every entry; the pick the
Animation workspace holds apart from the selection is not highlighted, only a bone selected in the
document the skeleton is in; the Bones box is not kept across a restart; a `.ypt` model's bones go
through the same path and no test or screen holds them; no staged Enhanced `.ydr` holds a rest pose to
check, so that target's drawable is the rig's alone; and no bone is clicked in a real window.

**Done when.** A test opens a staged `.yft` and a staged `.ydr` with bones, of each target where
one is staged, turns Bones on in the Model workspace and finds one line for each bone with a parent,
from the parent's rest position to the bone's; a test selects a bone row and finds its line drawn in
the highlight colour; a test clicks a bone's line in the Model workspace and finds its row
selected; a test chooses a skeleton source in the Animation workspace with no clip and finds its
bones drawn; a test finds Show bones on for a fresh preference file; and a test edits a bone's
position and finds its line moved.

### R28 Authoring and write-back targets

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** Write-back as an explicit choice between into the archive, to loose files, and
shadow-merged over the game; export bundles that carry an asset's secondaries (`_hi.yft`, `.yld`,
embedded textures materialised only where needed); and an export that writes OpenRage's canonical
form and asks before that alters a file's bytes, by the owner's rule of 2026-10-01
([recorded](Project_Context.md#an-export-canonicalises-and-asks-before-it-alters-the-bytes-2026-10-01)).

**Done when.** A save names its destination. An export writes OpenRage's canonical form, which
exporting it again reproduces byte for byte; where canonicalising would alter the bytes of the file
as opened, a game file or another tool's, the export first asks
"Do you want to apply OpenRage export fixes (this will alter the bytes)", and declining exports the
original bytes. A test asserts each.

**Where it stands.** The authoring half is done (#67) and closed R29's greying-out row: the
create-new menu offers what this build can author. `why_not_authorable`
(`src/state/document_io.rs`) attempts the authoring and reports what the writer said, so an offer
and its outcome are one computation (`nothing_offered_would_refuse_if_clicked`). ⚠ Predicting it
from whether a handler exists offers rows whose click then refuses. A row the build cannot author
is listed disabled with its reason (`an_unavailable_row_is_still_listed_with_its_reason`). The export
clause is met: a decoded extraction or a drag out compares each file's canonical form, what saving it
unedited writes, with its bytes, and where any differs asks
"Do you want to apply OpenRage export fixes (this will alter the bytes)" first; declining writes the
original bytes, and the answer can be remembered in Settings (`src/state/extract_job.rs`,
`src/state/an_export_asks_before_it_alters_the_bytes.rs`). A raw or decrypted extraction is never
canonicalised. The save clause is met: saving a document opened from an archive asks where it goes,
naming the archive on disk and the overlay copy, and writes only there
(`src/state/save_destination.rs`, `src/ui/panels/save_destination.rs`,
`src/state/a_save_names_its_destination.rs`). Back into the archive rebuilds the archive that holds
the entry; a loose file leaves the archive's bytes alone and asks the export question where an
unedited entry's canonical form differs from its bytes; the overlay writes its copy of the archive
with the entry in it, landed under a `.rpf.part` name as R49b's copy is, and the open archive then
saves to that copy. Each asks the edit gate about the file it writes. An entry of an archive nested
in another goes back through the archive on disk that holds it
([R20b](#r20b-nested-archives-survive-a-save)).

**Depends on.** [R20](#r20-archive-lifecycle).

**Risk.** Three write destinations multiply the states a bug can hide in; the export test is the
cheapest way to catch a class of them.

### R29 Kernel manners diagnostics probing and provenance

[↑ checklist](#phase-4--generic-create-edit-delete)

**Done** (#62): `Diagnostic::with(name, value)` carries a named `Arg` and `signature()` is the key
plus the argument names (`crates/io/or-io/src/diagnostics.rs`), so a consumer reads a diagnostic's
values without parsing its sentence and two instances of one defect group as one. Grouping by the
sentence with its digits masked instead splits one defect into a group per field it names, because a
field name has no digit to mask, and merges two faults whose sentences differ only in a number. Keyed
diagnostics, `ErrorMode`, capability probing and header validation before parsing exist, and the
greying-out clause closed in [R28](#r28-authoring-and-write-back-targets). A skip-list of known-bad
game files is struck, because [R24](#r24-search)'s sweep cannot tell a bad file from a reader
defect. Provenance stamping is not built, because a stamp is bytes the game did not ship.

### R30 Presets references and cross-record edits

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** A named property-set system over the declared structures, applicable to any structure,
appliable across a multi-selection, with catalogues contributed per domain and migration between
preset versions; and resolution of cross-file reference fields (room IDs, procedural IDs, model
sets, scenario types) into name dropdowns sourced from the scene's own data and the game's tables.

**Done when.** A preset defined on one record applies cleanly to a selection of a hundred, and a
reference field offers real names.

**Where it stands.** Done. In the Visual panel, where a metadata document's
fields are edited, a portal's `roomFrom` and `roomTo` and an entity set's `locations` offer the
enclosing interior's room names as a choice list, a room's or portal's `attachedObjects` offer the
interior's entities by archetype, and an `archetypeName` offers the archetypes its `.ytyp` declares
beside the install-wide model picker. An array's elements are looked up under the array's name
(`an_array_element_is_asked_about_under_the_arrays_name`). `ReferenceNames` in
`src/state/reference_names.rs` reads these from `or-world`'s `MapTypes`, `Archetype` and `Interior`,
by name hash, so a build with no schema snapshot offers the same names
(`a_room_id_field_offers_the_scenes_room_names`). A `CMloInstanceDef`'s `defaultEntitySets` offer the
sets its interior declares in another `.ytyp`, which `InstallInteriors` reads from the mounted install
in the background the first time a `.ymap` placing an interior is open; with no install, or before
the read finishes, they offer nothing
(`a_default_entity_set_offers_the_sets_another_ytyp_declares`).

A field whose names live in the game's own tables offers them from the mounted install, which
`InstallTables` reads in the background the first time a metadata document is open in the Visual
panel. A spawn point's `spawnType`, an override's or chaining node's `ScenarioType` and a scenario
region's `TypeNames` offer the scenario types and type groups `CScenarioInfoManager` declares;
`pedType`, `ModelSet`, `PedModelSetNames`, `VehicleModelSetNames` and `VehicleModelSet` offer the
ped and vehicle model sets `CAmbientModelSets` declares, in one list; and `procObjTag` and
`plantTag`, the names behind a procedural ID, offer the procedural object and plant tags
`CProceduralInfo` declares. `GameTables` reads the base game's four table files and every DLC pack's
data files of the same `fileType`; with no install, or before the read finishes, these fields offer
nothing (`a_scenario_type_field_offers_the_types_the_install_declares`,
`a_model_set_field_offers_the_sets_the_install_declares`,
`a_procedural_tag_field_offers_the_tags_the_install_declares`).

Presets work on a metadata document's records, the structures in one of its arrays. In the Visual
panel each record has a selection box and a button that saves its chosen fields as a preset, and the
array offers select-all and an Apply preset menu once any record is selected. A preset is one JSON
file in `<config>/openrage/presets/`, named after it: an object of `version`, `name`, `type_name`
and `fields`, a list of name and value pairs in `or_io::meta::Value`'s serde form; `PRESET_VERSION` in
`src/state/presets.rs` is 1, and a file of a newer version is refused by its name and not offered
(`a_preset_of_a_newer_version_is_refused_by_name`). Applying one sets its fields on every selected
record of its type as one undo step, and reports each record it did not apply to and why, such as a
record of another type
(`a_preset_from_one_record_applies_to_a_hundred_in_one_undo_step`).

**Risk.** Preset formats become a compatibility burden the moment users share them; version and
migrate them from the first release rather than after the first break.

### R31 The structure analyser and the entry-metadata pairing

[↑ checklist](#phase-4--generic-create-edit-delete)

**Done** (#64). `openrage-cli analyse` (`crates/tools/or-cli/src/analyse.rs` over
`crates/io/or-io/src/desc/analyse.rs`) walks a paged resource nothing describes and reports its
allocations, strides and refusals with reasons; `--game` and `--glob` print each entry's `EntryFacts`
beside it, grouped by RSC7 version first because one extension at two versions is two layouts.
⚠ A stride is named only where a count of two or more divides the span, because naming one from a
single element is how a block gets declared at the wrong size
(`a_single_element_allocation_refuses_to_name_a_stride`);
`crates/io/or-io/tests/what_an_analyser_cannot_know.rs` holds the other rules: a count that does not
divide is a disagreement, not rounded; a count sits beside the pointer, never beside its target; and
coverage is reported by bytes, because a block count flatters. The generic grid reads the declared
format's description at the file's target (`described_root` in `src/state/document_io.rs`), and
`or_editor::Row` keeps its string for display and its `EditKind` for writing
(`a_patched_distance_survives_a_save_and_reopen`), so `Summary` never has to become writable. The
per-format files under `src/ui/panels/inspector/` are outline builders, reader-based fallbacks for a
document with no description, and the particle inspector, not a grid awaiting replacement. The
analyser stays a CLI subcommand, out of the default inspector.

### R31b The inspector leads with a summary

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** The inspector opens on a summary in a modder's terms for each kind (name, edition, levels
of detail and their distances, shaders and their textures, bounds, flags), then the declared
structure under the engine's names, with vtable words, counts, reference counts and runtime pointers
behind "Show internals". Enums are choices, a hash has one spelling, numbers carry their units, and
editability shows per field.

**Where it stands.** Done. Each kind's inspector opens on a summary read from its owned value
(`src/ui/panels/inspector/summary.rs`): the file's name, its edition and resource version, each
level of detail with its vertices and the distance it is drawn to, each shader with the textures it
samples, the dimensions and radius in metres, the skeleton, lights, collision and embedded textures,
and a container's counts. Below it the declared structure keeps the engine's names, and
`or_editor::Summary::of_shown` marks internal what the owned value keeps to itself, as
`Reach::owned_public` says, and any count, padding or pointer, which the grid lists only while "Show
internals" is on and never writes (`what_the_owned_value_keeps_to_itself_is_internal_and_read_only`);
`no_vtable_word_count_or_pointer_shows_in_the_default_inspector_of_any_corpus_kind` opens a staged
file of every kind, per target, and walks its outline. OD-RUNTIME (2026-09-26) is thereby applied to
the inspector, and the analyser stays out of it, as R31 has it. An enum field is a choice of its
declared variants (`every_enum_field_draws_as_a_choice`); a value no variant names shows as its
number and stays until another is picked
(`an_unnamed_enum_value_survives_a_view_and_an_edit_of_another_field`), and a pick is an undo step
(`picking_a_variant_writes_it_as_a_step_that_undoes`). A number is edited from what the bytes hold
and shown whole (`a_float_keeps_its_full_value_after_an_edit_of_another_digit`). A hash reads `0x`
and eight digits, with its name where the name index knows it
(`a_hash_is_spelled_one_way_wherever_the_inspector_draws_one`), and so does a field the declaration
types `Hash32` whatever it is called, such as an Enhanced shader's `name`
([R59t](#r59t-the-default-views-in-a-modders-words)); ⚠ for a field typed otherwise the grid still
goes by the engine's name holding `hash`. The panel's one badge is gone: a field the panel does not
write draws as a value and
says why on hover (`every_field_of_a_drawable_says_whether_it_is_written_here`). An archive row says
what it holds and its size in units. Seen on `just shots` screens, not in a window.

**Done when.** No vtable word, count or pointer shows in the default inspector of any corpus kind,
and every enum field draws as a choice.

### R31c A pinned inspector shows its own document

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** An inspector pinned to a document draws and edits that document in every kind's body
while another document is active.

**Where it stands.** Done. Each kind's inspector body reads and writes the document the dispatch
hands it (`draw_document_body` in `src/ui/panels/inspector/mod.rs`) rather than the active one: the
summary and declared structure at a file's root, each Legacy body's sections, the material and
particle editors and the steps they record, and the read-only bodies of clips, pose databases,
shader effects, archives and text. A particle effect's live evolution sliders draw only while its
document is active: they drive the preview, which plays the active document.
`src/state/a_pinned_inspector_shows_its_own_document.rs` pins the inspector to a staged Legacy
`.ybn`, `.ycd`, `.ydd`, `.ydr`, `.yft`, `.yld`, `.ypt`, `.ytd`, `.fxc` and `.ypdb`, and to a scratch
text file, while another of its kind is active: the inspector draws what the pinned file's body
draws and nothing only the other's draws
(`a_pinned_inspector_draws_its_own_document_in_every_kinds_body`), and a drag in the root grid of a
bound, a drawable and a fragment, a click in a drawable's and a drawable dictionary's material and a
drag in a particle effect each make a step in the pinned file and leave the active one untouched
(`an_edit_in_a_pinned_inspector_lands_in_its_document_and_leaves_the_active_one_untouched`). Both
failed before the change. The bodies of a texture or cloth dictionary edit nothing. Since
[R31d](#r31d-undo-and-the-sliders-follow-the-pinned-inspector), Undo and the History panel follow a
focused pinned inspector, and a clip dictionary's grid edits its root. Seen on `just shots` screens, not in a window.

**Done when.** A test pins the inspector to a file of each such kind, makes another document
active, and finds the body drawn from the pinned document and an edit made in it landing there,
with the active one untouched.

### R31d Undo and the sliders follow the pinned inspector

[↑ checklist](#phase-4--generic-create-edit-delete)

**What.** Undo, Redo and the History panel act on the document the focused panel shows, a pinned
inspector's included; a particle file's Live Evolutions sliders are named for the effect the
preview plays; and a clip dictionary's inspector shows its declared structure under its summary, as the
other kinds do.

**Where it stands.** Done. Undo, Redo and the History panel act on the document the focused panel
shows (`focused_document` in `src/ui/commands/target.rs`): the document the inspector is pinned to
while the inspector has focus, since [R27k](#r27k-undo-from-files-acts-on-what-files-shows) the
archive or folder Files shows while Files has focus, else the active one. The History panel, while it has focus itself,
follows the panel it took focus from (`WorkspaceDocks::focused_before` in `src/ui/dock.rs`): View ▸
Panels opens it in the focused panel's place and gives it that panel's focus, and a click in its
rows gives it focus, so opened from a pinned inspector it lists, and undoes in, the pinned
document. `ctrl_z_undoes_an_edit_in_a_pinned_inspector_while_the_inspector_has_focus` drags in an
inspector pinned to one particle file while another is active and undoes the edit with `Ctrl+Z`,
and `the_history_panel_opened_from_a_focused_pinned_inspector_lists_and_undoes_its_files_edits`
opens History from that inspector, finds the edit listed and undoes it with `Ctrl+Z` there
(`src/state/undo_and_the_sliders_follow_the_pinned_inspector.rs`); the second also fails with the
check for focus moved between frames taken out. The Live Evolutions sliders take their names from
the effect whose values they move, the one the preview plays, and their heading names it
(`the_live_evolution_sliders_are_named_for_the_effect_the_preview_plays`). A clip dictionary's
inspector draws its root's declared structure, `CrClipDictionary`'s `AnimDictionaryOwner`,
`BaseNameKeys` and `Clips > AllowReCompute`, under its summary through `draw_root_summary`, as the
other kinds do (`a_clip_dictionary_shows_its_declared_structure_under_its_summary`). Each of the
four failed before the change. Seen on `just shots` screens, not in a window.

**Done when.** A test edits in a pinned inspector and undoes it with `Ctrl+Z` while the inspector
has focus; a test pins one effect, selects another and finds the sliders named for the effect the
preview plays; and a test opens a `.ycd` and finds its declared structure under its summary.

### R32 World data ymap ytyp and interiors

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**Done** in `crates/domain/or-world` and `openrage-cli world`, each suite run over both retail
installs: typed views borrowing the `MetaInstance`, named through `or_io::meta::binary::public_names`
without a snapshot (#109); `MapGroup` and the cross-file LOD hierarchy (#122); manifests and
create/delete plans (#129); partitioning and donor cloning (#135); 22 typed extension kinds
(`what_every_extension_in_an_install_carries`), batch archetypes
(`what_an_archetype_takes_from_its_drawable`) and byte-identical untouched re-encodes
(`every_untouched_world_file_reencodes`) (#172); XML, 44,942 of 44,965 world files reaching their
fixed point (`every_world_file_reaches_its_xml_fixed_point`, at `2a55ed1c`; #176, #185); and
world-state groups (`what_every_world_state_group_holds`), prefabs (`a_prefab_round_trips_every_map`)
and create/delete carried out, 9,113 Legacy and 9,079 Enhanced deletes keeping every link by identity
(`executing_a_plan_keeps_every_link`) (#200).

Traps the corpus settled:

- ⚠ `parentIndex` does not index the map that stores it: bit 3 of `CEntityDef::flags` selects the
  array. Ignoring it condemns 478,261 entities in maps the game loads; honouring it leaves 22,155 of
  22,155 files clean (#109), which is what `write::refuse_if_broken` gates on.
- A map name names a set of files, so `MapGroup` merges parts rather than replacing them; cross-file
  LOD links then resolve 707,456 of 707,456 over 10,603 maps (#122). A `numChildren` lower than its
  children is refused and a higher one is advisory, because the game ships nine. The group checks are
  not a write gate — a group needs every member and a save has one document, so
  `write::refuse_if_broken` stays per-file — and only a mounted install (`--game`) can answer them.
- `CEntityDef` stores the conjugate of its rotation and `CMloInstanceDef` the rotation itself:
  recomputing entity extents over 2,878 Legacy `.ymap`, conjugating reproduces 807 exactly and 1,559
  within a centimetre, not conjugating 552 and 867 (#109).
- `_manifest.ymf` is derivable from what a map places: 8,964 of 10,603 derived lists match the shipped
  manifest exactly over a mounted Legacy install (#129). `v_proc1` is resident and stated by no
  manifest, so the derivation excludes it.
- `contentFlags` is additive: bit 9 marks a `_critical_` map (1,338 of 11,076) and no derivation
  produces it, and `strm` against `long` is an authoring decision the file does not record (#135).
- An entity's index is its identity, so a delete renumbers every file; a map lists its LOD parents
  first, so a leaf delete almost never moves an index (3 of 7,528 Legacy), and a delete suite deletes
  parents (`delete_subtree`).
- A world-state group is named for the map it switches, which sets `flags` bit 0, and its `Bounds` is
  collision, not membership. A prefab is typed per title by its entities' members (`CEntityDef` has
  17 in Legacy and 18 in Enhanced), so a catalogue is one title's.
- An extension's kind is its type hash, and a namespaced type has two spellings: the file stores
  `rage__phVerletClothCustomBounds` and a staged snapshot presents `rage::`, so a kind that matches
  only the second reads every cloth bound as unnamed on a build with no snapshot (#200).
- A whole-install sweep folds into counters: one that kept its value trees reached 38 GB (#109);
  `openrage-cli world check` holds all 22,155 Legacy files in 382 MiB.

Whether the game loads what these write is [R62](#r62-the-rbm-layout-synthesiser)'s open clause, and
Menyoo spooner XML import is [R67](#r67-menyoo-spooner-xml-import).

### R32a Terrain height map and distant lights

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** The world's coarse terrain height grid (`heightmap.dat`, `heightmapheistisland.dat`) and
the far-off traffic and street lights (`distantlights.dat`, `distantlights_hd.dat`), which map
modders edit, read and written as ordinary code in `or-io`, reported by the CLI and opened in the
app.

**Done when.** Every such file both installs ship reads and saves byte-identical, the CLI reports
them, and the app opens them.

**Where it stands.** Done. `or_io::height_map::HeightMap` and
`or_io::distant_lights::DistantLights` open and save the five distinct files, two in `common.rpf`
and three in `update/update.rpf`, identical across Legacy and Enhanced, byte-identical
(`every_shipped_heightmap_reads_and_saves_byte_identical`,
`every_shipped_distant_lights_file_reads_and_saves_byte_identical`, install-gated).
`openrage-cli convert info`, `verify` and the round trip take them by file name. In the app they
are `DocumentKind::HeightMap` and `DocumentKind::DistantLights`, routed by file name ahead of the
text `.dat` route, from disk and from an archive; the texture workspace's preview draws the height
map's maximum heights as an image, zoomed and panned as a texture is, and the inspector its bounds
and cell size, and lists each light group's centre, radius, point count, lanes and kind.

- Both are big-endian. A height map saves each row's span trimmed of cells whose two heights are
  zero, which every shipped file already is.
- The distant lights file states no block count: the 16-block and 32-block layouts are told apart by
  length, which only one can match for the counts the header states.
- Neither file stores a light's colour; the game colours street lights and traffic from its visual
  settings, so the list names each group's kind.

### R32b Water height map, world blanket and nav precalc

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** The binary tables the owner found with no reader: the water height map
(`data/levels/gta5/waterheight.dat`), the world blanket (`data/levels/stripped/worldblanket.dat`)
and the nav precalc (`data/navprecalc.dat`), all in `common.rpf`, read and written as ordinary
code in `or-io`, reported by the CLI and the census, and opened in the app.

**Done when.** Each file both installs ship reads and saves byte-identical, the CLI and the census
report them, and the app opens them.

**Where it stands.** Done. `or_io::water_map::WaterMap`, `or_io::world_blanket::WorldBlanket` and
`or_io::nav_precalc::NavPrecalc` open and save the three files, identical across Legacy and
Enhanced, byte-identical (`every_shipped_water_height_map_reads_and_saves_byte_identical`,
`every_shipped_world_blanket_reads_and_saves_byte_identical`,
`every_shipped_nav_precalc_reads_and_saves_byte_identical`, install-gated).
`openrage-cli convert info`, `verify` and the round trip take them by file name, and
`openrage-cli formats` names the reader of a binary world `.dat` by its file name, the height map
and the distant lights included. In the app they are `DocumentKind::WaterMap`,
`DocumentKind::WorldBlanket` and `DocumentKind::NavPrecalc`, routed by file name where
`heightmap.dat` is; the texture workspace's preview draws each as an image and the inspector lists
its counts.

- The water height map is big-endian: a 183 by 249 grid of 50-unit tiles whose stored runs point
  at references to 13 rivers, 15 lakes and 314 pools, with a debug colour per body after the data.
  Its image colours each tile by its first body's type.
- The world blanket has no header: 121 by 121 little-endian 16-bit cells, all 0 but a four by four
  block of 1 at the centre. Nothing in the file or the `stripped` level's data list says what a cell
  means, so the image is the cells as stated, row 0 at the top.
- The nav precalc is little-endian: a `NAVP` header, 3600 cells of two signed values and four counted
  groups of four-byte entries (16,467 in all), then a `BITS` block of 45,171 bytes. Laid out 60 to a
  row, the cells that hold entries draw the world's outline, which is the one meaning measured; every
  other field keeps its stated bytes, and the image is each cell's entry count.

### R32c Cache files, metadata.dat and the audio sector grid

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** Three binary `.dat` files nothing read: the streaming cache loader's
`gta5_cache_y.dat` and each DLC pack's `<pack>_<hash>_cache_y.dat`, the `metadata.dat` the game
hands the Social Club library, and the audio engine's `audioworldsectorsinfo.dat`, read and written
as ordinary code in `or-io`, reported by the CLI and the census, and opened in the app.

**Done when.** Every such file both installs ship reads and saves byte-identical, the CLI and the
census report each, and the app opens them.

**Where it stands.** Done. `or_io::cache_loader::CacheFile`,
`or_io::social_club_metadata::SocialClubMetadata` and
`or_io::audio_world_sectors::AudioWorldSectors` open and save every shipped file byte-identical: 104
Legacy and 67 Enhanced cache files, the four `metadata.dat` (loose in `x64/` and `update/x64/`,
one file across both editions) and the one sector grid in `common.rpf`
(`every_shipped_cache_file_reads_and_saves_byte_identical`,
`every_shipped_metadata_dat_reads_and_saves_byte_identical`,
`every_shipped_audio_world_sectors_file_reads_and_saves_byte_identical`, install-gated).
`openrage-cli convert info`, `verify` and the round trip take them by file name, and
`openrage-cli formats` names each reader in its `dat` row. In the app the three are one kind,
`DocumentKind::DatTable`, routed by file name where `heightmap.dat` is; the inspector lists a cache
file's archive dates and its three stores' records, `metadata.dat`'s header, and the sector grid's
sectors, as tables.

- A cache file is a text header and archive date list, then the `fwMapDataStore`,
  `CInteriorProxy` and `BoundsStore` modules of fixed-size records. Some files add a third number
  to every date line, always a multiple of 104; what it counts is not measured, so it is kept as
  `third_column`.
- An interior proxy's 32-byte name slot holds stale memory in shipped files, and a bounds record's
  trailing bytes read `7f 00 00` throughout some Legacy files; both are kept as stated.
- Only `metadata.dat`'s header is measured (a big-endian word, `META` and a build time); its body,
  a hash-sorted index ahead of localized strings, is kept as stated bytes.
- A sector's last byte packs its tree count above the water bit: the low bit traces the coastline.

### R33 Paths navmesh scenarios and tracks

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** Traffic path nodes, the navmesh with index stability, vehicle and waypoint recordings,
train track node graphs, and scenario points resolved against the game's real indices.

**Done when.** R33a through R33e are done; this row is a programme header.

**Where it stands.** A recon over both retail installs (`openrage-cli formats` and
`openrage-cli meta types`, #157) found four unrelated programmes:

| | Legacy files | Enhanced files | Container | Part |
|---|---:|---:|---|---|
| `.ynd` path nodes | 1,027 | 780 | RSC7 v1 | [R33a](#r33a-path-nodes-ynd) |
| `.ynv` navmesh | 10,567 | 10,567 | RSC7 v2 | [R33b](#r33b-navmesh-ynv), [R33e](#r33e-navmesh-index-stability) |
| `.yvr` vehicle recordings | 7,533 | 7,530 | RSC7 v1 | [R33c](#r33c-recordings-yvr-and-ywr) |
| `.ywr` waypoint recordings | 1,612 | 1,612 | RSC7 v1 | [R33c](#r33c-recordings-yvr-and-ywr) |
| scenario points | 207 | — | PSO / RBM `.ymt` | [R33d](#r33d-scenario-points) |
| train tracks | 1 | — | plain XML | [R33d](#r33d-scenario-points) |

The counts crawl every copy; through the mounted VFS there are 259 distinct `.ynd` regions per
edition, and a reader is scoped by the distinct count. `.ynd` and `.ynv` are declared in `or-io` and
open as owned values, and so do `.yvr` and `.ywr`; scenario regions are declared in `or_io::meta`, and their
domain layer is `or_world::scenario`.

**Risk.** ⚠ Navmesh indices are referenced across files, so a well-meaning "compact the list" is a
game-breaking edit; that is why disable-not-delete is the default.

### R33a Path nodes ynd

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** The traffic path graph: nodes with their street hash, position and flags, the link range
each owns, junctions and the heightmap they index into, and the `(area, node) -> junction` table.

**Done when.** It reads, writes and round-trips against the corpus, and the editor opens it.

**Where it stands.** Declared in `or-io` as `CPathRegion` (#208) and opened as an owned value
(#220), whose derived fields a save writes (#291). Both editions ship `.ynd` at RSC7 version 1, so
every region reads at the Gen8 layout. `crates/io/or-io-verify/tests/path_region.rs` holds it over
the corpus: the node split, the link chain, the heightmap chain and the reference order close in
every region (`every_region_closes_its_chains`), the owned value saves as the tree saves
(`every_ynd_opens_owned_and_saves_as_its_tree_saves`), a save reaches the canonical fixed point
(`every_ynd_saves_to_a_fixed_point`), the re-keyed testimony holds
(`the_ynd_testimony_holds_through_its_declarations`), and a broken link chain is refused
(`a_region_whose_link_chain_is_broken_is_refused`). See
[YND_File_Structure_Specification.md](../extra/YND_File_Structure_Specification.md).

The editor opens `.ynd` as `DocumentKind::Ynd` over the owned `CPathRegion`, through a
`PayloadHandler` that parses and saves (`src/state/payload_handlers.rs`, #286), with a `KindInfo`
row, a `FormatCapabilities` row and an outline of its nodes, links and junctions; the inspector is
the described root grid
(`a_path_region_and_a_navmesh_open_as_kinds_of_their_own_with_an_outline`).

**Done.** The owner ruled on 2026-10-01 that opening as a kind of its own with an outline and a
save (#351) is "the editor opens it".

### R33b Navmesh ynv

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** The `.ynv` navmesh reader and writer; index stability is
[R33e](#r33e-navmesh-index-stability).

**Done when.** It reads, writes and round-trips against the corpus, and the editor opens it.

**Where it stands.** Reads, writes and round-trips (#159), its identities checked over the install
(#163). It is RSC7 v2, with chunked 16 KiB lists (the vertex and polygon pools) under a sector
quadtree. It is declared in `or-io` as `CNavMesh` (#215), the quadtree node a `packed(4)` record
because its pointers sit at four modulo eight, and opens as an owned value under the engine's names
(#287). A save writes the derived fields (#291), and a navmesh whose special link counts do not
close on its indices is refused (#317). `crates/io/or-io-verify/tests/navmesh.rs` runs the declared
gates, the split arrays' partitions (`every_split_array_states_the_partition_its_stride_implies`)
and a broken index run refused (`a_polygon_whose_corners_do_not_follow_the_last_is_refused`). See
[YNV_File_Structure_Specification.md](../extra/YNV_File_Structure_Specification.md).

The editor opens `.ynv` as `DocumentKind::Ynv` over the owned `CNavMesh`
([R66e](#r66e-move-the-consumers), P47, #306), with a `KindInfo` row, a `FormatCapabilities` row
and an outline of its sector quadtree and its polygons; the inspector is the described root grid.
The viewport draws the cell (`src/render/navmesh.rs`): every polygon as a filled fan in one neutral
colour, since no flag bit [R33e](#r33e-navmesh-index-stability) measured has a known meaning to
colour by, its polygons' outlines and each leaf sector's box, which draw over the fill in every
shading mode, and the quadtree root's box frames the camera
(`a_staged_navmesh_draws_its_polygons_and_sectors`, and `just self-check --viewport --open` on a
staged `.ynv`).

**Open.** The owner's judgement of the 3D view: on 2026-10-01 he kept the row open for one, and it
stays unticked until he has looked.

### R33c Recordings yvr and ywr

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** `.yvr` vehicle recordings and `.ywr` waypoint recordings: a small header and one flat run
of fixed-stride frames each.

**Done when.** Both read, write and round-trip against the corpus.

**Where it stands.** Both are declared in `or-io` under the engine's names, `gta5::vehicle_recording`
and `gta5::waypoint_recording`, with a `ResourceType` and a `FORMATS` row each, and open as owned
values. A `.yvr` is a 32-byte root and a run of 32-byte `CVehicleStateEachFrame`; a `.ywr` is a
40-byte root and a run of 20-byte `RouteData`, which the file pads to sixteen bytes before its page
map. Both editions ship both at RSC7 version 1, the Legacy layout.

Over both retail installs, every file opens owned, saves, reopens equal and saves again to the same
bytes, and each run at its declared stride ends where the file's page map begins: 7,533 Legacy and
7,530 Enhanced `.yvr`, and 1,612 `.ywr` in each
(`every_shipped_yvr_opens_owned_and_reaches_its_fixed_point`,
`every_shipped_ywr_opens_owned_and_reaches_its_fixed_point`, install-gated). Each format's
`declared_gates!` suite and census passed over those files staged as a corpus, and four of its
seven gates fail with the frame stride declared four bytes long. The app opens each as a
`Registered` kind with a payload handler, and `openrage-cli convert` reads both in `info`,
`verify`, `batch` and the round trip; nothing downstream asks for more.

The corpus stages four of each format from each install, by the owner's decision of 2026-10-03
([Project_Context.md](Project_Context.md#the-corpus-stages-the-recordings-2026-10-03)): `.yvr` of 0,
19, 518 and 5,230 frames, the 19-frame one a recording whose frame times do not ascend, and `.ywr`
of 2, 30, 271 and 493 waypoints, the 30- and 271-waypoint ones with bits set above bit 15 of
`packed_struct2_as_integer`. Every one is RSC7 version 1, so the gates read all eight of each as
Gen8, and each suite's four corpus gates run over them on a corpus run and pass
(`every_yvr_saves_to_a_fixed_point`, `every_ywr_saves_to_a_fixed_point`). No testimony was ever
recorded for either and none is added, so the two testimony gates skip everywhere.

### R33d Scenario points

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** Scenario points, chains, clusters and entity overrides, resolved against the game's real
type and model-set indices; and the train track node graph.

**Done when.** A scenario region's points, chains and clusters read as named domain values, an edit
re-encodes against the document's own layout, and a point's type and model set resolve to the names
the game uses.

**Where it stands.** `CScenarioPointRegion` is declared by hand in `or_io::meta` over the
document's own type table, as the world family's roots are, under the owner's extension of
OD-META-TYPED of 2026-10-01
([recorded](Project_Context.md#scenario-regions-take-typed-roots-by-hand-2026-10-01)): its points,
chaining graph, clusters and look-ups are typed records under the engine's names, its entity
overrides and acceleration grid are carried, and a save writes the typed members back into the tree
it was read from and refuses a point whose type index is past the look-ups.
`crates/io/or-io-verify/tests/meta/typed_scenario_regions.rs` holds every staged region, the 205 a
retail Legacy install ships (1 PSO, 204 RBM): each opens typed and agrees with its value tree,
names every point's type through its own look-ups and saves to the tree's own save
(`every_staged_region_opens_typed_and_names_each_points_type`), and a moved, re-typed point reaches
the file as that edit and nothing else
(`an_edited_point_reaches_the_file_as_that_edit_and_nothing_else`).

`or_world::scenario` is the domain layer: `ScenarioManifests` reads the scenario-info and model-set
manifests and names each point's scenario type, and its model set from the vehicle or the ped table
as its type spawns, against them; `set_scenario_type` re-types a point by name.
`crates/domain/or-world/tests/naming_every_scenario_point.rs` re-types a point in every staged
region and saves it as the value tree saves the same edit, and over a retail Legacy install names
all 159,866 points of its 205 regions against the manifests it ships, 103,608 of them with a model
set (`every_installed_points_type_and_model_set_is_named_by_a_manifest`, at `cfcd9d4c`). Creating a
region from scratch waits on R62. Train tracks are one XML file in `common.rpf`, which opens in the
text editor.

**Depends on.** [R62](#r62-the-rbm-layout-synthesiser), for creating a region from scratch.

### R33e Navmesh index stability

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** The API half of `.ynv`: disabling a polygon as the safe default, and true deletion, refused
with the edges, special links and cells it would break.

**Done when.** A polygon can be disabled and cannot be deleted without a refusal naming the cells
that reference it, enforced by the API, with the three measurements below reported first.

**Where it stands.** The three measurements are taken and the API is built on them.
`openrage-cli navmesh` (`crates/tools/or-cli/src/navmesh_census.rs`) reads every `.ynv` the mounted
game loads, the last copy of each name the mount enumerates, and prints three tables; `--index`
adds every cell's references. At `a439029e` with that census, a Legacy install mounts 10,583 copies
of 4,477 names: 4,440 grid cells and 37 vehicle meshes that all state cell 10000, so 4,441 cell
indices, holding 6,586,650 polygons and 25,552,480 `TAdjPoly` records. The Enhanced install prints
the same figures.

- **No shipped flag bit marks a disabled polygon.** Of the 21 flag bits at the bottom of a
  `TNavMeshPoly`'s first word, bits 0, 1, 2, 3, 6 and 7 are set on 3,049,921, 510,629, 99,209,
  223,744, 1,950,296 and 290,828 of the 6,586,650 polygons, and bits 4, 5 and 8 to 20 on none. 9,052
  polygons have no neighbour on any edge and 14,178 are named by no edge in any cell; no bit is set
  on all of them. Bit 7 is the most over-represented, on 4,002 of the 9,052 and 4,612 of the 14,178
  against 4.4% overall, and is also set on 286,826 polygons with a neighbour, so it is not a disable
  bit. If the game ships no polygon disabled, its disable bit is among the fifteen clear ones; which
  one it honours needs in-game confirmation, which is the owner's. A cell names no region, so there
  is no correlation with a named region to take.
- **The first word names a lookup slot in bits 0 to 4 and a polygon in bits 5 to 19.** A polygon
  field of all ones is no neighbour, on 2,378,535 edges, and bit 19 is set on exactly those. Every
  other edge names a slot its cell's `CAdjacentNavmeshLookups` fills (23,173,945 of 23,173,945).
  Every neighbour in the same cell is under the cell's polygon count (22,472,390 of 22,472,390), and
  21,137,163 of them hold both corners of the edge from corner k to k + 1 by position, against
  156,421 for the edge from k − 1 to k. 700,814 of the 701,555 neighbours in another cell are under
  its count. Bits 3 and 4 are never set, so the slot's width past three bits is not measured. The
  controls, the word read one bit higher and the slot narrowed to four bits, leave 62.8% and 55.2% of
  edges out of range or unresolved and hold the edge's corners on fewer than 0.1%. Above the
  polygon, bits 20 and 21 take four values, 22 to 26 take 32 and 27 to 31 are clear. The second
  word's bits 0 to 19 equal the first's on all 25,552,480 records; its bit 20 is clear on every one,
  bits 21, 22 and 23 are set on 2,173,726, 205,011 and 769,550, and 24 to 31 are clear.
- **4,441 cells reach 17,312 ordered pairs of other cells, every pair returned.** A cell reaches four
  others in 4,127 cases, three in 183, two in 125, one in 5 and none in 1, always at `index ± 1` or
  `index ± 100`; the lookup tables name 21,755 entries other than the cell itself, of which edges
  reach the 17,312. ⚠ The shipped game already holds stale references: 741 edges over 63 cell pairs
  name a polygon past the target's count, and every pair runs from a base-game cell to a cell the
  `mp2026_01` pack replaced, such as cell 664 naming polygon 1,606 of the 72 in cell 764.
  `an_edge_past_the_target_cells_polygons_is_out_of_range` holds that check.

`CNavMesh::disable_polygon`, `delete_polygon` and `references` are the API, in
`crates/io/or-io/src/gta5/navmesh.rs`, with `TAdjPoly::leading_to` and `neighbour` for an edge's two
fields, `NO_NEIGHBOUR` for the polygon field all ones and `disabled_polygons` for which polygons are
cut off. `openrage-cli navmesh disable` and `delete` take `<in.ynv> <polygon> <out.ynv>` and write
the result to a file of their own, and the `.ynv` outline's right-click menu on a polygon offers
Disable polygon and Delete polygon, each one undoable step with the refusal, or what still reaches
the polygon, in a toast.

**Disabling changes edges and sets no flag**, since no shipped bit marks a disabled polygon. Every
edge out of the polygon, and every edge of its own cell that leads into it, is written as leading
nowhere: the polygon field all ones in both words, the second repeating the first's low twenty bits
as every shipped edge does, every bit above them kept, and the slot the first that does not name the
cell itself, which all 545,435 shipped edges that lead nowhere over the 600 staged Legacy cells state
(`every_shipped_edge_that_leads_nowhere_names_the_first_slot_that_is_not_its_cell`, at `5ab768c3`).
Pathfinding steps from a polygon to its neighbour across an edge, and shipped polygons already state
no neighbour on any edge (the first measurement), so a polygon with no edge into or out of it
should never be entered.
The sector lists and special links still name it, and other cells' edges into it are out of one
file's reach: `references` names them after a disable, and a save leaves them. Whether the game skips
a disabled polygon is untested; the owner loads it. `disabled_polygons` reads that state back from
the file, so a shipped polygon with no edge in or out reads as disabled too, and a disabled one
saves and reopens disabled (`a_disabled_polygon_saves_and_reopens_disabled`).

**Deleting is refused unless every reference can be renumbered.** `delete_polygon` takes the
polygon's corners out of the index and edge lists and the polygon out of its list, refills the
sub-arrays so every one but the last stays full, and lowers by one every reference to a later
polygon that the mesh holds: its own cell's edges (both words), the polygons each sector lists and
the four polygon fields of each special link, while a save restamps the first corner, sub-array and
first special link of every polygon. It refuses, changing nothing, and names the edges that still
lead into the polygon (by polygon and corner), the special links that start or end at it or sit in
its run, the cells the lookup table lists, whose own edges name this cell's polygons by number and
cannot be renumbered from one file, and the polygons after it that would change number
(`a_deleted_polygon_is_refused_naming_what_leads_into_it`,
`a_deleted_polygon_renumbers_every_reference_to_the_ones_after_it`). A cell that lists another is
always refused: of the 600 staged cells, 15 list none, all vehicle meshes, whose other table entry
is 16383, a cell no shipped edge that leads somewhere states (3,997,316 edges,
`the_table_entry_of_all_ones_is_one_no_shipped_edge_leads_into`, at `5ab768c3`). Over a stride of the
staged cells, 49 of 50 refuse naming the cells around them and the vehicle mesh deletes
(`deleting_from_a_shipped_cell_that_lists_others_is_refused_naming_them`), and with those cells
struck from the table, every one of 50 renumbers its edges, sectors and special links as the mesh
states them (`a_shipped_cell_deletes_a_polygon_and_renumbers_what_names_the_ones_after_it`).

**Open.** Whether the game skips a disabled polygon is the owner's to load. Renumbering the listed
cells' edges together with a deletion, once every listed cell is supplied, is not built.

**Depends on.** [R33b](#r33b-navmesh-ynv).

### R34 Audio and media

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** `.awc` playback, export and a full round trip; `.rel` authoring and a cross-archive audio
index; and video preview for Bink.

**Done when.** [R34a](#r34a-the-audio-wave-container), [R34b](#r34b-the-audio-metadata-schemas) and
[R34c](#r34c-video-preview) are each done or declined. They share no dependency, risk or skill, so
this row is a programme header. Quote file counts from `openrage-cli formats --sample 0`.

**Depends on.** [R19](#r19-one-virtual-filesystem).

#### R34a The audio wave container

**What.** `.awc` end to end: the container, both scramblings, the codecs, export to WAV and an
install census; then multi-channel de-interleaving, the app's preview surface and an encoder.

**Done when.** Every container in a shipped install opens, accounts for every byte and writes back
byte-identically; every codec a shipped install uses decodes; and the decode is checked against
something the decoder does not read: the peaks a stream states, or, for a multi-channel stream's
channels, which no container states a peak for, the signal's continuity across blocks and a
hand-built container with a known signal per channel.

**Where it stands.** Read, write and both PC codecs (PCM16, ADPCM) landed in #152, and the reader is
ordinary code in `or-io` behind its `awc` feature (#266): `crates/io/or-io/src/awc/`, with
`openrage-cli awc info|census|export`. The decode is checked against the peak envelope each
stream's `format` and `peak` chunks state, which Rockstar's encoder measured and the decoder never
reads. At `899267d1`, `openrage-cli awc census` over both installs parses 7,590 of 7,590 Legacy and
7,607 of 7,609 Enhanced files with no byte unaccounted for, and reproduces PCM16 peaks on 23,367 of
23,368 and 23,691 of 23,692 `format` chunks and on every `peak` window; the one outlier per install
is the case CodeWalker's peak builder also notes. ADPCM is lossy, so
`a_wrong_decoder_misses_the_peaks_by_far_more_than_the_codec_does` requires a swapped-nibble decoder
to miss by an order of magnitude more: a mean error of 445.7 correct against 28,903.9 swapped, over
the same 5,235 checks on the 77 staged fixtures. The scrambling key comes from the user's own
executable ([R66b](#r66b-clean-what-the-crate-will-contain)). See
[AWC_File_Structure_Specification.md](../extra/AWC_File_Structure_Specification.md).

`AwcFile::decode_channels` de-interleaves a multi-channel container into one PCM16 buffer per
channel. At `79a27e99`, a census pass over both installs de-interleaved 3,412 of 3,412 Legacy
multi-channel containers and the 894 of 3,432 Enhanced ones in ADPCM; the other 2,538 are MP3. ⚠
**No container states a peak for a channel**: none of the channel streams in either install carries
a `format` or `peak` chunk, and `streamformat` has no peak field. So
`a_multi_channel_container_de_interleaves_into_channels_that_run_on_across_blocks` checks the
signal's continuity instead. Over the 14 staged channels, a channel moves by a mean of 788.9 across
its 86 block boundaries and 595.1 between any two samples, against 4,647.9 when each block is
joined to the next channel's. A decoder that swaps two channels in alternate blocks fails it. A swap
made the same way in every block keeps each channel continuous, so
`each_channel_of_a_hand_built_multi_channel_container_decodes_to_its_source` catches it instead:
three PCM16 channels, each a distinct ramp, laid out by hand from the specification's block layout
rather than by `or-io`'s writer, must each decode to their source, and a decoder that swaps channels
0 and 1 in every block fails it. `crates/io/or-io/tests/awc_roundtrip.rs` records no multi-channel
skip.

The app opens an `.awc` as its own kind (`src/audio.rs`, `src/state/opening_an_awc.rs`). The
inspector lists each single-channel stream and a multi-channel container's interleaved source by
name hash, codec, channel count, rate and length; plays one through `rodio` behind the `AudioSink`
trait, on every platform; exports one to WAV; and replaces a stream from a 16-bit PCM WAV in the codec it has, through
`AwcFile::with_stream_replaced`: PCM16, ADPCM or MP3 at the stream's own rate, rewriting its `format`,
`peak` and seek table chunks, or a multi-channel stream's blocks, and scrambling the new data when the
file scrambles each stream or block.
`AwcFile::set_chunk_payload` moves every later chunk by whole 16-byte steps, and the saved file
reopens equal. A whole-file-scrambled container waits for the AWC key the app extracted. A WAV at
another rate than an ADPCM or MP3 stream's own, since nothing is resampled, a WAV with other than a
multi-channel stream's channels, an XMA stream and an MP3 channel of a multi-channel stream are
refused by name for replacement; an MP3 stream plays through the MP3 decode below, which no app test
plays. `just self-check` draws the kind.

Every stream decodes through `or-media` (`crates/io/or-media`, the owner's decision of 2026-10-05,
[recorded](Project_Context.md#every-media-format-is-decoded-in-rust-inside-the-tree-2026-10-05)).
`or_media::awc::streams` lists the playable streams from their `format` and `streamformat` chunks
alone, and `or_media::awc::open` opens one as a source that states its codec, channel count, rate
and length, and decodes to interleaved samples that seek by frame. The codec table has a row each
for PCM16, RAGE ADPCM and MP3, whose decode stays in `or-io`, and one for XMA, decoded in
`or-media` itself below; a codec with no row is refused by name while its stream still lists.
`openrage-cli awc export` decodes through the same table. A multi-channel stream's blocks stay `or-io`'s to
de-interleave, so the table is asked only whether it decodes each channel's codec.
`crates/io/or-media/tests/a_synthetic_awc_opens_through_the_media_interface.rs` holds it over
hand-built containers, and the state tests of `src/state/opening_an_awc.rs` run through it. Sound
plays on every platform: `rodio` is a dependency of every target, and `ci.yml`'s Linux job installs
ALSA's headers, `libasound2-dev`, which its `cpal` links.

`codec::encode_adpcm` writes ADPCM in the blocks the decoder reads. Each sample takes the nibble
whose decode lands nearest it, never one that carries the predictor past `i16`, so both
accumulator models decode the result alike, and each block's header carries the state the
previous block ended on. Synthetic signals in `crates/io/or-io/src/awc/codec.rs` hold it: a sine
decodes back within a quarter of its largest step between samples, and re-encoding the encoder's
own decode gives the same bytes. `a_shipped_adpcm_stream_reencodes_within_adpcm_error` in
`crates/io/or-io/tests/awc_adpcm.rs`, install-gated, re-encodes the 1,273 mono ADPCM streams of
three Legacy sound-effect archives from `8793d1d5`: the worst decodes back at 18.0 dB
signal-to-error and the median at 41.3, and every even-length stream re-encodes to the
shipped payload's length. The re-encode is close rather than exact because the game's encoder
starts each block from a header of its own, near the block's first sample, rather than where the
previous block ended. `a_replaced_adpcm_stream_saves_and_reopens_within_adpcm_error` replaces a
stream in the app, saves it and reopens it.

Enhanced's MP3 (codec byte 7, none in Legacy) decodes through `nanomp3`, a pure-Rust port of minimp3
under `MIT OR Apache-2.0` behind the same `awc` feature
([Licence_And_Provenance_Policy.md](../guides/Licence_And_Provenance_Policy.md#sources)); MP3's patents have
expired. A stream is its frames' decode from sample 0, cut to its stated count, and a multi-channel
channel is one MP3 stream across its blocks, so one decoder per channel carries the bit reservoir and
the overlap from block to block; each block skips the samples it repeats from the previous one.
`every_enhanced_mp3_stream_decodes_to_its_stated_sample_count` in
`crates/io/or-io/tests/awc_mp3.rs`, install-gated, decodes all 213,875 MP3 streams and 7,570 MP3
channels of the Enhanced install to their stated counts. The peaks the encoder recorded fit the
decode from sample 0 at a mean error of 857.8 of 65,535 against 2,584.0 for one shifted by the
decoder's 529-sample delay, over 4,482,491 checks, and a channel moves by a mean of 606.2 across its
102,613 block joins and 824.3 anywhere; a fresh decoder per block moves by 3,667.4 there and fails.
One stream decodes 57 samples short and is padded with silence, and one container's last block
states −450 samples, taking back what the block before it overran. Silent frames built in
`crates/io/or-io/src/awc/codec.rs` check the count and its refusals without an install, and the
staged corpus's 4,096 MP3 peak checks measure 593.3.

`AwcFile::with_stream_replaced` replaces a multi-channel stream from an interleaved WAV of its channel
count and rebuilds the shared blocks, each channel in its own codec, PCM16 or ADPCM
(`crates/io/or-io/src/awc/replace.rs`). ⚠ **The blocks follow the shipped containers' own geometry,
measured rather than chosen.** A full block holds the most packets of each channel that fit beside its
table and one spare packet, so four channels take 63 packets each of a 256-packet block, two take 127
and sixteen take 31 of 504 (`a_rebuilt_container_has_the_blocks_the_shipped_ones_have` holds
thirteen such shapes from the installs). Each channel's first-packet word is where it would start if
the spare packets were dealt to the first channels, 0, 64, 128 and 191 for four, a running sum in the
last block, and −1 throughout a container that never states it. A packet's offset is its first sample
in the whole channel, not in its block, which corrects the specification's earlier account. The seek
table is a little-endian word per block, the sample it begins on. A full block is zero-padded to the
block size, and the last ends where its runs do, except that some containers pad it too and a
rebuild trims it. `every_shipped_adpcm_multi_channel_container_rebuilds_into_its_own_blocks`
(`crates/io/or-io/tests/awc_codecs_install.rs`, install-gated) rebuilds each container from its own
decode and compares. At `5ab768c3`, over 2,994 Legacy and 874 Enhanced ADPCM containers, 5,929 and
1,606 channels, every block but a cut container's last has the shipped packet counts, first-packet
words, offsets, samples, seek table and length, and every channel decodes back within ADPCM's error,
the worst at 18.1 dB signal-to-error in Legacy and 18.7 in Enhanced, the median at 43.1 and 42.6. A
container is cut to its first three blocks for the encode's cost (1,338 and 205 of them) and one over
16 MiB is left out (418 and 20); 600 of the 1,656 Legacy containers kept whole and 62 of the 669
Enhanced ones ship their last block padded to the block size. ADPCM does not restart at a block: a
channel is encoded whole and cut into packets.
`a_replaced_two_channel_adpcm_stream_saves_and_decodes_continuous` in the app and
`a_rebuilt_adpcm_channel_runs_on_across_its_block_joins` check it by the error at every sample and the
step across every join, where an encoder restarted at each packet moves by 2,113.0 across the joins
against 61.6 anywhere. A channel the WAV leaves as it decodes keeps its shipped packets, so an edit to
one channel costs no other a generation
(`a_channel_the_wav_leaves_as_it_decodes_keeps_its_shipped_packets`).

Replacing an Enhanced MP3 stream encodes it with `rusty_mp3`, a pure-Rust Layer III encoder under
Apache-2.0 and the only permissively licensed one a search of crates.io found on 2026-10-05
([Licence_And_Provenance_Policy.md](../guides/Licence_And_Provenance_Policy.md#sources) says why LAME and shine,
which are LGPL, are out). `codec::encode_mp3` writes the shape every shipped stream has, which
`every_enhanced_single_channel_mp3_stream_has_the_shape_the_encoder_writes` measures
(`crates/io/or-io/tests/awc_codecs_install.rs`, install-gated). At `5ab768c3`, over Enhanced's 213,875
single-channel MP3 streams and 16,422,938 frames, every frame is MPEG-1 mono with no CRC and no
padding at 32 or 48 kHz and borrows no bytes from the frame before it (its main-data-begin field is 0), each at
its own bitrate between 32 and 320 kbps, averaging 82.5; the seek table is the frames' byte lengths as
`u16`; and the frames hold between none and 1,151 samples more than the stream states, in all but one
stream. The encoder runs in its variable-bitrate mode, at 96 kbps on average, which borrows no bytes
either, and a frame outside that shape is refused rather than stored. ⚠ **The shipped streams' priming
frame is dropped.** The staged `weed.awc` has 112 MP3 streams that decode exactly 47 samples ahead of
the ADPCM of the same hash in Legacy's, which is 1,152 samples, a frame, less 1,105, the delay a
LAME-style encoder and a decoder add together
(`an_enhanced_mp3_stream_decodes_47_samples_ahead_of_its_legacy_adpcm_twin`). `encode_mp3` drops its
priming frame, and the information frame before it, after feeding 700 zeros: this encoder and the
decoder delay a signal by 1,057 samples (`the_mp3_encoders_delay_is_the_one_the_lead_is_cut_by`), and
a shorter lead drops the overlap the first 576 samples need, which returns as distortion the signal's
own size, 7,038 rms of a 7,492 rms signal for a lead of 95 zeros. The audio then begins 605 samples in
and the stream's count includes them, so a stream replaced from its own export gains that much each
time. The format chunk's rate is kept and the WAV must be at it, while the frames are written at the
rate its first frame states, since 36,237 of Enhanced's streams state a rate other than their frames'.
A sample from two speech archives, `SS_DE` and `SS_QR`, encodes in both
installs (`a_shipped_streams_audio_encodes_to_mp3_frames_of_the_shipped_shape`, install-gated): 7,283
ADPCM streams of Legacy's and 7,388 MP3 streams of Enhanced's become 560,947 and 562,960 frames, the
worst decoding back at 17.7 dB signal-to-error and 18.6, the median at 29.6 and 31.5.
A test that replaced one in the app (removed 2026-10-09: Cranelift cannot compile the decoder's SIMD) did. A multi-channel
MP3 stream is refused by name: each block of one repeats the last frame of the block before it, which
a rebuild does not write.

Legacy's eight streams in codec byte 5 are XMA, identified by their packets (`Codec::Xma`). All
eight are in one container, `vw_counterfeit_chips.awc` of the Vinewood casino
pack, at 28,160 to 51,712 samples, each a multiple of 512, and 31,948 to 32,049 Hz. ⚠ **Their payload
is whole 2,048-byte packets**, each a 32-bit header whose first six bits count its frames, then
frames that open with their own length in bits (15 bits, the field included), then bits that are all
ones to the packet's end; and the seek table is a big-endian word per packet, the samples decoded so
far, 512 for each frame. That is XMA's own packet geometry.
`legacys_eight_codec_5_streams_are_xma_packets_and_refuse_to_decode`
(`crates/io/or-io/tests/awc_codecs_install.rs`, install-gated) holds it, at `5ab768c3`, over every `.awc` of both
installs, 7,742 Legacy and 7,758 Enhanced: in Legacy 8 streams and 52 of 52 packets chain exactly to
their header's count with ones to pad, and the seek table is the frames' samples, ending on the
stream's count; Enhanced has none, nor any XMA channel. The coding inside a frame is WMA Pro's, and
its tables are carried by no permissively licensed source: a search of crates.io on 2026-10-05 for
`xma`, `xma2` and `wma pro` found no decoder, and `oxideav-wma` (MIT) reads WMA v1 and v2 baseline
only. So `or-media` decodes them with a port of FFmpeg 7.0's WMA Pro decoder and its tables
(`crates/io/or-media/src/xma/`, LGPL-2.1-or-later, each file headed with its origin and listed in
[LICENSE-third-party.md](../../LICENSE-third-party.md)), the owner's decision of 2026-10-05, as the
`Codec::Xma` row of its table. The app plays and exports them, and `openrage-cli awc export` writes
them, through that row; `or-io`'s own decode still returns nothing for codec 5, and the census counts
them as `XMA`. A stream decodes to its stated count: the first frame only primes the transform's
overlap, and FFmpeg drops it, 64 samples of the decoder's delay and what the frame headers trim (512
at the start of every stream here, 0 to 384 at the end), keeping 281,088 of the eight streams'
285,696 samples, so this decode gives those samples and then silence.

`legacys_eight_xma_streams_decode_as_ffmpeg_decodes_them`
(`crates/io/or-media/tests/xma_matches_ffmpeg.rs`, install-gated, skipping without an FFmpeg that has
`xma2`) wraps each stream's packets in the RIFF an `xma2` decoder reads and compares. At `07bb1dfe`
with this change, no sample of the eight differs from FFmpeg 7.0's by more than 3.6e-7 of full scale,
against a stated tolerance of 1e-6 (a 16-bit step is 3.05e-5), and FFmpeg logs nothing about them.
`the_decode_fits_the_peaks_each_stream_states_and_no_other_streams_does` checks it against what the
decoder does not read, the peaks Rockstar's encoder measured over its source: over 65 windows the
decode misses them by a mean of 1,741.7 of 65,535, while the other seven streams' decodes miss a
stream's peaks by a mean of 10,673.4 to 19,771.5. ⚠ **No shipped frame spans two packets**, so
`frames_that_span_packets_decode_as_they_do_whole` lays each stream's frames into packets that do,
which this decode reads exactly as the originals and FFmpeg within the same tolerance, and
`a_damaged_stream_is_decoded_or_refused_and_never_a_panic` damages 320 copies of the streams, of which
121 decode and 199 are refused. The channel transform of a pair is ported with the rest and checked by
no shipped file, since all eight are one channel. Streams of silent frames built by hand run the same
path with no install, in `crates/io/or-media/tests/a_synthetic_awc_opens_through_the_media_interface.rs`,
`crates/tools/or-cli/src/awc.rs` and `src/state/opening_an_awc.rs`.

`openrage-cli awc replace` takes a WAV into a stream by the id `awc info` lists, and `awc export`
writes a multi-channel container as one interleaved WAV named for its source stream, `00000000`.

**Open.** Replacing an XMA stream stays refused by name: `or-media` holds a decoder only, and no open
XMA encoder is known. Replacing an MP3 channel of a multi-channel stream and the six chunk kinds
carried verbatim are open too; the app names a stream by its hash, since the file stores no name. ⚠
No replaced stream has been played by the game: every check above is a decode by `or-io` of what a
rebuild wrote, so a replaced `.awc` is the owner's to try in the game.

#### R34b The audio metadata schemas

**What.** `.rel` and its `.datNNN` siblings — ambient zones, zone lists, rules, static emitters,
emitter lists, interiors and interior rooms — and a cross-archive audio index filterable by container
and item class.

**Done when.** Every `relType` in a shipped install reads and round-trips, an edit loads in game, and
the index answers "which container holds this sound".

**Where it stands.** The container reads and saves. `or_io::rel::RelFile`
(`crates/io/or-io/src/rel.rs`, behind the `rel` feature) keeps the data block verbatim, the name
table, the item index and the hash and pack tables, derives every length the file states, and
refuses one the bytes do not bear out; a damaged item table is refused by name
(`a_corrupted_item_table_is_refused_by_name`). An item is its key and its bytes, typed by its first
byte. `openrage-cli formats` reads `.rel` through it.

On top of `bb41d023`, `every_shipped_rel_reads_and_saves_to_its_fixed_point`
(`crates/io/or-io/tests/rel_container.rs`) opens, saves and reopens 259 of 259 Legacy and 275 of
275 Enhanced files with every item kept, and every save is byte-identical with the shipped file.
`openrage-cli formats --ext rel --sample 0` counts the same 259 and 275 there, against 244 and 260
at `899267d1`. The suite's census, Legacy / Enhanced:

| `relType` | Files | Items | Item type ids |
|---|---|---|---|
| 4, index by hash | 36 / 38 | 222,414 / 220,614 | none: the first byte takes 224 values |
| 4, index by name | 2 / 2 | 265 / 272 | 9: 0–3, 5, 7–10 |
| 10 | 43 / 45 | 6,340 / 7,245 | 2: 1, 3 |
| 15 | 45 / 47 | 8,411 / 8,649 | 10: 0–9 |
| 16 | 3 / 3 | 1,616 / 1,631 | 13: 1–10, 12, 13, 15 |
| 22 | 2 / 2 | 459 / 466 | 1: 0 |
| 54 | 57 / 60 | 196,250 / 205,473 | 35: 1–35 |
| 149 | 8 / 10 | 86 / 88 | 11: 3, 4, 17, 50, 57, 62, 63, 66, 76, 88, 90 |
| 150 | 9 / 11 | 244 / 246 | 22: 3, 4, 6, 8, 17, 32, 37–39, 47, 50, 52, 57, 62–66, 76, 88, 90, 117 |
| 151 | 54 / 57 | 51,505 / 63,592 | 111 / 113, between 1 and 124 |

⚠ The hash and pack tables count from the file's start, 8 bytes before the data block; read that
way, every offset in both installs lands inside an indexed item. Not every byte of the data block is
indexed: in one `relType` 54 file the first indexed item starts 339,922 bytes in.

`relType` 4, the most items in both installs, reads typed. `RelFile::speech_items`
(`crates/io/or-io/src/rel/speech.rs`) types a speech file's items, keyed by hash, as the six speech
types under the engine's names; no item states its type, so one is read from what points at it (a
hash table offset, a group reference, a pack table offset) before its length.
`RelFile::config_items` (`crates/io/or-io/src/rel/config.rs`) types an audio configuration file's
items, keyed by name, by their class id: nine types, `ErSettings` among them. A class id no shipped
item takes (4 and 6) is refused, and the item stays bytes in the container.
`every_shipped_rel_type_4_item_reads_typed_and_writes_back_its_bytes` clears every item's bytes,
writes each typed item back and saves, and every save is byte-identical with the shipped file:
222,679 items in 38 Legacy files and 220,886 in 40 Enhanced ones.

`relType` 54 (sounds) and 151 (game data), the next two by items, read typed by class id, each item
its first byte: `SoundItem` (`crates/io/or-io/src/rel/sound.rs`) holds the 35 sound types and
`GameObjectItem` (`crates/io/or-io/src/rel/game_object.rs`) the 113 game object types the installs
ship, under the engine's names, with `decode` and `encode`. A sound type opens with a header that
states a field only when its bit in a mask is set, and is packed; a game object lays its fields out
at natural alignment from the item's start, a derived type's after its base's with no padding
between, and pads its end to its own fields' alignment. A counted array stores only its entries,
but for four that store every slot whatever the count. `crates/io/or-io/src/rel/layout.rs` holds the
codec, which refuses padding that is not zero rather than drop it. A class id no shipped item takes
is refused. 22 Legacy `WeaponSettings` of 208 bytes and 9 Legacy `TrainAudioSettings` of 156 are
older layouts that stop at a field boundary before their type does; the fields they leave out are
each an `Option`, `None` from where an item stops.

The other six read typed by class id the same way, under the engine's names: `SynthItem`
(`crates/io/or-io/src/rel/synth.rs`) holds `relType` 10's 2 shipped types, `DynamicMixItem`
(`crates/io/or-io/src/rel/dynamic_mix.rs`) 15's 10, `CurveItem` (`crates/io/or-io/src/rel/curve.rs`)
16's 13 and `CategoryItem` (`crates/io/or-io/src/rel/category.rs`) 22's one. All but the curves are
packed; a curve lays its fields out as a game object does, but `EqualPower` pads its end to its
base's alignment too. A `CompiledSynth` states its program's size in bytes three words before the
program, and a write derives it. `relType` 149 and 150 are older game data and read as
`GameObjectItem`s: in each install 35 items of 149 and 68 of 150, of `WeaponSettings`,
`CarAudioSettings`, `PlaneAudioSettings`, `HeliAudioSettings` and `GranularEngineAudioSettings`,
stop at a field boundary before their type does and leave their newer fields out the same way. No
item of any `relType` stays bytes.
`every_other_shipped_rel_item_reads_typed_and_writes_back_its_bytes` clears every typed item's
bytes in every file of a `relType` but 4, writes each back and saves, and every save is
byte-identical with the shipped file: 264,911 items in Legacy (196,250 sounds, 51,505 game objects)
and 287,390 in Enhanced (205,473 and 63,592), the rest as the census above counts them.

The cross-archive index answers "which container holds this sound". `or_depends::audio::AudioIndex`
(`crates/domain/or-depends/src/audio.rs`) crawls an install, names each `.awc` by the bank its path
makes, the archive or folder holding it less `.rpf` and then its stem (`RESIDENT/weapons`), and
joins it to every wave a `relType` 54 item names: a `SimpleSound`'s, each named channel of a
`GranularSound`'s, and an `AutomationSound`'s data. A bank is its name's hash, and a wave the low 29
bits of its name's hash, all a container's stream table keeps. `openrage-cli awc which
<sound|0xHASH>` lists, for a sound's name or its wave's, each item naming it and the containers
holding it, filtered by `--container` (a path or a bank) and `--class`; `openrage-cli awc index`
counts. It crawls rather than mounts: Legacy keeps its wave banks in archives loose under
`x64/audio/sfx/`, which the VFS mount does not hold, and an index over the mount reached 3,474 of
the 7,742 Legacy containers. `a_sound_name_resolves_to_its_container_and_the_items_naming_it`
(`crates/domain/or-depends/tests/which_container_holds_a_sound.rs`) builds such an install with no
game present. A sound that reaches a wave through other sounds, and a game object or speech item
naming a sound or a bank, are not followed.

On top of `e1f03082`, `openrage-cli awc index` over both installs counts these wave references,
Legacy / Enhanced, in every copy of every sound file:

| Item class | Held | Bank holds no such wave | No container is the bank |
|---|---|---|---|
| `SimpleSound` | 70,811 / 72,381 | 57 / 65 | 134 / 127 |
| `GranularSound` | 5,312 / 5,452 | 0 / 0 | 1,054 / 1,082 |
| `AutomationSound` | 59 / 73 | 0 / 0 | 8 / 8 |
| All | 76,182 / 77,906 | 57 / 65 | 1,196 / 1,217 |

That is over 7,742 containers holding 261,369 streams and 57 sound files in Legacy, and 7,758
holding 262,161 and 60 in Enhanced, whose two refusals are files under `update2.rpf`'s
`common/shaders/fxdb/` that carry the `.awc` extension and no wave container. Of the references
whose bank is absent, 1,138 Legacy and 1,166 Enhanced name the bank `0`, which a sound file's name
table lists and no container is; the rest name 12 and 8 other banks, such as
`TEST\16ch_stream_test`.

A DLC ships each `.datNN.rel` beside a `.datNN` of the same stem: `.dat4`, `.dat10`, `.dat15`,
`.dat54`, `.dat149`, `.dat150` and `.dat151`, 229 in Legacy and 240 in Enhanced (for example
`x64/audio/dlcbeach_sounds.dat54` in `mpbeach`'s `dlc.rpf`). Each is the same container: on top of
`2a62e456`, a walk of both installs with `rel_container.rs`'s crawl widened to `.datNN` opened every
one through `RelFile`, saved every one byte-identically, and found its `relType` to be its
extension's number. 178 Legacy and 188 Enhanced are byte-identical with their `.rel` sibling, and
every `.datNN` but `.dat54` reads typed through `RelFile::typed_items`. Each item of a `.dat54`
holds three bytes after its class id that its `.rel` sibling's item lacks, and is otherwise the
same bytes, in all 50 Legacy and 52 Enhanced; `SoundItem::decode_dat54` reads one past them. One
Legacy `.dat15` differs from its sibling and still reads typed.

The app opens a `.rel` and those `.datNN` extensions as `DocumentKind::Rel`
(`src/state/document/rel.rs`). `RelFile::typed_items` types every item by the file's `relType`;
the inspector groups the items by type, lists each by its key, and shows a selected item's typed
fields read-only, with a `.dat54` item's three extra bytes beside them. A file whose items do not
read typed still opens, says why, and lists its items by class id as bytes. A save writes the
container as it was read and keeps the file's extension. `src/state/opening_a_rel.rs` holds a
synthetic file of each case to its types, its groups and an unchanged save, and `just self-check`
draws the kind. Editing a typed field is not built.

**Open.** An edit loaded in game, which the owner checks.

⚠ There is no `.psc` for `.rel` and every item type is hand-written, so it belongs in `or-io` as
hand-written code, as `.awc` does, not in the schema-driven `or_io::meta`; revisit only on evidence
that a schema exists.

#### R34c Video preview

**What.** Playing a `.bik` in the app, with its sound, on every platform: Bink 1 through `or-media`'s
own port of FFmpeg's decoders and Bink 2 through its own port of NihAV's (owner, 2026-10-05,
[recorded](Project_Context.md#every-media-format-is-decoded-in-rust-inside-the-tree-2026-10-05)),
Bink 2 held to the `bink2w64.dll` of the user's own install, which a test loads and the app does not
(the route of owner, 2026-10-03,
[recorded](Project_Context.md#a-bink-2-bik-is-decoded-by-the-bink2w64dll-of-the-users-own-install-2026-10-03),
until `or-media` decoded Bink 2 itself), its in-loop filter derived from that DLL (owner, 2026-10-06,
[recorded](Project_Context.md#the-bink-2-in-loop-filter-is-derived-from-rads-dll-2026-10-06)).

**Done when.** The app previews a `.bik` with play, pause and seek, and no media path runs an
external program or loads a library. Bink 1, picture and sound, is decoded in OpenRage's own Rust and
matches FFmpeg 7.0 on every Bink 1 movie both installs ship; Bink 2, picture and sound, is decoded in
OpenRage's own Rust and equals the `bink2w64.dll` of the install that ships it, plane for plane, on
every shipped Bink 2 movie.

**Where it stands.** Done. The app opens a `.bik` as
`DocumentKind::Bik`, read-only, and its
inspector plays it: `src/video.rs` opens the movie through `or-media` (`or_media::open`), decodes its
picture on a thread to RGBA frames at 640×360, the movie's own frame rate, with play, pause and seek,
and decodes its first sound track whole on another and hands it to the speaker
(`AppState::sound_the_movie`), started where the picture stands and moved with a seek. The movie's
length is its header's frames over its frame rate, which `or-media` reads. No FFmpeg is run, found
on the PATH or set in Settings; the preference and its Games-page field are gone. No library is
loaded either: `src/bink.rs` reads a movie's header, and the DLL is a Windows test's.

`or-media`'s table decodes Bink 1 and both Bink audio codecs, ported from FFmpeg n7.0 rather than
from NihAV's `nihav-rad`: FFmpeg's LGPL is the weaker licence of the two, the C is a flat run of
reads over byte buffers that ports to safe Rust nearly line for line, and an FFmpeg is at hand to
hold the port to (`LICENSE-third-party.md` lists each file's origin, upstream path and revision).
`crates/io/or-media/src/bink/video.rs` is `libavcodec/bink.c`, `crates/io/or-media/src/bink/dsp.rs` its
integer transform, `crates/io/or-media/src/bink/audio.rs` the DCT and RDFT audio decoders over
transforms of OpenRage's own (`crates/io/or-media/src/transform.rs`), and
`crates/io/or-media/src/bik.rs` the container: the header, the frame index and the packets of a frame. A picture comes out as planar 4:2:0 (`bik::Pictures`) and converts to RGBA
with BT.601, full range for revision `k`; a movie with an alpha plane carries it into the alpha
channel. The revisions `f`, `g`, `h`, `i` and `k` decode and `b` is refused by name; of the 764 Bink 1
movies the installs ship, 382 in each, every one is `BIKi`, so the others rest on the port alone.
Install-gated, FFmpeg-gated `every_shipped_bink1_matches_ffmpeg`
(`crates/title/or-title/tests/every_shipped_bink1_matches_ffmpeg.rs`) decodes every `.bik` both
installs ship, 826 of them, with FFmpeg 7.0 and with `or-media`: all 564,421 pictures of the 764
Bink 1 movies are equal plane by plane in YUV 4:2:0, 257.6 GB, the four with an alpha plane
included, and all 158 sound tracks, 116 Bink 1 and 42 Bink 2, are equal sample by sample within
`AUDIO_TOLERANCE` (1e-4), the largest difference 1.9e-5 over 970,748,160 samples. FFmpeg's DCT is
single-precision and noisier than the port's double-precision one, which is the whole of that
difference. Every shipped track is stereo and DCT, so
`hand_built_bink_audio_decodes_as_ffmpeg_decodes_it` (FFmpeg-gated,
`crates/io/or-media/tests/bink_audio_matches_ffmpeg.rs`) writes Bink audio from the format's layout,
mono and stereo, DCT and RDFT, at 11,025, 22,050, 44,100 and 48,000 Hz, and holds the decoder to
FFmpeg's output there: the RDFT agrees to single-precision rounding. The bit reader, the Huffman
tables, the tree and coefficient readers, the integer transform, the YUV conversion and the audio
transforms each have a synthetic test of their own in `or-media`, and
`a_random_picture_packet_is_decoded_or_refused_and_never_panics` and its sound twin feed both
decoders bytes that are no packet, which is what a corrupt file hands them.
`a_shipped_bik_opens_and_its_first_frame_decodes_at_the_preview_size` and
`a_shipped_bik_with_sound_hands_the_speaker_its_samples_and_pausing_silences_them` (install-gated)
drive the preview over a shipped movie. `a_preview_plays_a_movie_to_its_end_through_whatever_row_decodes_it`,
`playing_cues_the_sound_from_where_the_preview_stands_and_pausing_stops_it` and
`a_start_waits_for_the_sound_to_decode_and_a_movie_with_none_owes_the_speaker_nothing` drive the
clock, seek, sound cues and error paths over a row that yields numbered frames, with no install.

A Bink 2 movie is decoded by `or-media` too, ported from NihAV's `nihav-rad` (its `bink2.rs`,
AGPL-3.0-only, which is why the crate's licence is; `LICENSE-third-party.md` lists the three files),
because no other open decoder has one: `crates/io/or-media/src/bink2/video.rs` is the decoder,
`crates/io/or-media/src/bink2/dsp.rs` the transform, motion compensation and block DC, and
`crates/io/or-media/src/bink2/tables.rs` the tables. `bik::Pictures` picks it by the signature `KB2`
over the container code a Bink 1 movie uses, and the table's `Codec::Bink2` row opens it for the
preview. The revisions `g` to `k` decode, `g` and `j` being the only ones the installs ship, and an
earlier one is refused by name, as is a frame in the coding `KB2f` and before use, which no shipped
frame has. A picture is planar 4:2:0 in full range and converts to RGBA with the BT.601 of
revision `k`; its sound is a Bink audio track of the same container, decoded as a Bink 1 movie's is.
The port differs from NihAV in three sums, which the DLL's pictures fixed: the inverse
transform takes a block's columns before its rows, an inter block's DC takes the 32 of rounding an
intra block's does, and a chroma vector fractional both ways rounds its second pass to nearest; and
in two refusals where NihAV runs on, a read past the packet and a run of flags longer than the flags
left.

What NihAV leaves out is derived from the DLL's disassembly, for interoperability, with the owner's
ruling of 2026-10-06 ([recorded](Project_Context.md#the-bink-2-in-loop-filter-is-derived-from-rads-dll-2026-10-06)):
`crates/io/or-media/src/bink2/filter.rs`, with a header naming the DLL, its version and the function
addresses, and `Reach` in `video.rs`. A frame whose flags state a strength in their low byte, 24 in
most and 255 in some, is filtered after it is decoded, which is the same as the DLL's pass after each
32×32 block because nothing decodes from a filtered pixel: the blocks that code no coefficient lean
toward the DCs of the eight around them that lie within eight times the strength, and then every edge
between 8×8 blocks is drawn together, vertical edges before horizontal, by a code taken from how few
coefficients each side coded and, between two blocks predicted from the picture before, from whether
their 16×16 squares' vectors differ by more than a pixel; the frame's flags and its row and column
flags turn edges off, and a picture two blocks across never draws the left edge of its second
block. An inter block's residue is smoothed across its own 8×8 edges the same way before it is added,
unless the flags say not to. Each square's source is clamped onto the picture, the width and the height
rounded up to 16, before the prediction reads it: the half of a 32×32 block that lies below a picture
whose height is an odd count of 16 is predicted from the clamped rows, and its DCs, which the blocks
after it predict from, follow. The edge and fill kernels are held to the DLL's own functions by a
fuzz that loaded the DLL and called them by address, 700,000 random inputs, with no difference; that
fuzz is not committed, and the pictures below are the standing check.

Install-gated and Windows-only, `every_shipped_bink2_matches_its_installs_dll_plane_for_plane`
(`src/state/previewing_a_video.rs`) decodes every Bink 2 movie each install ships twice, with that
install's `bink2w64.dll`, loaded at run time and never bundled, and with `or-media`, four movies at a
time and in under five minutes: Legacy's 29 movies, 85,206 frames, with its 1.994a DLL, and Enhanced's
33, 89,859, `KB2g` and `KB2j` alike, up to 3840×2160, with its 2020.07 DLL, and every one of the
175,065 frames equals the DLL's in all three planes, byte for byte, which the test asserts of each.
Before the filter the same test measured 3,711 of Legacy's frames and 4,144 of Enhanced's equal and
the other 81,495 and 85,715 within 22 dB, the worst at 24.7 dB. Legacy's 29 movies, decoded by the
Enhanced DLL as well, equal it plane for plane too, so the two DLLs agree on them. Every 50th frame's
RGBA is held to the DLL's within a mean of three levels of 255: the worst of 3,524 pictures measured
2.74 against Legacy's older DLL and 0.84 against Enhanced's, which is that DLL's colour conversion
and not the picture's planes. The hand-written movie of
`a_bink2_movie_plays_to_its_end_through_or_media_whatever_the_platform` plays to its end on every
platform with no install, `a_shipped_bink2_plays_with_its_sound_through_or_media` (install-gated,
any platform) plays `rockstar_logos.bik` with the sound of its track,
`a_random_bink2_packet_is_decoded_or_refused_and_never_panics` feeds the decoder bytes that are no
packet, and the unit tests in `or-media` hold each reader, predictor, vector, clamp, filter kernel,
edge rule and block of the decoder to a hand-written packet or picture.

### R35 The remaining formats

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** The tail: `.yed` and `.ypm`, `.cut`, `.ypdb`, `.yfd`, `.ybs`, `.ybd`; bulk script
extraction including the script keys; and finishing `.fxc`.

**Done when.** `openrage-cli formats` states, per extension, the file count, the container, the
metadata root type and which reader parsed a sample; each remaining format reads and round-trips or
carries a refusal naming what is missing. A format with zero files in a shipped install closes as
absent.

**Where it stands.** The census is done (#59): `openrage-cli formats`
(`crates/tools/or-cli/src/formats.rs`) ranks the tail by population and separates "no reader" from
"detected only" from "not sampled"; since #427 it tries every reader `or-io` has for an extension
and names the one that parsed each sample. Closed:

- `.yed`, `.yfd` and `.ypdb` (#93). `.yfd` and `.yed` are declared in `or-io` (#203, #207) and open
  as owned values (#219, #220) that reach the canonical fixed point
  (`crates/io/or-io-verify/tests/frame_filter_dict.rs`,
  `crates/io/or-io-verify/tests/expression_dict.rs`). `.ypdb` is ordinary code in
  `or-io` by OD-YPDB, byte-identical on 112 of 112 (#93), and `every_ypdb_saves_to_a_fixed_point`
  holds it. Each paged format's `validate` refuses what the game would misread: an unsorted `.yfd`
  track table, which the game binary-searches, and a `.yed` program longer than its
  `max_packed_size` scratch allocation.
- `.fxc` (#98), ordinary code in `or-io` behind its `fxc` feature: 1,446 of 1,446 parse and 321 of
  321 staged files account for every byte; Enhanced ships none. 1,046 variables across that corpus
  carry type codes 17 to 20, which have no name and read as `GrcVariableType::Unknown`.
- `.cut` (#103). ⚠ Its root hash is over the qualified name with `::` written `__`:
  `rage__cutfCutsceneFile2` hashes to `0xD2455C61`, the root of all 788 files, while
  `cutfCutsceneFile2` matches nothing and makes the name look unknowable.
- `.ybd` v43 (#103), declared in `or-io` (#205) and opened as an owned dictionary of `.ybn`'s
  `PhBoundAny` (#253). Its v40 and v42 files are refused by a message naming what their layout
  leaves unfixed (`crates/io/or-io/src/gta5/formats.rs`), measured over the three files Legacy
  ships at those versions, all staged; Enhanced ships none (`openrage-cli versions`):
  des_setpiece at 40 (two BVH bounds and a geometry), des_coroner_window at 42 (a composite
  holding a box) and des_heli_biotech at 42 (two BVH bounds). The dictionaries and the 2 bounds that
  are not geometries read at 43's layout, and 5 of 5 geometries misread there
  (`a_ybd_at_40_or_42_misreads_its_geometries_at_43s_layout`). A v42 geometry is 0x10 shorter: `num_convex_hull_vertices` sits two bytes earlier, where 43 keeps
  `num_per_vertex_attribs`, and the vertex and polygon counts 8 bytes earlier, among pointers and
  padding that are zero in every geometry. A v40 geometry is 42's with 0x10 more after the bound
  header, holding 0.04 three times and a `w` of bits 1. What is missing is where
  `num_per_vertex_attribs`, `octant_vert_counts` and `octant_verts` sit at 40 and 42, since every
  vertex-attribute and octant pointer at both is null, and a name for v40's extra vector. The
  compiled reader refused all three files too, so the testimony records no field of them.
- `.ypm` and `.ybs` close as absent; `.ysc` moved to [R61](#r61-script-containers).

The editor opens `.yed`, `.yfd`, `.ypdb` and `.fxc` under their own `DocumentKind`, `.ybd` as a
`Registered` document and `.cut` in the metadata editor (#421).

Bulk script extraction and the script keys are [R61](#r61-script-containers)'s. `.ybd` at 40 and
42 reopens only with evidence that places the fields its refusal names.

**Risk.** Death by a thousand small formats; the census is the defence.

### R35b Scaleform movies

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** `.gfx`, the Scaleform movies behind the HUD, the minimap, the menus and the phone: the
container, its tags, the textures and characters it names, and a trip out to a standard SWF that
an editor such as JPEXS opens and back.

**Done when.** Every `.gfx` in both installs parses and saves byte-identical; a movie exports to
an SWF and that SWF imports back, refusing by name what a `.gfx` cannot hold; the app and the CLI
open one.

**Where it stands.** `or_io::gfx::GfxMovie` (`crates/io/or-io/src/gfx.rs`, behind the `gfx`
feature) keeps a movie as its frame header and its tags, each a code and its body, so a tag it
does not interpret round-trips untouched. A movie is an SWF body under `GFX`, or `CFX` with the
body a zlib stream; every shipped movie is SWF version 8, 9, 10, 15 or 17, and one per install,
`gfxfontlib.gfx`, is compressed. ⚠ Two things a naive writer loses: shipped movies write many
short tags with the long header, which `Tag::long` keeps, and no zlib level reproduces the shipped
compressed stream, so a compressed movie whose tags are unchanged keeps the stream it was read
from. `texture_refs` reads Scaleform's external images and gradients, which name a texture by
file (`aircraft_dials_G0.dds`) found beside the movie in its namesake `.ytd`; `imports` reads
`ImportAssets2`, through which movies take fonts and symbols from `font_lib_efigs.swf` and its
siblings.

`every_shipped_movie_lists_its_tags_and_saves_byte_identical` (`crates/io/or-io/tests/gfx_movie.rs`,
install-gated) holds, on top of `b78cefe5`, 1,160 of 1,160 Legacy and 1,163 of 1,163 Enhanced
movies byte-identical on save and through SWF and back, over 154,190 and 155,671 tags, 3,242 and
3,255 texture references and 3,023 and 3,035 imports. Hand-built movies hold the long header,
an edited compressed movie, the refusals and the SWF trip in the same suite.

The export is the same body under `FWS` or `CWS`, Scaleform's own tags kept, which JPEXS reads as
unknown tags and writes back: `busy_spinner.gfx` exported, converted to JPEXS's XML and back to
SWF by `ffdec-cli`, then imported, is byte-identical to the shipped file. An LZMA SWF (`ZWS`) has no
`.gfx` form and is refused by name. `openrage-cli gfx info|export|import` lists a movie's tags
with their lengths and the characters they define, its texture references and imports, and
converts either way. The app opens a `.gfx` as `DocumentKind::Gfx`: the inspector lists the same,
exports the movie to SWF and replaces it with an imported SWF, which saves as `.gfx`.
`openrage-cli formats` names `GfxMovie` as the reader of a `.gfx`
(`crates/tools/or-cli/tests/the_census_names_every_reader.rs`), and the self-check draws a
movie's tags and texture reference in the inspector.

**Open.** Editing a tag's fields, and drawing a movie, are unscoped.

### R35c Move network definitions

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** `.mrf`, the move network definitions in `networkdefs.rpf` that drive a ped's animation:
the state machines, their states, the transitions between them with their conditions, and the
animation nodes (clips, blends, filters, expressions) each state runs. The owner reported that
`update/update.rpf/x64/patch/anim/networkdefs.rpf/taskexitvehicle.mrf` did not open.

**Done when.** Every `.mrf` in both installs reads and saves byte-identical; `openrage-cli convert
info` and the census report it; the app shows the network.

**Where it stands.** `or_io::mrf::MvNetworkDef` (`crates/io/or-io/src/mrf.rs`) reads the header
(MoVE version 2.0.0.0, refusing any other), the requests and flags, every node and the string
table. The state machine, inlined state machine and state kinds read field by field, with each
transition's conditions typed by kind; every other node kind keeps the bytes after its name hash,
measured by its flags, so a kind it does not interpret round-trips untouched. Offsets between
nodes stay as stored, relative to the field that holds them, and `edges` resolves them to node
indices; a save rewrites each transition's size, condition count and filter bits from its
content. No shipped network has a cross-reference or a string table byte, so every name is a hash.

`every_shipped_network_reads_and_saves_byte_identical` (`crates/io/or-io/tests/move_network.rs`,
install-gated) holds, on top of `dfa73ef6`, 162 of 162 Legacy and 160 of 160 Enhanced networks
byte-identical on save, every state machine's states, every state's initial node and every
transition's target resolving to a node, over 8,000 and 7,996 transitions with 9,136 and 9,132
conditions; the owner's file is among them. Hand-built networks in the module's tests hold the
refusals and the rewritten transition flags. No shipped network has an animation, parameterized
motion, proxy, add-N, mirror, IK, joint limit or reference node, so those kinds' layouts are held
only by the engine's reading order; ragdoll, mover, ponytail and style nodes are refused by name.

`openrage-cli convert info` lists a network's node counts by kind and each state machine with its
states and their transition counts (also as JSON), and `convert`, `verify` and the round trip read
it. `openrage-cli search --verify --ext=mrf` parses 162 of 162 Legacy and 160 of 160 Enhanced
networks with none refused. `openrage-cli formats` names `MvNetworkDef` as the reader of a `.mrf`
(`crates/tools/or-cli/tests/the_census_names_every_reader.rs`). The app opens a `.mrf` as
`DocumentKind::MoveNetwork` in the animation workspace, from disk and from an archive
(`the_owners_network_opens_from_the_tree`); the inspector shows each state machine, its states with
the initial one marked, each state's transitions with their target, duration, blend modifier and
conditions in words, and the nodes grouped by kind, every hash named through the app's name index
where it resolves. The self-check draws a two-state network in the inspector.

**Open.** Editing a network, and naming its hashes where the name index does not know them, are
unscoped.

### R36 Console and platform variants

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**Done** as declined, on the measurement it asked for: neither reachable install carries a console
artefact (`.xex`, `.self`, `.sprx`, `.xzp`), none of the 6,374 staged corpus files is console, and no
partial big-endian, Xbox 360 or PS4 implementation exists to finish. Reopen only with fixtures.

### R37 Metadata and XML at parity

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** One metadata editor across every structured type, with a raw-bytes tab, writing back into
the archive; PSO/RBM/RBF0 ⇄ XML and binary ⇄ plain text as true round trips, in bulk; a published,
versioned list of the supported root types; validation on demand and on save, with a save-anyway;
withholding output that cannot be re-imported cleanly; two-pass serialisation with
encrypted-final-strings detection; and a declarative schema-mapping framework.

**Done when.** `openrage-cli meta types` publishes every root type with counts and containers;
`meta to-xml --batch` withholds what will not re-import; `binary → XML → binary` reaches a fixed
point; and with no snapshot, a name the build cannot resolve exports as `hash_XXXXXXXX` and
imports back to its hash. The schema-mapping framework the row also asked for is declined (owner,
2026-10-02): a file's own type table already opens any type with no schema to define, the
snapshot a user builds from the PSC files is the names list, and other names go through an
exported XML edited and imported back.

**Where it stands.** Done. The
metadata reader is `or_io::meta`, behind `or-io`'s `meta` feature (D5, #267), and the roots of the
world family, of scenario regions and of the ped variation family may be declared by hand over the
document's own type table (OD-META-TYPED, #280, R33d, and the Workbench plan's wave B);
everything else is schema-driven.

- **The published list** (#65, #112): `meta types` lists every root type with counts, containers
  and how each name was found; `binary::root_types` accepts a name only where its hash matches a
  shipped root, so a wrong entry attaches to nothing. Every one of a Legacy install's 29,492
  metadata files parses.
- **The XML ruling** (owner, 2026-09-22): the exported XML carries no types, field types come from
  the struct definitions, and export is canonical, so every cycle after the first is byte-identical
  (`C1 == C2`, not `C1 == O`). CodeWalker's XML is not a compatibility target.
- **Why** (#128): `write_meta` renders `String`, `Enum`, `Hash` and `Bitset` alike and `Int` and
  `UInt` alike, so no parser can invert the export
  (`crates/io/or-io-verify/tests/meta/what_the_xml_cannot_say.rs`). Binary to XML needs vocabulary;
  XML to binary needs structure. ⚠ A schema-free parser guessing `Int` on a `u64` field writes zero
  above `i64::MAX` rather than failing. A pointer keeps its `type=` attribute, because a polymorphic
  target is the one fact a struct definition cannot supply.
- **The canonical form** (#176, #185): `or_io::meta::canonical` exports with no types, imports
  donor-free through `binary::synth` (`meta from-xml --as`) and holds `import(export(C1)) == C1`;
  `meta to-xml` withholds what does not converge, and `--like` a donor remains for a structure the
  staged snapshot lacks. At `2a55ed1c`, over the 6,071-document corpus with a 3,128-struct snapshot
  (`crates/io/or-io-verify/tests/meta/xml_converges_against_the_snapshot.rs`), PSO 3,520 of 3,527,
  RBM 2,331 of 2,338 and RBF 206 of 206 reach the fixed point; the rest are refused, because their
  type table predates a member the PSC declares.
- **The editor** (#116): `src/ui/panels/meta_editor/` derives the field list from the instance when
  no snapshot is staged, each at its family's widest width, and a hash edits as a string a save
  keeps (`a_derived_type_manufactures_the_variant_it_came_from`).
- **Validation is the save** (#133): `MetaDocument::refresh_validation` calls `to_bytes`, and a
  refusal is a disabled Save in the writer's own words
  (`src/state/document/a_document_that_will_not_save.rs`) that names the element since #185
  (`a_refusal_names_the_element_it_stopped_at`).
- **What the editor opens** (#414, #439): `.ymap` and `.ytyp` open in it. A metadata XML whose
  root or pointer type no schema declares, such as `images.meta` and `weather.xml`, reads as the
  tree its own elements state (`or_io::meta::instance::parse_schemaless`) and saves back as XML;
  XML the parser's dialect could not have written is refused by name. A binary `.srl` reads as its
  frames (`or_io::meta::srl`) and saves byte-identical, and an archive entry is routed by its bytes
  as a file from disk is (`crates/io/or-io/tests/metadata_without_a_schema.rs`,
  `src/state/opening_metadata_without_a_schema.rs`). Of the 529 text `.meta` and `.xml` files in a
  Legacy `common.rpf`, measured for #439, 422 read that way and reach their own fixed point; the
  rest, maps keyed by `Item key=`, `content=` arrays, `ref=` and other XML outside the dialect,
  stay refused.
- **A cut or corrupted file is refused** (#449, #451): the RBF0, PSO and RBM readers check every
  length the file states, cap a capacity by the bytes left, and refuse re-entering an address as
  the same type or nesting past `READ_DEPTH`, so a cut or corrupted file is an error rather than a
  panic or an aborted process. `crates/io/or-io/tests/a_truncated_metadata_file_is_refused.rs`
  reads every strict prefix and single-byte corruption of a synthetic file and of the smallest
  staged file of each format. ⚠ The cap is 32 levels, not synthesis's 128: at `opt-level = 0`, as
  hosted CI builds, 128 levels overflow a test thread's 2 MiB stack, and the deepest corpus
  document nests 12.
- **A clean clone** (owner, 2026-10-01,
  [recorded](Project_Context.md#a-metadata-name-the-build-cannot-resolve-is-spelled-as-its-hash-2026-10-01)):
  no snapshot is shipped and none is needed. A name the build cannot resolve, whether a field, a
  type, an enum value or a hashed string, is spelled `hash_XXXXXXXX` in the editor and in exported
  XML, and import reads `hash_` and exactly eight hex digits back to that hash. Import takes the
  structure from type tables (`binary::type_tables`): a donor's, the document's own in the
  canonical check, else the install's, which `meta from-xml --as` reads once an import fails for
  want of a definition; a structure no table states is refused by name, and a snapshot's
  definition wins where it has one. With `OPENRAGE_SCHEMA=none`, 3,527 of 3,527 PSO, 2,338 of 2,338
  RBM and 206 of 206 RBF corpus documents reach their fixed point and none is refused, and a
  hand-edited value survives the import
  (`crates/io/or-io-verify/tests/meta/xml_converges_without_a_snapshot.rs`, over `2dfd9dcf`). The
  snapshot built from the PSC adds real names, found through `OPENRAGE_SCHEMA`, the user's
  configuration directory or the shared store in `openrage-corpus` (OD-SCHEMA-LOC) and never
  committed; without one only `binary::public_names` and `binary::root_types` name anything, 122
  of 9,629 corpus name-hashes (`crates/io/or-io/src/meta/README.md`).

- **A source in a project** (wave P10 of [Workbench_Plan.md](Workbench_Plan.md)): an imported `.ymt`,
  `.ymap` or `.ytyp` becomes this canonical XML beside the file it came from, compiled back
  through that file's own type table, and only when its XML reaches the fixed point with no value
  moved; any other is kept as a compiled file with the reason
  (`crates/domain/or-source/tests/metadata.rs`;
  [Source_Format_Design.md](Source_Format_Design.md#fragments-and-metadata)).

**Outside the done-when.** Whether the game loads a synthesised file
([R62](#r62-the-rbm-layout-synthesiser)'s clause). The writers refuse resizing an array in place;
growth goes through R62's re-synthesis.

**Depends on.** [R21](#r21-extraction-and-bulk-jobs), for bulk jobs.

### R38 Mesh interchange

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** glTF export; FBX import into a drawable from the browser; and an open, documented,
text-based interchange family for resources that must round-trip through external DCC tools, with a
validator that reports specific errors (unsupported version, missing referenced files, unknown bound
type, null bone name, non-skinnable shader, per-geometry bone limits, and the rest).

**Done when.** A drawable exports to glTF and opens in Blender with materials and skinning intact; a
mesh comes back in through FBX or the text family, and the validator catches the standard mistakes
before the file reaches the game.

**Where it stands.** Export (#111) and import (#134) live in `crates/domain/or-interchange` over the
owned drawable (#310), behind `openrage-cli mesh export|import|census`; the second clause is met
through glTF, itself an open, documented, text-based format. [Mesh_Interchange.md](../features/Mesh_Interchange.md)
owns the mapping and its decisions. ⚠ Found while closing
[R38b](#r38b-gltf-in-and-out-from-the-app): for a fragment with no common drawable, `--entry` counts
one way on export, which skips the missing drawable, and another on import, whose entry 0 is always
the common drawable.

- The validator is the save. `replace_in_file` opens the `.ydr`, `.ydd` or `.yft` as an owned value,
  replaces one drawable's geometry and saves it, and every drawable-bearing format's validation runs
  `or_io::gta5::drawable::each_defect`, so any owned save of an incoherent drawable is refused, not
  only an import (`every_mesh_defect_is_refused_at_save` in
  `crates/io/or-io-verify/tests/drawable.rs`; the import's own refusals in
  `crates/domain/or-interchange/tests/a_mesh_comes_back.rs`).
- A save that would rewrite a count a geometry's file stated against its run (`primitive_count`,
  `vertices_count`, `indices_count`) is refused by name unless the caller passes
  `or_io::decl::Repair::Lossless`, and then repairs only a count it can rewrite losslessly
  ([2026-10-03](Project_Context.md#a-drawable-whose-counts-contradict-its-runs-is-repaired-only-when-asked-and-only-losslessly-2026-10-03));
  two mod `.ydd` staged in `openrage-corpus/mods/` are refused unasked and repaired asked, and a
  coherent one saves the same either way (`asked_to_repair_a_mod_ydd_rewrites_its_counts_from_its_runs_and_says_so` in
  `crates/io/or-io-verify/tests/drawable_dict.rs`).
- Staged drawables of both targets export and pass the Khronos glTF-Validator's rules, which
  `crates/domain/or-interchange/tests/every_corpus_drawable_exports.rs` restates in Rust, and a
  triangle list is decided by the geometry's own counts, not `primitive_type`
  (`the_shipped_primitive_type_still_reads_as_a_triangle_list`). Both targets re-import to the
  same geometry, each read and written as itself, from the library and through
  `openrage-cli mesh export|import` (`crates/domain/or-interchange/tests/every_corpus_drawable_comes_back.rs`;
  the Gen9 vertex formats' rows are in [Mesh_Interchange.md](../features/Mesh_Interchange.md#what-gen9-adds)),
  and Gen8 ones export the `.glb` R66e's baseline recorded
  (`crates/domain/or-interchange/tests/every_corpus_drawable_exports_as_recorded.rs`).
- ⚠ A skinned primitive's mesh space comes from `joint_world · IBM`, not its node, and `Weight` and
  `Binding` are lane payloads whatever width they are declared at: a `Binding` declared `Color`
  through the normalised encoder clamps bone 5 to 1.

- An FBX is read by `or_interchange::fbx` (binary 7.1 and later, through `fbxcel`) into the same
  scene, and `openrage-cli mesh import` takes one by its extension, printing what the drawable had no
  place for (`crates/domain/or-interchange/tests/an_fbx_comes_in_as_a_scene.rs`); every reader says
  what it left out in `Scene::dropped`. A drawable or a drawable dictionary is also a source in
  `or-source` (wave I of [Workbench_Plan.md](Workbench_Plan.md)): extracted to a glb and a
  `drawable.json`, compiled back by replacing the geometry of the base it keeps, a ped component
  beside its skeleton, and recooked when a glb is saved
  (`crates/domain/or-source/tests/every_corpus_drawable_comes_back.rs`;
  [Source_Format_Design.md](Source_Format_Design.md#what-wave-i-built)). A fragment is a source of its
  own, its common drawable the glb and the rest of the fragment the base's
  (`crates/domain/or-source/tests/fragments.rs`;
  [Source_Format_Design.md](Source_Format_Design.md#fragments-and-metadata)).

- Wave P12 of [Workbench_Plan.md](Workbench_Plan.md) made the import pair a glb with the drawable by the
  names it carries and cut a mesh past 65,535 vertices or 256 bones into geometries with a report,
  where both were refusals (`crates/domain/or-interchange/tests/pairing_by_name.rs`), and let a
  source's `drawable.json` state the base's shaders and parameters, level distances, masks, bounds,
  flags and bone tags and set the ones an edit changes (`every_corpus_ydr_states_its_values_and_takes_an_edited_parameter`
  and its siblings in `crates/domain/or-source/tests/`;
  [Source_Format_Design.md](Source_Format_Design.md#what-wave-p12-built)). A shader is still the base's:
  changing one is authoring.

- Wave P30 of [Workbench_Plan.md](Workbench_Plan.md) added the mesh doctor, `or_interchange::doctor`, which changes a drawable through the import and keeps each geometry's own shader and vertex declaration: a level of detail the drawable lacks, made from its High level by a decimator that keeps every vertex it leaves exactly as it was and never tears a seam (`the_ped_components_of_the_corpus_get_the_levels_they_lack_and_keep_their_skin`, `every_body_geometry_of_both_targets_reaches_35_and_15_percent_of_its_triangles`), a mirror that is its own inverse (`mirroring_twice_is_the_identity`), the repair of a rig whose group no skeleton knows (`a_group_with_no_name_of_a_bone_is_found_and_its_weight_goes_to_the_bone_above_it`), cleaned weights, a look at a shoe's heel, and the part of a texture a mesh uses with the crop of every texture that reads it; the Wardrobe runs them over a garment ([Clothing_Tool_Design.md](Clothing_Tool_Design.md#what-the-engine-builds)). The level is the one geometry the import adds, and it chooses no material or layout ([Mesh_Interchange.md](../features/Mesh_Interchange.md#the-mesh-doctor)).

**Open.** Blender is unverified: the Khronos validator is a proxy for a reader, not that reader. An
FBX is never written, since no writer is permissive in Rust and CodeWalker's is GPL, and the FBX
reader has seen no file but synthetic ones and one animation-only 7.3 file, so what Blender and
Maya write is unverified. Importing an FBX from the app's browser is not started. `mesh census`
reads an Enhanced drawable only for its refusals. Authoring a drawable from scratch, adding or
removing a geometry of a level that is there, normal and specular generation, animation, rigid bone attachment and sampler state are not started.

**Depends on.** [R41a](#r41a-vertex-and-index-data-as-an-editable-model) and
[R40a](#r40a-the-shader-definition-database-as-data), both done.

### R38b glTF in and out from the app

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** Export to glTF and import from glTF as commands on a drawable, a drawable dictionary or a
fragment, with the report [R38](#r38-mesh-interchange) writes shown before anything is written.

**Where it stands.** Done. Export to glTF… and Replace from glTF… are registry commands
(`src/ui/commands/files.rs`) that a row's menu in Files and in the Files view offers on a `.ydr`,
`.ydd` or `.yft`, and a model's tab and its row in the Outline offer on the open document; the
palette and a key export what the focused panel has selected, else the active document, and import
over the active document. Each is worked out as a glTF job in the jobs list (`src/state/gltf.rs`)
through the calls `openrage-cli mesh export|import` makes, `or_cli::export_scene` and
`or_cli::replace_mesh`. The export's question shows what glTF carries of the model and each line of
R38's report, and offers the drawable of a `.ydd` or `.yft`, every level of detail, the model's own
textures, and `.glb` or `.gltf`; its Export… asks where with Export as…'s dialog, and the edit gate
is asked at the write, which is refused where the gate would ask about the game as shipped, as
Export as…'s is. A row exports as its file holds it, an open document as a save would write it. The
import reads the `.glb` or `.gltf` the platform's dialog names over the open model, which a row's
menu offers once its file is open, and shows each line of the import's report, or the library's
refusal in its words with nothing changed, beside the drawable it replaces and the two choices
`--fill-missing` and `--keep-skinning` make, in words alone
([R59t](#r59t-the-default-views-in-a-modders-words)); its Replace puts the model the import saves as
into the document as one step of its history, and nothing is written until the model is saved. An import is refused wherever
its save would be, with the save's reason, so the model it leaves always saves.
`a_ydr_exports_and_imports_back_from_its_menus_and_saves_what_the_cli_writes`
(`src/state/gltf_in_and_out_from_the_app.rs`) exports a staged Legacy and an Enhanced `.ydr` from
its row's menu and finds the bytes written the command's; it then imports a copy of that export
lifted 2 m over the model its tab's menu offers the import on, and finds the report the command's,
the model unchanged until Replace and one step after it, the viewport building it again with the
lift, the save the command's import and the undo the model as opened. It failed while the menus
offered no glTF. `the_exports_choices_reach_the_files_it_writes` and
`an_import_the_library_refuses_says_why_and_changes_nothing` hold the choices and a refusal. Seen
on `just shots` screens, not in a window. Found while closing
[R27e](#r27e-a-field-that-shapes-the-data-is-not-edited-in-place): nothing recomputes a drawable's
or a shader group's `container_size_qw` after a mesh import or a material change; it is 0 in every
Enhanced file walked, and what the game does with a stale one is unmeasured.

**Done when.** A `.ydr` exports and imports back from its context menu, and the result saves.

### R39 Textures

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**Done** over #83, #85, #86 and #88; every clause is delivered or deferred on a measurement. The
edits run on the owned texture types (`src/state/texture_edit.rs`). A texture's source form, its exact DDS and the rules a build compiles it from, is `or-source` ([Source_Format_Design.md](Source_Format_Design.md)); it extracts a `.ytd`, compiles the folder back and recooks one saved asset, with channel packing and a re-encode of only the blocks an edited image changed ([Workbench_Plan.md](Workbench_Plan.md), waves A and G), and, from wave P11, rules a DDS alone refuses by name, header words as named lists, inputs of more than 8 bits a channel and a JSON Schema for each source file.

- `_hi` and `+hi` companions (`or_io::gta5::data::asset_name`) rank below an exact stem. Embedding
  and unembedding work over a Legacy drawable's shader group (`GrmShaderGroup::embed` and `unembed`);
  unembedding rebinds every material that bound the texture to a reference by name and reports how
  many now need an external dictionary
  (`unembedding_and_embedding_again_keeps_every_binding` in `crates/io/or-io-verify/tests/drawable.rs`).
- A loose `.dds` opens in the texture workspace as `DocumentKind::Dds`, held as the one Gen8
  `GrcTexture` it is (`src/state/dds_document.rs`), so its mips are viewed, replaced from an image
  and saved back through `or_io::dds`; a cube map, volume or array is refused by name (#437). The
  preview exports any texture it shows, the mip on screen as a PNG or the chain as a DDS
  (`export_texture` in `src/state/textures.rs`).
- Format, quality, mip count and sRGB are edited in the texture preview through
  `reencode_texture_in_place`, over a Legacy `GrcTexture`; an Enhanced dictionary refuses with a
  reason. Search paths keep the order the user gave, which is the feature; dictionaries attach one at
  a time, `ResolutionRule::Attached` ranking below `Pinned` (`src/state/texture_resolution.rs`).
- One list of accepted sources, `or_io::authoring::texture_import` (PNG, JPEG, BMP, TGA, TIFF, DDS, and since
  wave P11 PSD, Radiance HDR and OpenEXR, with the rules an image of more than 8 bits comes down by),
  serves the CLI and the app, and `FormatChoice::Automatic` picks BC1 or BC3 by opacity. ⚠ An import
  never touches the classification byte, which would make an entry a render target in game
  (`importing_pixels_leaves_the_usage_classification_alone`).
- A pack's textures are made lighter without the editor's texture edit: `or_source::optimise` adds the mips a texture lacks under the blocks it has, brings a side to the nearest power of two, stores an uncompressed texture as BC1 or BC3 by its alpha and cuts the size by a ratio or to a longest side, writing only the DDS of a texture that changes, and the Wardrobe runs it over a copy of each source (Optimise textures… and Fix all safe, `openrage-cli wardrobe textures`; [Clothing_Tool_Design.md](Clothing_Tool_Design.md)). The Texture workspace still puts a new texture only into a Legacy dictionary.
- Wave P35 of [Workbench_Plan.md](Workbench_Plan.md) completes the rules a source states: `alpha` (straight or premultiplied) and `alpha_coverage` (a cutout keeps its share of texels at or above a cutoff down the mips) are honoured by the compile and refused by name where a texture cannot keep them, an image of 16 bits a channel compiles only with a `clamp`, which a picture is given (`[0, 1]`, its whole range) the moment it is put into a source, and `or_source::textures::inputs_of` says which file each texture compiles from. Normal and specular maps made from a diffuse (`or_source::maps`) and a texture budget with a PSNR for every choice (`or_source::budget`) write pictures and sizes into copies of a pack's sources ([Source_Format_Design.md](Source_Format_Design.md#maps-made-from-a-diffuse)); the optimiser reaches the textures embedded in a drawable.
- Usage and streaming are read-only rows (`usage_rows` in `src/ui/panels/inspector/mod.rs`): Gen8
  classifies by the low two bits of `resource_type_and_conversion_flags` and streams by
  `cut_mip_levels`; a Gen9 texture shows its own header through `textures::enhanced_texture_at`,
  with no `cut_mip_levels` row because that field is Gen8's.

Deferred on measurement: BC6H, which has no encoder and occurs in neither install; tiled Gen9
textures, which `GrcTextureG9::to_png` refuses rather than reading as linear
(`an_enhanced_texture_with_a_swizzled_tile_mode_is_refused_by_to_png`); and BCn rate-distortion
optimisation, which neither encoder in the tree offers. Refusing to delete a texture another
material still needs is a [known gap](#deferred-and-known-gaps) since #332.

### R40 Materials and shaders

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**What.** A material editor over every declared parameter; a shader definition database per title;
data-driven Gen8↔Gen9 parameter and texture-name remapping, which
[R55](#r55-retargeting-between-gen8-and-gen9) also needs; a compiled-shader registry with reflection;
and a maintainer-side pipeline that regenerates these tables from an install.

**Done when.** R40a through R40f are done; this row is a programme header. Each part has its own data
lifecycle, in the order `R40a → R40b → (R40c ∥ R40d ∥ R40e ∥ R40f)`. Every part is done.

**Risk.** The database is per title and per generation and rots against game updates unless it is
regenerated, which is why R40b is part of the programme.

### R40a The shader definition database as data

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**Done** (#87). The table is not authored: every field is stated by the game's own `.fxc` and `.sps`
(`crates/io/or-io/src/gta5/shaders/fxc/mod.rs` and `crates/io/or-io/src/gta5/shaders/sps.rs`), and
[Shader_Definition_Database.md](../features/Shader_Definition_Database.md) owns the format, the loader, the
maintainer CLI and the coverage figures. ⚠ A file addresses a parameter by the effect's semantic as
well as by its variable name, and the two spellings hash differently for many parameters, so the
database indexes both and tries the name first
([name, then semantic](../features/Shader_Definition_Database.md#name-then-semantic)). The Enhanced half is
[R40e](#r40e-the-enhanced-shader-container).

### R40b The regeneration pipeline

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**Done** (#89), with the diff as its deliverable. `or-shader-tool defs diff` compares two databases —
a stored `.json`, an install root or a directory of loose `.fxc` — keyed by name, and prints
`no change` on an unchanged install. `or-shader-tool defs check` compares a database against the files
the game ships and classifies the residue: a hash another effect declares is a material pointed at
the wrong shader, and one no effect declares is drift. That split is what found R40a's semantic
defect. The code is `crates/io/or-io/src/gta5/shaders/shader_defs/diff.rs` and `coverage.rs` beside
it, driven by `crates/tools/or-shader-tool/src/defs.rs`.

### R40c The material editor

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**Done** (#89, #94). The app loads a database (`src/state/shader_defs.rs`, from
`OPENRAGE_SHADER_DEFS` or the mounted install) in place of a hand-written catalogue. The editor,
`src/ui/panels/inspector/material.rs`, works on the owned `GrcInstanceData` of a Legacy `.ydr` or
`.ydd`: it lists every declared parameter with its name, type and register, edits constants and
texture names, adds a declared parameter the material lacks (`GrcInstanceData::insert_entry`), and
reorders shaders while renumbering every geometry. Clearing a texture slot and removing it are
separate commands, because the game falls back to the shader's default only for a removed slot.
Reset reads the file's value (`MaterialBaseline`), the preset's or the effect's, and refuses an
integer default, whose bits a shader may read with `asint`. With no database staged, the preview and
the inspector say nothing was asked rather than reporting every parameter as unrecognised.

⚠ The edits carry an invariant no reader checks: `grcInstanceData` counts its textures separately
and the engine reads them as a prefix, so an appended sampler reads back fine here and is misread in
game. The owned save writes `texture_count` from the entry list and refuses a texture after a
constant (`a_texture_after_a_constant_is_refused_at_save`), and every shipped Legacy material that
states a count satisfies the claim
(`a_texture_slot_is_a_prefix_of_the_entry_list_and_the_count_says_how_long`, both in
`crates/io/or-io-verify/tests/drawable.rs`). A Legacy entry is written under the declared name
(`Spelling::of`); the figures are in
[editing a material](../features/Shader_Definition_Database.md#editing-a-material). Re-pointing a geometry at
another material is a mesh operation for [R41](#r41-geometry-physics-cloth-and-lights-as-core-data).

### R40d The Gen8 Gen9 parameter remap

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**Done** (#100). Gen9 renamed nothing: a Gen8 file addresses a parameter by its declared name and a
Gen9 file by its semantic, both out of one `.fxc`, so the remap is the R40a database.
`or_io::gta5::shaders::shader_defs::remap` looks it up (`ParamRemap::of`, then `translate` or
`translate_name`), keyed by `(shader, hash)` because in many shaders one parameter's semantic is
another's name. Writing is one-way per target, chosen only by `Spelling::of`.

A Gen9 material inspector (`src/ui/panels/inspector/gen9.rs`) sits on the Shader node of a Gen9
`.ydr`, `.ydd` or `.yft`, over the owned `GrcInstanceDataG9` (#325), constants editable in place.
⚠ Gen9's invariant is not Gen8's, so porting Gen8's checks nothing: Gen9 states one count per region
and reaches each region by explicit index, so a count that disagrees with its lists moves every
later region, and an index outside its run reads the next region's pointer. The owned save writes
the counts from the lists (`a_materials_counts_are_its_lists_lengths`) and refuses a parameter whose
slot or range lies outside its block, and every shipped Gen9 material satisfies the claim
(`every_shipped_enhanced_material_already_satisfies_the_claim`, both in
`crates/io/or-io-verify/tests/drawable.rs`). Two parameters naming one slot are deliberately not
refused: each addresses a real slot. The Gen9 `preset` word is one constant FourCC in every shipped
material, so the key is the shader's name hash
(`the_preset_field_is_a_constant_tag_and_the_name_field_is_the_key`). Re-pointing a texture binding
and adding or removing a parameter are not offered.

### R40f The compiled-shader registry and the ABI slot table

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**Done** (#120, #126): a compiled shader's constant buffer reads as a typed struct, and
`or-shader-tool list` reported per-stage translation through the old manifest's coverage, since removed with the old chain.

- `or_io::gta5::shaders::fxc::DxbcReflection` (`crates/io/or-io/src/gta5/shaders/fxc/rdef.rs`)
  decodes `RDEF` into constant buffers with named, typed, offset-annotated members and resource
  bindings, printed by `or-shader-tool reflect`. The variable stride is 40 bytes in shader model 5
  and 24 in 4 (`both_variable_strides_read_the_same_variable`). `D3D_SVF_USED` is carried and a
  layout ignores it, because an unused member still occupies its offset.
- Most pixel-shader translation failures shared one message: an engine register the particle ABI
  table has no row for. That table is semantic and right for particles, but general effects bind
  registers such as `t0`, `s0` and `cbuffer 5` (`rage_cbinst_matrices`) under names with no fixed
  meaning, so the baker got a mechanical second table, since removed with the old chain — one bind group per resource
  class, binding equal to register — chosen by the effect's `ShaderKind`, with the host building its
  layout from the shader's own `RDEF`. At `364863f0` it covers all 49 rows the particle table lacks
  (a test printed that reading without
  asserting it); another pinned the
  particle table's sixteen rows. Both tables went with the old chain.
- ⚠ A per-stage limit bounds a count, not a register index: the baker refused a
  shader binding more than 16 sampled textures (7 of 9,521 blobs at `364863f0`), which would
  otherwise reach the baked shader pack as valid WGSL and fail at `create_render_pipeline` on a
  user's machine.

[R42](#r42-the-viewer)'s translation rate
([the cut mode](../features/Viewport_Shading_Modes.md#cut-a-mode-running-the-games-shaders)) predates both
tables and is not re-measured, because a bake needs an HLSL decompiler and `dxc`; R42's
real-game-shader mode re-opens when a re-bake measures it against the two tables.

### R40e The Enhanced shader container

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**What.** A reader for the container Enhanced keeps its shaders in, and an
[R40a](#r40a-the-shader-definition-database-as-data) database built from it. Enhanced ships no `.fxc`
and few `.sps`; its shaders are `sga_win32_60_final.awc` and its siblings in `update/update2.rpf`.

**Done.** The `common:/` mount reaches `update/update2.rpf` (`build_vfs_for_install`);
`crates/io/or-io/src/gta5/shaders/sga.rs` reads both containers to their last byte, checking every
variable's name against the `atStringHash` its record stores; and `load_shader_definitions`
assembles a `gen9` database from them and the 26 `.sps` the install ships. From a retail Enhanced
install, `defs build --target gen9` writes 398 shaders and 48,710 parameters
(`an_enhanced_install_builds_a_database_that_names_its_shaders`), and `defs check` reads every
staged Gen9 `.ydr`, `.ydd`, `.yft` and `.ypt` through `materials_in`, which opens a file at the
layout its RSC7 version names (`defs_check_reads_every_enhanced_drawable_in_the_corpus`): 48,498 of
48,741 entries named, the rest samplers a material binds and only another effect declares, and
three texture hashes no effect declares
([coverage](../features/Shader_Definition_Database.md#coverage-and-limits)). ⚠ The container keys annotations
and effect properties by a hash that is not `atStringHash` of their names, so a Gen9 entry carries
no `UIName`, no vertex type and a draw bucket only from a preset; and Enhanced ships 26 of the 411
presets its own preset list names. An Enhanced material is keyed by its `name` hash, because its
`preset` field is one FourCC, `0x6D657461`, in every shipped material. The Gen9 inspector's
add-a-parameter list is not built.

**Done when.** `defs build --target gen9` produces a non-empty database, and `defs check` over the
corpus's Gen9 drawables reports coverage rather than refusing.

### R41 Geometry physics cloth and lights as core data

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**What.** The data model the Phase 7 plugins build on — vertex data, bounds and BVH, physics, cloth,
shattermaps, lights and bone identity — in five parts,
[R41a](#r41a-vertex-and-index-data-as-an-editable-model) to
[R41e](#r41e-lights-shattermaps-and-bone-identity), all done. On the owned values a caller builds a
bound from a mesh (`crates/io/or-io/src/gta5/bounds/build.rs`), weights a cloth vertex by bone tag,
adds a light to `GtaDrawable::lights` and recomputes a bounding sphere (`Aabb::bounding_sphere` in
`or_io::codec::mesh`) through public `or-io` APIs, and each part's traps are stated in the code they
apply to. A light's cone is held to degrees over the corpus by `what_a_light_states_and_in_what`
([R41e](#r41e-lights-shattermaps-and-bone-identity)). Between core and plugin: a constant or a structure is core; an interaction or a heuristic
is a plugin. **Covers.** U-105, U-117, U-135, U-143, U-190, U-196, U-211, U-212, U-213, U-214,
U-216.

### R41a Vertex and index data as an editable model

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

Done in #68; the codec lives in `or-io` since #300. `or_io::codec::mesh` holds the one vertex model
(`VertexLayout` and `VertexReader`, over any `VertexSource`; `encode` writes a channel in place, and
adding one changes the stride, which is a rebuild); `or_anim::skin` and `src/render/meshes.rs` adapt
it, and `every_nibble_names_an_encoding` holds the FVF nibble decode. `openrage-cli channels`
(`crates/tools/or-cli/src/vertex_census.rs`) counts which channels ship, beside a control.
⚠ Widening `or_renderer::MeshVertex` for a per-channel view costs 2.7× per vertex of every mesh for
channels almost no buffer carries; repack the buffer for the selected channel instead.

### R41b Bounding volumes and BVH

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

Done in #91, #96 and #97. `or_io::codec::mesh` states the box rules (`model_aabbs`, `aabb_slots`,
`one_geometry_ships_one_box_and_more_than_one_ships_a_prefix`), and `GrmModel`'s box run is declared
with `prefixed_above_one` ([the field types](OR_IO_Specification.md#field-types)). An import
recomputes the boxes of every model it rewrites and keeps each shipped box's fourth lanes, which
carry engine scratch no vertex reproduces. The drawable's own box is authored, so nothing rewrites it
unasked; `openrage-cli bounds` (`crates/tools/or-cli/src/bounds_census.rs`) compares a recompute
against what shipped, which is the only evidence a recompute is safe to offer. BVH construction is
R41c's.

### R41c Bounds and physics construction

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

Done in #105, and owned since R66d. `crates/io/or-io/src/gta5/bounds/build.rs` builds a geometry
bound or a BVH from a mesh and rebuilds a BVH's tree; a polygon keeps its primitive tag in the low
bits of `area`, and a save refuses bits that name no primitive; a fragment's joints
(`PhArticulatedBodyType`) and archetype fields are owned and writable; and a `.ybn` save refuses what
the game would misread, in `or_io::gta5::bounds::validate`. Suites:
`crates/io/or-io-verify/tests/bounds.rs` (`a_mesh_becomes_a_bound_that_survives_a_save`,
`a_rebuilt_tree_covers_the_polygons_it_was_built_over`) and
`crates/io/or-io-verify/tests/fragment.rs`. ⚠ A rebuild permutes the polygons, and
`poly_mat_index_list` and every triangle's `neighboring_polygons` index them: carrying one without
the other repaints the collision surface or cross-links the adjacency with every index still valid.

### R41d Cloth as a model

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

Done in #105, and owned since R66d: `CharacterClothController` in `or_io::gta5::cloth_dict` reaches
a cloth vertex's binding by bone tag through `bone_id_map` (`bone_tag_for_blend_index`,
`binding_for_tags`), its per-LOD weight and pin radius and its pinnable bit (`is_pinnable`), over all
four LODs where CodeWalker reads three. A blend index past the bone map is refused at save by
`cloth_dict::validate`. Suite: `crates/io/or-io-verify/tests/cloth_dict.rs`
(`a_vertex_can_be_weighted_by_tag`, `a_binding_naming_a_bone_the_map_does_not_hold_is_refused`).

### R41e Lights shattermaps and bone identity

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

Done in #107, with bones addressed by tag wherever the addressing crosses a file: a garment's blend
index resolves through its own `bone_id_map`, and a geometry's `matrix_palette` stays an index in the
file and is mapped onto another skeleton at the boundary by `or_anim::retarget`
(`crates/runtime/or-anim/tests/what_a_matrix_palette_addresses.rs`). A skeleton's bone-id table is
derived from its bones, and an open refuses one that is not that derivation (`CrSkeletonData`). A
shattermap is the owned `ShatterMap` in `or_io::gta5::fragment::vehicle_glass`, and a saved `.yft` of
either target keeps its vehicle glass (`a_saved_yft_keeps_its_names_and_its_glass` in
`crates/io/or-io-verify/tests/fragment.rs`). A drawable's lights are `GtaDrawable::lights`, owned
`CLightAttr` values whose `light_type` says which fields apply. `CrSkeletonData::add_bone` refuses
a bone id another bone holds, and `crates/io/or-io-verify/tests/what_a_light_states_and_in_what.rs`
holds every staged light's `light_type` to point, spot or capsule and a spot light's cone to
degrees, refusing the same cones fed in radians.

### R42 The viewer

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

Done in #66 and #80. `ShadingMode::Channel` shows one vertex channel unlit
([contract](../features/Viewport_Shading_Modes.md#channel-mode)), and
`resolve_skeleton_automatically` adopts the best-ranked skeleton among open documents: the app does
not call [R26](#r26-index-dependencies-and-summaries)'s resolver, whose dependency sets are
CLI-only, so a skeleton in no open document is not found;
`selecting_a_clip_keeps_the_skeleton_and_meshes` holds the session. The mode driving the real game
shader is [cut](../features/Viewport_Shading_Modes.md#cut-a-mode-running-the-games-shaders) and
re-opens under [R40f](#r40f-the-compiled-shader-registry-and-the-abi-slot-table) when
`or-shader-tool list`'s pixel row moves. **Covers.** U-096, U-097, U-127.

### R42b Viewport basics for inspecting assets

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**What.** A click in the viewport selects the geometry, model, bone or bound under the cursor, and
the outline follows; `F` frames the selection and `Home` everything; a level-of-detail switch shows
each level's distance; a collision toggle draws a `.ydr`'s embedded bound too; bones draw in front of
the mesh; Material is the default mode for a preview; the mode and the background persist; a
dictionary opens on its first entry; the overlays leave the axis gizmo clear. There is no transform
gizmo: geometry is authored in Blender ([R38](#r38-mesh-interchange)).

**Where it stands.** Done. A click in the viewport casts
a ray through the cursor with the matrices the renderer draws through (`Ray::at` in
`src/render/viewport/pick.rs`) against the geometries the viewport draws, posed and retargeted as it
poses them, and selects the nearest; the Outline opens the headers above its row and scrolls to it.
A click on nothing clears the selection down to the dictionary entry or particle model the viewport
draws, and a drag still orbits. `F` frames the selection and `Home` everything, `.` still frames the
selection, and both commands keep their ids. Bones draw in front of the mesh in both render paths.
A switch under the shading pills draws each level of detail the drawable has and names the distance
the game draws it to, and picking, the wireframe, the caches and the gallery's tiles follow it; a
Collision toggle beside it draws the bound a `.ydr` or a dictionary's entry embeds, as lines in
every mode. Material is the mode of preferences that never set one, and since
[R59p](#r59p-settings-recents-and-small-editor-fixes) the mode, the background and the exposure
persist across a restart. A drawable dictionary opens on its first entry. The axis gizmo sits below
the bars under the pills and the box at the bottom left wraps clear of it; the Shaded bar and the
channel selector fit a narrow viewport; and the box says what the viewport shows in words, with no
kind code or frame rate. A gallery tile's key holds the mode, the channel and the level of detail,
and a particle model frames from the sphere its meshes fill rather than the smaller one it ships.
`a_click_selects_the_geometry_under_the_cursor_in_the_outline_f_frames_it_and_a_restart_keeps_the_mode`
and the tests beside it in `src/state/viewport_basics_for_inspecting_assets.rs` hold each clause,
for both targets where one is staged, and each failed before its fix; the renderer's half of the
bones is `lines_drawn_in_front_show_through_a_mesh_nearer_the_camera`, which needs a GPU. [R42f](#r42f-the-viewport-picks-what-it-draws) widened the click to a bone,
a bound and, with Alt, a model, and in the Animation workspace to every mesh the preview draws, and
the box there names the pick; bones outside the Animation workspace are
[R42k](#r42k-what-drawing-bones-outside-the-animation-workspace-leaves)'s. Seen on `just shots`
screens, not in a window.

**Done when.** A click in the viewport selects the geometry under the cursor in the outline, `F`
frames it, and a restart keeps the mode.

### R42c The viewport draws what it is given

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**What.** A dictionary of either target draws and frames the entry selected in it, and each shading
mode draws only its own colours.

**Where it stands.** Done. `ydd_selected_entry_index` resolves an entry in a dictionary of either
target (`an_entry_selected_in_a_dictionary_of_either_target_resolves`), so the entry selected in an
Enhanced `.ydd` draws its meshes and its wireframe and frames as a Legacy one's does
(`a_dictionary_of_either_target_draws_and_frames_its_selected_entry`), and the status bar names it
(`the_summary_names_the_entry_selected_in_a_dictionary_of_either_target`). The viewport drops the
scene's `debug_channel` in every mode but Channel (`follow_shading_mode`), so Solid, Material and
Shaded draw none of its colours, and Channel mode starts again at the first channel the geometry
offers (`leaving_channel_mode_leaves_none_of_its_colours_behind`). Each test failed before its fix.
Material mode's textures for an Enhanced entry came with
[R42d](#r42d-every-viewer-path-takes-both-targets).

**Done when.** An Enhanced `.ydd` draws and frames its selected entry as a Legacy one does, and
leaving Channel mode leaves none of its colours behind.

### R42d Every viewer path takes both targets

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**What.** An Enhanced file draws, textures, previews and selects in the viewer as its Legacy
counterpart does: Material mode finds its textures, the Animation workspace offers its meshes and
skeletons, and a particle effect's model selection draws.

**Where it stands.** Done. Material mode reads the dictionary a drawable, a dictionary entry, a
fragment or a particle model embeds at either target, so a staged Enhanced `.ydr` finds its
textures in each of those as a Legacy one does
(`an_enhanced_drawable_finds_its_textures_in_material_mode_as_a_legacy_one_does`). The Animation
workspace offers an Enhanced drawable's, entry's and fragment's mesh and skeleton as their Legacy
counterparts'
(`the_animation_workspace_offers_an_enhanced_drawable_and_skeleton_as_a_legacy_one`), the gallery
lists an Enhanced `.yft`'s and `.ypt`'s models
(`the_gallery_lists_an_enhanced_files_models_as_a_legacy_ones`), and a model selected in an
Enhanced `.ypt` draws its mesh and wireframe and frames
(`a_model_selected_in_an_enhanced_particle_list_draws_and_frames_as_a_legacy_one`).
`every_viewer_path_that_matches_a_legacy_asset_handles_its_enhanced_twin` reads the viewer's code
and fails on a path matching a Legacy asset without its Enhanced twin; the paths it lists as not
the viewer's are the texture editor's and the particle simulation, the texture browser's and the
status bar's `.ypt` summary having moved out of that list into the scan with
[R42e](#r42e-the-panels-around-the-viewer-take-both-targets). Each test failed before its fix. An
Enhanced `.ypt`'s models are selected in the gallery, and in its Outline since R42e.

**Done when.** An Enhanced `.ydr` and an Enhanced `.ydd` entry find their textures in Material mode
as their Legacy counterparts do, the Animation workspace offers an Enhanced drawable and skeleton,
an Enhanced `.ypt`'s selected model draws, and a test lists every viewer path that matches a Legacy
asset and finds its Enhanced twin handled.

### R42e The panels around the viewer take both targets

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**What.** An Enhanced file fills the panels around the viewer as its Legacy counterpart does: the
texture browser lists an Enhanced `.yft`'s and `.ypt`'s textures, an Enhanced `.ypt` has its outline,
its status-bar summary, its inspector and its timeline, and the texture preview checks embedded
textures of either target.

**Where it stands.** Done. Every panel that reads a particle list reads it through
`ParticleTarget` (`src/state/particle_target.rs`), at either target. An Enhanced `.ypt` has its
Outline, whose adds and deletes are steps of its history
(`an_enhanced_particle_file_fills_the_outline_as_a_legacy_one_does`,
`deleting_an_effect_from_an_enhanced_particle_files_outline_undoes_as_a_legacy_ones_does`), a
status-bar summary that names its effects
(`the_status_bar_names_an_enhanced_particle_files_effects_as_a_legacy_ones`), an Inspector for every
element it selects, edits included
(`an_enhanced_particle_file_has_the_inspector_a_legacy_one_has`,
`an_edit_of_a_rule_undoes_and_redoes_at_either_target`), and a timeline of its events and keyframe
curves, scaled to the effect it selects else the longest it authors
(`a_list_nothing_simulates_is_scaled_to_the_effect_it_selects_else_the_longest`), whose drags
are steps
(`an_enhanced_particle_file_has_the_timeline_a_legacy_one_has`,
`a_dragged_keyframe_undoes_to_the_curve_the_file_opened_with`). The texture browser lists an Enhanced
`.ypt`'s own textures and each model's, and an Enhanced `.yft`'s, each resolving to the texture the
preview reads and the Inspector shows
(`the_texture_browser_lists_an_enhanced_particle_files_textures_as_a_legacy_ones`,
`the_texture_browser_lists_an_enhanced_fragments_textures_as_a_legacy_ones`,
`an_enhanced_fragments_selected_texture_is_inspected_as_an_enhanced_texture`), and
`a_staged_enhanced_particle_file_opens_with_its_outline_summary_and_textures` opens a staged one.
The viewer scan reads these panels too, and what it excepts is the particle simulation and the
texture editor, which has no Enhanced twin because the encoder writes the Legacy layout. Each test
failed before its fix. Not done: the simulation of an Enhanced effect, the known gap
[Particle_Preview.md](../features/Particle_Preview.md) names, so an Enhanced effect's clock plays nothing and its
Live Evolutions sliders are absent. Seen on `just shots` screens, not in a window.

**Done when.** The viewer scan's exceptions hold only paths no Enhanced twin exists for and the
Enhanced particle simulation, and an Enhanced `.ypt` opens with its outline, summary and textures as
a Legacy one does.

### R42f The viewport picks what it draws

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**What.** A click picks a bone, a bound or a model as well as a geometry, and in the Animation
workspace it reaches every mesh the preview draws; the box there names what the preview draws.

**Where it stands.** Done. Found while closing
[R42b](#r42b-viewport-basics-for-inspecting-assets), whose picking `select_under` in
`src/render/viewport/pick.rs` now widens. A bone the Animation preview draws is picked first, since
bones draw in front of the mesh: the bone whose drawn lines (`bone_lines` in
`src/render/skeleton_lines.rs`) pass within 5 pixels of the cursor, as a row of the document its
skeleton comes from. A bound the Collision toggle draws is picked by the lines it draws
(`bound_node_lines` in `src/render/lines.rs`), as the node whose number its Outline row has, and only
where no mesh nearer the camera hides it; Wireframe draws no mesh, so there it always counts. Else the
nearest geometry is picked as before, and Alt+click picks the model that holds it
(`select_model_under`). The Outline opens the Skeleton and Bound sections, a composite bound inside
another, and the entry that holds them, and scrolls to the row. In the Animation workspace the ray
reaches every mesh the preview draws (`drawn_sources`), posed and retargeted as the viewport poses
them, in the document each comes from. A pick in a document other than the active one, which is every
pick there since the active document is a clip dictionary, is held in `PreviewScene::picked` and not
selected: the selection holds one focus, so selecting it would take the clip off the Outline, the
Inspector and the status bar. A click on nothing clears the pick and leaves the clip dictionary's
selected clip alone, and the box at the bottom left names the drawables the preview draws, the
skeleton whose bones it shows and the pick ("Picked: BONE_1 (1) · rig.ydr"), or says nothing is
drawn, in place of the clip dictionary.
`a_click_on_a_bone_selects_its_row_in_the_outline` (both targets, held as a drawable, a dictionary's
entry or a fragment), `a_click_on_a_bound_selects_its_row_in_the_outline` (both targets, a drawable
and a dictionary's entry), `a_click_reaches_a_mesh_source_drawn_beside_a_clip_dictionary`,
`a_pick_in_the_animation_workspace_leaves_the_clip_selected_and_the_box_names_it` and the tests
beside them in `src/state/the_viewport_picks_what_it_draws.rs` hold each clause, and each was seen
failing with its part taken out. Not done, and taken up by
[R42h](#r42h-what-a-pick-still-misses-after-r42f): ⚠ in the Animation workspace the Outline, the
Inspector and the status bar follow the active document, so a pick there shows in the box alone
until a double click on the box's Picked line makes its document the active one and opens it at the
row. Bones draw there too, where the active document is an animation document, so a rig's own
Outline, which lists its bones, is on screen only after that, and the test that finds a bone's row
in the Outline makes the rig active by hand;
[R27t](#r27t-a-skeleton-is-seen-without-an-animation) draws them in the Model workspace as well,
where the Outline is the rig's and a click on a line selects its row. `select_model_under` was held
by a test but not by one that sends Alt through the viewport panel, and no screen showed it, since the click step of
`just shots` carried no modifier; a fragment's physics bound, a `.ybn`'s bounds and the bind-pose
ghost were not picked. The preview also draws the skeleton source's own
drawable when the chosen meshes draw nothing, which a scene cannot tell. Seen on `just shots`
screens, not in a window: a bound clicked on a drawable opens Bound and its root in the Outline and
the Inspector names it; a bone or a mesh clicked in the Animation preview is named in the box and
the clip stays on the Inspector.

**Done when.** A test clicks a bone and a bound and finds each selected in the Outline, and a test
clicks a mesh source's mesh in the Animation preview while a clip dictionary is active and finds
it picked.

### R42g Enhanced controls that cannot work say so

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**What.** A control an Enhanced file cannot use yet says so before it is used: an Enhanced `.ypt`'s
transport and Drivers rail say nothing plays, and the Texture workspace's Replace from image…, Embed and Unembed
are greyed with the reason for an Enhanced `.ypt` or `.yft` texture.

**Where it stands.** Done. Found while closing
[R42e](#r42e-the-panels-around-the-viewer-take-both-targets): the timeline drew its transport (Play,
Rate, Loop) and the Drivers rail for an Enhanced `.ypt` though nothing simulates it, so Play did
nothing and a modder was not told why. `ParticleTarget::SIMULATED` now says which target `tick_sim`
builds a simulation from, Legacy only, and the timeline of an Enhanced `.ypt` greys Play, Rate, Loop
and the three Drivers, each saying "Nothing plays: Enhanced effects are not simulated yet." on
hover, writes that line under the transport, stops a clock that was playing and ignores Space
(`an_enhanced_particle_files_timeline_greys_play_with_the_reason_the_effect_is_not_simulated`,
`an_enhanced_particle_files_transport_and_drivers_rail_are_greyed_and_say_nothing_plays`,
`an_enhanced_particle_files_timeline_stops_a_clock_that_was_playing`,
`space_over_an_enhanced_particle_files_timeline_starts_nothing`). The Texture workspace's Replace from image…,
Embed and Unembed were already greyed over an Enhanced `.ypt` or `.yft` texture, since the editor
reaches a Legacy dictionary only, but with reasons that were wrong for it: an Enhanced fragment was
told only a drawable or fragment can carry textures. They, and Apply, now say that Enhanced textures
cannot be edited yet because the texture editor writes only the Legacy layout
(`the_texture_preview_greys_apply_import_embed_and_unembed_over_an_enhanced_texture_and_says_why`, with
`the_texture_preview_offers_import_embed_and_unembed_over_a_legacy_fragments_texture` as its
control). Each test but that control failed before its fix. The tests read a hover text as what a
hover painted beyond a frame with the pointer away, so the line under the transport is not taken for
it (`the_probe_tells_a_hover_text_from_a_line_painted_all_the_time`), and the two timeline tests
fail with the six hover texts emptied. Not done: the simulation of an Enhanced effect, the known gap
[Particle_Preview.md](../features/Particle_Preview.md) names and the [deferred list](#deferred-and-known-gaps)
holds, with `tick_sim` in `src/state/mod.rs` and `refresh_particles` in `src/render/viewport/mod.rs`
still matching the Legacy particle list alone, so the Live Evolutions sliders stay absent and the
timeline's summary still reads "(no effect) · 0 live"; and re-encoding, importing or embedding a
texture at the Enhanced target, which the texture editor does not write. The format, quality, mip
and sRGB controls stay live over an Enhanced texture though nothing they set can be applied. Seen on
`just shots` screens, not in a window.

**Done when.** A test draws an Enhanced `.ypt`'s timeline and finds Play disabled with a reason
saying the effect is not simulated; and a test draws the Texture workspace's toolbar over an
Enhanced `.ypt` texture and finds Replace from image…, Embed and Unembed disabled with the reason.

### R42h What a pick still misses after R42f

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**What.** A pick in the Animation workspace leads to its row, the viewport picks the lines R42f missed,
and the Alt click that picks a model can be tested and scripted.

**Where it stands.** Done. Found while closing [R42f](#r42f-the-viewport-picks-what-it-draws). In the
Animation workspace the Outline, the Inspector and the status bar follow the active document, a clip
dictionary, so a click on a mesh or a bone is held in `PreviewScene::picked` with the row it names,
shows in the viewport box alone and leaves you on the clip list: the Outline does not follow every
pick (owner, 2026-10-05). A double click on the box's Picked line opens the picked document at the
picked row, for a geometry, a model (Alt) or a bone, and the line says so on hover and is named for
a screen reader: `open_picked` in `src/render/viewport/pick.rs` makes the document the active one,
which switches to the workspace that shows its kind, selects the row, sends the Outline to it and
brings the Outline forward
(`a_double_click_on_the_picked_mesh_opens_its_document_at_the_geometry` and
`a_double_click_on_the_picked_bone_opens_its_rig_at_the_bone`, in
`src/state/a_pick_in_the_preview_opens_its_document_at_its_row.rs`, which double click where the box
paints the line, and hold that one click leaves the clip list and that a double click on the line
above it opens nothing). The viewport picks the lines it still missed, in
`src/state/the_viewport_picks_every_line_it_draws.rs`: the bind-pose ghost, as the bone whose segment
it is, named "in the bind pose", the one nearest the cursor in pixels and giving way to the posed
bone only where the two are equally near, as at the bind pose
(`a_click_on_the_bind_pose_ghost_picks_the_bone_it_draws` and
`a_click_where_a_bone_and_its_ghost_are_equally_near_picks_the_posed_bone`); in Wireframe, where a
document's own lines draw, a fragment's high-level physics bound, as the one row the Outline gives
it, and a `.ybn`'s bounds, as the node whose number its Outline row has, which the Outline now
scrolls to and opens its composites for, and whose numbers a closed composite no longer shifts for
the rows drawn after it, in a `.ybn`'s Outline and in a drawable's Bound
(`a_click_on_a_fragments_physics_bounds_picks_their_row`,
`a_click_on_a_ybns_bounds_picks_the_node_it_draws`,
`a_picked_node_inside_a_composite_ybn_bound_is_opened_in_the_outline`,
`a_picked_ybn_node_after_a_closed_composite_is_the_row_the_outline_selects` and
`a_picked_drawable_bound_node_after_a_closed_composite_is_the_row_the_outline_selects`); and a bone
while every chosen mesh source draws nothing, which `pick` used to return from
(`a_bone_is_picked_while_every_mesh_the_preview_was_given_draws_nothing`). The Alt click is
`select_on_click` in `src/ui/panels/viewport.rs`, which `draw` calls with the panel's response, so
`an_alt_click_in_the_viewport_panel_selects_the_model_and_a_click_the_geometry` holds it with a
frame's own modifiers; the gesture stays Alt and a click (owner, 2026-10-05). A script's click takes
the modifiers a key does, `click X Y alt` (`a_click_takes_the_modifiers_a_key_does`). The tests that
pick lines and the script's modifier failed on the tree before the fix, and the others each failed
with the part they hold taken out: the open call, the Alt branch, the model's row, the Outline send,
the composite open, the skipped numbers, the tie rule, the line's widget moved or as big as the box,
and the Wireframe and ghost conditions. Not done: a `.ybn` drawn as meshes in the other shading
modes carries no line and is not picked there, since its meshes are not keyed by node,
and a click on nothing in a `.ybn` leaves its selection; the cloth edges of a `.yld`, a navmesh's
outlines, a particle effect's gizmo lines and a plugin's overlay lines are drawn and not picked;
`near_lines` picks the line nearest the cursor in pixels, so looking exactly along a bound's diagonal
a far edge can stand for the near one in the test against a nearer mesh, and two visible edges at one
distance go to the first, which is rare and not worth a rule (owner, 2026-10-05); and the preview
still draws the skeleton source's own drawable when the chosen meshes draw nothing, which a scene
cannot tell. Seen on `just shots` screens, not in a window: an Alt click in the Model workspace
selecting `Model 0` where a click selects `Geometry 0`; a bone clicked in the Animation preview named
in the box, the tooltip on its Picked line, and a double click on it opening the fragment in the
Model workspace with the bone's row selected in the Outline; a click on the ghost naming
`SKEL_L_Hand (42) in the bind pose`; the edge of a `.ybn` in Wireframe selecting its child; and the
physics bound of a bike's fragment selecting the Outline's Bound row.

**Done when.** A test picks a mesh and a bone in the Animation preview and follows the way from the
pick to its row that the owner settles on, and finds the picked document open at the row; a test
clicks a fragment's physics-group bounds, a `.ybn`'s bounds and a bind-pose ghost line and finds
each picked; a test holds Alt over a click in the viewport panel and finds the model selected, and a
script's click takes a modifier so a screen shows it; and a test shows a skeleton while every chosen
mesh source draws nothing and finds a bone picked.

### R42i Enhanced controls that still do nothing after R42g

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**What.** Every control an Enhanced file cannot use says so, the reasons they give are the true
ones, and the tests that hold them cannot pass on a literal.

**Where it stands.** Found while closing [R42g](#r42g-enhanced-controls-that-cannot-work-say-so)
(#560): the pin rail is live over an Enhanced `.ypt`, since a Ctrl or Cmd click on an effect in the
Enhanced Outline reaches `toggle_pin` and `draw_pin_rail` in `src/ui/panels/timeline.rs` then draws
an unpin button and a live "Start offset for this pinned effect" DragValue under a greyed transport
that says nothing plays; the viewport toolbar's Exposure (`src/ui/panels/viewport.rs`) shares
`drivers.exposure` with the greyed Drivers rail and stays live; the format, quality, mip and sRGB
controls stay live over an Enhanced texture though nothing they set can be applied; and the
timeline's summary still reads "(no effect) · 0 live" and the Live Evolutions sliders are absent,
because `tick_sim` and `refresh_particles` match the Legacy particle list alone. The reason Replace from image…,
Embed and Unembed give over an Enhanced texture (`ENHANCED_TEXTURES` in
`src/ui/panels/texture_preview/mod.rs`) is not the whole truth over a `.ypt`: `dictionary_mut` has no
`Asset::Particles` arm, so a Legacy `.ypt`'s textures are not editable either and a particle file
carries none, which the Enhanced reason now drops, and the one Legacy control test uses a `.yft`, so
nothing holds what a Legacy `.ypt` says; no test probes Apply, which says `ENHANCED_TEXTURES` too.
`ParticleTarget::SIMULATED` claims to say what `tick_sim` builds a simulation from, though
`tick_sim` matches `Asset::Particles` on its own and no test ticks an Enhanced document and finds
nothing simulated, so if the simulation of Gen9 lands and the constant stays false the controls stay
greyed with a false reason.

**Done when.** A test pins an Enhanced `.ypt`'s effect and finds the rail's unpin and Start offset
greyed with the reason, and the toolbar's Exposure greyed or apart from the Drivers; a test draws
the Texture workspace over a Legacy `.ypt` and finds the reason it gives; a test probes Apply over
an Enhanced texture; and a test ticks an Enhanced document and finds nothing simulated, so the
constant and `tick_sim` cannot part.

**Closed.** `the_constant_that_says_a_target_is_simulated_is_what_tick_sim_does_with_its_list`
ties `ParticleTarget::SIMULATED` to what `tick_sim` builds, and
`the_texture_preview_over_a_legacy_particle_files_texture_gives_the_reason_that_is_true_of_it`
holds that a Legacy `.ypt`'s four texture controls are greyed and no longer blame an edition (the
sentence "Enhanced textures use a different layout" went from `why_not_encodable`).
`an_enhanced_particle_files_pin_rail_is_greyed_and_says_nothing_plays` holds the rail's unpin and
Start offset greyed with the reason, `the_shaded_bars_exposure_is_greyed_over_an_enhanced_particle_file_and_says_nothing_plays`
holds the Shaded bar's Exposure greyed apart from a document with no effect, and Apply joins the
texture controls the Enhanced and Legacy `.ypt` tests probe. The format, quality, mips and sRGB
inputs go grey with Apply wherever Apply cannot write, and the same tests probe them.

### R42j What a pick still misses after R42h

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**What.** A pick reaches its row from the keyboard as from the pointer, the Outline shows the row it
picked, and every line the viewport draws is picked or says it is not.

**Where it stands.** Found while closing [R42h](#r42h-what-a-pick-still-misses-after-r42f) (#569):
the Picked line is registered with `Sense::click()`, so it is a Tab stop named "Double-click to open
it…" and only `double_clicked()` acts, which leaves Enter, Space and a screen reader's default
action doing nothing and a keyboard user no way to open a pick; the line stays in
`PreviewScene::picked` after `open_picked`, so a return to the Animation workspace still shows it; a
pick of a fragment's physics bound selects the Bound row and sets `scroll_to` but never forces the
Physics section open (`yft_physics` in `src/ui/panels/hierarchy/yft.rs`), so a collapsed section
hides the selected row and `scroll_to` lingers until `REVEAL_PATIENCE` runs out; the Outline lists a
fragment's Medium and Low physics bound rows while the viewport draws only the High level's bound,
so they have no line to pick; selecting a bone row or the physics Bound row of a `.yft` left the
Inspector on the Fragment summary, which the bone row no longer does since
[R27p](#r27p-a-bones-numbers-can-be-edited), and the Bound row was not looked at again; a `.ybn`
drawn as meshes in the shaded modes carries no line, since `build_bound_meshes` in
`src/render/bound_meshes.rs` returns its meshes unkeyed, and a `.yld`'s cloth edges and collision
composite, a `.ynv`'s outlines, a particle effect's gizmo lines and a plugin's overlay lines are
drawn and not picked, though the row's What and its test module say every line is, and its Done-when
names three kinds. `the_outline_selects` in `src/state/viewport_basics_for_inspecting_assets.rs`
compares a painted colour with the process-wide palette without holding
`crate::ui::theme::serial()`, so a theme test that flips the palette between the draw and the read
fails it at random; no test fails with the `workspace_docks.reveal(workspace, OUTLINE)` line of
`open_picked` taken out, so "brings the Outline forward" is held by nothing; and the script step
`click X Y alt` is held by its parse alone, so nothing committed shows the modifier reaching the
viewport when a script runs it.

**Done when.** A test focuses the Picked line, presses Enter and finds the pick opened; a test picks
a fragment's physics bound with its section collapsed and finds the section opened; a test finds no
Outline row of a physics level the viewport does not draw; a test clicks a `.ybn` drawn as meshes in
a shaded mode, and each kind of line above, and finds each picked or said not to be; a test takes
the Outline send out of `open_picked` and finds a test fail; and a script clicks with Alt and a
screen shows the model selected. Six hold in the tree, each seen failing first: Enter or Space on the focused Picked line opens the pick as a double click does, and its hint says so (`enter_on_the_focused_picked_line_opens_the_pick_as_a_double_click_does`; a screen reader's default action is still not one of them, since it arrives as a single click that the line must not act on); and a fragment's Outline lists a Bound row for the High physics level alone (`the_outline_lists_a_bound_row_for_the_physics_level_the_viewport_draws_and_no_other`, the Medium and Low summary lines staying). A click on a fragment's physics bound opens the Physics section that was closed over its row (`a_click_on_a_physics_bound_opens_the_section_that_holds_its_row`), and a double click on the Picked line queues the Outline's reveal, which `assert_opened_at` in `src/state/a_pick_in_the_preview_opens_its_document_at_its_row.rs` fails without; `the_outline_selects` now holds the theme serial, which only a run in threads (`just test-lite`) could have shown. A click on a `.ybn` drawn as meshes picks the node whose mesh it meets, in the row its Outline gives it (`a_click_on_a_ybns_bounds_picks_the_node_it_draws`, seen failing first), and a click past it leaves the selection where it was, as in Wireframe. A click on a `.ynv`'s polygon selects its row in the Outline, which opens its section and draws it highlighted, and a click on the row selects it as well (`a_click_on_a_navmesh_polygon_selects_its_row_which_the_outline_opens_and_the_viewport_highlights`, each of the four seen failing with its part taken out). The remaining lines say they are not picked, and why: a cloth's edges and its collision composite draw for the cloth that is already the selected row and have no Outline row of their own to select; a particle effect's gizmo lines are drawn for the live particles of the running simulation, which move and have no row; a plugin's overlay lines are the plugin's, which a click in the viewport reaches as an event (`a_click_in_the_viewport_reaches_a_guest_that_asked_for_clicks_and_not_one_that_did_not`). The script clause was run: `open` of `des_bridge_root.ydr`, `workspace Model`, `click 660 470` and `click 700 600 alt`, through `just shots`, and the screens show Geometry 4 selected by the plain click and Model 0 by the Alt click (2026-10-20); no test holds the step since it needs a GPU adapter.

### R42k What drawing bones outside the Animation workspace leaves

[↑ checklist](#phase-6--assets-textures-materials-geometry-the-viewer)

**What.** A model's bones draw in the Model workspace as the Animation preview's do, the bone a click
picked draws as the picked one in either workspace, each choice made for them is held by a test or a
screen, and the viewport's row of controls keeps the Bones box inside a narrow viewport.

**Where it stands.** Found while trying to land
[R27t](#r27t-a-skeleton-is-seen-without-an-animation), which has landed since. R27t draws the bones
of a drawable, a fragment, a dictionary's selected entry and a `.ypt` model in the other workspaces,
behind a Bones box in the viewport's row and a flag of its own, draws the bone selected in the
Outline highlighted, and has a click on a line select its row. That closes the first clause of
**What**, the bone a click picked in the Model workspace drawn as the picked one (it is the
selection there), and the test of each workspace's start,
`show_bones_is_on_for_a_fresh_preference_file`; the rest it left. In the Animation workspace a click
on a bone is held in `PreviewScene::picked` and not selected
([R42f](#r42f-the-viewport-picks-what-it-draws)), so a highlight that follows the selection does not
follow it and shows only a bone selected in the skeleton's own document, for example after a double
click on the corner box's Picked line opens it; drawing `picked` too would be a few lines in
`Shown::selected` (`src/render/viewport/bones.rs`) and a test. The two flags are kept apart on
purpose: `show_bones` for the Animation workspace, which starts ticked, and a second for every other
workspace, off by default as Collision is, because the Model workspace's first reading of "turns
Bones on" implies off by default and bones draw in front of the model with no depth test; sharing
one flag, ticked by default, is a one-line change if the owner prefers it. The box is shown in the
Animation workspace too, bound to Show bones, but only once a skeleton is chosen, and with none
chosen the row has nothing to offer. A
dictionary's Bones draws only the entry the viewport draws, the selected one, and not each entry's
own skeleton, because a ped's component dictionary holds the ped skeleton in every entry and drawing
all of them would stack the same bones. A `.ypt` model's skeleton is drawn through the same path
(`drawn` in `src/render/viewport/pick.rs` already yields the selected model) and its Outline rows use
the same `Named::Entry` row ids, but no test or screen holds it. The skeleton is built again with
`Skeleton::build` every frame while Bones is ticked and once per click, which cost 0.77 ms for the 354 bones of the corpus ped player_zero.yft (500 builds timed in a test build at 1b56c77e, run from the agent/skeleton-cost lane); a
cache keyed by the document and its `payload_epoch` (`built` in `src/render/viewport/bones.rs`, held by `a_skeleton_at_rest_is_built_once_for_as_long_as_its_document_holds_it`) now builds it once while the document holds it. No staged Enhanced `.ydr` holds a
skeleton with a rest pose and a bone above another, so the Enhanced drawable route is held by a
synthetic rig alone, where the staged Enhanced `.yft` is covered. Found while closing
[R27t](#r27t-a-skeleton-is-seen-without-an-animation) (#583): no test clicks the Bones box in the
Animation workspace, where `draw` in `src/ui/panels/viewport_drawable_row.rs` binds it to
`show_bones` and not to `show_rest_bones`, since
`the_viewport_offers_a_bones_toggle_beside_a_skeleton_and_not_beside_none` clicks it outside that
workspace and the Animation test sets `show_bones` by hand; the row takes 336 points with all three
controls (170, 84 and 70, and two gaps of 6), `draw` clamps the bar to the viewport's width less 16
points and narrows only the level-of-detail box, so by a read of it, and not on a screen, in a
viewport narrower than 352 points the Collision and Bones boxes lie past the bar's background and
the viewport's edge, and `the_overlays_leave_the_axis_gizmo_clear_and_fit_a_narrow_viewport` in
`src/state/viewport_basics_for_inspecting_assets.rs` never draws the row; and `rigs` in
`src/state/a_skeleton_is_seen_without_an_animation.rs` records a corpus skip for each target and
extension with no staged file holding a rest pose, which `OPENRAGE_REQUIRE_FIXTURES` turns into a
failure, and no staged Enhanced `.ydr` holds one, so by a read of `skip_corpus`, and not run, the two
tests that call `rigs`,
`the_bones_toggle_draws_one_line_for_each_bone_with_a_parent_from_its_parents_rest_position` and
`the_bone_selected_in_the_outline_is_drawn_in_the_highlight_colour`, fail under `just test-strict`
on that corpus.

**Done when.** A test clicks a bone in the Animation preview and finds it drawn as the picked one,
or the owner says it stays as it is; the owner says whether Bones is one flag or two, and a test
finds each workspace's start; a test opens a `.ypt` holding a skeleton, ticks Bones and finds its
bones drawn and a click on one selecting its Outline row; a test or a timing of a frame with Bones
ticked finds the skeleton built once while the document's `payload_epoch` stands; and a test draws
the bones of an Enhanced `.ydr` built with a rest pose and a bone above another, since none is
staged; a test clicks the Bones box in the Animation workspace and finds `show_bones` follow it and
`show_rest_bones` stand; a test draws the viewport's row with all three controls at the 220-point
width of the narrow-viewport test and finds each inside the viewport; and the two tests that call
`rigs` pass under `just test-strict` on a corpus that stages no Enhanced `.ydr` with a rest pose,
whether by asking for the target only where one can be staged or by staging one. Four hold in the tree, three of them seen failing first: a bone picked in the Animation preview is drawn in the highlight colour when no Outline row of a bone is selected (`a_bone_picked_in_the_animation_preview_is_drawn_in_the_highlight_colour`); the viewport's row gives the switch the width the toggles leave, so each control lies inside a viewport 220 points wide (`every_control_of_the_row_lies_inside_a_viewport_as_narrow_as_220_points`); a click on the Bones box moves Show bones in the Animation workspace and the rest bones flag elsewhere (`the_bones_box_is_show_bones_in_the_animation_workspace_and_the_rest_bones_box_elsewhere`, held by swapping the binding and seeing it fail); and the skeleton is built once. A `.ypt` model's bones are drawn and a click on one selects its Outline row (`a_particle_models_bones_are_drawn_and_a_click_on_one_selects_its_outline_row`, at both targets, seen failing with the draw of a selected model taken out); the Enhanced `.ydr` rig built with a rest pose and a bone above another is `viewed::<Gen9>` held as a drawable, which `rigs` runs; and a target whose staged files hold no rest pose no longer records a corpus skip, so the two `rigs` tests pass under `OPENRAGE_REQUIRE_FIXTURES=1` (seen failing first). Bones stays two flags, `show_bones` for the Animation workspace, which starts ticked, and `show_rest_bones` for every other, off by default as Collision is: the owner delegated the call on 2026-10-20 and the state of the tree stands, with `show_bones_is_on_for_a_fresh_preference_file` and `the_bones_box_is_show_bones_in_the_animation_workspace_and_the_rest_bones_box_elsewhere` holding it; sharing one flag is the one-line change the row names, should the owner want it.

### R43 The external surface and open with

[↑ checklist](#phase-7--plugins-and-open-with)

**What.** The subprocess half of the plugin surface: the `openrage-cli serve` protocol as a versioned
contract, with writing and converting routes; a job model shared with
[R21](#r21-extraction-and-bulk-jobs); per-title command and settings registries; an MCP adapter that
discovers verbs at run time; a headless CLI and single-purpose entry points; and Open with — extract,
launch an external tool, notice the change on exit, offer to write it back into the archive.

**Where it stands.** The read routes exist, held equal to
[CLI_Server_Protocol.md](../reference/CLI_Server_Protocol.md) in both directions by
`crates/tools/or-cli/tests/the_external_surface_is_what_it_documents.rs`, and
`the_protocol_is_read_only_but_for_the_four_routes_that_change_a_pack_and_says_so` turns red with the first
write route beyond `wardrobe-add`, `wardrobe-set`, `wardrobe-fix` and `wardrobe-waive`, which change the
clothing pack of a project the caller names and never the install. The MCP adapter exists:
`openrage-cli mcp` offers those routes, every command of `openrage-cli` and the editor's frames drawn
from a script as tools ([MCP_Server.md](../reference/MCP_Server.md)), and names the commands from
`COMMANDS` at run time. File associations are
[R25b](#r25b-explorer-ergonomics)'s.

**Depends on.** [R19](#r19-one-virtual-filesystem),
[R29](#r29-kernel-manners-diagnostics-probing-and-provenance), [R20](#r20-archive-lifecycle), R25b.
**Done when.** An external process can open, query, convert and save over the versioned protocol,
long jobs report progress and can be cancelled, and an external edit lands back in the archive after
a confirmation. **Risk.** ⚠ Write-back loses data if the tool crashes mid-write: compare size and
modification time, keep the extracted copy until the write succeeds, never write back silently.
**Covers.** U-170, U-171, and the six rows [Appendix A](#appendix-a--union-coverage-index) indexes
against R44.

### R44 The plugin host

[↑ checklist](#phase-7--plugins-and-open-with)

**What.** A WebAssembly host inside the editor, in six parts, [R44a](#r44a-the-capability-model) to
[R44f](#r44f-payload-and-gpu-access), all done.
WebAssembly buys isolation and no capability control, so the capabilities came before the runtime;
a guest cannot take a Rust type across the boundary, so the host speaks `or_io::desc`'s description
model, which every declared format builds from its declarations (`Format::description`), and hands a
guest that links `or-io` itself the file's bytes.
`plugins/README.md` is where an author starts, and [Plugin_ABI.md](../reference/Plugin_ABI.md) is the contract.
The app embeds the host (`src/state/plugins.rs`) and `openrage-cli` does not; `or-plugin` stays
for R44e to R47 by the owner's ruling of 2026-09-30
([OD-AUDIT-KEEPS](Project_Context.md#the-plugin-host-the-registries-and-the-second-title-seams-stay-2026-09-30)).
**Covers.** U-165, U-166, U-168, U-175, U-176, U-178, answered by
[R43](#r43-the-external-surface-and-open-with) ([Appendix A](#appendix-a--union-coverage-index)).

### R44a The capability model

[↑ checklist](#phase-7--plugins-and-open-with)

Done in #71. The capabilities in `crates/domain/or-plugin/src/lib.rs` are an enumerated set with a
default grant of one, reading the opened document (`the_default_grant_can_do_almost_nothing`); every
writing capability also passes the embedder's `WriteGate`
(`every_writing_capability_passes_the_gate_and_no_reading_one_does`), and "not granted" and
"read-only" stay distinct refusals.

### R44b The WASM runtime

[↑ checklist](#phase-7--plugins-and-open-with)

Done in #72. `crates/domain/or-plugin/src/runtime.rs` runs guests on `wasmi` and registers host
functions from the grants (`an_ungranted_capability_has_no_function_to_call`); traps and spins leave
the host usable (`crates/domain/or-plugin/tests/what_a_bad_guest_cannot_do.rs`). ⚠ Another check in
front of the grant filter can do its work and keep every test green without it, so a change to the
host function set is verified by removing the filter and watching a test fail.

### R44c The guest ABI

[↑ checklist](#phase-7--plugins-and-open-with)

Done in #114. [Plugin_ABI.md](../reference/Plugin_ABI.md) and `crates/domain/or-plugin/src/abi.rs`: the guest owns
its memory, a status is never an answer (`a_host_function_returns_a_status_and_never_an_answer`), and
the version meant no compatibility then, and since 2026-10-20 an older plugin still loads
(`the_document_a_plugin_author_reads_states_the_versions_the_host_implements`).

### R44d The host API in the pack vocabulary

[↑ checklist](#phase-7--plugins-and-open-with)

Done in #117. `crates/domain/or-plugin/src/host.rs` reads a document through an `or_io::desc::Pack`,
for a GTA V format the description `or-io` builds from its declarations, one hop at a time
(`the_host_reads_one_field_and_never_a_document`), and answers a typed `or_io::desc::Value`, never
`or-editor`'s formatted text. It writes through `or_io::write`'s patch, never its build
(`the_host_patches_and_never_builds`). A write passes the grant, the edit mode, the description's
structural refusal and the format's own save check, which the embedder supplies as a `SaveCheck`
(`nothing_in_the_tree_accepts_any_document`), and a refusal restores the bytes
(`a_refused_patch_leaves_the_document_byte_identical`). Expect
[R46](#r46-first-plugin-a-clothing-tool-on-openrage) to ask for more.

### R44e Consent and refusal in the app

[↑ checklist](#phase-7--plugins-and-open-with)

**What.** Where a plugin's capabilities, declared in its manifest and never requested at run time,
are shown before it runs; how a grant is recorded and revoked; and what a user sees when a plugin is
refused. A refusal carries a stable key and the sentence a prompt would show, and unshown it makes a
plugin that appears to work and does nothing. Embedding the host in the app is part of this row.

**Where it stands.** Done. `src/state/plugins.rs` embeds the host: a plugin is a directory of
`plugin.json` and `plugin.wasm` under the configuration directory's `plugins`, run from Tools → Run
plugin on the open document's described root. Before its first run a prompt lists what the
manifest declares; the answer is kept in the preferences by plugin id and revoked in Settings →
Plugins, and a manifest that wants more is asked again. A patch passes the edit mode for the
document's path and the format's read-back and canonical save, and each run's report carries the
refusal's key, capability and sentence, also when the guest stopped after the refusal
(`HostError::StoppedAfterRefusal`). `src/state/a_plugin_asks_before_it_runs.rs` holds it with the
sample guest, and `just self-check` draws the prompt and the page;
[plugins/README.md](../../plugins/README.md#installing-a-plugin) says how to install one. Clicking
Allow, Decline, Run plugin and Revoke in a live window is unchecked: the self-check draws, it does
not click.

**Depends on.** [R44b](#r44b-the-wasm-runtime), [R25](#r25-the-edit-mode-gate).

**Done when.** The app embeds the host, and a user sees what a plugin asked for before it runs, can
change their mind, and can find out why a plugin did nothing.

### R44f Payload and GPU access

[↑ checklist](#phase-7--plugins-and-open-with)

**What.** An answer to raw payloads and GPU resources, measured before built: a guest's own memory
makes a texture a copy of megabytes per call, and the cloth tool may need only dimensions and
format, so this can close as a refusal. The owner's direction of 2026-10-05 settled the shape: the
clothing tool ([R46](#r46-first-plugin-a-clothing-tool-on-openrage)) is a WASM plugin that links
`or-io` itself, so the host hands a guest a file's bytes and accepts bytes back.

**Where it stands.** Done. `cap.read_file_bytes` buys `file_size` and `read_file`, and
`cap.write_file_bytes` buys `write_file` ([Plugin_ABI.md](../reference/Plugin_ABI.md#the-files-bytes)). The file
is the open document as a stored RSC7 file, built once per run and only for a plugin holding either
capability, and the guest names no path. A write passes the grant, the write gate, which in the app
is the edit mode for the document's path, and the format's own check, which reads the bytes back at
the document's own target and saves them in canonical form; the bytes then are the open document
until the user saves, which asks the gate again. A refusal reaches the guest as a status and the
sentence a user is shown, in a slot the guest supplies, and reaches the user in the run's report.
The sample guest built with `--features bytes` parses a `.ydd` with `or-io` inside WebAssembly and
writes it back
(`the_sample_guest_reads_a_ydds_bytes_and_parses_it_with_or_io_inside_the_guest`), and in the app
a refused write in read-only mode and an accepted one in the overlay are held by
`a_plugin_replacing_a_file_is_refused_in_read_only_mode_and_accepted_in_overlay_mode`.
`a_payload_capability_does_not_exist_yet` says which landed: a texture or a buffer apart from its
file, and a GPU resource, have no capability.

**Measured** on 2026-10-05 at `0b69053c` with this change, by `what_a_files_bytes_cost_a_guest` and
`what_a_guest_parsing_a_ydd_spends_against_the_default_fuel`, the best of five runs on the
workspace's test profile, over the sibling corpus's Legacy `.ydd` and `.ytd`:

- **Copy.** Into memory the guest has grown, 1 MiB takes 0.03 ms, 8 MiB 0.25 ms and 32 MiB 1.2 ms;
  a whole run that grows the room and copies 32 MiB takes 3.7 ms, of which growing is 2.5 ms. The
  median staged `.ydd` (63,378 bytes on disk, of 48) is 229,427 bytes stored, built in 0.11 ms
  (1.6 ms deflated) and copied in under 0.01 ms; the largest (1,436,813 on disk) is 2,359,672
  stored, 1.1 ms (24 ms deflated) and 0.08 ms; the largest `.ytd` (30,127,757 on disk, of 16) is
  75,517,200 stored, 34 ms (661 ms deflated) and 3.0 ms. Stored is 1.6 to 3.6 times the bytes on
  disk and takes 5 to 7 percent of the host's time to deflate, so the app hands the stored form.
- **Memory.** The copy is the file's size in the guest's memory, 1,153 pages of 64 KiB for that
  `.ytd`, beside the document's image, the copy every run keeps to put back, and the stored file
  the host holds, so a plugin on it holds four times 75 MB at once. ⚠ `Limits::memory_pages` is
  documented and not enforced, so a guest's memory is bounded by WebAssembly's 4 GiB and not by it.
- **Parse.** A guest parsing with `or-io` spends 10 to 30 million instructions on an authored
  `.ydd` of one drawable (88 bytes, the description is built first), 100 to 300 million on the
  median (0.27 s), and 3 to 10 billion on the largest (3.1 s), at about 1.2 billion a second under
  wasmi, against the 10 million of `Limits::default` that the app runs a plugin on. The app's
  `PluginSession::limits` holds it, and the owner set the editor's to 4 billion instructions, about
  3 s (2026-10-05, `EDITOR_LIMITS`): it covers the median `.ydd` and most large ones but not the
  largest, because a run is synchronous on the UI thread, so the fuel is the longest a spinning
  plugin freezes the editor.

**Depends on.** [R44d](#r44d-the-host-api-in-the-pack-vocabulary), [R44e](#r44e-consent-and-refusal-in-the-app),
[R25](#r25-the-edit-mode-gate). **Done when.** A copy path exists with its cost, or the refusal is
recorded with its measurement, and `a_payload_capability_does_not_exist_yet`'s message says which.
**Risk.** Fuel counts a guest's instructions and not the host's work, so a guest calling
`write_file` or `patch_field` in a loop makes the host read the document back each time.

### R45 The plugin SDK and registration surface

[↑ checklist](#phase-7--plugins-and-open-with)

**What.** A documented way to add a format, panel, capability, title, prefab catalogue, shader table,
command or setting without rebuilding the host — per-file-type actions, a provider protocol keyed by
`(format, generation)`, host math-type injection — plus a plugins directory, a manifest and a
guest-side library.

**Where it stands.** Four registries share one shape (format, title, panel, capability), two with
conformance tests (`src/state/adding_a_format.rs`, `crates/title/or-title/tests/adding_a_title.rs`),
and OD-AUDIT-KEEPS keeps them. The plugin manifest is `crates/domain/or-plugin/src/manifest.rs`.
The guest-side library is `plugins/guest`, on the ABI [R44c](#r44c-the-guest-abi) fixed, and
`plugins/sample`, built from it for wasm32, runs under the host in
`crates/domain/or-plugin/tests/the_sample_guest_runs_under_the_host.rs`;
[plugins/README.md](../../plugins/README.md#writing-a-guest) says how to build one.

A plugin whose manifest wants `cap.add_panels`, `cap.add_commands`, `cap.add_settings` or
`cap.draw_overlays` stays loaded as an `or_plugin::Session`, which `src/state/plugins/surface.rs`
holds once the user has answered for every capability ([R44e](#r44e-consent-and-refusal-in-the-app)):
at startup for a plugin already answered, from Tools → Run plugin otherwise. Its guest registers
from `register` and nowhere later, and only the functions its grants buy are linked
([Plugin_ABI.md](../reference/Plugin_ABI.md#a-guest-that-stays-loaded)). A panel joins the panel registry and is
drawn from the widgets the guest describes each frame (labels, buttons, sliders, checkboxes, text
fields and lists), which `src/ui/panels/plugin_panel.rs` draws and answers with what the user did,
so egui never crosses; a command joins the command registry, the palette and Tools → Plugin
commands with the shortcut it asked for where nothing holds it; a setting has a page in Settings and
a value in the preferences; and an overlay of points and lines in model space is added to the
viewport's lines, in front of the meshes, with a click delivered to a guest that asked for it as the
geometry, triangle and vertex under it (`src/render/plugin_overlay.rs`,
`src/render/viewport/pick.rs`). `crates/domain/or-plugin/tests/what_a_plugin_adds_to_the_editor.rs`
holds the session and what each grant buys, and `src/state/plugins/a_plugin_adds_to_the_editor.rs` holds each
registration in the app with the sample built with `--features surface`: a click on a panel's
button in a real frame reaches the guest, and waits for its own panel when the plugin's other panel
draws first, a command runs it, a setting is written to the preferences
and read back, a plugin dropped in the plugins directory registers all three, and an overlay's
primitives are the line vertices the viewport draws
(`the_render_bridge_draws_what_a_plugin_drew_in_front_of_the_meshes` is the GPU half, which needs an
adapter and is ignored without one). `just self-check` draws the host's side of a guest panel, because
it has no wasm32 module to run. A session reaches the document only as the file's bytes, and
`cap.read_install` buys no function, so a guest cannot yet read another archive's textures, which
[R46](#r46-first-plugin-a-clothing-tool-on-openrage) will ask for. A plugin that extends a build is the other
kind: `cap.read_plan`, `cap.report_findings` and `cap.add_files` buy `plan_size`, `read_plan`, `report` and
`add_file`, and the app runs its `validate` or `emit` export over each plan its scan and a build make
(`src/state/plugins/extensions.rs`), the plan going in as JSON and findings and text files coming out, with
nothing else crossing ([Plugin_ABI.md](../reference/Plugin_ABI.md#a-plugin-that-extends-a-build)). Not built, and asked for by no
plugin: a prefab catalogue, a shader table, per-file-type actions, a provider protocol keyed by
`(format, generation)` and host math-type injection. The clause that a plugin and its manifest open a
format or a title the build has never heard of is withdrawn: it has no carrier, because
[OD-OTHER-TITLES](Project_Context.md#no-runtime-pack-for-another-title-2026-09-26) drops the runtime
`.orpack` and a new title gets its description from its own declarations. **Done when.** A plugin
and its manifest dropped beside the executable add a panel, a command and a setting, and an author
builds a guest against a documented library. **Risk.** A registration replaces a whole entry and
never merges. **Covers.** U-038, U-177, U-191, U-195.

### R46 First plugin a clothing tool on OpenRage

[↑ checklist](#phase-7--plugins-and-open-with)

**What.** A DurtyClothTool equivalent on OpenRage, and the in-app surface's acceptance test:
per-vertex cloth authoring (weight, inflation scale, pinning, force transform, pin radius and
gradient) with viewport overlays, and diagnostics from a trial export, on
[R41d](#r41d-cloth-as-a-model)'s model.

**Where it stands.** Not started, and a rewrite: `archive/ydd-clothing-preview` at `337ee1bb`
predates the crate split (one of its files is `crates/or-io/src/rage_types/frag.rs`, a path that
does not exist in this tree) and reads its drawables as Rust types inside the app. A guest reaches a
`.ydd` as the file's bytes ([R44f](#r44f-payload-and-gpu-access)) and parses them with `or-io` linked
into the module, which gives it the owned `GtaDrawable` inside the guest, or one hop at a time
through the R44d host's reads of its description; the host never hands it a Rust type. Read the
branch for requirements: inspector panes, textures from other archives (`ReadInstall`), LOD
statistics. Two of its measurements carry over: a slot tag parses only once the
`dlc^` streaming prefix addon packs use is stripped, or an addon pack fails to find its own textures;
and a garment pairs to its textures by filename, because an addon pack's files carry stale internal
names (`jbib_000_u.ydd` holds `jbib_059_u`). The shell already pairs a garment opened from a folder
or an archive to the dictionaries beside it by filename, lists them in the Gallery apart from its
embedded textures, and draws the one chosen (`src/state/clothing.rs`).
[Clothing_Tool_Design.md](Clothing_Tool_Design.md) weighs where the pack-building half of the name lives.
**Depends on.** [R44d](#r44d-the-host-api-in-the-pack-vocabulary),
[R45](#r45-the-plugin-sdk-and-registration-surface). **Done when.** It installs as a plugin, draws
its panes beside the viewport, and needs no change inside OpenRage. **Risk.** The surface may prove
insufficient, which is the point; a guest cannot yet read another archive's textures, since
`cap.read_install` buys no function ([R45](#r45-the-plugin-sdk-and-registration-surface)), and a
texture or a buffer apart from its file has no capability
([R44f](#r44f-payload-and-gpu-access)). **Covers.** U-142, U-144.

### R47 The plugin slate after the first

[↑ checklist](#phase-7--plugins-and-open-with)

**What.** Applications the union list decomposes into, each separate and none core:

- **World and map editor** — picking, gizmos and snaps, markers, a thumbnail browser, pawn modes
  (U-042, U-043, U-047, U-049, U-050, U-053, U-057) — and **interior (MLO) builder** — portals,
  room bounds, rooms rebuilt from shell geometry (U-085, U-217).
- **Lighting and environment**: LOD-light baking, time of day, weather, timecycle, distant lights,
  an install-wide light index (U-078, U-101, U-120 to U-125).
- **Terrain and scatter** (U-079), **collision and navmesh authoring** (U-107, U-110), **audio world
  tools** with a `dat10` synthesiser (U-116, U-119).
- **Particle editor** with the engine's sprite-grid rule, a home for the in-app one (U-126), and
  **animation studio** (U-128).
- **Character and vehicle tools**: peds, vehicles, shattermap generation, weapon assembly (U-131,
  U-132, U-133, U-134, U-136); **cinematic capture** (U-145, U-146, U-147).
- **DCC bridge**: mesh, skinning and LOD authoring in Blender through [R38](#r38-mesh-interchange).

**Where it stands.** Each is filed by surface, in-app pane or whole process, when picked up after
[R46](#r46-first-plugin-a-clothing-tool-on-openrage), with its core data from
[R32](#r32-world-data-ymap-ytyp-and-interiors), [R33](#r33-paths-navmesh-scenarios-and-tracks) or
[R34](#r34-audio-and-media). The picking, the manipulator and the snaps of the world and map editor
are built in the app, on the Map screen (wave P44,
[what it built](Workbench_Views_Design.md#what-wave-p44-built-for-maps)), and the plugin route for
them stays open. Markers and pawn modes stay on this row, and so do the thumbnail browser, the walk
mode and the interior builder: [R70](#r70-the-gta-v-world-editor) proposes those three in the app,
which waits for [ruling 2](World_Editor_Plan.md#rulings-asked-of-the-owner). **Done when.** Each
exists, is owned by someone building on OpenRage, or is written off in one line. **Risk.** The pull
to build these in the shell, where a WASM pane and a shell pane look alike. **Covers.** The rows
above and the unnumbered mesh-authoring row.

### R48 Projects

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** A diffable project manifest with its lifecycle; membership across every editable type; an
edited file added to the project automatically; portable projects; and a project's copy of a file
resolving ahead of the game's, so an edit renders without repacking. [Project_Model_Design.md](Project_Model_Design.md) proposes modules, outputs and links on top of it, [Workbench_UX_Design.md](Workbench_UX_Design.md) draws the screens around it and [Workbench_Plan.md](Workbench_Plan.md) orders the work.

**Where it stands.** The manifest, lifecycle and File-menu submenu are built (`src/state/project.rs`,
`src/state/project_io.rs`); since #189 paths inside the project folder are relative, and
`or_title::mount_shadow` mounts the project's `vfs/` folder ahead of the overlay and the install.
The container is built in `crates/domain/or-project`: the file declares `modules` and `outputs`, a
file with none reads as one Files module over its documents, and `openrage-cli project` creates a
project, prints the plan of a build (a FiveM resource, a singleplayer DLC or an OIV package, each for the layout its target names, [what the targets build](Project_Model_Design.md#what-the-targets-build)), cooks the source folders of the project into the game files they stand for ([how a build cooks](Source_Format_Design.md#how-a-build-cooks)), builds it to a fresh folder, reads every file back and
keeps the previous build; the same command adds content and builds, removes a build, cleans what builds left and writes the redirect map, a build keeps what its verification found as data (every file reloaded and counted, metadata, the names checked, the estimated streaming memory against a stored ceiling, and a second build on request) and its sentences say Content and Build ([what the build plan builds](Project_Model_Design.md#what-the-build-plan-builds)). Content reaches a plan as a contribution, which the plan merges: a name that
two contents claim is refused across the whole project, a manifest key that describes the resource
must agree and any other adds up, figures are held to the limits an output stores, an output that
declares `split` divides at content boundaries, and a build says what was added, changed, moved and
removed since the last one by stable id, with a redirect map of the moves
([what the build plan builds](Project_Model_Design.md#what-the-build-plan-builds)). A build is declared and changed in the Builds window or with `openrage-cli project set-build` ([what the build plan builds](Project_Model_Design.md#what-the-build-plan-builds)): its content (a matrix of content by builds, with a cell locked where a kind of build cannot ship a content), split, ceilings and file types, and a second build is not named so that it reads as a part of a split. A link may lead to a whole project, whose Files content and packs are linked and whose own links are not followed, and a project of links alone builds ([what links build](Project_Model_Design.md#what-links-adoption-and-templates-build)). A clothing content is built as well: `crates/domain/or-wardrobe` contributes for it, and `openrage-cli wardrobe` and `openrage-cli project` plan it ([what the engine builds](Clothing_Tool_Design.md#what-the-engine-builds)), and the Wardrobe workspace shows it, takes dropped clothes with Undo, reads a shop's own metadata into the ids of the textures it names, links a cloth to a garment and rescans a pack when its files change, and offers Fix… and Waive… over its rules ([what the Wardrobe workspace builds](Workbench_UX_Design.md#what-the-wardrobe-workspace-builds)). The Project workspace shows it (`src/state/workbench/`, `src/ui/panels/project/`): the content and the builds, a virtualised table of every file and source folder the project holds with its state, its problems, the plan of each build and what the last one wrote and read back, a build on Ctrl+B and a saved image in a source folder recooked into the shadow folder, a model put into a drawable source with Replace geometry… or `openrage-cli source import-model`, and a source checked against the game on request ([the edit loop](Source_Format_Design.md#the-edit-loop)); the Project menu, the project name and Build button in the status bar and the pinned workspace buttons with More are [Workbench_UX_Design.md](Workbench_UX_Design.md)'s. A project is begun from the start page or File ▸ New project… (an empty one, loose files to ship as a FiveM resource, a clothing pack, or a folder adopted with nothing moved), a launch reopens the last one, the recent projects are cards with Locate… and Remove for one that moved, and a second project opens in a second OpenRage process ([what the front door builds](Workbench_UX_Design.md#what-the-front-door-builds)). A folder that exists is adopted with what is recognised in it grouped into content and what could not be placed listed, a pack that another project holds is linked and read here while that project stays its only writer, a project's shape is shared as a template file, a project file dropped on the window opens as a project and `.openrage-proj` is a file association Settings offers, and the game a project is for is the edition a new build ships for ([what links, adoption and templates build](Project_Model_Design.md#what-links-adoption-and-templates-build)). A game file, an archive entry's bytes or a folder is imported as sources under `sources/` by `openrage-cli project import` ([how an import writes](Source_Format_Design.md#how-an-import-writes)) and by the app's Import game files…, Import into project (a row's menu in Files, beside Open) and Add images as a textures source…, a job whose report window has Show in Assets and Undo import ([open, import and add](Workbench_UX_Design.md#open-import-and-add)); the other things the word Import named are Replace from image…, Replace from glTF…, Copy into archive… and Add n file(s). Membership is its own list: a project file's `documents` are its members and `open` the tabs among
them, so closing a loose file's tab keeps the file in the project (an archive entry is a member
only while its tab is open at a save), a schema-2 file opens with every member open,
and `save_active_project` with no project open creates one where the dialog says. `remove_from_project`
and the Project submenu drop a member, close its tab and keep the file, refusing a tab with unsaved
edits; the window title marks the project changed until its next save. Every change to what the project holds is a step of its History (`src/state/workbench/project_history.rs`, kept in the project file's own folder of `.openrage` by `or_project::snapshot`), which Undo, Redo and a click in the History panel move along, and a save writes the file only when its bytes change, with the workspace it states and no path of this machine ([what project history builds](Workbench_UX_Design.md#what-project-history-builds)). A clothing pack's edits are recoverable as well: a Redo beside Undo, one Undo for the Wardrobe workspace that joins the clothes' changes and the project's steps, a rotating set of versions of `wardrobe.json` kept beside the pack with a card that restores the last good one, one garment or texture put back from a version, a lock so a second window reads a pack and cannot write it, and the original of a replaced texture kept with its hash ([what the Wardrobe workspace builds](Workbench_UX_Design.md#what-the-wardrobe-workspace-builds)). Open: a project
texture wins only through the missing-texture fallback; a zipped project folder opened under another path plans as it did (`a_project_folder_zipped_and_opened_under_another_path_has_the_plan_it_had`).
A new project starts with the managed `.gitignore` and `.gitattributes` blocks, and Project ▸ Check before sharing… lists the paths of this machine, the sources taken from the game and the `.openrage` folder ([what sharing and help build](Workbench_UX_Design.md#what-sharing-and-help-build)). Found while closing [R60c](#r60c-the-front-door) and
[R60f](#r60f-the-unsaved-marker-tells-the-truth): a project saved an opened folder as one of its files
and then failed to reopen it, which is fixed since an opened folder is a place to look in and no member
(it adds no row and no problem), and a file opened while a project is open becomes a member at the next
save, which no longer marks the project changed: opening a file is not a change the project asks to save, and a command that puts something in it is. **Done when.** A project survives being zipped and opened on another machine,
and a project file renders live ahead of the game's. **Risk.** Project formats are forever:
a change to the file bumps `PROJECT_SCHEMA` and migrates on the next save. **Covers.** U-051, U-052, U-054, U-055, U-164.

### R49 The mods overlay

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** The `mods` overlay, OpenIV's key safety feature, in four parts, all
done: [R49a](#r49a-overlay-first-resolution), [R49b](#r49b-the-overlay-as-the-place-you-write),
[R49c](#r49c-staleness-after-a-title-update) and [R49d](#r49d-the-game-side-loader); R49a before
R49b, the other two independent. [R50](#r50-package-installation),
[R52](#r52-mod-manager-and-load-order) and [R60](#r60-onboarding-permissions-and-crash-reporting)
build on it. **Covers.** U-014, U-150, U-151, U-152, U-153.

### R49a Overlay-first resolution

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

Done in #81 and #78. `crates/title/or-title/src/vfs/overlay.rs` holds the rule — `mods` as one whole
path component directly below the install root, and `resolve_archive` per archive — behind
`MountOptions` (the app's `read_overlay_first` pref, on by default), and
`src/state/retail.rs` asks `or_title::is_under_overlay`, so one predicate decides read and write
(`crates/title/or-title/tests/the_overlay_is_read_before_the_install.rs`). `mods_` is how a user
parks an overlay, and a mount summary of "0 of N mounted archive(s) came from it" is what a
misplaced one looks like. ⚠ An overlay holds archives, mounted where the install's copy mounts, not
loose files under a folder device. `load_from_gta_install` (the materials palette) stays
install-only, so the toggle never has a read it cannot turn off. Two Done-when clauses belong to
other rows: the install tree is a disk walk ([R19](#r19-one-virtual-filesystem)), and the app shows
the mount report only in the log.

### R49b The overlay as the place you write

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** Copy-to-overlay and show-in-overlay, inline remediation when a user edits outside the
overlay, and the hard overlay-only guard.
**Done when.** A shipped file becomes an overlay file from inside OpenRage, and an edit outside the
overlay offers the copy. **Risk.** An archive copy can be gigabytes; say so first.

**Where it stands.** An archive's menu in the tree offers Copy to mods overlay…, and a write the
gate refuses to a shipped `.rpf` raises the same prompt while the edit mode permits the overlay
(`an_edit_outside_the_overlay_offers_the_copy`). The prompt states the size before anything is
written; the copy asks the gate about its destination, runs on a worker and lands under a
`.rpf.part` name renamed into place, so a half-copied archive never stands in for the install's
(`src/state/overlay_copy.rs`, `a_shipped_archive_becomes_an_overlay_file_through_the_copy`).
`overlay_copy_target` asks `overlay_path_for`, the rule the mount reads through, and an overlay
copy that exists, even one that appears after the offer, is never overwritten. Saving the mounted
install and every tree edit (new folder or file, import, rename, move, delete) ask the gate, so a
refusal stops the write and offers the copy rather than replacing the shipped archive
(`a_refused_archive_in_the_mounted_install_is_not_written`); the Overlay edit mode is the hard
overlay-only guard. Since [R25g](#r25g-the-edit-mode-visible-and-honest), an archive whose overlay
copy exists offers Show in mods overlay in place of the copy, and it selects the copy in the tree
that holds it. Not built: moving the open document onto the new copy, so an unsaved edit to the
install's archive stays with that archive.

### R49c Staleness after a title update

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** Detect an overlay copy of an archive the title has since patched, with a refresh that warns
it discards changes. **Done when.** A title update warns, naming the archives it invalidated,
without crying wolf.

**Where it stands.** A copy made from the app, by the archive menu or by the first save of an edit
into the overlay, is recorded under the app's config directory as the install's archive stood: its
size, modified time and entry table (`BaseRecord`, `crates/title/or-title/src/vfs/overlay_base.rs`).
Opening an install looks at each recorded copy. A stat that matches is fresh. One that differs reads
the original's entry table, which takes milliseconds where hashing the file takes seconds, and only
a different table is stale, so a touch, a verify or a move does not warn
(`a_base_that_was_only_touched_is_not_stale`). A copy with no record, which includes every copy made
before the record existed and any made by another tool, is never reported. A stale copy is named in
a warning, drawn in the warning colour in the tree and listed in the Mods overlay settings, and each
offers Refresh from the patched archive…: a worker weighs the copy's decoded files against the
patched archive's, and the prompt lists the user's edited, added and removed files, with the
update's own changes counted apart, before anything is written
(`a_refresh_lists_what_it_discards_and_writes_nothing_until_it_is_answered`, `refresh_plan`, in
`src/state/overlay_refresh.rs`). The suite's header, `crates/title/or-title/tests/an_overlay_copy_the_title_has_outgrown.rs`,
weighs the four candidates and holds the measured cost over a retail Legacy and Enhanced install.
⚠ The table cannot see a patch that changes contents and no size, flag or name
(`a_patch_that_moves_nothing_in_the_table_is_not_seen`), and only one patch level of each edition was
available, so whether a title update ever does is unobserved. Not built: re-applying the user's
changes onto the patched archive, so a refresh replaces the copy whole.

### R49d The game-side loader

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** Detect OpenRPF or RageOpenV, say whether the game will read the overlay, and help install
one. A decision before code: RageOpenV is GPL-3 and a separate process, so recommending it is viable
where embedding is not. **Done when.** OpenRage tells a user whether their install reads the overlay
and helps them get there by whichever of recommend, bundle or install this row decides.

**Where it stands.** Done by recommending: OpenRage names what is missing, the folder it goes in and
the project's own page, and downloads, bundles and installs nothing. `GameProfile::overlay_loaders`
states per title the plugins that redirect the game's reads to `mods` and the names the game loads an
ASI loader under, and `check_overlay_loader` (`crates/title/or-title/src/game/overlay_loader.rs`)
lists the install folder's file names, whatever their case, and runs nothing. Legacy reads `mods`
through `OpenIV.asi` or `RageOpenV.asi` with a `dinput8.dll`; Enhanced through `OpenRPF.asi`, which
starts itself from its 0.2, or `RageOpenV.asi` with an `xinput1_4.dll` or a `dsound.dll`. A plugin of
the other edition is named and counts for nothing
(`crates/title/or-title/tests/an_install_says_whether_the_game_reads_its_overlay.rs`).
Settings ▸ Editing ▸ Mods overlay says in a line whether the game reads this install's overlay, and
when it will not, what is missing, where it goes and each project's page, with Check again once a file
is added (`src/ui/panels/settings_overlay_loader.rs`). A write the edit gate permits into the overlay
of an install the game would not read warns once for that install until it is checked again
(`a_write_into_the_overlay_of_an_install_the_game_will_not_read_warns_once`). ⚠ Every name comes from
the projects' own pages and no run of the game has confirmed a verdict, which is the owner's: a
`dinput8.dll` is taken for an ASI loader by its name, a loader a user renamed is not seen, and an
OpenRPF 0.1, which needed a `dsound.dll`, beside none reads as working. Not built: a plugin's version.

### R49e Compare a file with another or with the original

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** A side-by-side diff of two text or XML files, a field-level diff of two resources from
their declared structure, and "Compare with original" for a file in the mods overlay, which answers
what a mod changed.

**Where it stands.** Done; other resources and archives are [R49f](#r49f-compare-every-resource-and-archive)'s. A file's row in Files or the Files view offers
Select for compare, Compare with selected, labelled with the file chosen, and Compare with original,
registry commands each disabled with its reason (`src/ui/commands/compare.rs`). Both files are read
on a worker, the background jobs' Comparison, and open as a document of their own,
`DocumentKind::Compare`, in a tab named for both sides that is never saved and whose Previous and
Next difference step through what differs (`src/state/compare/`, `src/ui/panels/compare.rs`).
Binary metadata diffs by record and field, walked and named as the metadata editor walks and names
them, an array's records lined up by their first hash or text, each change with both values
(`two_ymts_compared_from_their_rows_differ_by_record_and_field`). A texture dictionary diffs by
texture name: each added or removed, and of one both hold its format, size, mip levels and whether
its pixels differ, read from the owned values, so another layout of the same value differs in
nothing. Compare with original reads the same entry of the install's own archive, which
`or_title::vfs::install_path_for` names as the inverse of `overlay_path_for`, through one
`ArchiveDevice`, and a file the game lacks says so
(`a_texture_dictionary_in_the_overlay_compares_with_the_games_and_a_file_with_none_says_so`).
Texts, XML metadata among them, line up side by side
(`two_texts_line_up_side_by_side_and_previous_and_next_step_between_their_changes`), aligned by
what each side holds once and then the longest common run, by a table no larger than `TABLE_CELLS`
(`src/state/compare/align.rs`). Seen on `just shots` screens, not in a window. Any other
resource field by field from the rows `or-editor` builds, and
two archives entry by entry, are [R49f](#r49f-compare-every-resource-and-archive)'s, and done there.

**Done when.** Two `.ymt` files diff by record and field, and a texture dictionary in the overlay
compares with the game's.

### R49f Compare every resource and archive

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** Any resource compares field by field from its declared structure, two archives compare
entry by entry, and a metadata map compares by key.

**Where it stands.** Done. Two resources of one kind compare by part and field
(`src/state/compare/resource.rs`): each side is read as OpenRage saves it, so another layout of the
same value differs in nothing, and its parts and fields are the rows
[R31b](#r31b-the-inspector-leads-with-a-summary)'s Summary and Outline build for the inspector, matched by
path, a run's elements lined up by what each holds where the runs differ in length, each change with
both values (`two_drawables_compared_from_their_declared_fields_differ_in_each_changed_field_with_both_values`).
What no row lists, vertices or pixels among it, reads as Other data, with both sizes and where the
bytes first differ: the physical segment's, which a field edit leaves as it was, beside the fields
that differ (`a_difference_in_data_no_field_lists_is_reported_beside_the_fields_that_differ`), and
the whole file's where no field differs (`a_difference_in_data_no_field_lists_is_still_a_difference`).
The outline stops
at six levels (24 for a comparison, since [R49g](#r49g-what-a-comparison-still-misses-after-r49f)) and at 128 elements of a run, so a difference past either is no row: the tab says what
it did not read under its summary, and an element that one run pushes past the limit is not reported
removed from the other
(`a_model_added_to_a_run_past_the_limit_is_one_added_row_and_the_comparison_says_what_it_did_not_read`,
`a_difference_past_the_limit_is_not_listed_and_the_comparison_says_so`). Lifting the element limit
took one staged `.ypt` from 292,709 nodes to 38,875,685 (measured as [R49g](#r49g-what-a-comparison-still-misses-after-r49f)
gives it), so the limits stay. Of the twelve smallest shipped
files of each staged format, up to three that the app opens compare as the same as their own saves, and a staged
format none of whose twelve opens fails the test; a file the app refuses, such as a `.ybd` at RSC7
version 40 or 42, compares by its bytes
(`a_shipped_resource_and_its_save_in_another_layout_compare_as_the_same`). Two archives compare entry
by entry (`src/state/compare/archive.rs`): each file at its path, whatever its case, a nested archive
as the files in it, added, removed or changed by content
(`two_archives_compared_entry_by_entry_find_the_entries_added_removed_and_changed`), on a worker that
a cancel stops between entries
(`a_stop_ends_an_archive_comparison_between_its_entries_and_without_one_it_finds_the_change`,
`a_cancel_from_the_job_list_tells_the_comparison_running_to_stop`). An
archive's row in Files and in the Files view offers the three compare commands; an archive compares
with an archive and a file with a file, one with unsaved changes asks to be saved first, and Compare
with original reads the install's own copy of an archive in the overlay
(`an_archive_in_the_overlay_compares_with_the_games_and_one_with_none_says_so`). A comparison does
not start on an archive a defragment is rewriting or has on offer, as an extraction does not
(`a_comparison_waits_for_a_defragment_of_an_archive_it_reads`). A metadata `Map`
compares by key, wherever the entry stands (`two_maps_differ_by_key_wherever_the_entry_stands`).
Encrypted archives read through the keys the tree holds
(`an_encrypted_archive_in_the_overlay_compares_with_the_games_through_the_keys_the_tree_holds`, under
AES keys made for the test), and an install's own NG-encrypted packs, which needs
`OPENRAGE_GAME_ROOT`, or an install the app's discovery finds when it names none ([R49g](#r49g-what-a-comparison-still-misses-after-r49f)), and skips without one
(`an_installs_encrypted_archives_compare_through_the_keys_read_from_its_executable`, run once on a
Legacy install). Not built: an archive inside another has no original of its own, so Compare with
original asks for the one holding it; two resources of different kinds, or one kind at two targets,
compare by their bytes; a changed entry that is no resource, binary metadata or texture dictionary shows its sizes and where its bytes first differ, not what
the entry holds ([R49g](#r49g-what-a-comparison-still-misses-after-r49f) reads a changed resource field by field inside an archive); a difference past the comparison's limits is said to be unread, not listed; and data
the virtual segment holds beyond the rows and beyond the blobs below (a string table, an opaque
word) is told only where no field differs, since a changed field moves the segment. A blob in the
virtual segment is compared on its own and reads as a row of its field's name, with both sizes and
where the bytes first differ, beside the fields that differ: one a bytes pointer reaches
(`a_blob_behind_a_bytes_pointer_is_compared_beside_the_fields_that_differ_and_alone`, a `.ybn`'s
material indices) and a run of bytes trailing its struct
(`a_blob_a_field_reaches_is_compared_beside_the_fields_that_differ_and_alone`, a clip block's
channels), once where its length changes too
(`a_blob_that_changes_length_is_one_row_and_not_also_a_row_for_its_run`); one in the physical
segment is Other data. A bound's tagged base resolves to the variant that holds it; since [R49g](#r49g-what-a-comparison-still-misses-after-r49f) the outline builds no part under itself, and before it the outline
repeated a `.ybn` bound down to its depth limit, so the comparison read each part once, and a repeat was neither
listed nor counted among what the limits left out
(`a_bound_is_read_once_though_its_tagged_base_repeats_it_down_to_the_depth_limit`,
`a_bound_whose_base_resolves_to_another_that_holds_it_is_read_once_too`). Seen on `just shots`
screens, not in a window.

**Done when.** A test compares two drawables and finds each changed field with both values, and a
test compares two archives and finds the entries added, removed and changed.

### R49g What a comparison still misses after R49f

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** A comparison reports every difference between two resources and two archives, not only the
ones a field row lists within the outline's limits, and a compare command that cannot act says why
before it is used.

**Where it stands.** Done, bar what the last paragraph lists. A blob behind a `Pointee::Bytes`
pointer is compared beside the fields that differ in a drawable's bound as in a `.ybn`
(`two_drawables_that_differ_in_a_blob_behind_a_bytes_pointer_and_in_a_field_are_reported_in_both`),
and so is a run with no element type, wherever it lies in the virtual segment: trailing its struct,
behind a pointer or in a counted array
(`two_models_that_differ_in_the_bytes_of_a_run_with_no_element_type_are_reported`,
`a_run_with_no_element_type_changed_in_its_bytes_is_reported_whether_a_pointer_or_a_count_reaches_it`).
A change in such a run's length is one row with both sizes, not also a row for the part's label
(`a_run_with_no_element_type_that_changes_length_is_one_row_for_each_run`). A scalar array of more
than four elements and an opaque field carry what they hold in full in `Row::exact`
(`crates/domain/or-editor/src/lib.rs`), so a change past the third decimal, or in one opaque byte, is
a row with both values (`a_scalar_array_changed_past_its_third_decimal_is_reported_with_both_values`,
`an_opaque_field_changed_is_reported_with_both_values`,
`an_array_of_many_scalars_and_an_opaque_region_say_what_they_hold_in_full`). No declared format has
either today: the one opaque field, `OctantVertsTable.octant_verts`, is a private table of pointer
words and so no row, and no `f32` array is longer than four, so those tests lay out a pack of their own.

Two archives that hold a changed resource, binary metadata or a texture dictionary compare it field
by field, each row under its entry (`two_archives_holding_a_changed_resource_find_the_fields_that_changed_inside_it`);
any other entry, or one whose fields tell nothing apart, shows its sizes and where its bytes first
differ, and the tab says when a changed entry was not read whole
(`an_archive_whose_changed_resource_was_not_read_whole_says_so`). The Property column of an archive
comparison is wider for a path of parts. Seen on `just shots` screens, not in a window.

The outline reads a part once: a tagged base that resolves to the variant holding it, whose own inline
base resolves back, is a part under itself and is no node
(`the_outline_of_a_composite_bound_holds_no_part_under_itself`,
`a_tagged_base_that_resolves_to_the_part_holding_it_is_not_a_part_under_itself`), so the comparison
keeps no guard of its own. A comparison descends 24 levels of parts, not the inspector's six
(`Outline::of_to`), and finds a difference in the last part of a `.ybn` nested ten composites deep
(`a_difference_in_the_last_part_of_a_bound_nested_past_the_outlines_limit_is_found`); a part past that
is still left out, and the tab says so. Measured with the repeats gone, of 149 staged files, up to a
dozen from the smallest to the largest of each format, 54 have a part at the inspector's sixth level
that holds more and none at the comparison's
(`a_comparison_reads_every_staged_file_to_its_last_part`, at `07bb1dfe` with this change). A scratch
run of the outline at depths 6, 8, 10, 12, 16 and 24 over every staged file of each format, up to 40
`.ynv`, found at depth 6 all 50 `.ydd`, all 18 `.ydr`, all 68 `.yft`, all 60 `.ycd` and all 6 `.ypt`
cut, and no `.ybn` or `.yld`; at 8 no `.ydd` or `.ydr`; at 12 no `.yft`; at 16 2 `.ycd` and 3 `.ypt`;
at 24 none. The same run took the outline of the largest `.ypt` from 631,197 nodes at depth 6 to
2,815,892 at 24, and a comparison of the two largest `.yft` from 0.9 s to 4 or 5 s, in the dev profile
under other lanes' builds; two `.ypt` took 10 s at either depth. The outline has no node budget, and a stop is checked between archive entries and not inside one resource, so a comparison of one big `.yft` or `.ypt` runs to its end.

Compare with selected and Compare with original are off, with a reason, while a defragment holds or
offers to rewrite an archive their rows read: the row's own, the one Select for compare chose, or the
game's copy of an overlay archive, each a fixed sentence
(`compare_with_selected_is_disabled_with_its_reason_on_an_archive_being_defragmented`,
`compare_with_original_is_disabled_and_refused_while_a_defragment_holds_either_archive`). A click
after a defragment began is refused too: Compare with selected asks again
(`a_comparison_waits_for_a_defragment_of_an_archive_it_reads`), and Compare with original, which
does not, is refused at the start of the comparison, for a defragment begun since its menu drew
(the test of its command above). The install-gated test of encrypted
archives reads the installs the app's own discovery finds when `OPENRAGE_GAME_ROOT` names none, and
`or_test_support::game_roots` still reads the variable alone
(`an_install_the_app_finds_is_read_where_none_is_named_and_a_named_one_is_read_alone`); on the owner's
machine it ran. None of 5,411 untyped trailing runs in those 149 staged files starts in the physical
segment (`no_staged_file_holds_an_untyped_trailing_run_in_the_physical_segment`, at `07bb1dfe` with
this change); three declared types hold one, `CrBlockStream`, `CrExpressionStream` and
`PgBasePageMap`.

Left, which the Done-when does not reach. The physical-segment check assumes the writer lays physical
blobs out independently of the virtual segment, held on the staged corpus and on a synthetic drawable
and not proven for every format, so a format whose physical layout depended on a field would show a
spurious "Other data" row beside that field. Data in the virtual segment that no blob comparison
reaches (a string table, a word the owned value keeps to itself) that differs while a field row also
differs is not reported, since a changed field can move the segment and the rows carry no byte range
to mask what they explain. A run is still read to its first 128 elements: lifting that limit took one
staged `.ypt` from 292,709 nodes to 38,875,685, measured by the R49f lane at `e84b0c58` with its
change, so comparing a run's elements as a stream, not through the outline, would answer it. The
depth count in the sentence that says what the comparison did not read is type-level, since a part
at the last level whose type has pointer, array or inline fields counts even where a pointer is null,
so its wording is "whatever lies under N parts", not "N parts hold more". A value read from a shipped
`.yld`, `.ynv`, `.yft`, `.ydr` or `.ypt` differs under `==` from the value read from its own canonical
save in a field no non-internal row lists (15 of 26 staged files, measured by the R49f lane at
`e84b0c58`), so the comparison reads saved bytes instead; which field differs was not found, and the
carry's `PartialEq` is always true, so it is not the carry. In a `just shots` script the palette
listed no match for "Compare with selected" (typed after Select for compare through the palette and a
click on the second file) while the row's context menu offered "Compare with a.ydr" enabled; the
palette command is labelled with the file chosen, and "Compare with a.ybn" works in a script; not
investigated further. The Part column of a resource comparison starts 180 points wide, which a path of ten
distinct parts does not fit (`fields` in `src/ui/panels/compare.rs`), and was not looked at on a
screen. The inspector's Summary rows, which the comparison reads, still show a `Pointee::Bytes` field
as "-> bytes" with no content, so the two values of a changed blob come from the comparison's own sizes
row and not from a Summary row.

**Done when.** A test compares two drawables that differ in a blob behind a `Pointee::Bytes` pointer
and in a field and finds both reported; a test compares two archives holding a changed resource and
finds the fields that changed inside it; a test compares a `.ybn` with parts deeper than the
outline's limit and finds the difference at the last part; a test finds Compare with selected
disabled, with the reason, on an archive being defragmented; the install-gated test finds the
owner's install without `OPENRAGE_GAME_ROOT`; a test builds the outline of a `.ybn` composite and
finds no part under itself; the depth measurement is retaken with the repeats gone; a test changes
a scalar array's value beyond its third decimal and an opaque field's bytes and finds each
reported; a test compares two values that differ in the bytes of a run with no element type and
finds them reported; and the corpus is measured for an untyped trailing run in the physical
segment.

### R49h What a comparison still reads wrongly after R49g

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** A comparison tells the user what it could not read or did not start, reads every side as
OpenRage saves it, counts a difference once and shows a long path whole.

**Where it stands.** Found while closing [R49f](#r49f-compare-every-resource-and-archive) (#558) and
[R49g](#r49g-what-a-comparison-still-misses-after-r49f) (#567): Compare with selected and Compare
with original are offered and enabled for an archive row with no source to read, a nested archive
whose open failed (`ArchiveSource::Nested` with no `opened`), and the click does nothing and says
nothing, since `side` in `src/state/compare/mod.rs` finds none; if one side opens but will not save,
`compare` in `src/state/compare/resource.rs` falls back to that file's bytes while the other side is
canonical, so a raw layout against a canonical one shows a spurious Other data or field difference
and the user is told nothing; a field in a struct nested more than four inline levels under a part
is in no row, since `Walk::pair` skips the rows of an inline child past `MAX_INLINE_DEPTH`; one
column mixes two sizes, an added or removed entry's stored (compressed) size and a changed one's
decoded sizes; `fields_of` in `src/state/compare/archive.rs` skips an entry a line diff would only
throw away by the path's kind and not the refined kind, so XML metadata (`.meta`,
`DocumentKind::Meta`) is parsed and line-diffed for each changed entry and the result discarded,
while a `.dat` or `.xml` entry whose bytes refine to binary metadata is skipped by the text check
and not read field by field, as its doc comment says it is; `summary` counts `changes.len()` under
"by entry", so one changed drawable with forty fields reads "40 differences" and two changed entries
"by entry, 3 differences"; the By::Entry columns are 250, 290 and 100 points where the others share
320, 130 and 220, so a long left value can clip at 100 points and a long entry path at 250, and the
Part column starts 180 points wide, where a path of ten parts is cut, none of which was looked at in
a window; and the fixed sentence for a held archive, "The archive this is read from is being
defragmented", is also what the user reads when it is the game's own copy that is held, which a
third sentence naming it would say.

**Done when.** A test clicks Compare on a nested archive whose open failed and finds a toast saying
why; a test compares two files one of which will not save and finds it said; a test changes a field
five inline levels deep and finds it reported; a test finds the size column mean one thing; a test
compares two changed `.meta` entries and finds neither read twice nor skipped; a test compares one
drawable with forty changed fields and finds the summary counting entries; and a screen shows a long
path and a long left value whole.

### R49i What the checks of a comparison still leave after R49g

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** A comparison is bounded in memory and in time, its measurements can be run again, and its
suites skip on a machine they were not written for instead of failing or reading an install they
should not.

**Where it stands.** Found while closing [R49f](#r49f-compare-every-resource-and-archive) (#558) and
[R49g](#r49g-what-a-comparison-still-misses-after-r49f) (#567): the outline has no node budget and a
comparison descends 24 levels (`DEPTH` in `src/state/compare/mod.rs`), so the largest staged `.ypt`
is 2,815,892 nodes where it was 631,197 at six levels, at about 144 bytes a node and three or four
small strings each, several hundred MB on each side, which an out-of-memory abort turns into the
loss of the app and every unsaved document; a stop is checked between entries and never inside one
resource, so one large `.yft` (4 to 5 s against 0.9 s) or `.ypt` cannot be cancelled; the deepest
staged files need 12 levels (`.yft`) and more than 16 and at most 24 (`.ycd`, `.ypt`), and why the
`.ycd` and `.ypt` run deeper, the `atMap` bucket chain being the likely cause, was not looked at;
and the depth survey and its node counts come from a scratch run that is not kept.
`no_staged_file_holds_an_untyped_trailing_run_in_the_physical_segment` asserts `runs > 0` outside
its skip, so a corpus with files but none of `CrBlockStream`, `CrExpressionStream` or
`PgBasePageMap` fails it, its zero was never seen non-zero and nothing shows its test can detect a
run; `a_comparison_reads_every_staged_file_to_its_last_part` asserts that none is cut at 24 levels,
so staging one deeper `.ycd` or `.ypt` fails the suite and not the code; R49g's first Done-when
clause very probably passes on the tree before it, since R49f's guard already kept the repeated
bound rows out, and its failing-first evidence is from deleting that guard too;
`two_drawables_that_hold_the_same_compare_as_the_same_whatever_their_bytes` compares one byte string
with itself, which `compare` answers at its early exit before any row is read, so another layout of
the same value is held by the corpus test alone, which skips without a corpus; and the refusal for
an archive inside another, the `NESTED` branch of `original_of`, has no test. The install-gated
suite now runs wherever the app's discovery finds an install, `just test-no-pack` included, and
reads and copies two small encrypted packs from the owner's install on each `just ready`, about 4 s;
`install_roots_told_else_found` admits an Enhanced install while
`an_installs_encrypted_archives_compare_through_the_keys_read_from_its_executable` hard-codes
`GTA5.exe` for the check and the keys, which a sibling test calls a trap for Enhanced, so it can
panic on `has_keys` where it should skip; `game_roots` drops an environment entry that is not a
directory, so a stale `OPENRAGE_GAME_ROOT` falls through to discovery against "a root that is named
is the only one read"; and the suites of or-world, or-depends, or-title and or-io still read the
variable alone and skip on a machine whose install the app finds, which the helper's doc says is on
purpose.

**Done when.** A test builds the outline of the largest staged `.ypt` at 24 levels and finds a node
budget that says what it left out, and a test cancels a comparison inside one resource and finds it
stopped; the depth survey is a suite that runs again; the two corpus measurements skip where the
corpus holds no file they are about; R49g's first clause is seen failing on the tree before it; a
test compares two files laid out differently and finds them the same; a test finds Compare with
original refused for a nested archive; and the install-gated suite skips on a machine with an
Enhanced install alone and reads a named root alone.

### R50 Package installation

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** Installing `.oiv` packages, the way most GTA V mods ship, in three parts:
[R50a](#r50a-the-oiv-container-and-its-manifests), [R50b](#r50b-the-patch-languages) and
[R50c](#r50c-the-command-engine-the-journal-and-the-install), in that order. None exists; OpenIV's
published specification may be matched as a spec
([the licence policy](../guides/Licence_And_Provenance_Policy.md)). ⚠ The XPath clause is a query-language
implementation, so it is a part of its own, met early. Its binary half is cheap: `or_io::meta`
re-encodes an untouched document exactly (PSO and RBF byte-identically, RBM on its decompressed
payload), so patching a `.ymt` needs only addressing. **Depends on.**
[R49a](#r49a-overlay-first-resolution), [R37](#r37-metadata-and-xml-at-parity),
[R20](#r20-archive-lifecycle). **Covers.** U-154, U-156, U-157, U-158, U-159.

### R50a The OIV container and its manifests

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** The zip container, the 1.1, 2.1 and 2.2 manifests behind one model that remembers each
field's version, metadata, optional groups and the target title, in an installer module above
`or-title`. **Done when.** A sample of popular packages parses with every field a user chooses by,
and an unknown manifest version is refused by name.

### R50b The patch languages

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** Line patches with Equal, StartWith and Mask, and XPath over XML and over `or_io::meta`'s
`MetaInstance`. **Where it stands.** Which XPath subset shipped packages use needs a corpus of
community packages: count their axes and predicates first, and refuse what is unsupported rather
than mis-evaluate it. **Done when.** One query suite gets the same nodes from an XML document and its
binary form, and a patched binary re-encodes against its own layout.

### R50c The command engine the journal and the install

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** The command set against the overlay, the install-location chooser, target-title
enforcement, a concurrency guard, an install log, and a journal written as each command executes, so
it records a half-installed package. **Done when.** Popular packages install and the game runs, every
refusal names its cause, and [R51b](#r51b-package-uninstall) can be written against the journal.

### R51 Package authoring uninstall and DLC creation

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** Three things no tool in the inventory has: [R51a](#r51a-package-authoring) after R50a,
[R51b](#r51b-package-uninstall) after R50c, and [R51c](#r51c-dlc-pack-creation), which needs nothing
from [R50](#r50-package-installation). **Covers.** U-155, and the unnumbered mod-uninstall and
DLC-pack-creation rows.

### R51a Package authoring

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** The inverse of [R50a](#r50a-the-oiv-container-and-its-manifests).

**Where it stands.** A project output of the kind `oiv-package` writes `<name>.oiv` from its plan
(`or_title::oiv`, `crates/domain/or-project/src/oiv.rs`): a stored zip holding `assembly.xml` and each file under
`content/`, and a manifest with one add for each file, inside the archives its path runs through. The
manifest is written from the 2.2 shape as the author knows it and is read back by this tree alone,
because no OpenIV was available; it authors no other command, reads no package it did not write and
refuses a manifest version other than 2.2 by name. **Done when.** A package built in OpenRage installs
in OpenRage and in OpenIV, the second opinion against a shared misreading.

### R51b Package uninstall

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** Reversing a package from [R50c](#r50c-the-command-engine-the-journal-and-the-install)'s
journal: reverse what was recorded, refuse and explain when the files have since changed. **Done
when.** A package installs and removes cleanly, and a changed one is refused with the reason.

### R51c DLC pack creation

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** A generated DLC pack: `dlclist.xml`, `setup2.xml`, `content.xml` and the archive tree.
**Where it stands.** Built in #148: the three documents write, `build_pack` assembles a pack, and
`openrage-cli dlc create` and, since [R51d](#r51d-a-dlc-pack-from-the-app), the app's Create DLC pack
are its surfaces, through one call. ⚠ A model that drops `contentChangeSets` writes every file
disabled; `a_generated_pack_enables_every_file_it_registers` holds it. Loading in game is open,
since nobody here can run GTA V ([Dlc_Pack_Creation.md](../features/Dlc_Pack_Creation.md)). A project build is a third
surface: an output of the kind `singleplayer-dlc` writes the pack from its plan through the same
`build_pack`, with the types its files state, a path through an archive's name packed as that nested
archive, and the `dlclist.xml` line it needs ([what the targets build](Project_Model_Design.md#what-the-targets-build)). **Done when.** A
generated pack loads in game with no manual editing. **Risk.** Mount order
(`SetupData::mount_cmp`) decides whether its overrides win.

### R51d A DLC pack from the app

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** "Create DLC pack" on a folder, with the checks [R51c](#r51c-dlc-pack-creation)'s writer
makes shown before anything is written.

**Where it stands.** Done. Create DLC pack… is a registry command (`src/ui/commands/files.rs`)
that the menu of a folder on disk offers, in Files and in the Files view; the palette and a key
run it on the focused panel's selection. Its question asks what `openrage-cli dlc create` asks,
with the command's defaults: a name (none), the type in words (Compatibility pack, Level pack,
Title update or Paid pack), the order and the minor order (255 and 0) and the encryption (open).
Before anything is written it shows each file that goes into `dlc.rpf` with its `fileType`, what
the command refuses, in the writer's words, the device the files register against and the
`dlclist.xml` line, and Create… waits until nothing is refused. Create… asks for a folder with
a dialog of its own, Create DLC Pack Into Folder (`pick_dlc_pack_destination`, [R59u](#r59u-small-faults-in-the-editor-and-the-previews)),
refuses one inside a game install, its `mods` overlay included, and one
already holding the pack, and asks the edit gate. The build is a job in the jobs list with its
progress in bytes and a Cancel that stops at its next write and removes the archive it began
(`src/state/dlc_pack.rs`, `build_pack_with`); the app and the command share
`dlc_pack_of_folder`, `dlc_refusals` and `create_dlc_pack`. A built pack's window shows the line,
with Copy line and Open folder. `a_folder_packs_from_its_menu_into_the_pack_the_command_builds_of_it`
(`src/state/building_a_dlc_pack_from_the_app.rs`) packs a staged folder from its row's menu and
finds the archive byte-identical to the command's; it failed before the menu offered the command.
Seen on `just shots` screens, not in a window.

**Done when.** A pack built from the app matches the one the CLI builds from the same folder.

### R52 Mod manager and load order

[↑ checklist](#phase-8--projects-packaging-and-mod-management)

**What.** Installed mods with status, classified by kind (loose files, DLC pack, package, ASI, ASI
loader, mod loader) and readiness, ordered, and built into the overlay. The list is a panel over
[R50c](#r50c-the-command-engine-the-journal-and-the-install)'s journal and
[R49a](#r49a-overlay-first-resolution)'s resolution; the classifier is new, and what a modded install
holds needs `openrage-cli formats` run over one. **Done when.** Reordering two conflicting mods
visibly changes the winner, shown as the VFS resolves it (`vfs_lookup_walks_devices_newest_first`).
**Covers.** U-160, U-161, U-162, U-163.

**Where it stands.** View ▸ Panels ▸ Mods lists the active install's mods in the load order saved
for it. A mod is one folder directly under `mods` (enabled) or `mods_` (parked, so disabled), holding
the paths it writes as the install lays them out. `or_title::mods::scan_install` classifies each as
loose files, or a DLC pack when a `dlc.rpf` or `dlc1.rpf` sits under `dlcpacks/<name>` in it, with
whether the install's `dlclist.xml` names the pack (`read_dlclist`); in the game folder it finds an
ASI plugin, the ASI loader `dinput8.dll` and a script mod, a child of `scripts`. Two enabled mods
writing one path are one conflict per set of mods: a file, a file inside an archive laid out as a
folder named for it, or the same file inside two DLC packs' archives, bar each pack's own
`setup2.xml` and `content.xml`. `resolve` mounts each enabled mod as a device on one VFS mount in
load order, so the winner is what `Vfs::locate` answers, the last mod
(`reordering_two_mods_that_write_the_same_archive_entry_flips_which_one_wins`); a mod the order has
not met stacks last, and an archive that will not open is listed beside the conflicts, since an
empty list would otherwise claim there are none. The arrows move a mod past the next enabled one and
the order is saved per install in the preferences
(`reordering_two_conflicting_mods_changes_the_winner_and_the_order_survives_a_restart`). Disable and
Enable move the mod's folder between `mods` and `mods_` once the edit gate permits and the user
answers, and nothing is deleted
(`a_mod_moves_to_mods_underscore_only_when_the_mode_permits_and_the_user_confirms`). Measured at
`b63937f6` over the owner's Legacy install with `openrage-cli formats` (236,120 files of ten
extensions, none an `.asi`) and the scan: no `dinput8.dll`, `scripts` or `mods`, and a parked
`mods_` of nine mods holding 71 files, among them two `.rpf` that no DLC pack holds; the scan and
resolve took 9 ms. Not built: writing the order into the overlay, so the game reads it, which is
why Enabled is a place in the list; packages, which wait on [R50](#r50-package-installation) and
its journal; the stale and unreadable statuses and the mod-loader kind, which no longer wait on
[R49c](#r49c-staleness-after-a-title-update) and [R49d](#r49d-the-game-side-loader), both landed,
and are not listed; a DLC pack installed under `update/x64/dlcpacks`, which nothing tells from the
game's own; toggling an ASI plugin or a script, which renames a file in the install.

### R53 GTA V the reference title

[↑ checklist](#phase-9--titles)

Done. The GTA V and RDR2 profiles are in `crates/title/or-title/src/game/profiles.rs`, and
`crates/title/or-title/src/game/registry.rs` asks them in the order Enhanced, Legacy, RDR2 because
an Enhanced install ships `GTA5.exe`;
`crates/title/or-title/tests/adding_a_title.rs` holds it
([Crate_Reference.md](../reference/Crate_Reference.md#adding-a-title)). It stays as the title every other is
measured against. **Covers.** U-010, U-011.

### R54 GTA V Enhanced and the portable open reader writer

[↑ checklist](#phase-9--titles)

**What.** Enhanced parity ([R54a](#r54a-enhanced-parity), done) and a portable, open, native reader
and writer for Enhanced files, where PyMateria is Windows-only and licence-locked, CodeWalker's Gen9
path is unlicensed C# and szio is Python. Since D1 that is the public `or-io` crate
([OR_IO_Specification.md](OR_IO_Specification.md)): it declares GTA V's structures, opens each
format as an owned value for either target and carries the caller-side codecs
(`crates/io/or-io/src/gta5/drawable_codecs.rs`). Nothing loads a runtime pack (OD-OTHER-TITLES),
the `.orpack` export is deleted (owner, 2026-10-01), and [R66h](#r66h-publish-the-crate-and-the-editor-together) publishes the
crate. Left here: [R54b](#r54b-the-crates-as-a-published-sdk), with
[R54c](#r54c-the-orpack-format-and-its-compatibility-rule) and
[R54d](#r54d-a-pack-authored-from-outside-this-project) closed as obsolete; [R58](#r58-cross-platform) is the
cross-platform half.

### R54a Enhanced parity

[↑ checklist](#phase-9--titles)

Done on [R66d](#r66d-declare-gta-v-format-by-format)'s owned values. Every format `FORMATS` lists
(`crates/io/or-io/src/gta5/formats.rs`) opens and saves as an owned value for each target it ships,
held per format by the gates `declared_gates!` writes (`crates/io/or-io-verify/src/gates.rs`,
`owned_saves_as_tree` among them), and the app's Gen9 handlers in `src/state/payload_handlers.rs`
hold the Gen9 variants of `or_io::Asset`. A target parameter, not a second reader, closed the last
Gen9 read rows; the four types Gen9 changed keep a type per target.

The app's mount (`build_mounted_game_with`) layers `update/update2.rpf` after `update.rpf` for both
editions: its `common/` half on `common:/`, its `x64/` half on `platform:/`, and the whole archive on
`update2:/`, through `GameProfile::update2_archive`
(`an_enhanced_mount_layers_update2_over_the_base_archives`,
`a_legacy_mount_layers_update2_over_the_base_archives`,
`a_retail_enhanced_install_resolves_what_only_update2_holds`,
`a_retail_legacy_install_resolves_what_only_update2_holds`). Measured on a retail Enhanced
install, executable 1.0.1158.16: `update2.rpf` holds 1,104 files, 1,088 under `common/` and 16 under
`x64/`, and shares no logical path with `update.rpf` on either half, so the order between the two
decides no read. It holds 17 paths a base archive also holds, 2 of `common.rpf`'s
(`data/version.txt`, `data/gta5_cache_y.dat`) and 15 of `x64a.rpf`'s and `x64b.rpf`'s (twelve
`data/lang/*_rel.rpf`, `levels/gta5/script/script_rel.rpf`, `data/credits.ymt`,
`textures/credits.ytd`), and 1,087 logical paths no other mounted archive holds, among them the
shader containers. Its copies are the newer: its `version.txt` names 1158.16, the executable's
version, where `common.rpf`'s names 108, and its `gta5_cache_y.dat` dates its files from 2026 where
`common.rpf`'s dates them from 2014. No archive carries a mount list: `update2.rpf` has no
`setup2.xml` or `content.xml`, and a text search of the install's 3,254 `.xml`, `.meta`, `.txt` and
`.dat` files finds the name only in the loose `checksums_tu.txt`. The executable names the device
`update2:/` beside `update:/`, and the paths `update2:/common` beside `update:/common` and
`update2:/x64/data/lang/` listed after the same directory on `platform:/` and `update:/x64/`.

Measured on a retail Legacy install, executable 1.0.3889.0, by `openrage-cli rpf list` over
`update2.rpf`, `update.rpf`, `common.rpf` and the 23 platform archives: `update2.rpf` holds 2,211
files, 2,196 under `common/` and 15 under `x64/`. It shares one logical path with `update.rpf`,
`x64/data/credits.ymt`, and the two copies are byte-identical (823,972 bytes), so the order between
the two decides no read. It holds 339 paths a base archive also holds: 326 of `common.rpf`'s (321
under `shaders/win32_40_final/`, 3 under `shaders/db/`, `data/version.txt`,
`data/gta5_cache_y.dat`), 317 of which differ in stored size, and 13 of `x64a.rpf`'s and
`x64b.rpf`'s (twelve `data/lang/*_rel.rpf`, each about 1.4 times the size of `x64b.rpf`'s, and
`data/credits.ymt`). The other 1,872 logical paths no mounted archive holds, among them the
install's only `levels/gta5/script/script_rel.rpf`, `data/lang/chinesesimp_rel.rpf` and 1,042
`.ugc` files. Its copies are the newer: its `version.txt` names 3889.0, the executable's version,
where `common.rpf`'s names 108, and its `gta5_cache_y.dat` dates its 1,123 files from 2025-05-28
to 2026-07-08 where `common.rpf`'s dates its 1,124 at 2015-02-10. It carries no mount list: it
holds only `.fxc`, `.ugc`, `.sps` and `.rpf` files, one `preload.list`, `version.txt` and
`gta5_cache_y.dat`. `GTA5.exe` names the same device and paths as Enhanced's executable, with
`update2:/x64/data/lang/` after `platform:/data/lang/` and `update:/x64/data/lang/`, and
`update2:/common/shaders`.

### R54b The crates as a published SDK

[↑ checklist](#phase-9--titles)

**What.** The crates a third party depends on, with a stability promise. **Where it stands.**
Decided by D2 ([OR_IO_Specification.md](OR_IO_Specification.md#owner-decisions)): `or-io` and
`or-io-macros` are published with the editor at R66h, under the 0.x policy the specification states,
and the workspace `Cargo.toml` sets `publish = false` until then. `or-asset` is deleted
([R66f](#r66f-delete-the-old-crates), #332) and publishes nothing. **Done when.** R66h publishes
both.

### R54c The orpack format and its compatibility rule

[↑ checklist](#phase-9--titles)

**Closed: obsolete** by the owner on 2026-10-01, in his words "pack formats have been deleted
purposely", and the leftover `.orpack` export goes
([recorded](Project_Context.md#openrage-packs-is-deleted-and-the-orpack-export-goes-2026-10-01)).
What the row asked for follows.

**What.** `.orpack` was an eight-byte magic and a `bincode` encoding of one struct, written by
`Pack::to_bytes` and read by `Pack::from_bytes` behind the `bincode` feature, so only Rust could
author or read one, and no command wrote one. `PACK_VERSION` was 12 and stayed 12 (OD-PACKVER):
vocabulary added since went into the built-in descriptions only, and any other version was refused.
The bump rule existed — append a variant, never insert one, and take the bump with its first use —
but no compatibility rule: every bump invalidated every pack. The export, its feature and
`PACK_VERSION` are deleted (#395). **Done when.** The encoding is specified for a non-Rust decoder, or authoring is
documented as Rust-only against the published `or-io`, and the version rule says what becomes of a
pack one version behind.

### R54d A pack authored from outside this project

[↑ checklist](#phase-9--titles)

**Closed: obsolete by OD-OTHER-TITLES.** The row asked for an `.orpack` for a title `or-io` does
not declare, built by someone outside this project and loaded by OpenRage. Under
[OD-OTHER-TITLES](Project_Context.md#no-runtime-pack-for-another-title-2026-09-26) nothing in
OpenRage loads a runtime pack for any title: a new title gets its generic tree from its own
declarations, and no `.orpack` export is left to carry a description outside Rust
([R54c](#r54c-the-orpack-format-and-its-compatibility-rule)). The pack sources in `openrage-packs`
stopped building against this checkout when three of their path dependencies, `or-format-api`,
`or-macros` and `or-asset`, were deleted (#335, #339, #332), and the owner archived and deleted the
repository on 2026-10-01 ([R66i](#r66i-retire-openrage-packs)).

### R55 Retargeting between Gen8 and Gen9

[↑ checklist](#phase-9--titles)

**What.** Conversion both ways as a value-tree operation — read with one target's description,
write with the other's — plus a batch job (folders, progress, a per-file report, archives refused)
and a `{format} × {target}` export matrix, for `.ytd`, `.ydr`, `.ydd` and `.yft`. `.ybn`, `.yld` and
`.ycd` have one layout, and `.ypt` is out: converting one rebuilds its Gen9 drawable and texture
dictionaries, the translation layer this row avoids.

**Where it stands.** A drawable, a texture dictionary, a drawable dictionary and a fragment convert
both ways, one at a time through the library or a folder at a time through the CLI.
`crates/io/or-io/src/gta5/retarget.rs` turns a `GtaDrawable<Gen8>` into a `GtaDrawable<Gen9>` and
back (`to_gen9`, `to_gen8`), typed over the owned values rather than through the value tree: the
spine is one generic declaration (OD-10.8) and is carried as it is, and the four types Gen9 changed,
`GrcVertexBufferG9`, `GrcIndexBufferG9`, `GrcTextureG9` and `GrcInstanceDataG9`, with
`GrcTextureReferenceG9`, are rebuilt as the other target's. A Legacy material addresses a parameter
by name and an Enhanced one by semantic, and each is respelled through
[R40d](#r40d-the-gen8-gen9-parameter-remap)'s `ParamRemap`. Each call returns a `LossReport` naming
every value the other target has no field for, which is dropped and never guessed: a parameter the
database does not spell, a sampler state, the `.sps` hash, the entries' registers, a vertex channel
with no slot on the other side, and the texture and buffer words only one target states. A word
only the target states (bind flags, views, usage data, the preset tag, the per-thread copies) stays
at its declaration's default, and a tiled or non-2D texture refuses the drawable. ⚠ An Enhanced
material comes out in OpenRage's layout, textures in entry order and constants as whole registers
of one buffer, because a Legacy entry states no Enhanced placement. The shader's own layout is
what [R40e](#r40e-the-enhanced-shader-container)'s container reader states (`SgaVariable`'s
register or buffer offset), and `to_gen9` does not place by it. Over the staged corpus, with the remap built from
its own `.fxc`, 13 of 13 Legacy `.ydr` convert to Gen9 and back and 5 of 5 Enhanced ones the other
way, saving and reopening owned at each step, with the same geometry and parameter values and every
parameter the trip loses named
(`every_staged_legacy_drawable_converts_to_enhanced_and_back_with_its_geometry_and_values` and
`every_staged_enhanced_drawable_converts_to_legacy_and_back_with_its_geometry_and_values`, in
`crates/io/or-io-verify/tests/retarget.rs`). The other three formats go through the same leaves:
`texture_dictionary_to_gen9` and `texture_dictionary_to_gen8` rebuild each texture,
`drawable_dictionary_to_gen9` and `drawable_dictionary_to_gen8` each drawable under its hash, and
`fragment_to_gen9` and `fragment_to_gen8` each drawable a fragment holds (common, extra, cloth,
environment cloth and each physics child's), carrying the physics, glass, lights and cloth as they
are because neither target lays them out differently. The report names each loss under the drawable
it came from (drawable 0x…/shader 0, or physics level 0, child 3/level0/model0/geometry0), and a
texture the other target cannot describe refuses the whole file under the same name
(`a_texture_the_other_target_cannot_describe_refuses_the_whole_file_by_name`). In the same suite,
16 of 16 Legacy `.ytd` (290 textures) and 2 of 2 Enhanced ones (26) come back with the same pixels,
format and size; 48 of 48 Legacy `.ydd` (339 drawables, 4467 parameter values) and 2 of 2 Enhanced
ones (2 drawables, 32 values); and 63 of 63 Legacy `.yft` (897 drawables, 4652 values) and 5 of 5
Enhanced ones (81 drawables, 111 values), each with the same geometry and values, reopening owned
at every step. No staged file refuses; the one staged `.ytd` at RSC7 version 2 is a layout OpenRage
does not declare. The batch half is `openrage-cli convert retarget <dir> --out <dir>`
(`crates/tools/or-cli/src/retarget.rs`), a job through `or_job` and
[R21](#r21-extraction-and-bulk-jobs)'s `crates/tools/or-cli/src/jobs.rs`, as `bulk` is. It
converts every `.ydr`, `.ytd`, `.ydd` and `.yft` under the folder to the other target by each
file's RSC7 version, writes each at its relative path under `--out`, and refuses an archive and a
version neither target declares; the parameters are respelled through the database `--shader-defs`
or `OPENRAGE_SHADER_DEFS` names, and with none every one is dropped and named. One
`retarget_report.json` names, per file, each value dropped or why the file was refused, and Ctrl-C
stops between files with the report written so far
(`a_stopped_run_writes_nothing_but_its_report`). Over a staged folder of the first two files of
each format in each target's corpus directory and an archive, 15 of the 16 files convert and
reopen owned as the other target, each with the drops the report names equal to the retarget's
own, and the `.ytd` at version 2 and the archive are refused
(`a_folder_of_every_format_converts_to_the_other_target_with_one_report`, in
`crates/tools/or-cli/tests/a_folder_retargets_unattended.rs`). No `openrage-cli serve` route
reaches it.
**Depends on.** R40d, and nothing else. **Done when.** A Gen8 drawable converts to Gen9 and back
with a report of what could not be carried, and a folder converts unattended. **Risk.** A `.sps`
name, sampler state, sRGB variants and tile modes cannot be recovered; how often that bites needs an
install (`openrage-cli formats`, `channels`, `versions`). **Covers.** U-020, U-186.

### R55b Convert from the app

[↑ checklist](#phase-9--titles)

**What.** Convert to Enhanced or to Legacy as a command on a file or a selection, with the loss
report shown before anything is written. Install-wide batches stay in `openrage-cli`
([R21](#r21-extraction-and-bulk-jobs)).

**Where it stands.** Done. Convert is a registry command (`src/ui/commands/files.rs`) that a row's
menu in Files and in the Files view offers on a `.ydr`, `.ytd`, `.ydd` or `.yft`, and a
selection's menu when every row is one; the palette and a key act on the focused panel's
selection, and on the open document where it has none ([R55d](#r55d-what-converting-still-gets-wrong-after-r55c)).
Its label, Convert to Enhanced… or Convert to Legacy…, follows each file's own version.
The conversion is a background job with its progress and a Cancel that stops between files
(`src/state/convert.rs`), through `or_cli::retarget_file`, the call
`openrage-cli convert retarget` makes, respelling parameters through the shader database the app
loaded. Before anything is written a question lists each value the other edition has no field
for, in words and from the same values the command reports, or why a file is refused, and says
nothing is invented. Its Convert… asks where with a Convert dialog for one file and Convert Into
Folder for several, each under its own name, and a folder that already holds some of them asks
Replace, Skip or Keep both. The edit gate is asked at the write, and where it would ask about the
game as shipped the write is refused instead, as Export as…'s is.
`a_drawable_converts_from_its_menu_and_its_report_equals_the_clis`
(`src/state/converting_from_the_app.rs`) converts a staged Legacy and an Enhanced `.ydr` from the
row's menu and finds the question's lines the words of the values the command reports, and the
bytes written equal to the command's, for the same database; it failed before the menus offered
Convert, and fails when the app converts without its database. Seen on `just shots` screens, not in
a window. What it left was [R55c](#r55c-converting-in-a-modders-words)'s, and that row is done.

**Done when.** A `.ydr` converts from its context menu, and its report holds the CLI's values.

### R55c Converting in a modder's words

[↑ checklist](#phase-9--titles)

**What.** The conversion's question names each dropped value in words; its job and its dialogs say
Convert; a selection converted into a folder that holds one of its names offers Replace, Skip and
Keep both; and Convert is offered on a document's tab.

**Where it stands.** Done. The question gives each value a conversion drops one line in words, such
as "Shader 0: parameter 0x1f8a49b7, not in the shader database for Enhanced" or "Geometry 0 of model
0 at high detail: the encodings of the channels it does not carry", under the drawable of a
dictionary or a fragment where there is one, and never a declaration's path
(`a_drawable_converts_with_each_value_it_drops_in_words_and_no_engine_path`, and
`every_value_the_staged_files_drop_reads_in_words_with_no_engine_path` over every staged file of
both targets, with and without a shader database). `LossReport` holds typed values, `Dropped` of a
`Place` and a `Lost`, in `crates/io/or-io/src/gta5/retarget.rs`. Their `Display` is what
`openrage-cli convert retarget` prints and writes to its report, unchanged
(`a_dropped_value_prints_as_the_command_reports_it`; the report text of all 155 staged `.ydr`,
`.ytd`, `.ydd` and `.yft` was byte-identical before and after, with and without a shader database,
by a scratch run that is not kept). `src/state/convert_words.rs` words the same values by matching
every variant, so a new kind of loss does not compile until it has words. The write of converted
files is the conversion's own job in the list, "Converting…" with Cancel Conversion, and its
dialogs are "Convert" with the file's name and "Convert Into Folder"; it still runs on the
extraction's machinery, which now carries the kind it is listed as
(`cancelling_a_conversions_write_stops_it_and_never_stops_an_extraction`). A folder that holds some
of the files asks the question an extraction asks, Replace, Skip or Keep both once for the job,
instead of refusing
(`a_selection_converted_into_a_folder_holding_one_of_its_names_offers_replace_skip_and_keep_both`,
`replace_writes_over_the_file_already_there_and_skip_leaves_it_as_it_is`); one file's own dialog
still asks about replacing it. A document's tab offers Convert to the other edition, and its
question says it converts the open document as it stands, unsaved edits included, as a glTF export
does (`a_document_converts_from_its_tab_as_it_stands_its_unsaved_edits_included`). The Done-when's
three tests failed before the fix, and the screens are `just shots` ones, not seen in a window.
Left: a shader is "Shader 0", as the Outline numbers it, and a parameter is its hash, since the
database does not spell the ones listed. The palette and the key converting the selected rows only,
and the glTF export's write being listed as "Extracting", were left here and are
[R55d](#r55d-what-converting-still-gets-wrong-after-r55c)'s, and that row is done.

**Done when.** A test converts a `.ydr` and finds no engine path in the question; a test converts a
selection into a folder that holds one of its names and is offered Replace, Skip and Keep both; and
a test converts from a tab.

### R55d What converting still gets wrong after R55c

[↑ checklist](#phase-9--titles)

**What.** The jobs list names a glTF export's write and Export as…'s write for what they are, the
palette and its key convert the open document as the tab's menu does, and the conversion's question
wraps and selects line by line.

**Where it stands.** Done. A glTF export's write (`src/state/gltf.rs`) and Export as… in the Files
view (`src/state/files_view/mod.rs`) start their write as `JobKind::Export`, which the jobs list
names for what it is: "Exporting prop.glb" with Cancel Export, and under Finished "Export", where
the write used to replace an extraction's line. Its Cancel reaches the extraction machinery as a
conversion's does, so Cancel Extraction leaves it running and Cancel Export stops it between files.
The Convert command's `focused` target is `target::selection_or_document`, as Reveal's and Export
to glTF…'s are, so the palette and a key convert the open document, as its tab's menu does,
wherever the focused panel has no selection to give; selected rows still win. With a folder open
and its centre view focused the reason is still "Select the files to convert first", and where
`target::selection` finds nothing at all it is now the folder's own, "Only a drawable ... converts".
The question's report is a label for each line (`Converted::report` returns a `FileReport` for each
file), each wrapped at the panel's edge and selectable on its own, a value hanging under an
indent so that a wrapped one is not read as a line of its own. The panel is `QUESTION_WIDTH` wide,
fixed before its heading so that a window narrower than that narrows it (`question_within`, in
`src/ui/panels/mod.rs`). The guess that a long texture name widened the question was wrong: the one
label already wrapped at 760 points. What failed was that the rows after a wrap began under the
file's line, and that a window narrower than 760 clipped the question at both edges.
`src/state/what_converting_still_gets_wrong_after_r55c.rs` holds each. Five tests failed before the
fix: `a_gltf_exports_write_is_listed_as_an_export_and_cancel_export_reaches_it` and
`an_export_as_write_is_listed_as_an_export_and_cancel_export_reaches_it` (both listed as
"Extracting"), `the_palette_and_its_key_convert_the_open_document_with_no_row_selected` (the
command was disabled), `a_texture_with_a_long_name_wraps_within_the_question_under_its_own_line`
(a wrapped row began under the file's line, and the narrow window's question ran past it until the
width was set before the heading) and `each_line_of_the_question_selects_on_its_own` (a drag across
a line selected nothing under the app's own style).
`cancelling_an_exports_write_stops_it_and_never_stops_an_extraction` failed with the Cancel's route
taken out, and `a_folder_open_with_nothing_selected_still_says_to_select_the_files` guards the
reason above and passed from the start.
Seen on `just shots` screens at 1600 and 640 points wide, not in a window. Left: the toasts
"An extraction is already running" and "An extraction is running; write once it is done"
(`src/state/extract_job.rs`, `src/state/files_view/mod.rs`, `src/state/convert.rs` and
`src/state/gltf.rs`) still say so while an export's or a conversion's write holds the one slot;
`every_kind_of_job_is_listed_while_it_runs_and_stops_on_a_cancel_from_the_list` cannot hold Export
beside Extraction, since the machinery runs one job at a time; the Convert command's hover text
still says a whole folder converts with `openrage-cli convert retarget`, which names a command and
not a flag, and R59t's test for flags in the app's words passes it; and `ConvertClash` still
repeats four fields of `ExtractClash` and shares only the question's text, because one struct would
change the struct literals of three other files (`src/self_check.rs`,
`src/state/settings_recents_and_small_editor_fixes.rs` and
`src/state/search_results_that_open.rs`).

**Done when.** A test lists a glTF export and an Export as… write in the jobs list and finds each
named for what it is, and its Cancel stopping it; a test converts the open document from the palette
with no row selected; and a test converts a file with a long texture name and finds the question
wrapped within its panel.

### R55e What converting still gets wrong after R55d

[↑ checklist](#phase-9--titles)

**What.** A write that holds the extraction's one slot says what it is, every question the app asks
fits a narrow window, and the Convert command says why it is off in words that say what to do.

**Where it stands.** Found while closing [R55d](#r55d-what-converting-still-gets-wrong-after-r55c)
(#573): the toasts "An extraction is already running" and "An extraction is running; write once it
is done" (four sites, in `src/state/extract_job.rs`, `src/state/files_view/mod.rs`,
`src/state/convert.rs` and `src/state/gltf.rs`) still call a running export's or conversion's write
an extraction, so the jobs list says Exporting while a second start toasts an extraction; the other
questions use `set_min_width`, the glTF export's and import's 640 points, the clash's 420 and the
delete's 380, so a window narrower than that clips them at both edges as the conversion's was, and
`question_within` would fix each; where Files is focused and nothing is selected the Convert
command's reason, once "Select a file or folder in Files first", is the document's own, "Only a
drawable, a texture dictionary, a drawable dictionary or a fragment converts", which does not say to
select a file; its hover still names `openrage-cli convert retarget` as the way to convert a folder,
a command and not a flag, which R59t's test for flags in the app's words passes;
`every_kind_of_job_is_listed_while_it_runs_and_stops_on_a_cancel_from_the_list` has no
exhaustiveness guard, so Export, the seventeenth kind now that the game shader bake is one, is left out of its `ALL` of sixteen, as the
row's Left admits it must be, and its "16 jobs running" string names a coverage it lacks; the cancel
assertion of `an_export_as_write_is_listed_as_an_export_and_cancel_export_reaches_it` is
`.filter(doing == doing).all(stopping)`, which passes on an empty set if the job leaves the list,
though the find at its top shows the job was there; and `ConvertClash` repeats four fields of
`ExtractClash` and shares only the question's text, one struct changing the struct literals of
`src/self_check.rs`, `src/state/settings_recents_and_small_editor_fixes.rs` and
`src/state/search_results_that_open.rs`. The question's per-frame cost over a large selection is
[R59f](#r59f-performance-and-cache-tuning)'s.

**Done when.** A test starts a conversion's write while an export's runs and finds the toast naming
the export; a test draws the glTF export's, the import's, the clash's and the delete's questions in
a window narrower than each and finds each within it; a test presses Convert with Files focused and
nothing selected and finds the reason telling the user to select a file; and the jobs list's test
fails when a kind is added to `JobKind` and left out of its `ALL`.

**Closed.** A second write now names what holds the slot as the jobs list does, "An export",
"A conversion" or "An extraction" (`a_second_write_names_the_job_that_holds_the_slot_as_the_list_does`);
the glTF export's and import's, the clash's and the delete's questions fit a window narrower than
they are, by `question_within` and a wrapping container choice
(`every_question_fits_a_window_narrower_than_it_is`, seen failing at the export's); the jobs list's
test is held to every kind of job by a `held` with no wildcard, which stops compiling when a kind is
added until it is placed (`the_kinds_the_list_test_runs_are_those_no_other_test_holds`); and the
cancel assertion of the export-as test no longer passes on a job that left the list; the Convert
command with Files focused and nothing selected, on an open folder or archive, says "Select a file
or folder in Files first" (`convert_with_files_focused_and_nothing_chosen_says_to_choose_in_files`,
seen failing on the document's own reason); and a conversion written under an export's or an
extraction's slot names it and keeps its question
(`a_conversion_written_while_another_job_holds_the_slot_names_it_and_keeps_the_question`, seen
failing with the guard removed). Left as noticed: `ConvertClash` still repeats four fields of
`ExtractClash`; one struct would change the literals of four files for no behaviour.
### R56 RDR2 the second container

[↑ checklist](#phase-9--titles)

**What.** Mount an RPF8 archive and extract a file byte-identically, then RSC8, then RDR2's
structures declared in `or-io` as GTA V's are, which gives the title its generic tree
(OD-OTHER-TITLES); decompose it after that first experiment. The profile exists, and
`is_supported()` is true now that `ContainerFamily::Rpf8` opens archives, though no resource decodes
(`rdr2_is_browsable_but_extracts_no_gta_keys`).

**Where it stands.** The archive reads and writes
([recorded](Project_Context.md#every-rpf-and-img-generation-reads-and-writes-in-or-io-2026-10-07)),
so the first half of *Done when* holds for the archives that can be opened. On the owner's install
of 2026-10-07 one of twenty-four top-level archives has its table in the clear, `shaders_x64.rpf`,
and it lists its four files by the names its own blob spells, extracts each at the size its row
states, and a rebuild around its bodies as stored writes a file of the same size whose rows equal
the shipped ones but for where each body lies
(`an_archive_with_a_plain_table_opens_and_every_other_is_refused_by_name`,
`a_names_blob_spells_exactly_the_hashes_its_rows_carry` and
`a_plain_archive_rebuilt_around_its_stored_bodies_reads_back_the_same` in
`crates/io/or-io-verify/tests/rpf8.rs`). The other twenty-three tag their table with a key id, which
`RpfEncryption::Tagged` refuses by name until a provider holds the title's cipher. `Rdr2CryptoProvider`
holds it: the AES key every Rockstar executable carries, found in the install's `RDR2.exe`, unlocks
a snapshot of the cipher's tables (the user's own, `OPENRAGE_RDR2_SNAPSHOT`, else the one `or-io`
bundles by the owner's ruling of 2026-10-08), and all twenty-three tables then open with every row
inside its file, a nested archive opens, and an enciphered body reads as its metadata
(`every_tagged_archive_opens_and_its_rows_land_inside_the_file` and
`an_enciphered_body_in_a_nested_archive_deciphers_to_readable_metadata` in
`crates/io/or-io-verify/tests/rdr2_keys.rs`). The names blob of a tagged table is not under that
cipher and stays as stored, so its files carry hash names, and nothing encrypts a table or a body:
the cipher in the snapshot only deciphers, so an edited archive is saved with a clear table (key id
`0xFF`, which the format has for it), each body it carries keeps its own key, and what is added is
stored clear (`an_edited_archive_reads_back_file_for_file`). A compressed body is ciphered over its
stream, which the row does not state: it is the stored size or 256 bytes less, and `RpfArchive`
tries both and keeps the one that inflates, which reads every one of 21,312 sampled bodies
(`every_enciphered_oodle_body_of_a_patch_archive_inflates`). Three nested archives of `update_2.rpf`
and `update_4.rpf` name key ids `0xC5`, `0xC6` and `0xD4`, which the snapshot lacks (it was taken
from an older build); they stay closed, and the provider reports the ids it was asked for
(`RpfCryptoProvider::missing_keys`), which the Games page shows. `RDR2.exe` does not embed the
snapshot: of its 164 key sets one is found verbatim, and none of the 17 round tables, so there is
nothing to extract from it. An Oodle body decodes on every platform: LZB16 (stream type 2) is OpenRage's own, measured
against the output of the install's `oo2core` and matching it on 361 of 361 small bodies and on
large synthetic streams, and writing uses it too, so a file added to an RPF8 archive is compressed
with it and the game's own library reads it back
(`an_oodle_body_decodes_to_the_size_its_row_states` and `what_is_encoded_decodes`). Kraken (type 6)
is the pure-Rust `oozextract`, vendored in `crates/io/or-kraken/oozextract` with one fix (a block
that holds a single symbol returned its start, not its size), which `or-kraken` runs one 256 KiB
quantum at a time because the port copies a quantum's first eight literals only at the start of a
stream: 257 of 258 sampled bodies then match `oo2core`, the 17 larger than a quantum among them, and
nothing here writes Kraken.
`Rsc8` (`crates/io/or-io/src/container/rpf/rsc8.rs`) opens a resource as the virtual and physical
bytes its flags state: all 2908 resources sampled in three archives open
(`a_resource_opens_as_the_bytes_its_flags_state`). The `rdr2` module declares its structures as
`#[record]`s of its own, the first being the texture dictionary
(`crates/io/or-io/src/rdr2/texture.rs`): 467 dictionaries of the sampled archives read, and all
3134 textures in them are listed under the hash of their name and decode to a picture
(`a_texture_dictionary_lists_the_textures_it_holds_and_they_decode`), every one at the tile mode
`Standard` that a PC texture stores row by row. Nothing writes an RSC8 file yet. The app browses an RDR2
install as a folder of its archives with the install's cipher, and no resource opens as a document there. The RSA block a file writes is
zeros, and whether the game accepts an archive whose block is not Rockstar's is unmeasured.
[Crate_Reference.md](../reference/Crate_Reference.md#the-container-seam) prices each seam, and OD-AUDIT-KEEPS
keeps `PagingModel`, `ResourceEnvelope` and every `GameProfile` method for this row. The resource
envelope is a trait with `Rsc7` its one implementation, and paging reads and writes at foreign bases
(`crates/io/or-io/tests/a_second_paging_model_on_the_write_half.rs`,
`crates/io/or-io/tests/a_paging_base_is_named_and_never_spelled.rs`). `ResourceType`
(`crates/io/or-io/src/container/resource/versions_registry.rs`) is a closed enum of GTA V's resource
types. **Depends on.** The formats inside an RSC8 resource.
**Done when.** An archive lists and extracts byte-identically; only then does a resource decode.

**Risk.** Every estimate made here before a format was measured was low; do not estimate this one.
For a machine, in order: is an install reachable (it is); does RSC8 keep the two paging bases and
the virtual-then-physical payload (it does); and what an install holds
(`openrage-cli formats`, `archives`).

### R57 GTA VI readiness

[↑ checklist](#phase-9--titles)

**What.** No format speculation; three properties kept true and re-checked when a layer changes:

| Property | What holds it |
|---|---|
| A title is a profile and a registry line | `crates/title/or-title/tests/adding_a_title.rs`, with `a_title_on_gta_vs_target_reads_gta_vs_declarations_with_no_pack` |
| A target is an open identifier | `an_unregistered_target_labels_itself_rather_than_borrowing_legacy` |
| A container sits behind seams | the paging model, `crates/io/or-io/tests/a_paging_base_is_named_and_never_spelled.rs`, and the resource envelope, `crates/io/or-io/tests/what_still_names_the_envelope.rs` |

**Where it stands.** The archive is an `RpfVersion` and a layout module, so a new generation edits
`or-io` alone; `ContainerFamily` and `ResourceType` are closed enums, so a third title with its own
container edits both. **Done when.** Not applicable: a property to preserve, and a row whose test
goes is a property nothing holds. **Risk.** Claiming "GTA VI support" before a file exists.

### R57b Earlier RAGE titles

[↑ checklist](#phase-9--titles)

**What.** GTA IV, EFLC, Max Payne 3 and RDR1, which the owner made future goals rather than
non-goals on 2026-10-01
([recorded](Project_Context.md#earlier-rage-titles-are-a-future-goal-2026-10-01)): their archive
containers, RPF2, RPF3, RPF4 and RPF6, then what those archives hold.

**Where it stands.** The containers read and write in `or-io` (RPF2, RPF3, RPF4 and RPF6, with RPF0
and the images IMG1 to IMG3, [recorded](Project_Context.md#every-rpf-and-img-generation-reads-and-writes-in-or-io-2026-10-07)),
checked against archives assembled by hand from their published layouts, since no retail file of
any is here (`crates/io/or-io/tests/every_archive_version_round_trips.rs`). Nothing above them is
done: no profile, no install discovery, and nothing reads what the archives hold.

**Done when.** A first of these titles reads its archives.

### R58 Cross platform

[↑ checklist](#phase-10--the-application)

**What.** Linux and macOS supported in full, to the standard Windows has: the checks, the app,
install discovery, the platform features and a package of each. Decided by the owner on
2026-10-06, every tier and macOS unsigned for now
([recorded](Project_Context.md#linux-and-macos-are-supported-in-full-and-the-tooling-is-rust-that-runs-the-same-on-every-os-2026-10-06)).

**Where it stands.** Hosted Linux CI builds and tests the app crate on every non-draft pull request
(#170), and the binary reaches its game picker on a Linux VM (`49aa018e`). Off Windows, install
discovery (`crates/title/or-title/src/install/discovery.rs`, which `src/state/game_discovery.rs`
runs off the UI thread) finds Steam under `~/.local/share/Steam`, `~/.steam/steam` and the Flatpak
root, through each root's `libraryfolders.vdf`, so a Steam install, Proton's included, is found
without typing a path; a synthetic root under a temp directory checks it on every platform
(`a_linux_steam_root_and_its_libraries_are_found_under_home`). Started on Linux without a library
winit or wgpu loads at run time (`libxkbcommon`, `libwayland-client`, the X11 libraries, the Vulkan
loader), the app exits naming each one and its Debian and Fedora package rather than panicking
(`src/platform/runtime_libraries.rs`). macOS compiled and ran on 2026-10-06, on aarch64 at
`1427e655`: the workspace builds and passes clippy with warnings denied, the `or-io`, `or-io-macros`
and `or-renderer` library tests and the app's `selection` tests pass, and `--self-check` draws its
80 units with none failing. It still exits 1 with no GTA V install, because the run is judged by
whether install discovery wrote the preferences ([R58c](#r58c-correctness-off-windows)). What
remains is in the parts below. Every picture and sound plays on every platform, a Bink 1 or Bink 2
movie's picture included ([R34c](#r34c-video-preview)), through `or-media` and `rodio`
([recorded](Project_Context.md#every-media-format-is-decoded-in-rust-inside-the-tree-2026-10-05));
`cpal` links ALSA, so a Linux build needs `libasound2-dev`, which `ci.yml`'s Linux job installs.
R58a's first step moved the tooling to Rust, so no recipe needs PowerShell off Windows but the
janitor's and the limits'. No Linux run of the nextest tiers or `ready` is recorded, and hosted CI's
Linux job runs `just fmt`, `just lint` and the `xtask` tests.

R58a lands first, because every later part is checked with its recipes. R58g takes the file-type
table R58e builds; no other part waits on another.

**Done when.** A Linux or macOS contributor runs the same checks as a Windows one and gets the
same answer, with no PowerShell installed; the app does everything on all three that the OS allows
and says plainly what it does not; an install is found without typing a path wherever one can be;
a missing library is named; and a release builds a package for each. **Risk.** Under Proton a Steam
install still sits at `steamapps/common/Grand Theft Auto V`, so only the Steam root differs; the
Linux job catches a compile break but not a missing runtime library, and nothing here is checked
on Windows by a Mac, so each part ends with the owner's `just ready-full` on Windows.
**Covers.** The unnumbered cross-platform row.

### R58a One Rust implementation of the dev tooling

[↑ checklist](#phase-10--the-application)

**What.** A new `xtask` crate holds what the PowerShell scripts do, so one implementation runs on
every OS: the chunked formatter, the encoding and provenance lints, the hook, the lane and janitor
commands and the inline logic of `baselines-record`, `scaffold-format` and `test-no-pack`. The
`justfile`'s recipes become one-line calls that parse the same under `sh` and PowerShell. The
pre-commit hook runs the built binary and refuses, naming `just install-hooks`, when it is missing
or stale. The agent limits stay PowerShell, behind a Windows-only recipe, since Job Objects have no
equivalent elsewhere.

**Where it stands.** In the tree, but for two things. `crates/tools/xtask` holds the chunked
formatter, the encoding lint (which prunes a nested `target` on every OS, where the script matched
backslashes only), the provenance lint with its history mode, the hook and its installer, the test
tiers' environment and skip ledger, the baseline recorder, the format scaffold, `lane-new` and the
janitor. The janitor is a port of `worktree_janitor.ps1` with its refusals kept: a lane goes only
when the ancestor test or GitHub, asked about its exact HEAD, says it landed, a closed pull request
counts only when the reference records the landing, and a dirty lane, an ignored file no branch
carries, a recent write or a lock holds it; it reports unless `--apply`, and never deletes a branch
(tests break each of those and see the lane removed, and `janitor` runs from a lane, which holds
itself). It does not print the drive's free space, which needs a platform call nothing else here
makes. The `justfile` has no `set shell`, and every recipe but the agent limits' and
`cw-roundtrip` is a call to `cargo` or `cargo xtask`, held by
`a_recipe_that_is_not_windows_only_names_no_powershell`. The hook is a short shell script that
runs the binary the installer copied beside it, and refuses, naming `just install-hooks`, when that
copy is missing or older than its source. `ci.yml`'s provenance and encoding steps call
`cargo xtask`. Open: the old `.ps1` scripts, which go after one green `main-tip.yml` run on the
self-hosted runner has used the Rust and after `just janitor-dry` on Windows has been read beside
`worktree_janitor.ps1`'s report, and the owner's `just ready-full` there, which nothing here could
run; the agent limits and `cw-roundtrip` stay PowerShell on Windows. ⚠ AGENTS.md said the hook
compiles nothing; it now says it runs a built binary, and the Project_Context entry of 2026-10-06
records why. A recipe that calls `cargo xtask` is a call to `cargo`, which answers Appendix B's
question 5 the way the Linux job already reads it.

**Done when.** Every recipe but the agent limits' and `cw-roundtrip` runs on Linux and macOS with no
PowerShell installed; the hook runs the format, encoding and provenance
checks there; and one `main-tip.yml` run on the Windows runner passes with the Rust in place of the
scripts, which are then deleted.

### R58b CI on macOS and Linux

[↑ checklist](#phase-10--the-application)

**What.** A hosted macOS job and a fuller Linux job that run what a Windows contributor runs:
clippy with warnings denied, the workspace tests, `--self-check`, the BC7 encoder's tests, and the
renderer's tests under a software Vulkan adapter on Linux; and the wasm32 target in
`rust-toolchain.toml`, which the plugin tests need.

**Where it stands.** In the tree, and open until a macOS run and the Linux run have been read. The
macOS job runs weekly and on request, as the hosted Windows job does. That is the agent's default by
analogy with the cost decision of 2026-09-23
([recorded](Project_Context.md#hosted-ci-is-linux-per-pull-request-the-windows-suite-runs-on-each-tip-of-main-2026-09-23)),
which is about Windows; nobody has ruled on macOS ([Appendix B](#appendix-b--judgement-calls-to-confirm), item 6).
The Linux job installs lavapipe and the Vulkan loader, tests the whole workspace, `or-renderer`
included, and runs the BC7 encoder's tests and the self-check. The BC7 encoder, `intel_tex_2`, is
the one native link, and its 12 tests pass on aarch64 macOS. The renderer's `wgsl_compile` test
panicked on Metal, which has no software adapter; it skips through the ledger, as
the shaded pipeline test did. `rust-toolchain.toml` carries the wasm32 target.

**Done when.** One green weekly macOS run and a Linux run that executes the renderer's tests, each
with the self-check, and a test that no workflow's toolchain line differs from the pin
(`every_workflow_builds_on_the_pinned_toolchain`).

### R58c Correctness off Windows

[↑ checklist](#phase-10--the-application)

**What.** What a Windows user takes for granted works on the other two. `--self-check` exits 0 with
no install. A primary modifier is Ctrl on Windows and Linux and Cmd on macOS, with its label. The CLI
and the app read one configuration directory. A Wayland app id and an X11 class name the window.
System fonts are found by their family on each OS. A game executable and an install folder match
without regard to case on Linux and macOS. User-facing words name the platform's file manager. A
shader that uses clip distances is refused where the adapter lacks them.

**Where it stands.** Open, all found on 2026-10-06. Done: the self-check used to judge "preferences
written" by install discovery writing them, so it failed on a machine with no GTA V; it now saves
once through the editor's own function and looks where that save writes
(`a_self_check_that_finds_no_install_still_writes_its_preferences_and_passes`, which fails without
the save), so `just ready` exits 0 on macOS. A shortcut's `ctrl` is the platform's primary modifier,
Command on a Mac and Control elsewhere, with its own name in every label, so a saved binding or a
plugin's `Ctrl+K` means the same on every OS; a `control` modifier is the Control key itself on a
Mac, which Switch to previous tab uses because Cmd+Tab is the system's, and the tests press the
primary modifier through `PRIMARY_STATE` and `PRIMARY_EGUI`, which a real keyboard produces and
`Modifiers::CTRL` is not. `or_title::config` names the configuration directory, `OPENRAGE_CONFIG_DIR`
or else the user's, and the app, `openrage-cli` and the AWC test's key lookup (which had no macOS
case) all read it, where the CLI ignored the variable in four of its five places
(`the_keys_cache_and_the_snapshot_follow_openrage_config_dir`, which fails without it). The window
asks winit for a Wayland application id and an X11 class, `openrage`, on Linux and the BSDs, which a
package's `.desktop` file ([R58g](#r58g-packages-for-linux-and-macos)) matches; that code was
type-checked against winit for a Linux target and has run on no Linux machine. Open: some words a
user reads still say Ctrl, among them the default views' hints in `src/default_view_strings.rs`; the
font search is a fixed list of file names and directories in `src/app/gpu.rs`; the wording says
Explorer and "your Windows account"; the executable and folder matches are case-sensitive on Linux;
and wgpu's Metal backend does not offer `CLIP_DISTANCES`, which the renderer requests when offered,
while the old WGSL validator checked with every capability on. Not doing: `OPENRAGE_GAME_ROOT` stays
`;`-separated on every OS, as some forty tests and one install lookup document it, which costs a
Unix path holding a semicolon and nothing else; and `build.rs` embeds the Windows icon only when the
host is Windows, which matters to nobody who builds for Windows on another system.

**Done when.** The self-check exits 0 with no install on macOS and Linux; a test finds a font by
family on each OS's directories; a test finds `gta5.exe` accepted on a case-sensitive folder; and the
open words above say what the platform calls things.

### R58d Install discovery off Windows

[↑ checklist](#phase-10--the-application)

**What.** An install is found without typing a path wherever one can be: more Steam roots on
Linux and a Mac, and a Wine prefix read as a root, with the Rockstar and Epic probes run against its
`drive_c` in place of the registry. Wine's own, Lutris, Heroic, Bottles and Proton's `compatdata` on
Linux, CrossOver and Whisky bottles on macOS.

**Where it stands.** In the tree, held by the synthetic homes in `crates/title/or-title/src/install/discovery.rs`
(`a_wine_prefix_of_any_kind_is_read_as_a_windows_drive` and
`more_steam_roots_are_looked_in_on_linux_and_a_mac`, which failed against stubs first). The Steam
probe also looks under `~/.steam/root`, `~/.steam/debian-installation`, the snap and
`~/Library/Application Support/Steam`. A prefix is a `drive_c`; inside one, the Rockstar launcher's
default folders under `Program Files` and `Program Files (x86)`, Epic's manifests under
`ProgramData` with each `InstallLocation` mapped from `C:\` onto the prefix, and a Windows Steam
with its libraries are read as the Windows paths they are, and an install found there ranks below
one the platform's own store names. The Epic parser is shared with Windows, which type-checks, and
the crate type-checks for Linux and Windows from macOS; it has run on no Linux machine. ⚠ The
folders each tool keeps prefixes in are its documented defaults and were not seen on a real
machine, a prefix elsewhere needs a path the user gives, the launcher's registry lives in
`system.reg` inside the prefix and is not read, and GTA V has no native macOS build, so on macOS a
game is a bottle or a folder the user picks.

**Done when.** A person with GTA V under Heroic, Lutris, Bottles, CrossOver or Whisky opens the app
and finds it without typing a path; until one has, this part is open.

### R58e Windows-only features elsewhere

[↑ checklist](#phase-10--the-application)

**What.** The Windows-only stubs in `src/platform` become real or honest. Delete goes to the
platform's trash. Reveal selects the file in the file manager. Ctrl-C in an `openrage-cli` job stops
it and still reports. A file list goes on and comes off the clipboard where the OS has one. A drag
out of the app works on macOS and the control is absent where it cannot work, in place of a toast on
every attempt. File types are associated through one table that writes the registry on Windows, a
desktop entry and a MIME package on Linux, and the bundle's document types on macOS
([R58g](#r58g-packages-for-linux-and-macos)).

**Where it stands.** Three of the seven are in the tree. Delete: off Windows, `src/platform/recycle_bin.rs`
hands the path to the `trash` crate, Finder's Trash on a Mac and the freedesktop one on Linux, where
it used to delete for good; a Trash that cannot take it is an error and the file stays; the words
say Trash where Windows says Recycle Bin, and Windows keeps its own shell call and its stand-in for
tests. `a_real_delete_goes_to_the_system_trash_and_not_for_good` is `ignore`d, since it puts a file in
the real Trash, and was run once on a Mac: the file left its folder, was found whole in `~/.Trash`
and was removed from there. Reveal: Linux asks the session bus's `org.freedesktop.FileManager1` to
show the file selected, through `dbus-send` and a tested `file://` encoder, and falls back to
opening the parent folder; a Mac's `open -R` and Windows' `/select,` already selected it. Ctrl-C: a
Unix `openrage-cli` job installs a `signal(2)` handler of its own, with no crate, which sets a flag a
thread acts on and leaves the second Ctrl-C to the default, as the Windows console handler does
(`the_first_interrupt_runs_the_callback_and_a_second_installer_is_refused` raises a real SIGINT, and
failed against the old no-op). ⚠ The Linux reveal and the `trash` crate on Linux have run on no
Linux machine; the function was type-checked for Linux in a scratch crate. Open: off Windows
`src/platform/clipboard.rs` and `src/platform/drag_drop.rs` return an error and the associations page
always fails. Unverified until built: that the `arboard` the app pins carries a file list on macOS and
Linux, that winit 0.30 delivers a macOS open-document event, and what Wayland and X11 report as the
cursor during a drop.

**Done when.** A test per capability finds it present or reports it absent by a flag the UI reads, a
delete on each OS is recoverable where the platform has a trash, and Ctrl-C in a CLI job prints its
partial report.

### R58f Shader baking off Windows

[↑ checklist](#phase-10--the-application)

**What.** Every shader of GTA V Legacy, GTA V Enhanced and RDR2 runs natively on Windows, Linux and macOS (Metal), through one translator from each game's bytecode to SPIR-V in a helper program the editor never loads, as [the design](Shader_Pipeline_Design.md) sets out; the whole frame is the goal, with the material preview first.

**Where it stands.** Three steps are in the tree. The old chain's download of two Windows tools, which was refused elsewhere and said why, went with the chain. `or-shader-tool inventory` reads a Legacy install (1.0.231.0, at `2884a9b5` with this change) as 375 effects in each of three folders and 35,468 DXBC containers in all, none malformed, with no DXIL or SPIR-V; `common.rpf` holds 321 of the 375 effects and `update/update2.rpf` replaces 314 and adds 54, which the extraction now reads through the mount. And the first spike ran on macOS: dxbc-spirv at `c7f06970` is vendored in `or-shader-front` behind a `native` feature that is off until a Windows build is checked, and `or-shader-tool translate-check`, which legalizes what it translates and runs naga 29.0.1 and its MSL writer, translates all 35,468 DXBC containers of that install and writes 30,304 as MSL, of which 30,217 compile on this Mac's Metal device with `--device` (at `47597854` with this change); the rest stop at the geometry, hull and domain stages and the vertex shaders that feed them ([measured](Shader_Pipeline_Design.md#what-the-first-spike-measured)). Counted by what a viewer draws, `or-shader-tool pass-check` builds 11,937 of the 12,129 vertex-and-pixel passes of that install's main folder (98.4%), and `draw`, `deferred_draw` and `drawskinned` in every effect but one between them, at `15bef620` with this change; only the instanced, tessellated and compute passes do not, so the viewer needs no geometry, hull or domain emulation ([measured](Shader_Pipeline_Design.md#what-a-pass-needs)). Each register now binds at a place its class, number and stage fix, and `translate-check` finds that 35,456 of the 35,468 containers bind exactly what their reflection lists, the other 12 declaring a constant buffer that nothing reads, at `b35e1384` with this change ([measured](Shader_Pipeline_Design.md#where-each-register-binds)). `or-shader-tool bake-pack` writes the first SPIR-V pack, one legalized module per distinct program a pass runs plus a manifest, and a second run reuses every module ([measured](Shader_Pipeline_Design.md#what-a-bake-writes)); the app does not read it yet. Built as whole pipelines on the Metal device, only 1,996 of the 12,375 passes of that folder built at `161d6a20`, against 11,937 of 12,129 vertex-and-pixel passes by stage; 5,164 build once each stage has its own set at `693ec247`, 12,316 once every interface location is four components wide at `881bf219`, and all 12,375 once the stages are linked for interpolation, at `fdff0ed9` with this change ([measured](Shader_Pipeline_Design.md#whole-pipelines)): every pass whose programs built builds as a pipeline on the Metal device, and the renderer's `game_shaders` draws each of them with stand-in resources, 12,375 of 12,567 at `95de47a2` ([measured](Shader_Pipeline_Design.md#drawn-with-stand-ins)); the first real render is `default.unlit_draw` drawing a textured quad on Metal, at `c7d3be4b` with this change ([measured](Shader_Pipeline_Design.md#a-first-render)); the app has the helper bake that pack in the background and reads it, 4,970 programs in 5.4 seconds, at `91419492` with this change ([measured](Shader_Pipeline_Design.md#baked-from-the-app)); and a drawable of the install draws with its own material, 2,816 of 3,000 sampled drawables showing something and 10,924 of 10,941 of their surfaces drawing, at `d270f330` with this change, and 2,983 once the engine sets a wheel's matrices and a cable's width ([measured](Shader_Pipeline_Design.md#meshes-with-materials)). Windows and Linux are checked from the Mac and not run: the whole workspace and its tests pass clippy for `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu` and the C++ translator compiles for both (`cargo xtask cross-check`, at `786e9ce4`; [how](../guides/Development_Workflow.md#cross-checking-another-os)), and naga's Metal, Vulkan and Direct3D 12 writers all take the same 4,829 of 4,970 programs, of which SPIRV-Tools' validator accepts 4,803 (Metal took 4,751 and the others 4,789 before seven more lowerings, at `8047e4f6`), and the Vulkan backend through MoltenVK draws the 3,000-drawable sample exactly as Metal does, at `be1fef44`; on Linux (aarch64 in a virtual machine, Mesa's lavapipe) the workspace tests, clippy and the headless self-check pass and 2,813 of the same 3,000 drawables show something, at `af56b88c` ([measured](Shader_Pipeline_Design.md#what-each-platforms-writer-takes)). Open: the Enhanced and RDR2 inventories and the design's other phases.

**Done when.** No download or failing bake happens on a platform that cannot run the tools (done); the inventory's result for each install is recorded here, which settles which formats Enhanced and RDR2 ship; the front-end spike builds on all three OSes and Vulkan accepts its output; and a shader that Direct3D draws from the original bytecode on Windows draws within a recorded tolerance through the new path on each OS.

### R58g Packages for Linux and macOS

[↑ checklist](#phase-10--the-application)

**What.** A release builds a package of each OS from one `xtask` command, so the three share their
packaging code. Linux: a tarball and a `.deb` with a desktop entry, icons and a MIME package.
macOS: a universal `.app` and a `.dmg`, ad-hoc signed, with its icon and its document types. Each
carries `openrage`, `openrage-cli`, the licence files (`or-media` is LGPL-2.1-or-later and
statically linked) and a checksum.

**Where it stands.** In the tree, except a Windows run. `cargo xtask dist windows`,
`linux` and `macos` make a zip, a tarball with an `install.sh` and a `.deb`, and `OpenRage.app` with a `.dmg`,
each beside a `.sha256`; `release.yml` runs it on all three and publishes the lot. The formats a
package claims are asked of the built app with `openrage --file-types`, which prints the list the
Settings page offers (`the_formats_a_package_is_told_are_the_ones_the_settings_page_offers`), so a
package cannot name a format the app does not open, and only the game's own, never `xml` or `png`,
are claimed. Held by tests that stage each package from stand-ins on any OS and check the desktop
entry, the shared-mime-info package, the plist (with `plutil` on a Mac), the `.icns` and a SHA-256
written here against the system's `shasum`, which fails when a round constant is off by one. A real
universal `.app` and `.dmg` were made from release builds on an Apple-silicon Mac (145 MB and 64 MB)
and checked with the system's tools: `plutil -lint`, a valid ad-hoc `codesign` signature,
`hdiutil verify`, the checksum, and both executables run from inside the mounted image, the arm64
and the x86_64 slice each. A Linux package was built and installed in an aarch64 virtual machine
(`cargo xtask dist linux --require-helper` with the C++ translator compiled there, 43 MB, at `8790d047`):
the checksum matches, `install.sh` puts the editor, `openrage-cli` and `or-shader-tool` in one
folder, `ldd` finds no missing library, the installed editor's headless `--self-check --viewport` draws 106 units
without a failure, the helper beside it names the same translator and legalizer as the Mac's, the
desktop entry passes `desktop-file-validate` with one hint (`Development;Graphics;` are two main
categories) and the MIME package is well-formed and updates the database. ⚠ Not checked: a Windows
package on its own OS (the owner runs it on a Windows machine), a Linux x86_64 package,
and the workflow on GitHub. The `.deb` is written by `xtask` itself (an `ar` archive of two ustar
tarballs, which `gzip` compresses, so no `dpkg` is needed to make one) and holds the same files under
`/usr`, with `libc6`, `libasound2` and `libvulkan1` as its dependencies; in the same virtual machine
`dpkg-deb` reads it, `dpkg -i` installs it with the desktop and MIME triggers running, the three
executables run from `/usr/bin`, and `dpkg -r` removes every file, at `28488c39`. `lintian` still reports
no changelog, unstripped binaries and no manual pages; the maintainer's address is a placeholder. The bundle identifier, `io.github.openragee.openrage`, is a placeholder until a
domain is owned. The `.icns` holds one 256 px image, which the system scales. A Linux machine needs
`libasound.so.2` to start, since `alsa-sys` links it, so the `.deb` depends on it and
`install.sh` says so. Notarisation waits for a Developer ID and is a later row; an unsigned `.app`
opens with a right-click the first time, and `hdiutil create` warns that it is deprecated in favour
of `diskutil image`.

**Done when.** A tag builds all three packages, the tarball installs on a clean Linux machine and the
desktop offers the app for a `.ytd`, a `.deb` does the same in a clean container, and the `.app`
opens on a Mac and offers the app for a `.ytd`.

### R59 Presentation input and workspaces

[↑ checklist](#phase-10--the-application)

**What.** Themes, accent, scale and font; localisation with a community pipeline; rebindable
shortcuts; remembered dock layouts; a camera per document with each workspace a layout preset
(decided 2026-09-23, [R59e](#r59e-workspaces-that-hold-their-own-scene)); and user-facing
performance and cache tuning (R59a to R59f). Since the UX review of 2026-10-03 it also holds the
shell's screenshots, commands, shortcuts, regions, tabs, workspaces, words, feedback and
accessibility (R59g to R59z). **Where it stands.** Shortcuts predate the parts
(`src/ui/shortcuts.rs`). [R59a](#r59a-the-egui-034-deprecations), [R59b](#r59b-appearance-and-scale),
[R59d](#r59d-the-layout-that-survives-a-restart) (`src/ui/dock_layout.rs`) and R59e are done;
[R59c](#r59c-localisation-and-the-translation-pipeline) is open, its early start declined against a
measured string count, and [R59f](#r59f-performance-and-cache-tuning) is open. R59g to R59w
and [R59x](#r59x-self-checks-and-tests-leave-nothing-behind) and [R59y](#r59y-what-a-test-run-or-a-self-check-still-leaves-behind) and [R59z](#r59z-small-faults-left-in-the-default-views-menus-and-keys) are done.
No part blocks another. The family has no letter after R59z, so the shell findings that follow
are [R68](#r68-shell-faults-after-r59z)'s.
**Covers.** U-174, U-197, U-198, and the unnumbered docking and performance-tuning rows.

### R59a The egui 0.34 deprecations

[↑ checklist](#phase-10--the-application)

Done in #119. No file in `src/` suppresses `deprecated`, and
`no_file_in_src_suppresses_a_deprecation` in `src/suppressions.rs` refuses one coming back.

⚠ `show(ctx, …)` → `show_inside(ui, …)` is not a rename: it moves who owns screen space. The
top-level panels become children of one root `Ui` per window, built by hand in `src/ui/root.rs`
because `Context::run_ui` re-runs its closure when a pass is discarded and would run the shell twice
against one `&mut AppState`. `show_inside` also does not register the panel with the context, so an
unanchored `egui::Window` added later opens at the viewport's top-left rather than below the menu
bar.

### R59b Appearance and scale

[↑ checklist](#phase-10--the-application)

Done in #182. Settings ▸ Appearance picks a dark or light theme, an accent, an interface scale from
`SCALE_STEPS` and a font, and every window restyles at once. `src/state/prefs.rs`
reads `appearance` field by field, so an unreadable value costs only itself. How it looks has not
been checked by a human.

### R59c Localisation and the translation pipeline

[↑ checklist](#phase-10--the-application)

**What.** A localised UI with a community translation pipeline and an outdated-translation
indicator, which OpenIV (17 locales) and RPFKit (four) have and the tree has none of.

**Where it stands.** Starting out of phase order was weighed and declined on 2026-09-22 (#137),
against a loose upper bound of 1,954 capitalised string literals across `src/` at `9e37af37`. The
rest of Phase 10 adds strings, so extracting first means extracting twice; the rule that users read
Legacy and Enhanced, never Gen8 and Gen9, was prose then, so a sweep would bake whatever it caught
into every language at once; and without a catalogue nothing fails on a string that bypassed it.
That rule is a test now: `no_user_facing_literal_names_gen8_or_gen9` in `src/edition_strings.rs`
refuses a string literal outside test code in `src/` or `or-cli` that names `Gen8` or `Gen9`. The
catalogue and the translation pipeline stay declined.

**Done when.** A translator can contribute a language without building the app, and a string added
after a translation was written shows as outdated rather than silently falling back.

### R59d The layout that survives a restart

[↑ checklist](#phase-10--the-application)

Done in #136. `src/ui/dock_layout.rs` persists each workspace's arrangement in `prefs.json`,
versioned by `LAYOUT_SCHEMA`, written once it has settled for `SETTLE` and flushed on close; a layout
from another schema is discarded whole. An unknown panel id keeps its tab as a placeholder
(`an_id_nothing_provides_keeps_its_tab`), and View ▸ Panels floats a panel into its own window or
brings a closed one back. The traps — a `DockState` never laid out, whose infinite rects would take
every other preference down with it, and the bound on interned ids — are in
[Crate_Reference.md](../reference/Crate_Reference.md).

### R59e Workspaces that hold their own scene

[↑ checklist](#phase-10--the-application)

The title is the question as first asked, kept as the anchor. The owner decided on 2026-09-23
([Project_Context.md](Project_Context.md)) that each document keeps its own camera and a workspace
stays a layout preset, not a scene container: what a user notices losing is the camera.

Done in #169. `src/state/viewport_camera.rs` holds an `OrbitCamera` per document in a side table
keyed by document id, dropped when the document closes;
`a_documents_camera_survives_switching_documents_and_workspaces` holds each camera across a
document switch and every workspace switch. Cameras are session-only: if one is ever
persisted, it goes in the `.orproj` project file, not `prefs.json`. How a switch back looks has not
been checked by a human.

### R59f Performance and cache tuning

[↑ checklist](#phase-10--the-application)

**What.** Cache sizes, GPU budgets, a statistics window and a profiler, surfaced to the user.

**Where it stands.** The Statistics panel (`src/ui/panels/statistics.rs`) shows the frame time
`src/state/frame_stats.rs` smooths, broken into the phases the editor's frame loop
(`src/app/editor_window.rs`) times at its boundaries: background results, surface acquire,
simulation, UI layout, render encode, and submit and present, each smoothed like the frame time so
they never sum past it, with the rest shown as waiting. It shows the decoded texture preview
cache (`src/state/texture_cache.rs`) against its byte budget, which the panel sets, the preferences
keep (512 MiB by default), and lowering evicts at once. It also shows the textures the viewport
has uploaded to the GPU against a second budget the preferences keep (1024 MiB by default), and
the active mesh's vertex and index buffers. The render bridge counts each texture's
bytes as it uploads it (`src/render/gpu_budget.rs`), evicts the least recently used past the
budget, and publishes the counts to the state each frame. Beside them the panel shows what no
budget evicts: the particle and particle-model textures (`src/render/particles/texture_cache.rs`),
the renderer's own targets (colour, depth, the G-buffer, HDR, bloom, the shadow cascades, fog, a
loaded sky HDRI, the refraction snapshot and the lighting inputs), which the viewport's size sets,
the gallery's tiles with everything the renderer that draws them holds (`src/render/thumbnails.rs`),
the particle-model meshes (`src/render/particle_models.rs`), and, from `ResidentBytes` in
`crates/runtime/or-renderer/src/viewport.rs`, the line buffer, the particle, trail, model and decal
passes' instance and quad buffers, the uniform and storage buffers, and the 1x1 default textures
with the sky's fallback HDRI. Every texture's bytes come from its format, extent, mip levels and
samples (`texture_bytes`), so a figure is the same on every backend and leaves out the driver's
padding. The panel shows egui's own textures from egui's texture manager, less the texture
previews, and each open document's bytes on the CPU (`Document::held_bytes` in
`src/state/document/mod.rs`): the resource image it keeps until an edit, a text, video, shader or
string table's contents, an archive's in-memory entries and read cache, and an owned value's heap,
which `Heap::heap_bytes` (`crates/io/or-io/src/decl/owned.rs`) sums over its fields: each run at its
capacity and what its elements hold, each pointee, each string and what its carry keeps. The
`#[record]` macro derives it and the leaf types implement it by hand. A `.ypdb` and a plugin's
document still count at the size of the image they were decoded from. The app's other
long-lived CPU state has a line each (`CpuState` in `src/state/memory.rs`), each at the capacity of
what it allocates and read by the panel at most once a second, because a walk visits every string
and record the app holds and took 100 ms with a large effect open: the name index (its sets and
snapshot, and its hash index estimated from its count of names, which `or-io` gives no size for),
the file index, the search results and each archive find's matches, the archive trees' rows, the
tables of contents of every archive the trees opened, each once (`RpfArchive::table_bytes`; a
nested archive counts from the listing of its parent, which opens it), each document's undo
history (its steps' labels, bytes, images beside the one the document holds, archive rows and
textures; any other part at its own size), the simulation (the rules it runs from, every emitter's
particles, the snapshots a seek replays from, the pins), the HDRIs the sky loaded, and what Uses
and Placed by read (the root archives' tables of the mount, the file catalogue and the placements).
The session byte cache was counted already, in an archive document's `held_bytes`. A test in
`src/state/the_panel_counts_the_apps_cpu_state.rs` grows each line's structure through the app's own
API and holds its figure, down to the terms that dominate it: the name index's hash index, a nested
archive and its parent's table, and the simulation's rules, particles and snapshots. Measured once,
by a scratch test that is not kept, on the Legacy install with the game root open and a few heavy
documents, as the panel's figure against the bytes dropping the structure freed under a counting
allocator: the name index 35.7 of 36.0 MiB, the file index 43.0 of 43.0, an archive tree and its
tables 7.4 of 7.4 for eight archives, the simulation of a seven-emitter effect from `core.ypt` 147.1
of 147.0, and for the Uses and Placed by read 98.3 of 136.9 (the mount 4.4 of 5.7, the catalogue
37.3 of 37.5, the placements 56.6 of 55.9). The driver's objects come from wgpu's own report, read
through the instance the render bridge is given (`DriverObjects` in `src/render/gpu_budget.rs`):
wgpu exposes a count and no size for a pipeline, bind group, sampler or shader module, so each shows
as a number, and its allocator's totals show where the backend keeps a report, as the check on the
GPU classes above. Not counted: what `or-io`, `or-cli` and `or-depends` keep private (the shader parameter
remap, 7.3 MiB on the Legacy install; the clip region table, the materials palette and the
install's interiors and tables, each under 0.1 MiB there; the archetype, table and model indexes of
the Uses read, 37.8 of its 136.9 MiB), the 7.6 MiB that opening a first metadata file added beyond
the file, the nested archives a mounted install's devices keep open, the simulation's behaviours
and pool batches, the diagnostics log, and the part of an undo step beyond the types named; there
is no profiler beyond the phase timers.
Found while closing [R24b](#r24b-go-to-file) and [R60e](#r60e-one-place-for-background-jobs): the
file index holds about 42 MB for the Legacy install, one string per path, where sharing the
archives' path prefixes would roughly halve it; the name scan and the index each walk the install
at launch, where the scan could take its names from the index; and the bake installs its tools on
the UI thread, a 26 MB download on first use that freezes the window, as an install's archive save
does while it writes. Since [R27f](#r27f-the-viewport-draws-every-edit) the thumbnail cache keeps
a tile for each change of a document's contents, up to its 256 tiles (about 64 MiB), and drops a
selection's older tiles only by age; a material slider's drag builds the meshes again every frame,
and since [R27i](#r27i-every-view-follows-every-edit) hashes each bound texture every frame too.
Since [R27h](#r27h-the-grid-and-undo-survive-an-edit-of-the-contents) a document's history keeps up
to four images, about 288 MB for a 28.7 MB `.ytd`, which the undo history line counts, and an image
built again for a grid pauses the window once, 1.3 s for that `.ytd` and 4 s for `core.ypt`, half
of it a compression undone at once. A DLC pack's question lists its source folder on the UI thread
([R51d](#r51d-a-dlc-pack-from-the-app)). Found while closing
[R42f](#r42f-the-viewport-picks-what-it-draws): the Outline's `draw_ydr` and `draw_ydd`
(`src/ui/panels/hierarchy/ydr.rs`, `src/ui/panels/hierarchy/ydd.rs`) clone the whole drawable or
dictionary every frame.
Found while closing [R68a](#r68a-what-the-accessibility-baseline-still-leaves): `theme::graphic`
runs `legible` for every key and span of the timeline on every frame, taking the palette lock,
copying the palette and searching lerp steps against five surfaces, a luminance conversion that
calls `powf` at each, even where the colour already passes at the first step, as the whole dark
theme does, so frame time rises with no gain there and by more in the light theme; the `format!`
arguments to `described` and `named` are built once per visible handle, dense row and table cell
with no screen reader attached, the usual case; and none of it is measured. Found while closing
[R55d](#r55d-what-converting-still-gets-wrong-after-r55c): the conversion's report is one selectable
Label per file line and per dropped value, built every frame with no virtualisation, where it was
one label with one cached galley, so a selection of many drawables that each drop many values lays
out very many widgets a frame while the question is up. Found while closing
[R60i](#r60i-what-a-write-still-does-not-ask-after-r60h): `no_job_touches` runs
`as_the_disk_has_it`, a walk of canonicalisations, over every file every running job reads on each
call, and its per-path callers (an extraction's loop, `add_file_to_row`, each row
`rpf_move_node_inner` moves) call it once a path, so while a job that reads many archives runs, a
large import, a drag of many rows or an extraction into a related folder costs paths times reads
canonicalisations on the UI thread, where `no_job_reads_any` looks the files up once and only a
conversion and a glTF export use it; it costs nothing when no job runs.

**Done when.** A user can see what the application spends memory and frame time on, and can change
the budgets it spends them against. Prefer surfacing counters the code already keeps to
instrumenting for the panel: a profiler that ships has to keep working.

### R59g Screens an agent can see

[↑ checklist](#phase-10--the-application)

Done in #453. `just shots SCRIPT DIR` runs the editor's own frame from a script of steps and saves
each `shot` as a PNG, so a change to what the app draws carries its screens without anyone opening a
window. The steps and their limits are in
[Development_Workflow.md](../guides/Development_Workflow.md#screenshots).

### R59h No command that does nothing

[↑ checklist](#phase-10--the-application)

**What.** A command works, or it is hidden, or it is shown disabled with its reason. File ▸ Exit and
the palette's Exit close the app; the window title names the document and the project and marks
unsaved work; `Ctrl+Space` maximises the panel or is not bound; frame selection frames the
selection; Channel shading draws meshes or is not offered; "Open in new tab" opens a tab; Cut, Copy,
Paste and Delete stay hidden until [R25f](#r25f-copy-cut-and-paste) lands.

**Where it stands.** Done. Since #455, File ▸ Exit and the palette's Exit close the app
(`file_exit_asks_the_window_to_close`, `the_palettes_exit_asks_the_window_to_close`) and the window
title names the document and the project and marks unsaved work. `Ctrl+Space` is not bound
([R59i](#r59i-shortcuts-by-character-live-while-typing)); Maximize, once bound in Settings, shows
the focused panel alone and puts the arrangement back when it runs again. The panel is drawn from a
dock of its own, so neither the arrangement nor `prefs.json` changes, and any change to the
arrangement ends it (`maximize_shows_the_focused_panel_alone_and_again_puts_the_arrangement_back`).
Frame selection frames the box the file ships for the selected model or geometry, in a drawable, a
dictionary entry, a fragment or a particle model, and otherwise what Frame all frames
(`frame_selection_frames_the_selected_geometry_or_model_and_otherwise_everything`,
`frame_selection_frames_a_geometry_inside_a_dictionary_entry`). Channel mode hands the renderer the
document's meshes (`every_mode_but_wireframe_hands_the_renderer_the_documents_meshes`). An archive
entry's menu says Open, which opens it under its archive, in a tab named with its archive since
[R59l](#r59l-tabs-that-behave-like-an-editors)
(`an_entrys_menu_offers_open_and_open_opens_it_under_its_archive`), and Edit ▸ Cut, Copy, Paste and
Delete were hidden until [R25f](#r25f-copy-cut-and-paste) brought them back working
(`the_edit_menu_offers_no_command_that_does_nothing`). Maximize has not been seen in a window: no
`just shots` step reaches a shortcut winit dispatches.

**Done when.** A test runs Exit and finds the window asked to close, and none of these commands is
offered while it does nothing.

### R59i Shortcuts by character live while typing

[↑ checklist](#phase-10--the-application)

**What.** A letter shortcut matches the character its key types, so `Ctrl+Z` undoes on every layout;
`Ctrl` and `Alt` combinations reach the app when a focused widget does not use them; keys a panel
owns (Space, the arrows, Home) act only in the panel under the pointer or with focus; `Ctrl+Y`
redoes.

**Where it stands.** Done. `action_for_press` in `src/ui/shortcuts.rs` decides from winit's logical
key, its physical key, the modifiers and whether a widget has keyboard focus. A letter or a
punctuation mark matches the character its key types, ignoring case, and a key that types none
matches by name; Shift does not count for a digit or a punctuation mark, whose character already
says whether the layout needed it, so `.` frames the selection on AZERTY. A character no binding can
name (a Cyrillic layout's) falls back to the key's US-layout position, as egui does, and bindings
saved as winit key codes (`KeyZ`) still read. While a widget has focus, a key without Ctrl or Alt
stays with it, and so do the editing combinations (Ctrl with A, C, V, X, Z, Y, the arrows, Home,
End, Backspace and Delete, and the text editor's F and H); every other Ctrl or Alt combination
reaches the app, which takes the key from egui so that the field does not act on it too. The
timeline's Space, arrows, Home and End and the viewport's framing keys act only under the pointer
and never while a widget has focus, and the viewport's come from the bindings table: Frame all is
`Home`, and since [R42b](#r42b-viewport-basics-for-inspecting-assets) Frame selection is `F` or `.`. `Ctrl+Y` redoes beside `Ctrl+Shift+Z`, and New session and Maximize are listed but
unbound. Settings ▸ Keyboard shortcuts captures and shows a binding by its character.
`ctrl_and_the_key_printed_z_undoes_on_a_french_layout`, `ctrl_s_saves_while_a_text_field_has_focus`
and `a_space_typed_into_a_text_field_does_not_start_playback` hold the three clauses below. No panel
holds keyboard focus of its own, so a panel's keys follow the pointer alone, and no French keyboard
has been tried in a real window.

**Done when.** A test on a French layout sends `Ctrl+Z` and undoes, `Ctrl+S` saves from inside the
text editor, and a space typed into a filter does not start playback.

### R59j One command registry

[↑ checklist](#phase-10--the-application)

**What.** Every action is a command with an id, a label, a category, a default binding and an
enabled-when that returns its reason. Menus, context menus, the palette, shortcuts and plugin
commands ([R45](#r45-the-plugin-sdk-and-registration-surface)) render from it; the palette ranks
fuzzy and recent-first, shows bindings and takes the arrow keys; every hint names the current
binding. OD-AUDIT-KEEPS keeps the app's tiered registries, and this is one more.

**Where it stands.** Done for the menu bar, the palette, shortcuts and plugins. `COMMANDS` in
`src/ui/commands/table.rs` lists every command the app defines with its id, label, category (its
menu path), default bindings, an enabled-when that returns why it cannot run, and what it does;
`src/ui/commands/mod.rs` adds the commands each recent file, folder and project, project member,
workspace, panel and plugin offers. The File, Edit, View, Tools and Help menus draw every item
from it with the user's current binding, a disabled item saying why on hover, and
`every_menu_item_is_in_the_palette_with_its_label_binding_and_enabled_state` walks them through
AccessKit and finds each in the palette with the same label, binding and enabled state. The
palette lists every command with its category and binding, ranks letters matched in order and
what it ran last first (`svas_finds_save_as_first`,
`a_command_run_from_the_palette_comes_first_next_time`), runs the row the arrow keys highlight
(`the_arrow_keys_move_the_highlight_and_enter_runs_the_highlighted_row`) and says why the
highlighted one cannot run. A key runs its command through the table
(`a_key_bound_to_any_command_runs_it_through_the_registry`), Settings ▸ Keyboard shortcuts lists
and rebinds every command, a binding saved before keeps its command's id
(`a_binding_saved_before_the_registry_still_binds`), the text editor's find and replace follow
their bindings, and `Ctrl+Shift+F` reaches the app from a field. The status bar, the viewport's
overlay, the find bar and the palette name keys as bound
(`the_status_bar_names_the_key_open_is_bound_to`). Since [R59r](#r59r-context-menus-from-the-command-registry)
the context menus draw their app commands from it as well, acting on the row clicked. Seen on `just shots` screens, not in a window.

**Done when.** A test walks every menu item and finds it in the palette with the same label, binding
and enabled state.

### R59k The shell's regions

[↑ checklist](#phase-10--the-application)

**What.** One job per region. On the left, Files (installs, folders, projects and favourites) and an
Outline of the active document only, which follows its tab; in the centre, the document or the Files
view; on the right, the Inspector; every panel's title shows. Saved layouts are dropped once, as a
layout schema change already does. Workspaces stay layout presets and cameras stay per document
([R59e](#r59e-workspaces-that-hold-their-own-scene)).

**Where it stands.** Done. Every workspace puts Files above the Outline on the left, its own centre
in the middle and the Inspector on the right (`src/ui/dock.rs`). Files
(`src/ui/panels/hierarchy/files.rs`) holds each install, folder and archive open as a tree, with the
filter, history, breadcrumbs and ★ Places, which lists the favourites, recent projects and
recent files; an archive entry open as a document of its own is a leaf there. Its Details and Icons
views are Browse's centre since [R25d](#r25d-the-files-view-in-the-centre). The Outline
(`src/ui/panels/hierarchy/outline_panel.rs`) draws the active document's own tree only, and follows
its tab (`with_three_documents_open_the_outline_shows_only_the_active_one_and_switching_tabs_switches_it`);
with nothing open, an install, folder or archive active, or a document with no parts, it names what
to open. Every tab bar shows its titles whatever its leaf holds
(`every_panels_title_is_drawn_whatever_its_leaf_holds`). A workspace starts with its centre
focused, the focused panel's title is drawn in the accent, and Maximize shows it with no click first
(`the_focused_panel_is_marked_and_maximize_shows_it_without_a_click`); a panel's keys still follow
the pointer ([R59i](#r59i-shortcuts-by-character-live-while-typing)). A panel shows another through
`WorkspaceDocks::reveal`, applied once the dock is drawn, so the tree's eye button shows the preview
pane (`a_reveal_from_inside_a_panels_draw_shows_its_panel`). A click, Back, Forward or a breadcrumb
in Files selects its row and makes its document active only where the workspace on screen shows
that kind, as [R60c](#r60c-the-front-door) does for rows, so moving through Files never switches the
workspace (`back_in_files_keeps_the_workspace`); a double click opens, which may, and a place from ★
Places that is a file of its own is shown wherever it opens. A workspace whose active document is of
a kind it does not handle draws its own empty state, so Data no longer draws a model
(`a_workspace_the_active_document_does_not_belong_to_shows_its_empty_state`). `LAYOUT_SCHEMA` is 3,
so a layout naming the Hierarchy is set aside once
(`an_old_layout_naming_the_hierarchy_reads_and_falls_back_to_the_defaults_once`). The tree's Focus
checkbox went with the stacked documents, whose job the Outline does. Workspaces stay layout presets
and cameras stay per document. Seen on `just shots` screens, not in a window.

**Done when.** With three documents open, the Outline shows only the active one, and switching tabs
switches it.

### R59l Tabs that behave like an editor's

[↑ checklist](#phase-10--the-application)

**What.** Tabs always show, with a close button and one mark for unsaved work. A single click in
Files opens a preview tab that the next one replaces, and an edit or a double click keeps it; an
archive entry gets a tab named with its archive; opening a file that is open focuses its tab;
`Ctrl+Tab`, `Ctrl+Shift+T` and a list of the tabs that do not fit.

**Where it stands.** Done. The strip (`src/ui/panels/document_tabs.rs`) has a tab for every
document, one alone included, each with a close button and a middle click that go through
[R60d](#r60d-unsaved-work-survives-closing-and-quitting)'s close request, and one unsaved mark, the
dot, set exactly when closing would lose work, so a clip's pose keys not yet baked mark its tab as
they mark the window title (`every_unsaved_tab_carries_one_mark_and_pose_keys_not_yet_baked_are_unsaved`).
The tabs that do not fit are in a list at the strip's end, the active tab always shows, a long name
is cut short and the strip never runs off the window
(`every_tab_has_a_close_button_and_those_that_do_not_fit_are_in_a_list`). An archive entry's tab is
named with the nearest archive holding it, else with the folder or install it came from, and closing
it keeps the archive open
(`an_archive_entry_has_a_tab_named_with_its_archive_and_closing_it_keeps_the_archive`); a texture
dictionary the app opens from an archive for a model's textures still has none. Opening a file that
is open focuses its tab, whether it comes from disk, a recent file, a walk (search results and Go to
File) or an archive, and while its first load is under way; two archive entries are one file only as
one row (`opening_one_file_twice_leaves_one_tab`). Where the workspace on screen shows a file's kind,
a single click on its row in Files opens it in a preview tab, drawn in italics, that the next single
click replaces; a double click on the row or the tab, any other open of it, or an edit keeps it, and
in Browse a click shows the row in the centre's Files view and its preview pane
([R25d](#r25d-the-files-view-in-the-centre))
(`a_single_click_in_files_opens_a_preview_tab_the_next_replaces_and_a_double_click_keeps`). `Ctrl+Tab`
switches to the tab used before, and with Ctrl held goes further back; `Ctrl+Shift+T` reopens the tab
closed last, from disk or from its archive, and a preview is not remembered. Both are registry
commands that reach the app while a field has focus
(`ctrl_tab_returns_to_the_tab_used_before_and_with_ctrl_held_goes_further_back`,
`ctrl_shift_t_reopens_the_tab_closed_last`). Seen on `just shots` screens; the two keys have not been
pressed in a window, since no step reaches a shortcut winit dispatches.

**Done when.** Opening one file twice leaves one tab, and an archive entry has a tab.

### R59m Every kind has a home

[↑ checklist](#phase-10--the-application)

**What.** Workspaces by task: Browse (the default), Model, Texture, Animation, Particle and Data
(metadata, text, string tables, tables and audio). Video and audio play in the centre; a dictionary's
gallery is its document's centre; a new format gets a document view, not a workspace; the last
workspace persists. The particle editor stays (owner, 2026-10-01), as one workspace among the others.

**Where it stands.** Done. `Workspace` (`src/ui/dock.rs`) is Browse, Model, Texture, Animation,
Particle and Data, and a first start is in Browse (`a_fresh_state_starts_in_browse`). Only a
`.ypt` opens into Particle, and Particle keeps nothing else on screen
(`no_kind_but_particle_effects_opens_into_the_particle_workspace`). Archives, the install,
folders, video, an unread file's bytes and a format with no row of its own open in Browse: the
tree on the left, the Document panel in the centre and the inspector on the right. Data holds
text, metadata, string tables, the `.dat` tables, `.rel`, `.gfx` and audio, with the Document
panel and the text editor in its centre. The Document panel (`src/ui/panels/document_view.rs`)
draws the active document as its kind draws itself: an install's, folder's or archive's Files view
([R25d](#r25d-the-files-view-in-the-centre)), the text editor,
the metadata form, the string table, or the inspector body of a kind with no editor of its own,
video and audio among them, while the inspector shows that document's facts
(`a_kind_with_no_editor_of_its_own_is_drawn_once_in_the_centre`,
`video_and_audio_play_in_the_centre_and_never_under_a_timeline`). The gallery sits beside the
preview in Texture and beside the viewport in Model, and Explorer is gone. The workspace on
screen is saved with the dock layouts
(`the_workspace_on_screen_survives_a_save_and_reload_of_the_preferences`), and a project keeps
its own; since [R60c](#r60c-the-front-door) a start that mounts the remembered install opens it in
Browse, and one that opens nothing returns to the saved workspace when it can show the start page. `LAYOUT_SCHEMA` is 2, so a layout saved before is set aside once, and Explorer,
Geometry, Text and Meta read as Texture, Model, Data and Data in a layout or a project
(`a_layout_naming_workspaces_that_are_gone_reads_and_falls_back_to_the_defaults_once`,
`a_workspace_an_older_build_named_reads_as_its_replacement`). It has been seen on `just shots` screens,
a restart that opened where the last run left off among them, and not in a window.

**Done when.** No kind but particle effects opens into the Particle workspace, and the app starts in
Browse.

### R59n Glyphs and feedback by severity

[↑ checklist](#phase-10--the-application)

**What.** Every symbol the shell draws is in its fonts; a toast is coloured by its level and readable
in both themes, and toasts of different heights do not overlap; log lines from other crates, such as
the GPU loader's, stay out of the status bar's error count.

**Where it stands.** Done. `font_definitions` in `src/app/gpu.rs` puts Hack, egui's bundled
monospace font, last among the interface font's fallbacks, so the arrows and geometric shapes Ubuntu
Light lacks (▸, →, ●, ▤) draw, and the clear and close buttons draw Phosphor's X for ✕, which no
bundled font has. `every_symbol_the_shell_writes_is_in_its_fonts` reads every string literal
outside test code in `src/` and finds each non-ASCII character in both families. A toast is a
success, information, a warning or an error (`ToastType`), drawn as the panel tinted toward its
level's colour with the level's icon and border and the theme's own text, at a fixed width that
wraps and is cut at `TOAST_MAX_ROWS` rows, the Log keeping the rest:
`a_toast_reads_at_4_5_to_1_in_both_themes` holds the text to 4.5:1 and the icon to 3:1 against the
fill, and `toasts_of_different_heights_stack_without_overlapping` stacks each toast on the measured
top of the one below. Callers that reported a failure or plain information as a
success say which it is, except the count an install's archive save ends with, which stays a
success when an archive failed beside that archive's own error. A first start logs the extracted
keys, the material colours and the clip-region table instead of toasting them; a failed extraction
still toasts. The status bar's counts and the Log panel's sentence take only the app's and the
`or_` crates' targets, and another crate's line, such as the Vulkan loader's, stays in the Log
panel (`another_crates_error_is_shown_but_not_counted`). On `just shots` screens a first start
shows no error, and since [R59o](#r59o-words-empty-states-and-the-status-bar) no warning either.

**Done when.** A test finds every symbol the shell writes in the bundled fonts, and a clean start
reports no error.

### R59o Words empty states and the status bar

[↑ checklist](#phase-10--the-application)

**What.** One name per action; "archive", not "pack file"; no engine internals in the default views
(RSC7, PSC, TOC, WGSL); the New menu in words ("Particle effects (.ypt)"); one ellipsis and one menu
separator; error titles that name the operation; empty states that offer actions and never point at a
panel that shows another file; a status bar with the game and edition (a switcher), the edit mode,
background jobs and the selection; and the product's own name everywhere, never "Particle Effects
Editor".

**Where it stands.** Done. `src/default_view_strings.rs` reads every string literal the app writes
outside its tests, log lines included and byte strings not, and finds none naming RSC7, PSC, TOC or
WGSL (`no_default_view_string_names_rsc7_psc_toc_or_wgsl`), "pack file", three dots for an
ellipsis, a menu path with another separator than ▸, or "Particle Effects Editor"; the About pages
share the start page's tagline. Close all, which was New Session, is the one name for closing every
file but the install, and Project ▸ New Project, the same action, is gone
(`closing_everything_is_called_close_all_and_the_palette_still_finds_it_as_a_new_session`). The New menu
reads "Particle effects (.ypt)" (`the_new_menu_names_each_kind_in_words`), and the error window
names what failed (`a_failed_save_is_titled_as_a_save_and_a_failed_open_as_an_open`). A workspace
with nothing open, or only an install, folder or archive outside Browse, offers Open file…, the
install and its recent files, in the Document panel where it has one and otherwise in the inspector
(`every_workspaces_empty_state_offers_an_action`); a prompt to select names its file, not the
Hierarchy panel. The status bar keeps R60c's switcher, the extraction's progress (every job is
R60e's), the selection and the log; the edit mode is the menu bar's control (R25g), and the frame
rate the Statistics panel's (`the_frame_rate_is_left_to_the_statistics_panel`). A first start warns
of no layout (`a_first_start_has_no_saved_layout_and_sets_none_aside`), and the tree's toolbar
wraps. Seen on `just shots` screens, not in a window.

**Done when.** A test finds no default-view string naming RSC7, PSC, TOC or WGSL, and every
workspace's empty state offers an action.

### R59p Settings recents and small editor fixes

[↑ checklist](#phase-10--the-application)

**What.** The settings search finds every option, and opening Settings again raises its window; the
viewport's preferences persist; recents show when a file was last opened, drop missing files and
include archive entries; Save As proposes the right name and extension; the text editor wraps or
scrolls sideways and goes to a line; the texture view's 100% stays, its encode options belong to each
texture, and re-encoding a compressed texture warns that it loses detail.

**Where it stands.** Done. The settings search matches every option as its page draws it, the
values of its choices, the games, the file types and the shortcuts' commands, and shows the first
page holding a match (`the_settings_search_finds_every_option`); opening Settings while it is open
asks its window forward, which no window has shown yet. The viewport keeps its mode (not Channel),
background and exposure across a restart, which answers R42b's clause that the mode and the
background persist. Recents are kept under `recents` with where and when each file was opened, the
list a build before 6 kept is read once, a file that is gone is not offered, and an archive entry
reopens through its archive. Save As proposes the file's own name with its extension once. The text
editor scrolls sideways to keep the caret in view, and Go to line… (`Ctrl+G`) moves it to the line
typed. The texture view keeps 100% until another zoom is chosen and encode options per texture, and
warns before re-encoding a compressed texture. Every finding is fixed: a question is an egui modal
(`question` in `src/ui/panels/mod.rs`) that leaves the shell behind it inert, a dropped file
included; the shell loads only Phosphor's Regular font; a line written through the `log` crate is
filed under its own target; Clear on the active game's folder closes its install once its unsaved
work is answered; a switch asked while another close waits says it was not made; an Enhanced file
waits for the switch it asked for and is not opened when that is cancelled; the gallery's and the
texture view's empty text name no extension; the install's heading reads "Game Install"; the
catch-all error names a kind in words; the Search panel keeps room for its selected hit's path; and
Copy path on a match inside archives copies its path in the archive. The recents, viewport,
question and game-folder cases are in `src/state/settings_recents_and_small_editor_fixes.rs`, the
rest beside their code, and each failed before its fix. Seen on `just shots` screens, not in a
window. Found while closing it: the text editor's footer draws over its first line, its find bar
sits at the window's corner over the menu bar, and in a narrow panel the texture view's header row
overlaps and its encoding bar runs past the edge.

**Done when.** Each clause lands with a test, or with `just shots` screens where a test cannot see
it.

### R59q An accessibility baseline

[↑ checklist](#phase-10--the-application)

**What.** Every text colour reaches WCAG AA's 4.5:1 against the panels it is drawn on; nothing a user
needs is only small dim text; keyboard focus always shows; kind icons differ in shape as well as
colour; the shell carries AccessKit labels; every panel works from the keyboard.

**Where it stands.** Done for the colours, the focus ring, the icon shapes and the names; the rest
is listed last. `every_text_colour_reads_at_4_5_to_1_on_the_panels_it_is_drawn_on_in_both_themes`
computes the contrast of every text colour the theme names, of the colours egui reads for its own
text and of a highlighted disabled palette row's faded text, against the four panel colours and a
selected row, in both themes, and `text_reads_at_4_5_to_1_whatever_accent_the_user_picks` does the
same for twelve accents. It found 112 pairs under 4.5:1 before the fix, in the list the test held
then, which has grown, and the old palette lacks the slots added since, so the count cannot be run
again. Muted text is one colour (`TEXT_SECONDARY`, which `TEXT_DIM` and `TEXT_DISABLED` now name),
because any colour that reaches 4.5:1 on a hovered widget leaves no room for a second; the accent
has a text colour of its own (`ACCENT_TEXT`, lightened or darkened until it reads, for any accent);
strong text is black in the light theme, where it was white on white and hid the Settings shortcuts
header, a hovered dock tab's title and the inspector's titles; the selection and the find matches
are opaque, so the colour text sits on is known, and every match has an outline; a disabled control
keeps egui's fade, which WCAG exempts, with no setting to raise it (the owner, 2026-10-05), and
`a_disabled_control_still_reads_as_disabled_in_both_themes` holds to a fade that shows, and only a
disabled palette row under the highlight fades less and draws in the primary colour
(`theme::disabled_alpha_under_highlight`), so that it reads there; the boxes over the viewport are
filled with the panel colour, so their text reads in the light theme too, and the gizmo's letters
are black on a bubble filled opaque, a bubble behind the others being smaller where it was
translucent, which reads on each axis colour where white did not on the green and where a
translucent fill let the scene through (`the_gizmos_letters_read_on_every_axis_colour`). Where the
dim colour was the only mark of a state, the row is italic too: a step undone in the History panel
and a Settings page the search misses
(`an_undone_step_in_the_history_panel_is_in_italics_as_well_as_dim`,
`a_page_the_search_misses_is_in_italics_as_well_as_dim`); the particle outline's unused rules carry
an "unused" badge, and an unbound shortcut reads "(unbound)".
`no_text_the_shell_paints_reads_under_4_5_to_1_in_either_theme` draws every workspace, an inactive
tab under the pointer, the palette, each menu, a folder open with a row picked, each Settings page
and a clip's timeline (a stand-in source with a row picked, hidden, muted and locked, a badge of
every channel format, labelled spans and the curve view), and judges every run of text against the
fills painted before it, apart from a disabled control's at egui's fade, which it exempts, and it
requires the palette to paint a disabled row at the opacity that reads; a kind's icon reaches 3:1
(`theme::legible`). Small text was egui's 9 points and is 10.5 (`SMALL_TEXT`), and the timeline's
chevrons, toggles, format badges, curve axes and span labels, which were set at 8 and 9 points, are
10.5 too: `no_text_the_shell_paints_is_set_under_10_points` finds none under 10 on those pages. A
hidden or muted channel's label kept the secondary colour at half opacity, about 2.5:1, and now
keeps the colour with a line through it; a toggle that is on has a box round it as well as the
accent text colour; a span's label sits on a plate filled like a panel, because no text colour reads
on every span colour; a channel format's badge text is its colour moved until it reads on every
panel. `src/ui/focus_ring.rs` draws a ring around the widget Tab or the arrow keys reached, whether
or not it paints its own focus, and a click takes it away
(`every_widget_tab_reaches_has_a_ring_around_it_in_every_workspace`,
`a_click_takes_the_ring_away_and_the_next_tab_brings_it_back`). The ring hugs the part of the widget
that shows and is drawn in the widget's own layer, so a widget scrolled out of its area keeps none
and a menu opened over it covers it (`a_widget_scrolled_out_of_its_scroll_area_keeps_no_ring`,
`a_menu_over_the_focused_widget_covers_its_ring`). The code editor reads typed text itself, so the
ring treats it as it does an egui `TextEdit`: Space, Enter and the arrows typed in it bring no ring,
and the arrows stay with it, where egui would have moved the keyboard to the splitter beside it
(`typing_in_the_code_editor_draws_no_ring_round_it`). The ring is drawn after the dialogs, so a
dialog's button shows it on the frame Tab reaches it
(`a_dialog_shows_the_ring_round_the_button_the_keyboard_is_on`). Two kinds and two outline rows that
shared a glyph have their own (`no_two_kinds_of_document_share_a_glyph`,
`no_two_rows_of_an_outline_share_a_glyph`). The icon-only buttons of Files and the Files view and
the Settings combo boxes carry names
(`every_control_the_shell_draws_has_a_name_a_screen_reader_can_speak`), and each tab of the dock is
a button named after its panel (`every_tab_in_the_dock_is_a_button_named_after_its_panel`, which
finds the buttons by name because egui_dock describes no tab, so the test before it is given no role
to judge and cannot see one missing).
`tab_reaches_into_the_panels_that_have_controls_before_a_file_is_open` holds that Tab goes into
Files, the Document panel and the 3D viewport. Seen on `just shots` screens in both themes, not in a
window. Left, besides what [R68a](#r68a-what-the-accessibility-baseline-still-leaves) took over (the
tree reaching a screen reader, the Tab stops with no name in the dock and the Files view's table,
the density graph and curve handles with no description, and the literal colours of the timeline's
marks and the gizmo's axes): the code editor describes no AccessKit node, so a screen reader is told
of no text field, and the Tab that brings the keyboard into it inserts four spaces in an editable
document, because the editor reads the key on the frame it gains the keyboard; the panels that need
a file open (outline, gallery, texture preview, inspector fields) are tested only in their empty
states, the timeline only against a stand-in source, and the 3D viewport's keys still act only under
the pointer ([R59i](#r59i-shortcuts-by-character-live-while-typing)), so a keyboard user cannot
frame or orbit; no test frame has a renderer, so the viewport's boxes, HUD and gizmo are not drawn
by any scan and read at 4.5:1 by construction (`theme::overlay_bg`) and on `just shots` screens at
the default background, which the user can change, and the gizmo's letters are judged on the fill
they are drawn on and not on the scene behind it; the viewport's shading icons and the renderer's
own axes are not judged as graphics; and only icons within the kinds and within each outline share
no glyph, not across them. What its checks cannot see is
[R68g](#r68g-what-the-accessibility-checks-still-cannot-see)'s, and the code editor's node, the Tab
that inserts four spaces and the panels that need a file open are listed under
[R68a](#r68a-what-the-accessibility-baseline-still-leaves).

**Done when.** A test computes every text colour's contrast against the panel colours it is drawn on,
in both themes, and finds none under 4.5:1.

### R59r Context menus from the command registry

[↑ checklist](#phase-10--the-application)

**What.** Every context menu draws its app commands from the registry: the tree's rows, the search
results, the tabs and the particle tree offer Open, Reveal, Extract, Rename, Delete and the like
through commands that take what was clicked (a row, a hit, a tab), with one label and one binding
each. The menu bar's edit-mode control and the workspace buttons are commands too, and a plugin
registers as many commands as its guest describes.

**Where it stands.** Done. A command can act on something: its `On`
(`src/ui/commands/table.rs`) gives it an enabled-when and a run over a `Target`, the rows of Files
or of the Files view, the records ticked in a form, a search's results, a document or a row of a
particle file's outline, and says what the menu bar, the palette and its key act on: the focused
panel's selection, else the active document (`src/ui/commands/target.rs`). Every context menu
offers its app commands on what was clicked, each with the registry's one label, binding and
enabled-when: the rows of Files and of the Files view, a selection's menu, the Files view's empty
space, a document's row, the results of the Search panel and of a search inside archives, the tabs
and the particle outline's effects and rules. Open, Reveal in tree, Reveal in Windows Explorer,
Extract…, Export as…, Rename (`F2`, in Files as in the Files view), New folder, Copy into archive…, Copy
path, Copy name, Copy hash, Properties, Add to and Remove from favourites and Close others are
commands (`src/ui/commands/files.rs`), Close document closes the tab clicked, Delete takes a
particle file's effect or rule, and a double click on a row or a result runs Open.
`every_context_menu_offers_its_app_commands_with_the_registrys_label_binding_and_enabled_state`
walks fifteen menus through AccessKit with a key bound to every command, and finds each app
command with the registry's label, binding and enabled state for what was clicked and nothing else
but the items named as the menu's own;
`a_command_acts_on_the_row_clicked_in_a_menu_and_on_the_selection_from_the_palette_and_its_key`
holds the three ways in. The edit mode's three modes are commands the menu bar's control draws and
the palette lists, and the workspace buttons draw and run the workspace commands
(`the_edit_modes_and_the_workspaces_are_commands_the_menu_bar_and_the_palette_both_run`). A menu's
own items are those one kind of row has or that list choices: Activate, New file, Move to, the
other ways to extract, Attach as texture dictionary, the mods overlay's copy and show,
Defragment…, the particle outline's Add, Duplicate, Bind and Find usages, and since
[R26b](#r26b-uses-and-used-by-in-the-app) a model's or map's Uses and Placed by. A plugin's
commands, as many as its guest registers, join the registry
([R45](#r45-the-plugin-sdk-and-registration-surface)), and run from the palette, Tools → Plugin
commands and the shortcut the guest asked for. Seen on `just shots` screens, not in a window.

**Done when.** A test walks every context menu and finds each app command in it with the registry's
label, binding and enabled state.

### R59s Nothing draws over what it belongs to

[↑ checklist](#phase-10--the-application)

**What.** No panel draws one of its parts over another or over the shell: the text editor's footer
sits below its lines, the find bar inside the editor, the texture view's header and encode bar
within a narrow panel, the viewport's overlay within its box, and toasts clear of the job list.

**Where it stands.** Done. The text editor draws its footer first, in a strip of its own below the
lines, so the footer shows the caret as the last frame left it; a narrow editor wraps the caret,
counts, line ending and encoding and cuts the name short. The find bar is anchored inside the
editor's lines, as Go to Line was, and its rows wrap. The texture view's header and encode bar wrap,
their labels truncate rather than flow across rows, and each combo box gets a slot of its own size,
since egui lays one out in what is left of a row; the view controls now follow the texture's details
rather than the right edge. The viewport's overlay sizes its box from its text. Toasts stack left of
the job list while it is open. `src/state/nothing_draws_over_what_it_belongs_to.rs` draws each where
the default layout puts its panel on the default window and again as narrow as the Inspector's
column at the smallest window, the toasts on both windows, and finds no two parts overlapping and
none outside the panel; each test failed before its fix. Seen on `just shots` screens, not in a
window.

**Done when.** A test draws each of these at the default size and at a narrow one and finds no two
of their rectangles overlapping and none outside its panel, and `just shots` screens show each.

### R59t The default views in a modder's words

[↑ checklist](#phase-10--the-application)

**What.** What a view shows before Show internals is on is in a modder's words and complete: a
field that holds a hash reads as one whatever the engine calls it, the metadata panel's header
names the file without its binary format's code or its schema's path, the geometry view leaves
vertex strides to Show internals, Show internals stays as the user left it across a restart, an
archive on disk not yet opened shows its size, and a file row's inspector names what the file is.

**Where it stands.** Done. A field the declaration types `Hash32` reads as a hash whatever the
engine calls it: the macro lists such fields, `Reach::hash_fields` hands the list to the grid, and
the engine's name stays the rule for a field typed otherwise, so an Enhanced shader's name and
preset and a particle's property id read `0x` and eight digits
(`every_corpus_kind_draws_each_hash_as_a_hash_and_no_format_code_schema_path_or_stride_in_a_default_view`
walks a staged file of every kind of both targets, and found them as decimal numbers while the grid
went by the engine's name alone). The metadata panel's header names the file's type and leaves its
container's code (PSO, RBF or RBM) and its schema's path to Show internals
(`a_metadata_files_format_code_shows_under_show_internals_and_not_before`), as the geometry view
leaves the vertex stride (`a_geometrys_vertex_stride_shows_under_show_internals_and_not_before`).
Show internals is a preference, `Prefs::show_internals`, kept as the user left it
(`show_internals_stays_as_the_user_left_it_across_a_restart`). The inspector of an archive on disk
not yet opened shows its size, read from its file or from its entry in the archive around it
(`an_archive_on_disk_not_yet_opened_shows_its_size_in_its_inspector`), and a file row's is headed by
what the file is, "File" where nothing names it, with how the archive stores it behind Show
internals (`a_text_files_inspector_is_headed_as_text_and_a_file_nothing_names_as_a_file`). The glTF
questions name their choices in words alone (`no_question_the_app_asks_names_a_command_line_flag`),
and so does the refusal of a channel the mesh leaves out, which the import's question shows as the
library words it: it offers to accept a stated default, where it told a command line to pass a flag
(`a_channel_the_mesh_leaves_out_is_refused_in_words_and_never_by_a_flag`, and the library's
`a_channel_the_geometry_declares_and_the_mesh_omits_is_refused`). The import's report counts one in
the singular, after an edit of the contents the Edition row says which version a save would write
and which the file has
(`after_an_edit_of_the_contents_the_edition_row_says_the_version_a_save_writes_is_not_the_files`),
and once a save has rewritten the file, even while an edit came after it began, the row names only
the version the file now has
(`after_a_save_the_edition_row_says_the_version_the_file_now_has_and_no_other`,
`an_edit_made_while_a_save_is_written_leaves_the_edition_row_no_old_version_of_the_file`). A clip
dictionary lists its two flags once
(`a_clip_dictionarys_facts_under_show_internals_do_not_repeat_rows_of_its_grid`). A comparison names
the two files it compares where a path would go, in the status bar and its tab's hover
(`a_comparison_says_what_it_compares_where_a_path_would_go`), its Outline says it compares two files
and where their differences are, and a texture whose pixels changed reads "of pixels" beside "of
different pixels" (`a_texture_whose_pixels_changed_says_the_pixels_differ_rather_than_two_equal_sizes`).
Each test failed before its fix but three, which no fix was taken out to see fail: the first Edition
row's (the two that follow a save did fail, on the row that kept the file's old version), the
report's singular (`a_report_counts_one_of_anything_in_the_singular`) and the vertex stride's own,
which the walk's finding of a stride before the fix stands in for. The restart's failed with the
saved preferences deleted after the click, "a restart turned Show internals off". Seen on
`just shots` screens, not in a window, but for the hashes, the Edition row and a changed texture's
pixels. A clip dictionary's "Hash buckets" repeating the grid's slots, the particle model's second
index count and the "no schema" hover's path in the repository were left here and are fixed in
[R59z](#r59z-small-faults-left-in-the-default-views-menus-and-keys). Left: a save of a file the
document left unedited can write a version other than the one it opened with, which the Edition row names only once the contents are edited; and an undo past a
save puts back an older image, so the row then names that image's version and not the file's, as
does a live reload of a `.ypt` (`reload_ypt_from_disk`), which drops the file's version.

**Done when.** A test walks every corpus kind of both targets and finds every hash field drawn as a
hash and no format code, schema path or stride in a default view; a test turns Show internals on,
restarts and finds it on; a test selects an unopened archive on disk and finds its size; and a test
selects a text file's row and finds its inspector headed as text; and a test finds no command-line
flag in the app's questions.

### R59u Small faults in the editor and the previews

[↑ checklist](#phase-10--the-application)

**What.** The find bar says nothing until it has searched; Go to Line's field fits a narrow editor;
no wrapped row starts with a separator; with a texture dictionary active, the Particle workspace's
texture preview shows it; the Files view's `Delete` follows the user's binding as every other key
does; a folder picker says what it picks a folder for; an Open Folder document's own row offers
what any folder's row offers; an open list closes when the document under it changes; and the
Outline starts each document at its top. The last two were found while closing
[R42b](#r42b-viewport-basics-for-inspecting-assets): a level-of-detail list stays open across a
document switch, and the Outline's scroll carries from one document to the next.

**Where it stands.** Done. The find bar's status row says nothing until the search for what is
typed has run (`FindState::searched`), and then a count or "No results". Found as "No results"
during the 150 ms wait, it was worse: the wait was stamped again on every frame the query differed
from the last one searched, so a new query was never searched at all. It now waits once for a pause
in the typing and asks for a repaint when the wait ends. Go to Line's field wraps within what is
left of the editor, as the find bar's rows do. Each separator of the texture view, in its header
and in the encode bar, leads a unit that wraps whole (`kept_together`) and is left out when the
unit starts a row, so no row starts or ends with one at any width from 150 to 520 points. The
Particle workspace's texture preview shows whatever the Texture workspace handles, so a `.ytd`
active there is previewed. The Files view no longer reads `Delete` itself: the key dispatcher runs
Delete on the key the user bound it to, as it runs F2. A DLC pack's folder is asked for as "Create
DLC Pack Into Folder", and an Open Folder document's own row in Files offers Create DLC pack…. A
list open when the active document changes is closed, and so is a menu of the document left
(`close_lists_over_another_document`, which the level-of-detail list needed; a menu of the shell or
of another document stays open since [R59z](#r59z-small-faults-left-in-the-default-views-menus-and-keys)), each document's
Outline keeps a scroll offset of its own, and Go to File's rows cut the place short so the name
keeps one line. `src/state/small_faults_in_the_editor_and_the_previews.rs` holds
`the_find_bar_says_nothing_until_it_has_searched`,
`the_find_bar_says_no_results_only_once_a_search_found_nothing`,
`go_to_lines_field_stays_within_a_narrow_editor`,
`no_row_of_the_encode_bar_starts_with_a_separator`,
`a_texture_dictionary_active_in_the_particle_workspace_shows_in_its_texture_preview`,
`an_open_list_closes_when_the_document_under_it_changes` and
`the_outline_starts_each_document_at_its_top`; the others are
`the_files_view_deletes_on_the_key_the_user_bound_to_delete_and_no_other`,
`a_folder_opened_with_open_folder_packs_from_its_own_row`,
`the_dlc_packs_folder_picker_says_what_it_picks_a_folder_for` and
`a_files_name_stays_whole_on_one_line_however_long_the_folders_it_is_in`. Each failed before its
fix. Seen on `just shots` screens, not in a window; a script cannot press `Delete` through the key
dispatcher, so its screens show only that the panel no longer acts on the key. Not done here and done in
[R59z](#r59z-small-faults-left-in-the-default-views-menus-and-keys): an Open Folder document's row
offered Create DLC pack… but not the rest of a folder row's menu (Reveal in Windows Explorer, Copy
path, Properties), and a key press that a disabled command refuses said why in the log and nowhere
else, so `Delete` with several rows selected no longer toasted. Not done: the list test opens a
stand-in list, since the level-of-detail switch needs a drawable with levels.

**Done when.** A test types a query and finds no "No results" before the search has run; a test
draws Go to Line and the encode bar in a narrow editor and finds nothing past the edge and no row
starting with a separator; a test opens a `.ytd` in the Particle workspace and finds a texture
shown; a test binds `Delete` to another command and finds the Files view deletes nothing; and a
test finds Create DLC pack on an Open Folder document's own row.

### R59v Screens never reach the desktop

[↑ checklist](#phase-10--the-application)

**What.** A `just shots` script runs every command as the app does, but nothing it does reaches past
the app: Reveal in Windows Explorer, Open folder, Open with, the Recycle Bin and the clipboard go to
stand-ins that record what they were asked, as the tests' clipboard does.

**Where it stands.** Done. A self-check's first act is `stand_in_for_the_desktop`
(`a_self_check_stands_in_for_the_desktop_before_anything_else`), and from then every call past the
app goes to the stand-in in `src/platform/` that a test build uses: Reveal in Windows Explorer and
Open folder tell the file manager's; a delete tells the Recycle Bin's, which leaves the file where it
is; a copy of files is held by the clipboard's, and a copied text is only logged by the `just shots`
frame, so the system clipboard keeps what it held; a file dialog picks what a script's `pick` step
named, or is cancelled (`a_file_dialog_answers_what_a_script_named_and_otherwise_cancels`); and
Settings ▸ File associations, which adds OpenRage to Windows' Open With, opens no registry
(`the_page_leaves_the_users_registry_to_a_stand_in`). Each stand-in logs what it was asked, which the
run prints and the Log panel shows (`every_stand_in_says_what_it_was_asked`), and
`nothing_outside_the_platform_layer_reaches_past_the_app` refuses a file dialog, the clipboard, the
file manager or the shell called from anywhere else in `src/`. No command opens a row in another
program yet ([R25d](#r25d-the-files-view-in-the-centre)), so Open with has nothing else to stand in
for. A `just shots` script that clicks each of these printed, and showed in the Log panel, what each
stand-in was asked; it started no Explorer, and left the deleted file in its scratch folder and the
Recycle Bin and the clipboard's sequence number as they were.

**Done when.** A script that runs each such command opens no window, touches no file outside its
scratch folder and leaves the clipboard as it was, and the screens show what each was asked.

### R59w A self-check keeps to itself

[↑ checklist](#phase-10--the-application)

**What.** A self-check, plain or with shots, writes no configuration but its scratch folder's and
plays no sound: the shader tools go under the scratch configuration, audio plays to a silent sink,
a toast says what a stand-in did, and rows copied from an archive stage in the run's own folder.

**Where it stands.** Done. The shader pack and the tools that bake it sit under `config_root`, as
the preferences do, so a self-check's are in its scratch folder, and the editor reads the pack where
the bake writes it. From `stand_in_for_the_desktop` on, Bake game shaders asks the bake's stand-in,
which downloads and bakes nothing, since a bake is 26 MB of tools and minutes of work no script
checks; and a stream plays to `NullSink`, the speaker's stand-in, which sounds nothing. Each logs
what it was asked. Rows a copy writes out stage in the run's own folder, out of reach of the
editor's sweep of day-old copies, and the toast of a rebake, a delete or a copy says what the
stand-in did, a delete's that nothing was moved. One test per action runs it as a self-check does:
`a_self_checks_stream_plays_to_a_silent_sink`, `a_self_checks_rebake_starts_nothing_and_writes_nothing`,
`a_self_checks_delete_says_nothing_was_moved` and `a_self_checks_copy_stages_in_its_own_folder`.

**Done when.** A test runs a self-check that plays a stream, starts a rebake, deletes a row and
copies one, and finds the owner's configuration, output device and staging folder untouched and
each toast saying what its stand-in did.

### R59x Self-checks and tests leave nothing behind

[↑ checklist](#phase-10--the-application)

**What.** A test run and a self-check remove what they made, the first-launch bake stands in under a
self-check as a rebake does, and a script's `open` of a folder waits for its rows.

**Where it stands.** Done. A test build's configuration is a folder `openrage-test-…` under the
temporary folder, which `src/state/test_scratch.rs` removes as the process exits, a failed test's
included: a `TempDir` held in a static was never dropped, and nextest starts a process per test.
`a_test_process_leaves_no_folder_in_the_temporary_folder_when_it_ends` runs two tests, one building
the sample plugin guest, in a process whose temporary folder is its own and finds it empty after,
and a second test (removed 2026-10-09: Cranelift cannot unwind) did the same for a
failing test, whose process libtest ends with `ExitProcess` on Windows, skipping `atexit`. The
guest's build cache moved from `%TEMP%\openrage-plugin-sample` into the test binary's target
directory, and a whole `cargo test --bin openrage` with `TEMP` redirected left nothing. In a
self-check `maybe_auto_bake` asks the bake's stand-in before it installs a tool
(`a_self_checks_first_launch_bake_asks_the_stand_in_once`), the silent sink
holds only the stream sounding (`a_silent_sink_keeps_only_the_stream_that_is_sounding`), and a
script's `open` waits while its document's folders are listed
(`a_scripts_open_of_a_folder_waits_for_its_rows_so_a_right_click_at_once_finds_the_menu`). Left:
what earlier runs put in `%TEMP%` stays for the owner to delete; other crates' tests, and a self-check
that stops early, left folders of their own there then, and [R59y](#r59y-what-a-test-run-or-a-self-check-still-leaves-behind) settled both.

**Done when.** A test run leaves no folder of its own in `%TEMP%`, a test finds a self-check's
first-launch bake asking the stand-in, and a script that opens a folder and right-clicks a row at
once finds the row's menu.

### R59y What a test run or a self-check still leaves behind

[↑ checklist](#phase-10--the-application)

**What.** A self-check that stops early, a test run of any crate and a test that persists a
preference leave nothing in the owner's temporary folder or in the way of the next test, and a
script's `game` step waits for its listing.

**Where it stands.** Done. `run` in `src/self_check.rs` holds a `ScratchToRemove` guard from its
first line, so `openrage-self-check-<pid>` goes however the run ends, an early return and a panic's
unwinding included (`a_self_check_refused_at_its_first_check_removes_its_scratch_folder`);
a second test (removed 2026-10-09: Cranelift cannot unwind) ran a
self-check whose control panel fails in a process of its own, since a self-check replaces the panic
hook and registers panels for good, with an empty temporary folder, and finds it empty after. Every
crate's tests now make their folders with `tempfile`, which removes them on drop, a failed test's
included: 58 calls of `temp_dir()` in 38 files at `a3e0fe57` are 14 in 10, each named with its reason in
`no_test_makes_a_folder_in_the_temporary_folder_that_nothing_removes`, which fails on a new one in any
crate, and skips the folders a harness keeps inside the checkout (`.claude`, `.cursor`), each a full
copy of the repository, and cargo's `target-*` folders
(`a_harness_worktree_and_a_cargo_target_folder_are_not_workspace_code`). Runs of the app's tests with `TEMP` pointed at an empty folder found what that scan cannot:
between 1 and 8 test processes in a run of 1,520 left their `openrage-test-…` folder. A process
whose worker is in the loader as it ends never gets its `DLL_PROCESS_DETACH`, which the removal
waited for, so it now also runs from `atexit`, which runs first when `main` returns
(`a_test_process_that_ends_while_a_worker_loads_libraries_leaves_no_folder_in_the_temporary_folder`);
and `atexit` alone left 8 a run, because the game-discovery worker of every `AppState` was writing an
install's icon into the folder as the process ended, so a test build's discovery leaves the installs'
executables alone. Ten runs of the 1,520 after that left nothing.
A test thread's `prefs.json` is a folder of its own under that scratch, so a test that saves a
preference, or finishes a save, leaves the next `AppState::default()` as it was
(`a_test_that_saves_a_preference_leaves_the_next_default_state_as_it_was`); `keep_prefs_in` stays for
a test that seeds the folder or reads it back. `a_self_checks_copy_stages_in_its_own_folder` failed
when a dead process's entry in the editor's `%TEMP%\openrage-clipboard` carried this process's id,
which Windows hands on to a later one (331 entries under 119 ids sit there, from before this change,
and no test stages there); it now counts what that folder gained under its own id since the test began
(`an_entry_a_dead_process_left_under_this_ones_id_is_not_what_this_one_staged`). `staging_dir` adds a
count to its names, because the clock ticks in 100 ns on Windows and two asks that read one stamp were
given one folder (`two_copies_in_one_tick_of_the_clock_stage_in_folders_of_their_own`). The `game` step of
`just shots` draws frames until its install's rows are listed, as `open` does
(`a_scripts_game_step_waits_for_the_installs_rows_so_a_right_click_at_once_finds_the_menu`). The
preference, staging and `game` tests were each run against `main` at `a3e0fe57` and failed for the
reason they name: the next default state read a scale of 150 where it was 100, one name came out
twice, and the click found "Reading this folder…" where the row should be. Left:
what earlier runs put in `%TEMP%` stays for the owner to delete (53,439 `.tmp*` folders, whose maker
is not found, 1,020 `or_io_texture_dict_*`, the other crates' test folders and the 331
`openrage-clipboard` entries); a test that fails still ends its process with `ExitProcess`, so a
failure with a worker in the loader can leave its scratch; the ignored scaffold test keeps
`openrage-scaffold-check` between runs on purpose; and three plugin tests name a path under `%TEMP%`
and write nothing there.

**Done when.** A self-check stopped by its control panel failing leaves no folder of its own in
`%TEMP%`; every crate's test run leaves none either; a test that toggles a persisted preference
leaves the next `AppState::default()` as it was; two copy tests in one process stage in folders of
their own; and a script whose `game` step is followed at once by a right click finds the menu.

### R59z Small faults left in the default views, menus and keys

[↑ checklist](#phase-10--the-application)

**What.** An Open Folder document's own row offers what a folder row does to the folder opened
(Reveal in Windows Explorer, Copy path, Properties) and the palette packs the folder opened, a key a
disabled command refuses says why (bar Cut, Copy and Paste), a menu held open while a load makes another document active
closes only if it belonged to the document left, no default view repeats a row, says "1 models" or
tells a path in the repository, the status bar shows no warning on a clean start or names the one it
counts, and an address typed into the Files view cannot panic.

**Where it stands.** Done but for the faults listed last. An Open Folder document's own row offers
Reveal in Windows Explorer, Copy path and Properties beside Create DLC pack…, and each acts on the
folder opened: Properties on a document's row shows the root of its tree (`row_or_root`)
(`an_open_folder_documents_own_row_reveals_it_copies_its_path_and_shows_its_properties`, and the
registry's walk of every context menu expects the four). The palette's Create DLC pack… packs the
folder opened when an Open Folder document is active and no row is chosen, its target being
`target::selection_or_folder_opened` where it was `target::selection`
(`the_palette_packs_the_folder_opened_when_no_row_is_selected`). A key a disabled command refuses
toasts why, once while the toast is up, so `Delete` over several rows says "Several rows cannot be
deleted at once yet" again
(`delete_pressed_with_several_rows_selected_says_why_nothing_was_deleted`), but not for Cut, Copy and
Paste, which egui also takes on its own, a selected label's text copying on Ctrl+C, so a refusal of
those follows no fault
(`a_clipboard_key_refused_for_want_of_a_selection_says_nothing`). A popup's contents say
whose it is while they draw, `popup_of_the_shell` for the menu bar's menus, the hidden tabs' list and
the game switcher and `popup_of_a_document` for a tab's and a Files row's menu, and a change of
document closes a popup unless its contents named the shell or a document that is not the one left:
a menu held open while a load finishes stays open, a tab's own menu closes with the tab's document,
and a list in a document's panel, which names nobody, closes as it did
(`a_menu_of_the_shell_stays_open_when_a_load_makes_another_document_active`,
`a_tab_menu_stays_open_when_the_load_leaves_another_document`,
`a_tab_menu_closes_when_a_load_leaves_the_document_it_belonged_to` and
`an_open_list_closes_when_the_document_under_it_changes`). No default view says "1 models": the
Outline's LODs row and the Gallery's toolbar count with `plural`, the toolbar's textures too
(`a_drawable_of_one_model_says_1_model_in_its_outline_and_its_gallery`), as do the Outline's count
and the status bar's summary of a texture dictionary, Legacy and Enhanced
(`a_dictionary_of_one_texture_says_1_texture_in_its_outline_and_its_summary`); a particle model's
geometry states its index count once (`a_particle_models_geometry_states_its_index_count_once`), a clip
dictionary's hash buckets stay in its grid
(`a_clip_dictionarys_facts_under_show_internals_do_not_repeat_rows_of_its_grid`), and the metadata
header's "no schema" hover names no path in the repository
(`the_no_schema_hover_names_no_path_in_the_repository`).
`every_corpus_kind_states_no_row_twice_counts_one_model_in_the_singular_and_names_no_repository_path_in_a_default_view`
walks a staged file of every kind through its inspector, its Outline and its Gallery, and found "1
models" in the Gallery and the Outline of the files with one model. A clean start counts nothing: on
`just shots` screens an install with no executable, Legacy or Enhanced, and one with a model open
draw "Log" with no count, as `a_clean_start_on_a_synthetic_install_with_no_keys_counts_no_warning`
finds, so the count of one that [R26c](#r26c-uses-reaches-every-pack) saw was not reproduced; the
install that does show one is a stub `GTA5.exe`, whose key extraction fails and counts as an error,
and the log entry's hover now names it, "The latest: Key extraction failed…"
(`the_one_warning_a_start_counts_is_named_on_hover`, from `LogBuffer::latest_own_serious`).
`inner_of_address` sliced the typed address by the root's length in bytes and panicked when that
fell inside a letter of it; it refuses the address as it refuses any outside the root
(`a_typed_address_the_roots_length_cuts_inside_a_letter_of_is_refused_not_a_panic`). Each new test
failed against `main` at `aed2437e` for the reason it names (a panic at byte 13 of "D:/Spiele/ÄÖ", no
Reveal in the row's menu, no toast, a File menu closed under the pointer, "Hash buckets" repeating
"Clips > Slots", both index counts drawn, the Outline saying "1 models", the hover with no name),
but two: the clean start's, which passes before and after and pins it, and the tab menu that closes
with its document, which held before and still does; the hover's name was seen to fail with it taken
out. Seen on `just shots` screens, not in a window: the folder's row menu and its Properties
window, a drawable's Outline and Gallery saying "1 model", and the hover.

Left: the same plural fault in the other counts, "1 drawables", "1 bones", "1 children", "1 filters"
and "1 expressions" (`src/state/capabilities.rs`, `src/ui/panels/hierarchy/ydd.rs`,
`src/ui/panels/hierarchy/yed.rs`, `src/ui/panels/hierarchy/yfd.rs`,
`src/ui/panels/hierarchy/yft.rs`, `src/ui/panels/hierarchy/drawable.rs`,
`src/ui/panels/preview_settings.rs` and `src/ui/panels/timeline_ycd.rs`); a list in the Settings
window, or in a panel that is no document's, still closes when the active document changes, since
only the menus above name an owner; `drive_find` takes two unused parameters, `_keys` and
`_read_only`, and dropping them leaves `FindKeyOutcome`'s `next` and `prev` unread; Go to Line's
field passed the edge only below about 160 points in the tests, against the R59s narrow size of
about 279 points, so the row's "about 200 points" is looser than what its test measures; the
import's refusal line still begins "Invalid data: lod0/model0/geometry0:", the library error's
display prefix and an internal address, which a modder reads in the import question; the
`or-interchange` refusal no longer tells a CLI user the flag in the error itself ("or accept a
stated default"), and `openrage-cli mesh import --help` lists `--fill-missing` in the same words,
where a typed refusal would let each front end word its own way out; a save of a document left
unedited writes the canonical version, which can differ from the version it opened with, while the
held image keeps the opened one, so the Edition row names the opened version after such a save, as
it did before [R59t](#r59t-the-default-views-in-a-modders-words), whose claim is limited to an edit
of the contents; the status bar's hover on a comparison says "Click to copy full path" and a click
copies the whole "A compared with B" sentence; and an Enhanced shader name and preset and a
particle's property id are reachable in the UI only through an Enhanced `.ypt`'s described outline,
many expansions deep, since the Enhanced `.ydr` and `.ydd` outlines go to the material editor, which
already printed hex.

**Done when.** A test right-clicks an Open Folder document's own row and finds Reveal in Windows
Explorer, Copy path and Properties; a test packs the folder opened from the palette with no row
selected; a test presses `Delete` with several rows selected and finds a toast saying why nothing
was deleted; a test holds a menu open while a background load makes another document active and
finds it closed only if it belonged to the document left; a test walks every corpus kind and finds
no row stated twice, no "1 models" and no repository path in a default view; a test starts the
app on a synthetic install with no keys and finds the status bar counting no warning, or naming the
one it counts; and a test types an address whose root's length falls inside one of its letters, such
as "D:/Spiele/ÄÖ" against a root of another length, and finds it refused with no panic.

### R60 Onboarding permissions and crash reporting

[↑ checklist](#phase-10--the-application)

A programme of parts that share a title more than anything else: onboarding
([R60a](#r60a-onboarding-and-permissions)), a flow over install discovery, with its front door in
[R60c](#r60c-the-front-door); diagnostics ([R60b](#r60b-diagnostics-a-user-can-read)); unsaved work
that survives closing ([R60d](#r60d-unsaved-work-survives-closing-and-quitting)) and a marker that
says when it is unsaved ([R60f](#r60f-the-unsaved-marker-tells-the-truth)); one place for
background jobs ([R60e](#r60e-one-place-for-background-jobs)); and no save or other write that replaces a
file a job reads ([R60g](#r60g-no-save-replaces-a-file-a-job-reads),
[R60h](#r60h-every-write-asks-the-jobs-reading-it),
[R60i](#r60i-what-a-write-still-does-not-ask-after-r60h),
[R60j](#r60j-what-a-write-still-does-not-ask-after-r60i)). OpenIV's permission fixer is why it never needs
administrator rights; the console makes
[R29](#r29-kernel-manners-diagnostics-probing-and-provenance)'s diagnostics reachable by a user.
Crash reporting stays local and user-initiated: telemetry is an anti-goal.

**Covers.** U-199, and the unnumbered onboarding-permissions row.

### R60a Onboarding and permissions

[↑ checklist](#phase-10--the-application)

**What.** A first-run flow that detects the install, explains the overlay, and repairs filesystem
permissions on the game folder instead of demanding elevation.

**Where it stands.** `crates/title/or-title/src/install/discovery.rs` probes Steam, the Rockstar
launcher and Epic, `src/state/game_discovery.rs` runs it off the UI thread, and
`src/ui/panels/game_picker.rs` renders the result as the start page inside the main window, the
front door [R60c](#r60c-the-front-door) built: a first run reaches a browsable install in one click
and a restart in none, so the Done-when's first two clauses hold. Nothing on a first run explains
the overlay, and nothing reads or repairs the game folder's permissions.

**Depends on.** [R49d](#r49d-the-game-side-loader): what a first run has to explain is whether the
install will read an overlay at all, which is the loader's decision.

**Done when.** A new user reaches a browsable install without reading anything, a returning user
does not repeat the first-run flow, and OpenRage never needs administrator rights to write where it
says it can. Permission repair is Windows ACL work, so on the platforms [R58](#r58-cross-platform)
opens it degrades to an honest explanation.

### R60b Diagnostics a user can read

[↑ checklist](#phase-10--the-application)

Done in #180. `src/diagnostics/` keeps every `tracing` event in two bounded rings,
`SERIOUS_CAPACITY` for warnings and errors and `ROUTINE_CAPACITY` for the rest, so a chatty debug
stream cannot push out the error a user wants to report; the Log panel filters, copies and states
the bound, and toasts are logged as they are shown. A panic writes `<config>/crash-reports/crash-*.txt` and a window shows
it on the next frame. The hook takes locks only with `try_lock` and never logs
(its test was removed 2026-10-09: Cranelift cannot unwind). How it looks has not been checked by
a human.

### R60c The front door

[↑ checklist](#phase-10--the-application)

**What.** A restart opens the last game's install in the Files view with no picker. A first run, or
one that finds no install, shows a start page inside the main window: the installs found, Open file,
Open folder for a mod or a FiveM resource, recent files, folders and projects, and the state of the
keys. The game switches from the status bar.

**Where it stands.** Done. The picker window is gone. A launch whose remembered install is still
on disk opens into Browse with that install active and its tree open, whatever workspace was last on
screen (`a_restart_opens_the_remembered_install_in_browse_with_its_tree_open`). While nothing is
open, the Document panel is the start page (`src/ui/panels/game_picker.rs`, with its ways in and its
project cards in `src/ui/panels/start_page.rs`): New project…, Open project…, Open folder… for a mod
or a FiveM resource and Open file…; the goals of a first launch, or the recent projects as cards; each
install found, one click to open, and a folder to choose for a title not found; the recent files and
folders; and whether the keys are ready
(`a_launch_with_no_install_to_open_starts_on_the_start_page`,
`a_first_run_reaches_the_install_it_found_in_one_click`). A launch with nothing to open, left in a
workspace without a Document panel, opens in Browse. A launch with no path named reopens the project
that was open when OpenRage last ran, as a setting says, and the page is one action away from
anywhere ([what the front door builds](Workbench_UX_Design.md#what-the-front-door-builds)). The status bar names the game and lists the
installs found; a switch closes the open install and the files opened from it through R60d's close
request, then opens the new one active
(`switching_the_game_asks_before_it_closes_a_file_opened_from_the_old_install`,
`the_status_bar_names_the_game_and_switches_to_an_install_found`). New session and opening a project
keep the install mounted, and a project no longer lists it among its files
(`a_new_session_and_an_opened_project_keep_the_install_mounted`). In Browse a click on an archive's
row makes that archive the active document, so the row previews whatever was active; in any other
workspace a click only selects it, and a drawable in Model stays on screen
(`a_row_clicked_in_browse_previews_and_elsewhere_the_document_on_screen_stays`). The self-check
draws the start page, the open switcher and an install in Browse, and all three are on `just shots`
screens; the launch has not been tried in a window.

**Done when.** A new user reaches a browsable install without reading anything, and a restart reaches
it with no click; that is [R60a](#r60a-onboarding-and-permissions)'s first clause, and permission
repair stays R60a's.

### R60d Unsaved work survives closing and quitting

[↑ checklist](#phase-10--the-application)

**What.** Every path that would drop a document with unsaved work asks to save it, discard it or
cancel: closing a tab by any means, New session, opening a project, closing the window, Exit, and a
reload because the file changed on disk. Quitting asks once and lists the files. New session gives up
`Ctrl+N`. A session that comes back after a restart is later work, in the project file, where
per-document state belongs (2026-09-23).

**Where it stands.** Done. Every path goes through one close request in `src/state/closing.rs`:
the six close paths (`close_document`), Close others, New session, opening a project and exiting (the
window's close button, File ▸ Exit and the palette's Exit) drop their documents at once when none is
unsaved, and otherwise wait on the "Unsaved changes" prompt in `src/app/modals.rs`, which lists
every unsaved file, the entries opened from inside a closing archive included, and answers Save,
Don't save or Cancel. Save runs each file's `Ctrl+S` save in turn, its prompts included, and a save
that fails or is declined closes nothing; the editor exits only once its close has gone ahead. A
`.ypt` that changes on disk while it holds unsaved edits asks whether to reload over them.
`a_modified_document_survives_every_path_until_the_user_answers` replaced the test that held the old
behaviour. Pose keys not yet baked count as unsaved; an install or a folder counts only archives
not yet written, because it writes a disk edit as it happens. New session asks whatever starts
it, and has no default binding since [R59i](#r59i-shortcuts-by-character-live-while-typing). The prompt is on `just shots` screens; the window's own close button has not
been checked by a human.

**Done when.** That test is replaced by one in which a modified document survives every path until
the user answers.

### R60e One place for background jobs

[↑ checklist](#phase-10--the-application)

**What.** An indicator in the status bar opens a list of every running job (extraction,
defragmenting, overlay copies, searches, key extraction, the shader bake, the name scan, loads and
saves) with its progress, a Cancel and an action on its result; nothing veils the window.

**Where it stands.** Done. `src/state/jobs.rs` reads every job from its own controller each frame,
so the list never shows one its controller has dropped: an extraction, a defragment, a copy into
the mods overlay, a content search, the file index (since
[R24b](#r24b-go-to-file), what a search inside archives reads), a key extraction or NG derivation,
the shader bake, the name scan, the file being opened with those waiting, a save, and since
[R55b](#r55b-convert-from-the-app), [R51d](#r51d-a-dlc-pack-from-the-app) and
[R38b](#r38b-gltf-in-and-out-from-the-app) a conversion, a DLC pack's build and a glTF export or
import, since [R26b](#r26b-uses-and-used-by-in-the-app) the read of what an install declares, and
since [R49e](#r49e-compare-a-file-with-another-or-with-the-original) a comparison. The status bar
counts them and opens the list (`src/ui/panels/jobs.rs`), as View ▸ Background jobs does: each row
says what its job is doing, how far it has got and offers a Cancel, and the last job of each kind to
end stays with what it left, Open folder for an extraction, a defragment or a save, Show in Files
for an overlay copy, Show results for a search, and its error and the Log for a failure. A job
stops at its next safe point and says it is stopping until then: an extraction after the file in
hand, a defragment or a key extraction once it has read everything and before it writes, the bake
between effects, a save before its write begins, and an overlay copy between chunks, leaving no
`.part`. A content search, the name scan, the file index and an open drop at once; a parse takes no
cancel, so its worker runs on unheard. The defragment's and the overlay copy's
progress windows, the status bar's extraction bar, bake and loading labels, and the bake's veil are
gone; an extraction's clash question and a defragment's answer stay where they were.
`src/state/one_place_for_background_jobs.rs` starts one job of each kind, with a stand-in where a
real run needs an install or the network, finds each in the list, cancels each from it and finds
each stopped, and finds nothing drawn over the window while the bake runs; both failed before the
list existed. Seen on `just shots` screens, not in a window.

**Done when.** Every kind of job appears in the list and cancels from it.

### R60f The unsaved marker tells the truth

[↑ checklist](#phase-10--the-application)

**What.** A document is marked unsaved exactly while a save would write something, the marker clears
once a save has written it, and nothing that holds unsaved work is dropped without asking.

**Where it stands.** Done. `Document::dirty` is what the tab strip, the status bar, the hierarchy,
the window title and the inspector mark, and the close request asks exactly when it is set; pose
keys not yet baked mark the window title and, since
[R59l](#r59l-tabs-that-behave-like-an-editors), their tab. An install or Open Folder
document is marked while an archive in its tree is unwritten: `mark_edited`, and since
[R27l](#r27l-a-change-on-disk-keeps-the-undo-steps-beside-it) `Document::changed_on_disk`, read that
from the tree, so an edit on disk leaves it unmarked, and `Document::archive_written` clears it once the
last one is written. An archive entry saved as a loose file is saved; its archive keeps the bytes
it was opened from, so the export question offers those only for an entry never edited.
`Document::patch_resource_image` marks the document as `mark_edited` does. Detaching a dictionary the app opened
goes through the close request, and so does replacing the install from Settings ▸ Games' folder
pick or Set active, or the Enhanced-open prompt's Switch, by `request_game_switch`. Exiting, New
session and opening a project offer to save a project whose members changed.
`src/state/the_unsaved_marker_tells_the_truth.rs` makes each edit, detaches an edited dictionary and
switches the game; before the fix every edit on disk and every archive save left the tree marked, a
loose save closed nothing, exiting dropped the project's change, a patch bypassed `mark_edited`,
detaching and Switch dropped unsaved work, and Set active opened nothing.

**Done when.** A test makes each of those edits and finds the marker set exactly while something
is left to save and gone after the save, and a test detaches a dictionary with unsaved edits and is
asked first.

### R60g No save replaces a file a job reads

[↑ checklist](#phase-10--the-application)

**What.** A save of an archive, or of a file inside a folder, waits for or is refused while a job
reads it, as an extraction's already is: a conversion, and a DLC pack's build.

**Where it stands.** Done. An extraction, a conversion, a comparison and a DLC pack's build each
say which files on disk their worker reads, and name themselves: the archives an extraction or a
conversion plans from, the archive or game file each side of a comparison is read from, and the
folder a pack is built from. The read behind Uses and Placed by said so too when this closed;
[R60h](#r60h-every-write-asks-the-jobs-reading-it) took it out, since it opens nearly every archive
through the mount and nothing reports which it has passed, and reads the install again after a file
under it is replaced.
`AppState::no_job_reads` (`src/state/jobs.rs`) asks them all, and every save that replaces
a file asks it first: a tree's `Ctrl+S` and an entry saved back into an archive on disk, a file
saved in place or by Save As, an entry saved as a loose file, and an archive opened on its own
taking an entry back. While a job reads the file, or a folder holding it, the save is refused with
a sentence naming the job, and nothing is written; a refused Save As keeps the document's path. A
delete and a defragment ask `no_job_holds`, the same question, which
[R20i](#r20i-what-archive-rows-still-get-wrong-after-r20h) widened to the files an opening
extraction's rows sit in. A glTF export or import is not asked: its model is
read before its worker starts, and an import reads the glTF it was handed once, whole; nor are a
content search, the file index and the name scan, which read an install's archives for what they
list. `src/state/no_save_replaces_a_file_a_job_reads.rs` saves an archive while a conversion and a
comparison each read it, deletes and defragments one a conversion reads, and
saves a file inside a folder a pack is built from by each of those routes; each is refused naming
its job, and each failed before the fix.

**Done when.** A test saves an archive while a conversion reads it, and a file inside a folder while
a pack is built from it, and is refused each time with the job named.

### R60h Every write asks the jobs reading it

[↑ checklist](#phase-10--the-application)

**What.** Every write asks the jobs that read what it would replace, not only a save, a delete, a
rename or a move on disk and a defragment: a conversion's or a glTF export's write. A write that
replaces nothing a job reads goes ahead, and the read of an install for Uses is not asked: it
reads the install again once a file under it is replaced.

**Where it stands.** Done. A conversion's write and a glTF export's write ask `no_job_reads` for
each file they would create or replace, a glTF's texture and `.bin` beside it included, and a job
reading one refuses the write by name and keeps the question. A rename on disk asks the jobs too,
through `no_job_holds`, as [R20h](#r20h-what-archive-rows-still-get-wrong-after-r20g) left it, and `archive_bytes` asks about
the file an overlay save writes, not the shipped archive it copies, so that save goes ahead. The Uses
read is not asked and holds back no save: a save, a defragment, a rename, a move, a delete, or an
undo or a redo of a cut under the active install calls `AppState::file_replaced`, which drops what
the read had made and the read running, and the next menu to ask reads the install again, a map's
bounds ([R26c](#r26c-uses-reaches-every-pack)) included. Holding back only what the read has not
read yet was not built: it opens nearly every archive through the mount until its last step, and
nothing reports which it has passed.
`src/state/every_write_asks_the_jobs_reading_it.rs` holds each: the conversion, the export and the
texture beside it, the overlay save, the map's bounds read again, and every route to `file_replaced`
(a tree's archive save, a loose file saved in place, an entry saved as a loose file, into the overlay
and into an archive open on its own, a defragment, a rename, a move, a delete, an undo and a redo).
Removing the call from any route fails its test.
[R60i](#r60i-what-a-write-still-does-not-ask-after-r60h) closed what it left: an extraction's and an
Export as…'s write asking
the jobs reading the folder they write into, an import into a folder, a New file and a paste asking
the jobs nothing, and a file a conversion, an import or a paste writes under the install leaving
the Uses index as it was.

**Done when.** A test writes a conversion into a folder a pack is built from and is refused; a test
saves an archive into the overlay while a job reads the shipped one and finds it saved; and a test
saves an archive after the Uses read has passed it and finds the save done and the index read again.

### R60i What a write still does not ask after R60h

[↑ checklist](#phase-10--the-application)

**What.** Every write under an install asks the jobs reading where it writes and tells the Uses
index what it replaced: an extraction's and Export as…'s write, an import into a folder, a New file,
a paste, a move into a folder on disk and the paste that sends a cut file to the Recycle Bin.

**Where it stands.** Done. An extraction asks `no_job_reads` for each file it would write, under its
destination, after the defragment check and before its questions, and an Export as… asks for its
file the same way; each is refused naming the job. The folder is asked first, so an extraction into
a folder no job reads costs one question, and a job reading one file of the folder holds back only
that file. `add_file_to_row`, where every import, New file and paste of a copy into a folder on disk
ends, asks for the file it is about to create and calls `file_replaced` once it has written it. A
drag, and the paste of a cut file within one tree, move the file instead: `rpf_move_node_inner` asks
`no_job_holds` for the file it takes and `no_job_reads` for the path it lands on, before the rename,
so a file from outside a pack's folder is refused going into it. An extraction, an Export as…, a
conversion's write and a glTF export's write all end in `poll_extract_job`, which calls
`file_replaced` for the folder written into unless every item was skipped, so Uses reads the install
again after any of them. The paste that sends a cut file to the Recycle Bin already did:
`rpf_delete_node_inner` calls `file_replaced`, so that clause of the row was stale when it was
written, and its test passed from the start. `no_job_reads_any` asks about several paths at once and
looks up the files the jobs read once. It spells them, and the path asked about, as the disk has
them (`as_the_disk_has_it`: what exists canonicalised, the rest as given), so a destination reached
through a `..`, or in another case where the disk ignores case, is the folder a job reads, where
`starts_with` compared spellings. The read of the install for Uses takes its turn: a mount cannot
stop, so a read a save drops runs on unheard, and the next waits for it, while a read dropped before
its turn does not begin; the placements and bounds reads stop at their next map or bound and are not
held. `src/state/what_a_write_still_does_not_ask_after_r60h.rs` holds each. Its one test that passed
before the fix is the cut file sent to the Recycle Bin, a guard on a call that was already there;
R60h's glTF sidecar test was seen failing with the sidecars' names left out of the paths asked
about. Left: the jobs are asked before an extraction's or a conversion's Replace, Skip or Keep both,
so an answer that would not touch a file a job reads is refused all the same; a New folder asks
nothing, and a folder imported into one a job reads makes its directories before each file inside is
refused; the SWF, WAV and search-results exports write without asking the jobs or calling
`file_replaced`, and a mod folder's rename asks nothing of where it lands; the first Uses read after
a save waits out the dropped mount, which was not timed, and the jobs list shows it with no detail;
`Dependencies::replaced` still decides that a write is under the install by `starts_with`; and the
case test is the disk's to answer, so on a disk that keeps case apart it finds another folder and
checks that nothing is refused.

**Done when.** A test extracts into a folder a pack is built from and is refused naming the pack's
job; a test writes an Export as… the same way; a test imports a file under the install and finds the
Uses index read again; a test drags a file, and pastes a cut one, into a folder a pack is built from and
is refused naming the pack's job; a test asks Uses right after a save and finds one read running; a test
writes to a destination spelled in another case than a job's path and is refused; and the glTF sidecar test
is seen failing with the sidecar check taken out.

### R60j What a write still does not ask after R60i

[↑ checklist](#phase-10--the-application)

**What.** A write that makes a folder or a file the jobs read asks them first however it is reached,
an answer that touches nothing a job reads is not refused, and the read of an install for Uses can
be seen waiting.

**Where it stands.** Found while closing [R60i](#r60i-what-a-write-still-does-not-ask-after-r60h)
(#572): a folder pasted, Ctrl dragged or dropped into a folder a job reads is refused file by file,
but its directories are made first with no ask (`import_one` in `src/state/os_drop.rs`,
`rpf_add_folder_inner` in `src/state/rpf_tree.rs`), leaving empty folders in the folder a pack is
built from and one message a file, and the test imports single files only; an extraction asks the
jobs before its Replace, Skip or Keep both, so one into a folder where a job reads a file that
already exists is refused though the user would answer Skip, as `write_conversion` does; the SWF,
WAV and search-results exports (`src/ui/panels/inspector/gfx.rs`, `src/ui/panels/inspector/awc.rs`,
`src/ui/panels/search.rs`) write to a dialog's path with a bare `std::fs::write`, asking neither
`no_job_reads` nor `file_replaced`, so each can write over a file a comparison reads or leave Uses
answering from the read before; a mod folder's rename (`src/state/mod_manager.rs`) asks
`no_job_holds` of its source and nothing of where it lands; the Uses index is read again in full
after any write under the install, an extraction's included, which took 75 to 90 s over retail; the
read that waits for the dropped one's turn is listed "Reading what the install declares, for Uses"
with an empty detail, so nothing says it is queued, a mount that never returns blocks every later
read, of another install too, since the turn is kept across installs, and Cancel drops only the
waiting read; `Dependencies::replaced` decides a write is under the install by a lexical
`starts_with` of the preferences' root, which `as_the_disk_has_it` would settle for a root spelled
in another case; the case-spelling test is the disk's to answer, so hosted Linux CI runs its
negative check and the `..` spelling only, and it starts a pack build with `building` and never
calls `ended` for `JobKind::DlcPack`; the guard in `poll_extract_job` that spares Uses a read after
an extraction that skipped every item has no test; and the header of
`src/state/what_a_write_still_does_not_ask_after_r60h.rs` omits the drag and the cut paste it
covers. The cost of `no_job_touches` on the UI thread is
[R59f](#r59f-performance-and-cache-tuning)'s.

**Done when.** A test imports a folder into one a pack is built from and finds no directory made and
one message; a test extracts into a folder where a job reads one file that already exists, answers
Skip and finds it done; a test writes an SWF, a WAV and a results export over a file a comparison
reads and is refused; a test renames a mod folder onto a path a job reads and is refused; a test
asks Uses right after a save and finds the queued read said to be queued; and a test finds the guard
of `poll_extract_job` fail with it taken out.

---

### R61 Script containers

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** `.ysc`: read a `scrProgram`. Split out of [R35](#r35-the-remaining-formats) because the
container and the useful half — disassembly and native names, in
[R63](#r63-the-script-disassembler-and-the-natives-database) — are different programmes.

**Where it stands.** The container reads: 3,022 of 3,022 scripts over both retail installs
(`crates/title/or-title/tests/ysc_census.rs`, #150). The reader is ordinary code in
`crates/io/or-io/src/gta5/script/`, with no `#[record]` declaration until a corpus is staged
(OD-YSC). An Enhanced install ships 710 v10 files beside the 1,154 v12 files both editions carry, so
the two versions are alternates and neither names a generation (`YSC_FILE_VERSION_GEN8_ALT`). A
native call's hash is recovered by rotating the stored value by `code_length + index`, which the
census verifies by counting distinct values: over a Legacy install 653,093 slots hold 161,254
distinct stored values and 7,402 once rotated back; a name needs a per-build list OpenRage does not
ship, and `ScrProgram::native_names_are_unavailable` says so wherever `openrage-cli script` meets
it. The layout and census are in
[YSC_File_Structure_Specification.md](../extra/YSC_File_Structure_Specification.md).

The app opens a `.ysc` as `DocumentKind::Registered(ResourceType::Ysc)` through a read-only
`PayloadHandler` whose payload is a `ScriptDocument`: the listing `openrage-cli script disasm`
prints, from the one `or_cli::script_listing`, headed by that sentence, which the inspector also
states above it (`a_script_opens_as_its_listing_and_says_why_natives_are_hashes`, and the
self-check's script unit), and the source `openrage-cli script decompile` prints
(`or_cli::script_source`), which the text workspace shows
([R63](#r63-the-script-disassembler-and-the-natives-database)). The app supplies no natives list, so every native shows by hash. The
declared `#[record]` form is still owed once a corpus is staged (OD-YSC).

**Done when.** The container reads, and the app opens a script and states plainly that names are not
available and why.

**Touches.** `or_io::gta5::script` for `ScrProgram`, `or_io::container` for `ResourceType::Ysc`,
`or-cli` for the surface.

**Covers.** Nothing of its own; U-129, U-138, U-139 and U-140 stay with R35.

---

### R62 The RBM layout synthesiser

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** Encode an RBM document whose array outgrew its allocation: a synthesised type table and
page, and an RSC7 seal invented rather than resealed. World data needs it: RBM holds 12,773 of 12,783
Legacy world files and 12,759 of 12,759 Enhanced ones (#146).

**Where it stands.** Landed in #151: `or_io::meta::binary::synth` lays out a grown document,
`Rsc7::invent_single_chunk_seal` seals it as one power-of-two chunk (`DatResourceSizes::single_chunk`,
since an object straddling chunks resolves against the wrong allocation; no shipped page has one
straddling, 0 of 29,059 regions in `what_shipped_rbm_pages_are_shaped_like`), and
`or_world::write::encode` falls back to it only on the writer's `MetaError::OutgrewItsLayout`
refusal, because re-encoding against a synthesised layout changes every document
(`synthesising_a_document_that_did_not_grow_would_change_it`: none of the 2,338 staged RBM documents
re-encodes to its own bytes, the figure its file's module comment states).
An invented seal carries the version in both flag words' nibbles, because an RPF7 entry keeps only
the flag words (`an_invented_seal_survives_having_its_header_thrown_away`). ⚠ Wrapping the
refusal in `MetaError::Other` to name the failing member flattens the variant and the fallback never
fires; `MetaError::in_context` preserves it. A nested class is hashed as `A__B`, the string the game
hashes, and not as the library spells it, `A::B`
(`a_nested_class_is_hashed_as_the_string_the_game_hashes`); a synthesised table named such a type
by a hash no parser holds, and no gate saw it, because each reads back through the same hash.

**Done when.** A `.ymap` with an entity added encodes, the game loads it, and every document that
did not grow still re-encodes byte-identically. The first and last clauses hold on 2,338 of 2,338 RBM
documents (`an_array_that_grew_encodes_and_one_that_did_not_is_untouched`, beside
`adding_an_entity_to_a_map_encodes_and_reads_back` and `rbm_fixtures_round_trip_payload_identical`);
whether the game loads one is unverified, because nothing in the tree runs the game.

**Open.** A class's and an enum's `signature` word is a pure function of the declaration that no
tool derives: 20,511 of 20,511 classes and 5,636 of 5,636 enums in the staged corpus carry a
non-zero one (#151), the same in every document holding that type. Synthesis takes it from the document that already names the type
(`a_donor_changes_nothing_but_the_signature_words`); authoring a type no document names would write
a zero no shipped file carries.

**Touches.** `or_io::meta::binary::synth`, `crates/io/or-io/src/meta/binary/rbm/writer.rs`,
`or_io::container` for the invented seal, and `or-world` where the refusal becomes a fallback.

**Covers.** Nothing of its own; the world rows stay with
[R32](#r32-world-data-ymap-ytyp-and-interiors).

---

### R63 The script disassembler and the natives database

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** Turn a `scrProgram`'s bytecode into readable instructions and name the natives they call;
neither half has a consumer without the other. Decompile it to C-like source, which the owner made a
goal on 2026-10-03, reversing the non-goal that stood here
([recorded](Project_Context.md#a-ysc-decompiles-to-source-2026-10-03)).

**Where it stands.** The mechanism landed in #178. `openrage-cli script disasm` names natives from a
list, says which list and how many natives it named, and leaves the rest as hashes. All 3,022 retail
scripts decode under the instruction set their RSC7 version names
(`crates/io/or-io/src/gta5/script/opcode.rs`), checked against each file's length, pages, jump targets
and native table (`crates/title/or-title/tests/ysc_disasm_census.rs`).

The natives database is downloaded, never bundled
([recorded](Project_Context.md#the-natives-database-is-downloaded-on-request-never-bundled-2026-10-03)).
A version 10 script calls a native by its first hash, which the community lists are keyed by; a
version 12 script calls it by its current hash, shared by both editions (6,601 of the 6,603 a Legacy
install's version 12 scripts call are among the 6,650 an Enhanced one's call), which a list names
only through a crossmap of `first, current` pairs. That is why a list named about 2.5% of a current
build's natives (#178). Over the distinct natives the scripts of each install call (7,402 Legacy,
11,504 Enhanced), measured once at `5be80465` by a harness over the `ysc_census` crawl that was not
kept:

| Source | Legacy | Enhanced |
|---|---|---|
| alloc8or `natives.json` / `natives_gen9.json`, directly | 1,003 / 1,002 | 5,075 / 5,123 |
| citizenfx `natives.json`, directly | 790 | 4,436 |
| alloc8or through `TupoyeMenu/BigBaseV2-fix`'s crossmap | 7,340 | 11,460 |
| alloc8or through `CSY0N/GTA-V-Crossmaps`'s `Enhanced Crossmap.h` | 7,341 | 11,460 |
| alloc8or through CSY0N's Legacy crossmap or ZeddMOCO's 1.70 crossmap | 7,338 | 11,457 |
| citizenfx through the best crossmap | 6,050 | 9,696 |

The two best crossmaps never disagree; TupoyeMenu's holds 132 pairs CSY0N's lacks, none of them
called, and CSY0N's 7 TupoyeMenu's lacks, one of them called, so OpenRage takes alloc8or's two
lists through CSY0N's crossmap alone.
`openrage-cli script download-natives` and the `.ysc` inspector's button fetch the three files on the
user's request into the configuration directory with each upstream commit
(`NativeDbSet::from_download`, `NativeDb::cross_map`); a list given with `--natives-db` or
`OPENRAGE_NATIVES` comes first. Each script takes the list naming most of its natives, and every
output names that list, its commit and the crossmap's commit. With that download, 7,341 of 7,402
Legacy natives (99.18%) and 11,460 of 11,504 Enhanced ones (99.62%) are named, and 26,955,389 of
27,028,405 native call sites (99.73%)
(`a_downloaded_natives_database_names_what_retail_scripts_call` in `ysc_decompile_census`, on
`agent/w13-natives-db` over `231f7bcb`, with alloc8or at `424fb51b0890` and CSY0N at
`5378f84da53c`).

A list's types reach the output: the listing prints a named native's signature, and in the source
a local stored only from one native's result, and integer constants, is declared with its return
type, a `BOOL` argument given 0 or 1 reads `false` or `true`, and a `Hash` argument reads in hex
(`a_native_list_names_a_call_and_types_its_arguments_and_result`). Arguments are typed only when
there is one per parameter. An unnamed hash stays a hash.

The decompiler, in `crates/io/or-io/src/gta5/script/decompile/`, lifts each basic block's stack code
into expressions (arithmetic, comparisons, casts, vectors, array and field access, string literals,
pointers, calls, native calls, `&&` and `||`) and structures each function into if/else, `while`,
`do`/`while`, `for` and `switch` with `break` and `continue`, naming parameters, locals, statics and
globals by slot. An indirect call states no signature, so it takes that of an address-taken
function. The app opens a `.ysc` read-only as that source in the text workspace, with the listing
still in the inspector, and `openrage-cli script decompile` prints the same text. The synthetic
programs in `crates/io/or-io/tests/ysc_decompile.rs` hold an if/else in a loop, nested `for`s, a
switch, native calls, a string, a call between functions and a short-circuit condition. Over all
3,022 retail scripts of both installs (`crates/title/or-title/tests/ysc_decompile_census.rs`, on
`agent/w12-ysc-decompiler` over `447e7c6e`), nothing panics, all 708,862,576 instructions are
emitted exactly once, every source balances its braces and the stack model never fails; 310 of
6,146,028 functions (0.005%, in 102 scripts) fall back to a labelled goto, 8 of 237,278 at version 10
and 302 of 5,908,750 at version 12. 263 pops in retail code read below the operand stack, which is
the frame's last slot: a native given one argument fewer than it takes.

A Legacy install holds scripts under 20 distinct globals signatures and an Enhanced one under 28
(`ysc_census`, #150), so the database is not keyed by build: the hashes a version 12 script calls
are one set across both editions, and the per-script choice of list covers the rest. It takes
[R40a](#r40a-the-shader-definition-database-as-data)'s shape, with its provenance, shipping rule and
version rule in the decision record.

**Done when.** `openrage-cli script` prints an instruction listing in which a native call carries a
name and says which build's database produced it; an unnamed hash stays a hash. Every retail script
decompiles to structured source with no dropped instruction, shown in the text workspace and printed
by the CLI.

**Covers.** U-137, since 2026-10-03; U-129, U-138, U-139 and U-140 stay with R35.

### R64 Move the containing readers onto pack offsets format by format

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

Closed: stopped on 2026-09-23 for [R65](#r65-no-rage-structs-in-openrage), which R66 superseded;
kept as the record of the migration. It moved one containing reader at a time onto pack offsets,
because a leaf bound under a parent still read by hand reads the bytes it read before: `.ynd` (#171,
which added the gate that a binding alone does not count as moved), `.yfd` (#175), `.yed` (#179),
`.yld`, `.ycd` and the drawable lights (#187), and `.ybn`'s BVH and composite records (#191), and
the gate counted hand-written records from #183. None of its machinery stands: every one of those
formats is declared in `or-io` under engine names and opens as an owned value
([R66d](#r66d-declare-gta-v-format-by-format)), and the compiled-layout ceiling went with `or-asset`
at [R66f](#r66f-delete-the-old-crates) (#332). New format work goes to `or-io` declarations.

### R65 No RAGE structs in OpenRage

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Status: superseded on 2026-09-24 by [R66](#r66-the-or-io-crate); kept as the record of what was
built and measured.

The owner ruled on 2026-09-23 that `openrage` holds no RAGE resource structs, and answered its scope
questions on 2026-09-24; the or-io decisions (D1–D14 in
[OR_IO_Specification.md](OR_IO_Specification.md)) replaced that direction the same day, and D15 and
D16 followed on 2026-09-25. R65a, R65b
and R65d–R65g are done, and what they built is `or-io`'s description model and generic tree
([what carries over](OR_IO_Specification.md#what-carries-over-from-the-pack-native-design)); R65c's
testimony is one of the checks that outlived the compiled readers; the other rows are closed as
their checklist lines say.

#### R65a Type plans the pack builds

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Done in #195. A description builds a `TypePlan` per type (`Pack::plans`, in `or_io::desc`), so a
checked record reads with one bounds check and no per-field compare. The record tree, the owned
declarations and the testimony check read through it. `what_a_plan_read_costs` holds its cost
against a typed struct, and `a_consumer_asking_for_a_field_the_pack_lacks_is_told_which` the error
that names a missing field; the call-site memo `at!` has no caller outside tests (that suite and `or_io::desc::plan`'s own). The consumers
it was built for read owned values since [R66e](#r66e-move-the-consumers).

#### R65b Records a tree whose scalars are bytes

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Done in #195 and #196. The generic tree holds a struct as `Value::Struct(Record)`, whose scalars stay
bytes in one shared copy of the image, and a run of pointer-free structs as one
`Value::Records(RecordRun)`. It is what the inspector edits and what the canonical writer saves:
`a_record_saves_to_a_fixed_point` holds the fixed point on synthetic declarations, and each
format's `owned_saves_as_tree` gate (`crates/io/or-io-verify/src/gates.rs`) holds its owned save
against the tree's.

#### R65c Record the oracle's testimony

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Landed in #192: what the compiled readers read, field by field, over the corpus and both retail
installs, so that a layout stays checkable with them deleted ([R66f](#r66f-delete-the-old-crates)). The record stays in `openrage-corpus/testimony` by the owner's exception
(OD-TESTIMONY); do not move it or repoint `the_recorded_testimony_holds`.
[Oracle_Testimony.md](../guides/Oracle_Testimony.md) owns it.

Every declared format's `declared_gates!` testimony test
(`the_ydr_testimony_holds_through_its_declarations` and its siblings) checks the corpus record
field by field, re-keyed by offset (`DeclaredReading`); `the_recorded_testimony_holds` holds the
record as a whole (every recorded file is still the file recorded, and every described scalar fits
its declared kind), and `the_install_testimony_holds` holds the installs' records (#296).
The fields the private pack witnessed are read through a frozen offset key beside the record, taken
once from the version 12 pack and never retaken (#332, OD-TESTIMONY-KEY-FROZEN). Nothing new is
recorded.

**Closed as done on 2026-09-26** (OD-TESTIMONY). It holds 13 of the 16 `(format, target)` rows: the
shipped game carries `.ybn`, `.ycd` and `.yld` only at Gen8 RSC7 versions, so there is no Gen9 file
to record, and the owner ruled those three rows not recordable.

#### R65d Pointers at a member of their owner

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Done in #197, with R65e. A pointer at a member of its own record is `Pointee::Member` or
`Pointee::Members` in a description and `Member<T>` in an owned declaration: `.ypt`'s keyframe
property lists and the Gen9 texture's view pointer are declared with it, and the writer patches the
pointer after placing its owner. Describing it found the Gen9 texture view at 0x58 where the pack
said 0x50; the testimony recorded at 0x50 is answered by a named correction (`FieldMoved`).
[Member pointers and generic layouts](OR_IO_Specification.md#member-pointers-and-generic-layouts)
owns the mechanism and its measurements.

#### R65e Generic types instantiated at export

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Done in #197. A layout may be generic over what it holds when every use of the parameter sits behind
a pointer, and `#[record]` refuses a parameter held by value. `or-io` declares them as generic
records (`clip_dict::AtMap<T>` and `AtMapEntry<T>`, `.ynv`'s `AiSplitArray<T>`), and
`ElementTypes` stays in `or_io::desc::value` for the generic tree's reads.
[Member pointers and generic layouts](OR_IO_Specification.md#member-pointers-and-generic-layouts)
owns the rest.

#### R65f The ypt write row

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Done in #197. `.ypt` is written by the canonical writer at both targets and opens owned under the
engine's names ([R66d](#r66d-declare-gta-v-format-by-format)); `every_ypt_saves_to_a_fixed_point`
and `every_enhanced_ypt_saves_to_a_fixed_point` in `crates/io/or-io-verify/tests/particle.rs` hold
each target's fixed point. The compiled `.ypt` writer went with `or-asset` in
[R66f](#r66f-delete-the-old-crates).

#### R65g The ycd write row

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Done in #197. `.ycd` is written by the canonical writer and opens owned
([R66d](#r66d-declare-gta-v-format-by-format)); `every_ycd_saves_to_a_fixed_point` in
`crates/io/or-io-verify/tests/clip_dict.rs` holds the Gen8 row. The game ships no Gen9 `.ycd`, so
no file holds that row.

#### R65h A ratchet that counts every RAGE struct

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Closed: replaced by [R66b](#r66b-clean-what-the-crate-will-contain), whose denylist ratchet D15 then
withdrew.

#### R65i Animation on the pack

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Closed: replaced by [R66e](#r66e-move-the-consumers), with the same corpus gate:
`every_corpus_clip_poses_as_recorded` holds every clip to the same poses frame for frame.

#### R65j The simulation and the particle model on the pack

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Closed: replaced by [R66e](#r66e-move-the-consumers), with the same corpus gate:
`every_corpus_effect_simulates_as_recorded` holds every effect's deterministic simulation snapshot.

#### R65k Interchange on the pack

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Closed: replaced by [R66e](#r66e-move-the-consumers). `or-interchange` reads a drawable of either
target as itself, and `every_corpus_drawable_exports_as_recorded` holds the `.glb` content every
Gen8 corpus drawable exports.

#### R65l The editors on records

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Closed: replaced by [R66e](#r66e-move-the-consumers).

#### R65m The app holds one resource payload

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Closed: replaced by [R66e](#r66e-move-the-consumers), whose payload is one resource value.

#### R65n The CLI on the pack

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Closed: replaced by [R66e](#r66e-move-the-consumers), with the same gate:
`every_documented_command_reports_as_recorded` holds every documented command to the same values
over the corpus.

#### R65o Gen9 collapses into the bound pack

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Closed: obsolete. Each target is declared as itself: the four types Gen9 changed live in per-target
modules of `or_io::gta5`, and a type that differs only in what it points at is generic over
`DrawableTarget`. Only [R55](#r55-retargeting-between-gen8-and-gen9)'s retarget converts a drawable
between the targets; the lossy converter went with `or-asset` (#332).

#### R65p Delete the structs

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Closed: replaced by [R66f](#r66f-delete-the-old-crates).

#### R65q The RPF7 archive through the pack

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Closed: replaced by [R66d](#r66d-declare-gta-v-format-by-format). RPF7's header and entry table
are declared in `or_io::container` (`FiPackHeader`, `FiPackEntry`, #262), and mounting an install
never needed a pack.

#### R65r The RSC7 header through the pack

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Closed: replaced by [R66d](#r66d-declare-gta-v-format-by-format), which declared the header as
`DatResourceFileHeader` (#216).

#### R65s GXT2 and YPDB through the pack

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Closed: replaced by [R66d](#r66d-declare-gta-v-format-by-format). GXT2 is declared and opens
owned as `Gxt2` (#265); YPDB is ordinary code in `crates/io/or-io/src/gta5/pose_db.rs` and is not
declared, by the owner's scope ruling of 2026-09-26
([OD-YPDB](Project_Context.md#the-pose-database-is-ordinary-code-2026-09-26)).

#### R65t Data addressed by offset

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Closed: obsolete. Declarations compute their offsets, and RPF7 and GXT2 are declared (#262, #265),
so the generic tree reaches them through their descriptions with no offset repr.

#### R65u Roles engine names out of source

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Closed: withdrawn. The declarations carry the engine's names (D15), checked by the compiler.

#### R65v Metadata containers

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Closed: replaced by [R66d](#r66d-declare-gta-v-format-by-format). Metadata is `or_io::meta`
behind the `meta` feature (D5, #267).

### R66 The or-io crate

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

A programme, [R66a](#r66a-decide-and-record-the-direction) through
[R66h](#r66h-publish-the-crate-and-the-editor-together), adopted by the owner on 2026-09-24 and
superseding [R65](#r65-no-rage-structs-in-openrage). One public crate, `or-io` with `or-io-macros`,
holds GTA V's format knowledge, named and laid out from the GTA source (D15). A file opens as an
owned Rust value (D16), with a zero-copy peek on request and the record tree behind the inspector.
The design, decisions D1–D18 and phases P0–P7 are
[OR_IO_Specification.md](OR_IO_Specification.md). Order: a → f, then g and i, which do not wait on each other, then h.

R66a–R66f are done: every format `FORMATS` lists is declared and opens owned for the targets it
ships, the app and `openrage-cli` read no pack, and `or-asset`, `or-format-api`, `or-ptx` and
`or-macros` are deleted. [R66i](#r66i-retire-openrage-packs) is done: the owner archived and
deleted `openrage-packs` on 2026-10-01 (D17). Open: [R66g](#r66g-rewrite-the-history), the history
rewrite, for later; and [R66h](#r66h-publish-the-crate-and-the-editor-together), publication,
which waits on it and is parked with the lawyer questions.

#### R66a Decide and record the direction

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Done in #199 (P0): D1–D14 recorded as dated decisions in [Project_Context.md](Project_Context.md),
the standing documents pointed at the specification, and R66 on this roadmap. The lawyer questions
are parked (D14) and gate only R66h.

#### R66b Clean what the crate will contain

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Done in #202 (P1). It replaced [R65h](#r65h-a-ratchet-that-counts-every-rage-struct); D15 withdrew
one of its parts and D18 another.

- **Keys.** The AES key, the hash table and the AWC key come from the user's executable
  (`what_the_extractor_recovers_per_executable`). The key snapshot left the tree in this row; D18
  bundles it once in `or-io` (`crates/io/or-io/data/magic.dat`, behind the default `bundled-keys`
  feature), held by `the_key_snapshot_is_in_the_tree_once` ([Keys](OR_IO_Specification.md#keys-d4)).
- **The version table is measured.** `crates/io/or-io/src/container/resource/versions.rs` states
  observed versions from `openrage-cli versions`; `.ypm`, which neither install ships, is tagged
  unmeasured.
- **Bundled game-derived data dropped**: the material palette and CodeWalker's fallback colour.
  `LICENSE-third-party.md` names every module ported from CodeWalker and CodeX.
- **Citations out of the tree**, and the coverage test that walked the leaked tree deleted. The
  particle simulation, fallback shaders, VFS device code and reset values are unchanged (D9, D13).
- **Lints.** `scripts/lint_provenance.ps1` fails on a leaked-source path or a source-file citation
  in every tracked file (P67, #340), run by the pre-commit hook and hosted CI. The engine-name rule
  and the leak-name denylist this row also built are withdrawn by D15 (#218).

#### R66c The crate and the macro on synthetic types

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

Done in #201 (P2). `crates/io/or-io` holds what `or-rage-core`, `or-rage-types`, `or-format-api`,
`or-pack-build`, `or-pack-io` and `or-container` held, each as a module, with every importer
rewritten and no facade, and `crates/io/or-io-verify` holds the testimony check and the gates. On
invented types (`crates/io/or-io/tests/synthetic/mod.rs`), `declared_records` reads typed and
through the tree with equal values and saves to a fixed point, `decl::refusals` holds the
compile-time refusals as doctests, and `census_and_diff` holds the alignment census and the
structural diff. A typed field read costs a plain struct's: `what_a_typed_read_costs` measured
0.079 ns a field for both (release, quiet machine, #201). Hosted CI's Linux job builds `or-io` for
`wasm32-unknown-unknown` on every non-draft pull request that changes code.

#### R66d Declare GTA V format by format

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**Done** (closed with R66f, P67, #340). Every format `FORMATS` lists
(`crates/io/or-io/src/gta5/formats.rs`) is declared in `or_io::gta5` under the engine's names (D15)
and opens as an owned value (D16) for each target it ships. The RSC7 header (`DatResourceFileHeader`,
#216) and RPF7's header and entry table (`FiPackHeader`, `FiPackEntry`, #262) are declared in
`or_io::container`. The pose database (OD-YPDB), metadata (`or_io::meta`, #267), AWC and DDS
(behind features, #266), the codecs and authoring path (#268) and the shader definitions (#270) are
ordinary code in `or-io`. Once every format was owned, P62 deleted
`or-asset` and its compiled readers (#332). R66d replaced
[R65q](#r65q-the-rpf7-archive-through-the-pack), [R65r](#r65r-the-rsc7-header-through-the-pack),
[R65s](#r65s-gxt2-and-ypdb-through-the-pack) and [R65v](#r65v-metadata-containers), and carries
[R65c](#r65c-record-the-oracles-testimony)'s record re-keyed by offset.

**Done when, per format and target.** The
[gates per format](OR_IO_Specification.md#the-gates-per-format) hold: typed reads, the re-keyed
testimony, the canonical fixed point with `owned_saves_as_tree`, the alignment census and D15's
names, with no hand-written offset and no `unknown_*` field; D17 retired the structural diff, gate
6, format by format. For metadata: every metadata suite is green in `or-io-verify`'s `meta` suite and
every document still reaches its fixed point.

**Rules that stand.** A file's target is its RSC7 version (`Format::target_of_version`), never the
install it came from. A canonical save places every record run at its element type's natural
alignment, and `packed(N)` stays only on `.ynv`'s quadtree node, whose pointers sit at four modulo
eight (OD-ALIGN, #338; `a_canonical_save_places_every_record_at_its_natural_alignment`).

**Where it stands.** The Owned column names each root as its `impl Resource` or `impl File` line
spells it, which `the_owned_column_is_the_resources_or_io_declares` holds against `or-io`. A
fixed-point figure counts corpus files, printed on every run by the format's suite in
`crates/io/or-io-verify/tests/` (the module's name, or `rsc7_header`), which the self-hosted
`layouts` job runs on its staged corpus and `just test-format` runs one module at a time. The
`.ydr`, `.ydd`, `.yft` and `.ypt` cells last printed unchanged at `c7efd092` (#325), the Legacy
`.ytd` cell at `994dfe75`, the rest at `b1badfd7` (#255), and every one again with every run
aligned (#338). The re-keyed testimony, each format's `declared_gates!` testimony test, shows 0
differences with nothing unreached on every recorded format bar its named set-asides, each suite's
`OVERREAD` and `RELOCATED` lists: the older record read one box past every one-geometry model's box
run, took a `.yft` glass pane for a 0x70-byte struct, and read the Gen9 texture view at 0x50 where
it sits at 0x58 ([Oracle_Testimony.md](../guides/Oracle_Testimony.md)).

| Format | Module | Targets | Declared | Owned | At a fixed point |
|---|---|---|---|---|---|
| RSC7 header | `container::resource::DatResourceFileHeader` | both | #216 | — | 2,500 of 2,500 headers write back to their own 16 bytes; not recorded |
| `.yfd` | `frame_filter_dict` | Gen8 | #203 | `CrFrameFilterDictionary`, #219 | 7 of 7 |
| `.ytd` | `texture_dict` | Gen8, Gen9 | #204 | `PgDictionary<GrcTexture>` (Gen8), #219; `PgDictionary<GrcTextureG9>` (Gen9) | 16 of 16; 2 of 2 |
| `.ytd` (RDR2) | `texture` | RDR2 | — | `TextureDictionary`, read only | 3134 of 3134 textures named by their hash and decoded; no save |
| `.ybn`, `.ybd` | `bounds`, `bounds_dict` | Gen8 | #205 | `PhBoundAny`, `PgDictionary<PhBoundAny>` | 21 of 21; 1 of 1 |
| `.yld` | `cloth_dict` | Gen8 | #206 | `PgDictionary<CharacterCloth>` | 58 of 58 |
| `.yed` | `expression_dict` | Gen8 | #207 | `CrExpressionsDictionary`, #220 | 471 of 471 |
| `.ynd` | `path_region` | Gen8 | #208 | `CPathRegion`, #220 | 518 of 518 |
| `.ydr` | `drawable`, `skeleton`, `drawable_codecs` | Gen8, Gen9 | #209, #210 | `GtaDrawable<Gen8>` (Gen8); `GtaDrawable<Gen9>` (Gen9) | 13 of 13; 5 of 5 |
| `.ydd` | `drawable_dict` | Gen8, Gen9 | #211 | `PgDictionary<GtaDrawable<Gen8>>` (Gen8); `PgDictionary<GtaDrawable<Gen9>>` (Gen9) | 48 of 48; 2 of 2 |
| `.yft` | `fragment` | Gen8, Gen9 | #212 | `GtaFragType<Gen8>` (Gen8); `GtaFragType<Gen9>` (Gen9) | 63 of 63; 5 of 5 |
| `.ycd` | `clip_dict` | Gen8 | #213 | `CrClipDictionary` | 60 of 60 |
| `.ypt` | `particle` | Gen8, Gen9 | #214 | `PtxFxList<Gen8>` (Gen8); `PtxFxList<Gen9>` (Gen9) | 4 of 4; 2 of 2 |
| `.ynv` | `navmesh` | Gen8 | #215 | `CNavMesh` | 1,200 of 1,200 |
| `.yvr` | `vehicle_recording` | Gen8 | #418 | `CVehicleRecording` | 8 of 8; every installed file: 7,533 of 7,533 Legacy and 7,530 of 7,530 Enhanced |
| `.ywr` | `waypoint_recording` | Gen8 | #418 | `FwWaypointRecordingRoute` | 8 of 8; every installed file: 1,612 of 1,612 in each install |
| `.ypdb` | `pose_db` | both | — | `PoseMatcherDatabase` | 112 of 112 |
| `.gxt2` | `text` | both | #265 | `Gxt2`, #265 | 5 of 5 tables the writer produces; none staged |
| `.ymap`, `.ytyp` | `typed` | both | — | `CMapData`, `CMapTypes` | none staged; a Legacy extraction outside the corpus: 11,076 of 11,076 `.ymap` and 1,707 of 1,707 `.ytyp` (#280); no Enhanced file run |
| scenario region `.ymt` | `typed` | both | — | `CScenarioPointRegion` | 205 of 205 staged regions (`every_staged_region_opens_typed_and_names_each_points_type`); no Enhanced file run |
| ped variation `.ymt` | `ped` in `typed` | both | — | `CPedVariationInfo`, `CCreatureMetaData` | 1,839 of 1,839 staged variations and 25 of 25 staged PSO and RBM creature documents (`every_staged_ped_variation_opens_typed_and_saves_to_its_fixed_point`, `every_staged_creature_metadata_opens_typed_and_saves_to_its_fixed_point`); the 64 staged RBF creature documents through the tag adapter, each saved back byte for byte (`every_staged_rbf_creature_opens_typed_and_saves_byte_for_byte`); the types declared in code and held to the tables the documents state (`every_declared_ped_class_is_the_table_the_documents_state`); no Enhanced file run |
| alternate variations and first person alternates `.meta` | `alternate` in `typed` | both | — | `CAlternateVariations`, `FirstPersonAlternateData` | the owner's server pack: 372 switches over 10,125 sources read, saved to a fixed point (`every_switch_and_source_of_the_text_is_in_the_typed_root`, `the_canonical_form_of_the_file_is_a_fixed_point_and_means_what_the_file_meant`); a root built here saved as XML and read back (`a_root_built_from_nothing_saves_as_xml_with_no_snapshot`, `a_first_person_file_built_here_saves_as_xml_and_reads_back_equal`); the types declared in code, with no staged binary document to hold them to |
| weapon files `.meta` | `weapon_blob`, `weapon_components`, `weapon_archetypes` in `typed` | both | — | `CWeaponInfoBlob`, `CWeaponComponentInfoBlob`, `CWeaponModelInfoInitDataList` | the owner's Legacy install, read through its mounted packs: 220 of 220 weapon files, 44 of 44 component files and 47 of 47 archetype files saved to OpenRage's canonical text with every element they state arriving, that text a fixed point, and every member the family declares one the install states (`every_weapon_file_of_the_install_opens_saves_and_arrives_as_it_was`, install-gated; none staged); the root each file registers under (`a_weapon_file_registers_under_the_type_its_root_names`); no Enhanced file run |

- **Refused by name and left out**: `markers.ytd`, at RSC7 version 2, which neither target
  describes; and `.ybd` at versions 42 and 40, whose dictionary envelope matches version 43 so a
  version-blind reader would misread them.
- **`.ysc` waits on a corpus** (OD-YSC). Its reader is ordinary Rust in `gta5::script`, with no
  `#[record]` and no writer (#289); it is declared once corpus files are staged, and the corpus
  holds none.
- **A save that drops nothing.** By the owner's ruling of 2026-09-24 a save that drops data is a
  correctness defect. `census::STILL_LOST` names the one site a canonical save still loses, a Gen8
  `.ydr` `GrmShaderGroup.shader_datas` written null, in 2 files (`what_a_save_drops_is_the_list`,
  at `497a4363`); it leaves the list once read, save, read equality and the testimony show the save
  keeps it, the gate shown failing first. `PgBase.first_node` is written null by decision
  (`census::RULED`): in no shipped `.yed`, `.yfd` or Gen8 `.ytd` does it lead to a non-zero byte.
- **Bit ranges** (OD-BITS): `#[bits]` is in `#[record]`, and RPF7's header and entry name their
  ranges with it (#262); no GTA V resource declaration declares its bit ranges yet.
- **What references still leave.** A copied `.ypt` rule and a hand-written `RefPath` are tracked
  by position in a run: reordering breaks the copy (`or_particle::list::canonical` lays a list out
  so the dictionaries own the rules, #320), and removing an earlier element saves an equal copy as
  its alias (OD-ALIAS-BY-VALUE). [The owned form](OR_IO_Specification.md#the-owned-form-d16) owns
  the detail.

#### R66e Move the consumers

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**What.** Phase P4: the viewport mesh walk, `or-interchange` for both targets, `or-anim`,
`or-simulation` and `or-particle` with behaviour unchanged (D9), the editors, `or-cli` and the
app's one resource payload, onto the owned values of D16. It replaces
[R65i](#r65i-animation-on-the-pack) through [R65n](#r65n-the-cli-on-the-pack).

**Done when.** No crate outside `or-io` names `or-asset`'s types or `or-format-api`'s `names`;
`just self-check` paints every panel; every corpus drawable exports the same `.glb` content, every
clip the same poses frame for frame, every effect the same deterministic simulation snapshot, and
every documented CLI command reports the same values.

**Where it stands.** Done (P39 to P60, #286 to #329), and both crates are deleted (R66f). The app
holds one resource payload, `DocPayload::Resource(or_io::Asset)`, for every format `FORMATS` lists
at every target it ships, and authors each blank from `Default` through its `PayloadHandler`;
`every_declared_format_opens_with_no_pack` (`src/state/authoring.rs`) makes, opens, saves and
reopens a blank and the smallest staged file for each, and
`every_enhanced_row_opens_and_saves_with_no_pack` holds the Enhanced rows. The viewport walks the
owned drawable (`drawable_meshes` in `src/render/meshes.rs`) and bound
(`src/render/bound_meshes.rs`); `or-anim` plays the owned `CrClipDictionary`; `or-simulation` and
`or-renderer` build from the owned `PtxFxList`; `or-interchange` exports both targets' owned
drawables; `or-cli` reads every file at its own target.

The behaviour clause is held by four suites (#233) that compare a digest per corpus file with a
record in `openrage-corpus/baselines`, written only by `just baselines-record` (OD-BASELINES) and
run by `layouts`: `every_corpus_drawable_exports_as_recorded`,
`every_corpus_clip_poses_as_recorded`, `every_corpus_effect_simulates_as_recorded` and
`every_documented_command_reports_as_recorded`. At `989ad5ff`, against the records taken at
`29722735`, 124, 17,775, 1,006 and 58 entries match, in that order. At `3e822330`, with no pack,
the CLI suite matches 56 of its 58 entries; the other two are the skeletons the compiled reader
dropped, named in the comparer's `CORRECTED` list (OD-P60-CORRECTIONS). The CLI record retaken
at `94ff4041` holds the lines the owned read prints, so that list is empty.

Outside the done-when: `every_corpus_drawable_uploads_as_recorded` checks the viewport's mesh
uploads against a record `just baselines-record` takes and `layouts` runs (#388); mesh export in
`openrage-cli` reads both targets' drawables, each at its own RSC7 version (#376); `defs check`
reads both targets' materials through `materials_in` (#384); and an Enhanced `.ypt` is not
simulated, since `or-simulation` builds from `PtxFxList<Gen8>`.

#### R66f Delete the old crates

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**What.** Phase P5: delete `or-asset` with its compiled readers and writers, its engine-named
structs, the old code generators and their `inventory` registration, GTA V pack discovery,
`or-format-api` with its `names`, and `GameProfile::pack_title` (OD-OTHER-TITLES). It replaces
[R65p](#r65p-delete-the-structs). `or-meta` moved into `or-io` whole as `or_io::meta` in
[R66d](#r66d-declare-gta-v-format-by-format) (#267). The private pack is authored in nothing
afterwards, because D17 retires it.

**Where it stands.** Done (P61 to P67, #331 to #340). P61 deleted the pack recipes, the pack
tooling and `layouts`' pack staging (#331); P62 deleted `or-asset` and pack discovery (#332); P70
deleted `or-ptx` (#333); P63 deleted `or-format-api` and `GameProfile::pack_title` (#335), so a
title is one `GameProfile` impl and one registry line; P65 deleted `or-io`'s classic layer
(`prim`, the `binrw` paging paths, the `inventory` registration, #336); and P66 deleted
`or-macros` (#339), so `or-io-macros` is the one macro crate. P64 dropped `or-test-support`'s
upward edge to `or-io` (#334).

Three checks outlive the compiled readers: `the_recorded_testimony_holds`, in `or-io-verify`
through the frozen key (OD-TESTIMONY-KEY-FROZEN); R66e's behaviour baselines; and
`every_corpus_legacy_texture_decodes_as_recorded`, which holds 290 textures in 16 files to RGBA
digests recorded at `eb500483`, each equal to the compiled reader's decode before the crate went
(OD-TEXTURE-DIGESTS). P67 (#340) brought the standing documents to the result, documented the
description model and the `.orpack` bytes in
[the specification](OR_IO_Specification.md#descriptions), and made the
provenance lint fatal on a leaked-source citation in every tracked file. The cleanup of
2026-09-30 (#342 to #348) deleted what the crates left behind, `PackSet` among it;
`or_io::desc::Pack` stays as `or-io`'s description model (OD-OTHER-TITLES).

**Done when.** `or-asset` is gone from the workspace (`or-meta` left already, in R66d); the pattern
lint finds no source-tree citation in the tip tree; the workspace opens every declared format with
no pack; the testimony is green against the built-in description; the standing documents describe
the result.

#### R66g Rewrite the history

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**What.** Phase P6, as [the specification](OR_IO_Specification.md#rewriting-the-history) sets out:
freeze open pull requests, land the tip's clean-up, rewrite a mirror clone with `git filter-repo`,
verify, push, have GitHub purge its pull-request references, re-clone, and wipe the runner and
caches. Removed from every commit: citations and documents about leaked code, the coverage test that
walked the leaked tree, and the metadata schema snapshot and `.ydr` fixture that #1's merge of the
unrewritten old `main` carried back into history. The key snapshot stays, because D18 ships it in
`or-io`. Every branch and tag is rewritten, `archive/ydd-clothing-preview` and
`feat/ydd-clothing-preview` included, since both hold the files. Commit messages are otherwise
kept, so every `(#N)` survives. Engine-named types in old revisions keep the engine's names, so no
identifier is renamed (D12, taken on 2026-10-01).

**Where it stands.** The rewrite has not started. The tip's clean-up it carries backwards has
landed (R66b, and P67's provenance lint, fatal in every tracked file), and the scan the
verification runs is written: `scripts/lint_provenance.ps1 -History`
(`just lint-provenance-history`) reads every commit message and every blob reachable from every ref of a
repository with the tip's patterns and, when given one, a filter-repo expressions file, and names
each hit by commit, path and line; the pre-commit hook still runs the tip mode. It has named both
leaks planted in a throwaway mirror of the tip, one in a file a later commit deleted and one in a
commit message, and finds nothing in the same mirror without them. The freeze has not begun: on
2026-09-30 the draft #131 is open and `origin` holds agent branches beside `main`. D12 is taken
(2026-10-01), and the owner left the rewrite for later the same day.

**Done when.** The history scan has failed on a copy with planted leaks and passes on the rewritten
mirror; no commit holds a removed path; the tip tree is unchanged and every
`(#N)` subject is present; GitHub has confirmed the purge; no local clone, runner checkout or cache
holds pre-rewrite objects except one offline backup, if kept.

**Risk.** Every commit hash changes, so an open branch, a fork or a link to a commit breaks, and
old revisions that embedded the metadata schema snapshot stop building, by design.

#### R66i Retire openrage-packs

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**What.** D17 (2026-09-26): GTA V has one description, the one `or-io` builds from its
declarations, so the private pack repository, `../openrage-packs`, is archived offline and deleted.
`PACK_VERSION` stays 12 until the pack is deleted (OD-PACKVER). The `.orpack` export that
OD-OTHER-TITLES kept, its feature and `PACK_VERSION` are deleted by the owner's
ruling of 2026-10-01.

**Depends on.** [R66f](#r66f-delete-the-old-crates), done.

**Where it stands.** Nothing in the tree, the workflows or the recipes reads the repository: P61
deleted the pack recipes and `layouts`' pack staging (#331), and P62 deleted GTA V pack discovery
with `or-asset` (#332). The testimony reads the fields the pack witnessed through the frozen key,
`pack-witnessed.key` beside the testimony (`or_io_verify::testimony::Key`), written once from the
version 12 pack and never retaken (OD-TESTIMONY-KEY-FROZEN);
`a_frozen_key_row_one_byte_late_is_named_by_the_testimony` moves the frozen `GrcTexture.width` row
one byte and is caught 290 times (Gen8) and 26 times (Gen9), at `225abbb9`. **Done** on
2026-10-01: the owner archived the repository as a private, offline git bundle and deleted it, and
it no longer sits beside the checkout
([recorded](Project_Context.md#openrage-packs-is-deleted-and-the-orpack-export-goes-2026-10-01)).

**Done when.** Nothing in the tree, the workflows or the recipes reads the repository, and the
owner has archived it as a private, offline git bundle (outside
[R66g](#r66g-rewrite-the-history)'s rewrite, never published) and deleted it.

#### R66h Publish the crate and the editor together

[↑ checklist](#phase-1--the-or-io-crate-the-critical-path)

**What.** Phase P7 (D8): make this repository public and publish `or-io` and `or-io-macros` 0.1.0
on crates.io the same day.

**Depends on.** [R66f](#r66f-delete-the-old-crates) and [R66i](#r66i-retire-openrage-packs), both
done; the repository becomes public only after [R66g](#r66g-rewrite-the-history)'s purge (D3).

**Where it stands.** Not started, and parked with the lawyer questions by the owner on 2026-10-01. The repository is private and the workspace sets
`publish = false`. Hosted Linux CI runs the synthetic round trips on every non-draft pull request
and is green on `main` at `abc3d091` (#349). No `cargo-deny` configuration or CI step exists, and
the lawyer questions stay parked (D14).

**Done when.** The parked lawyer questions (D14) are answered; the licence of every file ported
into `crates/io/or-media` is reviewed (owner, 2026-10-05,
[recorded](Project_Context.md#every-media-format-is-decoded-in-rust-inside-the-tree-2026-10-05));
the tip-tree lints are clean; a
clean clone passes hosted CI with the synthetic round trips; `cargo-deny` is clean; and
`cargo publish --dry-run` succeeds for both crates.

---

### R67 Menyoo spooner XML import

[↑ checklist](#phase-5--formats-openrage-has-never-scoped)

**What.** Read a Menyoo spooner placement file into entities a map can hold, and place them through
the path a prefab takes. Split out of [R32](#r32-world-data-ymap-ytyp-and-interiors), which has the
world side: `or_world::prefab::place` turns and moves a set of entities into a map's value tree,
`WorkingSet::place_prefab` writes that placement into a working set, and `WorkingSet::state_types`
adds the type files the map's manifest entry must name; `openrage-cli world prefab` does both. What
is missing is the format — a third-party tool's XML, versioned by that tool, with no specification.

**Where it stands.** Unstarted: nothing in the tree reads a spooner file, and `openrage-corpus`
stages none.

**Depends on.** A staged corpus of spooner files from more than one Menyoo version.

**Done when.** Every spooner file in the corpus either imports, placing each object where Menyoo
placed it, or is refused with a reason, and the rotation convention each Menyoo version writes is a
measurement, not a comment.

**Risk.** Menyoo stores Euler angles in its own order and sign, and an entity stores a conjugated
quaternion; R32 needed a measurement to settle the same asymmetry.

### R68 Shell faults after R59z

[↑ checklist](#phase-10--the-application)

A programme of the parts the [R59](#r59-presentation-input-and-workspaces) family has no letter left
for, since a part takes one lowercase letter and R59z is its last: what the lane of
[R59q](#r59q-an-accessibility-baseline) left of the accessibility baseline
([R68a](#r68a-what-the-accessibility-baseline-still-leaves)), and a file nothing declares and a
module at its gate in the app's source tree
([R68b](#r68b-a-file-nothing-declares-and-a-module-at-its-gate)), and what the lanes that closed them
and the app's test runs found: tests that fail under load and the errors a run logs
([R68c](#r68c-tests-that-fail-under-load-and-errors-a-run-logs)), what the scan for leftover folders
misses ([R68d](#r68d-what-the-scan-for-leftover-folders-still-misses-after-r59y)), the small faults
R59z left ([R68e](#r68e-small-faults-left-after-r59z)), the comments and the module walk the lanes
left ([R68f](#r68f-comments-and-a-module-walk-the-ux-lanes-left)) and what the accessibility checks
cannot see ([R68g](#r68g-what-the-accessibility-checks-still-cannot-see)), a floor in the
suppression check that a deletion crossed ([R68h](#r68h-a-test-floor-that-a-deletion-crossed)) and
what closing it left: the floors of the same kind in other tests
([R68i](#r68i-count-floors-that-a-deletion-could-still-cross)) and what the suppression scan still
skips ([R68j](#r68j-what-the-suppression-scan-still-skips)). No part blocks another, and a shell
finding after R68j that fits none takes the next letter here.

### R68a What the accessibility baseline still leaves

[↑ checklist](#phase-10--the-application)

**What.** The running editor sends an AccessKit tree, every Tab stop in the dock has a name, and what
R59q's contrast and frame scans exempt or cannot see is judged or given a second cue.

**Where it stands.** Every clause but the owner's was in the tree when the lane closed; what it and
[R59q](#r59q-an-accessibility-baseline) left is listed last. The window sends a screen reader
its tree: `egui-winit` is built with its `accesskit` feature, both windows are made hidden, given an
adapter and then shown, and `src/app/accessibility.rs` starts the tree when the platform asks for
it, stops it when the screen reader goes and hands its action requests to the next frame
(`a_window_sends_a_screen_reader_no_tree_until_it_asks_and_then_one_with_the_names_of_the_shell`
reads the tree egui builds in a frame, which is all the window forwards, finds none before it is
asked and finds the tabs and buttons R59q named after; no window opens). It failed with the
tree switched off. The platform adapters are in the build, only Windows's has been compiled here,
and none has run: the owner checks it with a screen reader. Every Tab stop in the dock has a name:
`every_tab_stop_in_the_dock_has_a_name_a_screen_reader_can_speak` walks Tab through every
workspace with a folder open and found 87 stops with none. `name_the_focus`
(`src/ui/accessible_name.rs`) calls the stop the keyboard is on by the text inside it, or a
Divider when it is a thin bar, which covers egui_dock's splitters and egui_extras' column handles,
and it writes the stop's rect back, because a `Ui` allocated with a sense, as a table's cell is,
keeps the empty rect it began with in the tree; the regions that Files, the Files view, the
viewport and the timeline allocate, the tree's toggles and the Files view's cells carry names of
their own. The density graph is an image named by its row and its key count, and each curve handle
a button named by its series, channel, key, time and value, which describes it and takes no input,
since the pointer alone grabs a handle; the stand-in timeline draws both, and the naming test
judges the handles
(`a_row_drawn_as_a_density_graph_is_described_with_its_label_and_its_key_count`,
`every_handle_of_a_curve_is_described_inside_the_curve_view_and_a_read_only_clip_draws_none`).
The frame scan exempts a disabled control's text only inside the bounds AccessKit gives a disabled
widget, and at the fade's alpha there (`is_exempt`,
`a_text_at_the_fades_alpha_is_judged_unless_a_disabled_widget_draws_it`, which failed with the
exemption by alpha); the owner decided on 2026-10-05 that no setting raises the fade, and at the
0.5 fade the light theme's disabled primary text is 1.89:1 on the panel, which reads as disabled.
The axes' colours read 3:1 on the gizmo's disc over the default background
(`the_axis_colours_read_at_3_to_1_on_the_gizmos_disc`, which fails with a dark red; the colours as they stand read). The
timeline draws every colour its source names through `theme::graphic`, so a key, a span's edge, a
density graph, a curve and its handles read at 3:1 on their rows
(`every_colour_the_timeline_draws_a_mark_in_reads_at_3_to_1_on_its_row_in_both_themes` found 99
marks under it, every one in the light theme, from 1.67:1 to 2.95:1). The focus ring is cut to the
part of a widget egui shows: where `interact_rect` falls short of `rect` the clip's edge is its
edge, which needs no rect egui's Response lacks, so the ring no longer lies over what is beyond a
scroll area
(`a_widget_clipped_at_its_scroll_areas_edge_has_its_ring_inside_the_clip`, which saw it four
points over the edge). An unsupported file in a tree carries a badge beside its icon as well as
its colour (`an_unsupported_file_in_a_tree_is_marked_by_more_than_colour`). The badge and the ring were seen on `just shots`
screens, not in a window, and the timeline's marks are judged by their test alone, since its screen needs a skeleton. Left, besides the owner's check: the Files view's tiles mark an
unsupported file by colour alone, since a tile draws its icon without the badge, and so does one
whose icon already carries an unsaved or a new badge; a widget flush with its scroll area's edge
and not cut by it keeps a ring four points over the edge; the bubbles and axes behind the others
on the gizmo are drawn at an alpha that reads under 3:1 on purpose, and the viewport's shading
icons and the renderer's own axes are not judged; the tree's toggles are called Expand or Collapse
and not after their row; the Tab walk is of every workspace with a folder open, so the panels
that need a file open are not walked, nor is the timeline's own Tab stops; and a curve handle
still moves only by the pointer.
Left too, found while closing [R59q](#r59q-an-accessibility-baseline) (#555) and by this lane's
review (#563), and not yet in the tree: the code editor (`src/ui/panels/text_editor`) calls
`ui.interact` with no `widget_info`, so its node is Role::Unknown with no name and a screen reader
is told of no text field, and the Tab that brings the keyboard into an editable document inserts
four spaces, because the editor reads the key on the frame it gains the keyboard, no test holding
either; when the focused widget is no longer built (a dialog dismissed with Enter on its button, a
panel or workspace switched, a tab closed) the focus ring is drawn for one frame at its old rect in
the dead widget's layer; `moves_focus` in `src/ui/focus_ring.rs` ignores modifiers, so a Ctrl, Alt
or Shift chord on Tab, an arrow, Space or Enter that no handler consumes turns the keyboard mode on
and rings whatever has focus; the Files view's key readers (`src/ui/panels/hierarchy/files_keys.rs`)
consume arrows, Space and Enter with no focus lock, so egui can move the keyboard to a neighbour
while the selection moves separately, and the ring now marks the one egui chose; each tab of the
dock is a button with a name and no selected state (`on_tab_button` in `src/ui/dock.rs`), so a
screen reader cannot tell which tab is in front; `name_the_focus` names and bounds only the stop the
keyboard is on, so a screen reader that browses by virtual cursor or by touch still meets an Unknown
node with no name, or the empty bounds of a `Ui` allocated with a sense, at every other splitter,
table cell and column handle, and its name is the first one to three labels inside the rect in
reading order, so a stop over a toast or a changing counter is named after it and a stop with four
labels or more stays unnamed; the accent swatches of the Settings window
(`src/ui/panels/settings_appearance.rs`) are 20-point regions with `Sense::click()` and only hover
text, which `name_the_focus` cannot name; `ctx.accesskit_node_builder` runs for the focused id even
where the id has no node that pass, so egui makes an empty node under the root and the code gives it
bounds, a phantom for a frame; a negative gizmo bubble with no letter is marked by its ring alone;
and every panel working from the keyboard is partly met, since the panels that need a file open are
tested only in their empty states and the 3D viewport's keys still act only under the pointer
([R59i](#r59i-shortcuts-by-character-live-while-typing)). What the checks of R59q and of this row
cannot see is [R68g](#r68g-what-the-accessibility-checks-still-cannot-see)'s.

**Done when.** The window enables AccessKit, a test finds the tree it sends carrying the names R59q
added, and the owner checks it with a screen reader; a test walks the dock's Tab stops and finds
none unnamed; a test finds the density graph's and the curve handles' click targets described; a
test draws a text whose literal alpha equals the fade and finds it judged; a test judges the
gizmo's, the axes' and the timeline's literal colours against what they sit on; a test draws a focus
ring on a widget clipped at its scroll area's edge and finds it inside the clip; a test finds an
unsupported file in a tree marked by more than colour; a test focuses the code editor and finds a
text field named, and tabs into it and finds no space inserted; a test dismisses a dialog with Enter
and finds no ring on a widget that is gone; a test presses a Ctrl chord that nothing consumes and
finds no ring; a test finds each dock tab's selected state; and the owner checks the ring, the
contrast and the second cues in a window.

### R68b A file nothing declares and a module at its gate

[↑ checklist](#phase-10--the-application)

**What.** Every `.rs` file under `src/` is declared by a module, and `src/state/mod.rs` has room for
the next module registered in it.

**Where it stands.** Done. The empty file src/ui/widgets/mod.rs, which no `mod` declared, is deleted, and
`every_rs_file_in_src_is_declared_by_a_module` follows the `mod` declarations from `src/main.rs` and
`src/cli_main.rs` as rustc does, `#[path]` included, and fails on a `.rs` file under `src/` that none
reaches; it failed naming that file before the deletion and passes after it. The edit gate's verdict
and the prompts a write into an install or the open of an Enhanced file waits on moved from
`src/state/mod.rs` to `src/state/pending_prompts.rs`, re-exported under the same names, and
`src/state/mod.rs` measures 798 code lines against the gate of 900
(`report_the_tree_by_code_lines`), room for 102 more code lines; a test-module registration counts as none.
The test reads a `mod` declaration only at column 0, so one indented inside an inline module would be
reported as a file nothing declares; no file does that now.

**Done when.** A test walks `src/` and finds no `.rs` file that no `mod` declares, and
`src/state/mod.rs` measures under the gate with room for a module declaration.

### R68c Tests that fail under load and errors a run logs

[↑ checklist](#phase-10--the-application)

**What.** The app's test run passes on a loaded machine and beside its own other tests, and logs no
error for a staged file the app refuses on purpose.

**Where it stands.** Found while closing [R68a](#r68a-what-the-accessibility-baseline-still-leaves)
(#563), [R25k](#r25k-a-drag-copies-and-a-change-of-nothing-does-nothing) (#570),
[R27q](#r27q-the-skeleton-files-own-model-can-be-hidden) (#566),
[R42h](#r42h-what-a-pick-still-misses-after-r42f) (#569),
[R20i](#r20i-what-archive-rows-still-get-wrong-after-r20h) (#565) and
[R59z](#r59z-small-faults-left-in-the-default-views-menus-and-keys) (#575):
`a_self_checks_stream_plays_to_a_silent_sink` failed once under a full `just ready` ("the speaker's
stand-in does not say what it was asked: []") and
`a_self_checks_first_launch_bake_asks_the_stand_in_once` in 2 of about 8 runs
("the bake's stand-in was asked 0 times: []"), both through `crate::platform::tests::said`, which
captures with `tracing::subscriber::with_default` and so probably loses a message whose callsite
another thread registered first; `a_setting_the_user_changes_is_written_to_the_preferences`
(`src/state/plugins/a_plugin_adds_to_the_editor.rs`) failed once in three runs and again with "no
entry found for key" under load, reads the real preferences file and spawns a release build of a
plugin inside the test; `a_first_save_into_the_overlay_keeps_what_it_was_made_from`
(`src/state/overlay_refresh.rs`) failed once under load; and
`the_find_bar_says_no_results_only_once_a_search_found_nothing` failed once with "Searching…" still
up after 10 s while other lanes loaded the machine, each passing alone and in the next full run. A
test that assumes no keys after `AppState::from_prefs(Prefs::default())` can pass alone and fail in
a full run, since the test binary's shared scratch configuration can hold keys another test cached;
the new status-bar tests isolate themselves with `keep_prefs_in`, and the others that assume no keys
were not audited. The run also logs an ERROR for the staged `gen9/markers.ytd` ("a .ytd at RSC7
version 2 is not a layout OpenRage declares") and for `platform__des_coroner_window.ybd` from
`opening_a_file_of_every_kind_and_drawing_every_panel_modifies_no_document` and tests like it, which
pass: the files are at versions no layout declares and the lines come from none of the changes that
showed them.

**Done when.** Each of the five tests passes in ten runs of the app's whole suite with another build
compiling beside it; a test that reads `said` finds a message another thread's callsite registered
first; no test assumes the scratch configuration holds no keys; and the run logs no error for a
staged file the app refuses on purpose.

### R68d What the scan for leftover folders still misses after R59y

[↑ checklist](#phase-10--the-application)

**What.** A test or a self-check that leaves a folder in the temporary folder, or a panic that loses
its crash report, is found.

**Where it stands.** Found while closing
[R59y](#r59y-what-a-test-run-or-a-self-check-still-leaves-behind) (#568): the two XMA oracle tests
(#564) left `or-media-xma-oracle` and `or-media-xma-spanned` in the temporary folder on any machine
with the corpus and an `ffmpeg` that has an xma2 decoder, the second even without `ffmpeg`, and both
skip here, so their conversion to `tempfile` was checked by the lint and the scan and not by a run.
The scan
(`crates/foundation/or-test-support/tests/no_test_leaves_a_folder_in_the_temporary_folder.rs`) drops
a line's text after its first `//` before it looks for a call, so a call after a `//` inside a
string on the same line is missed; `ALLOWED` works per file, so a new `temp_dir()`, `keep()` or
`into_path()` in an allowed file passes silently, and it scans production sources too, so a
production `temp_dir()` that is right needs an entry that argues it leaves nothing; `sources()`
follows a directory link or junction through `is_dir()`; and its prune list (`external`, `docs`,
`node_modules`) matches a folder of that name at any depth, so a crate's own `docs` folder is not
scanned, and none holds a `.rs` file today. A panic outside `catch_unwind` in a real `--self-check`
(in `AppState::new()` or a shots step) loses its crash report: the hook writes it under the scratch
folder and prints the path, then the guard of `run_removing` deletes the folder as the panic
unwinds, and no test drives a panic through it; the new copy test holds `staged_name` and not
`staging_dir`, so `staging_dir` could stop calling it and none would fail; and
`every_place_this_allows_still_makes_the_call_it_allows` and the copy test's failure with a dead
process's entry under this process's id were never seen failing.

**Done when.** A test runs each XMA oracle with its tools and finds nothing left; a test finds the
scan taking a call after a `//` inside a string, a new call in an allowed file and a crate's `docs`
folder; a self-check that panics leaves its crash report where the path it prints says; and a test
holds `staging_dir` itself.

### R68e Small faults left after R59z

[↑ checklist](#phase-10--the-application)

**What.** The faults R59z left in the default views, menus and keys are mended, and what its tests
could not see is seen.

**Where it stands.** Found while closing
[R59z](#r59z-small-faults-left-in-the-default-views-menus-and-keys) (#575): the faults its Left
paragraph lists sit in a row ticked done, which no open row owned. They are the same plural fault in
`src/state/capabilities.rs`, `src/ui/panels/hierarchy/ydd.rs`, `src/ui/panels/hierarchy/yed.rs`,
`src/ui/panels/hierarchy/yfd.rs`, `src/ui/panels/hierarchy/yft.rs`,
`src/ui/panels/hierarchy/drawable.rs`, `src/ui/panels/preview_settings.rs` and
`src/ui/panels/timeline_ycd.rs` ("1 drawables", "1 bones", "1 children", "1 filters", "1
expressions"), which `plural` in `src/ui/panels/inspector/summary.rs` fixes; a list in the Settings
window or in a panel that is no document's, which still closes when the active document changes,
since only a popup whose contents call `popup_of_the_shell` or `popup_of_a_document` stays;
`drive_find`'s unused `_keys` and `_read_only`, which cannot be dropped alone because
`FindKeyOutcome`'s `next` and `prev` are then written and read nowhere and clippy refuses the
leftover, so the whole F3 path in `src/ui/panels/text_editor/mod.rs` needs a look; Go to Line's
field, the import's "Invalid data:" prefix, `--fill-missing`'s wording, the Edition row after a save
of an unedited document, and the comparison's hover, "Click to copy full path", where a click copies
the whole "A compared with B" sentence. New with the review: Cut, Copy and Paste refused say
nothing, so Ctrl+V on a Files row with nothing on the clipboard no longer toasts "Nothing to paste",
which the owner may want back for a refusal whose focused panel selected something; a menu of the
shell now stays open when a load makes another document active, so Edit's Undo and Redo and File's
Save and Close act on the document active at the click and not the one the user opened the menu
over; Properties on a document row shows the tree's root, so an Open Folder lists Type Folder, a
hash of its name and its path beside dashes, and an install's own row (kind `GameRoot`) offers no
Reveal, Copy path or Properties; the corpus walk's "no row stated twice" is one hard-coded pair in
`worded_wrongly` and not a detector, so a row repeated in another view passes; the two status-bar
tests read `log_hover` over a private `LogBuffer` and not the drawn bar, and with no executable
`state.keys.job()` is none, so the clean-start test never runs the poll loop of `started_on`; the
dedupe that a held, refused key does not stack toasts has no test; and the Gallery's "1 texture" and
the `.ypt` LODs row of `ypt_model_section_lods` are reached only through the corpus walk, which
skips without a corpus, since the test the Roadmap cites for the toolbar's textures draws the
Gallery in Models mode.

**Done when.** A test walks every corpus kind and finds none of the plurals above; a test holds a
Settings list open across a load and finds it kept; a test presses Ctrl+V with nothing on the
clipboard and finds what the owner decides; a test opens Edit over one document, lets a load change
the active one and finds Undo naming it; a test right clicks an install's own row and finds Reveal,
Copy path and Properties; and the status-bar tests draw the bar, and a row stated twice in another
view fails the walk.

### R68f Comments and a module walk the UX lanes left

[↑ checklist](#phase-10--the-application)

**What.** A comment, a doc comment or a name the UX lanes left says what is so, and the walk that
finds a file nothing declares is held by a test for each rule it follows.

**Where it stands.** Found while closing
[R68b](#r68b-a-file-nothing-declares-and-a-module-at-its-gate) (#554),
[R49f](#r49f-compare-every-resource-and-archive) (#558),
[R49g](#r49g-what-a-comparison-still-misses-after-r49f) (#567),
[R55d](#r55d-what-converting-still-gets-wrong-after-r55c) (#573),
[R27p](#r27p-a-bones-numbers-can-be-edited) (#574) and
[R27t](#r27t-a-skeleton-is-seen-without-an-animation) (#583): the doc comment moved to `RetailGuard::Refused`
in `src/state/pending_prompts.rs` links `EditMode` as `prefs::EditMode`, and `prefs` is not in scope
in the new file, so the link no longer resolves; `src/state/compare/resource.rs` says "blob.s row";
the doc of `Cut::Depth` in `crates/domain/or-editor/src/outline.rs` says it lies `MAX_DEPTH` levels
down where the limit is the argument of `of_to`; the doc of `JobKind::Export` says one model out to
a file other tools open, but Export as… writes any row; the comment on `question_within` in
`src/ui/panels/file_prompts.rs` says the fixed width makes a long name wrap inside it, which R55d's
row disproves; `pack_root_summary` takes a `_state` it never uses; `is_unsaved` in
`src/state/compare/mod.rs` duplicates a helper that exists; `FieldRepr::trailing_element_size` has
no caller in the app and only the testimony of `or-io-verify` uses it; the
`FormatCapabilities::none` rows for GameRoot and Folder are dead, kept only by the exemption in
`every_reachable_format_has_an_inspector_or_a_described_root`; and the tab bar's 18 points are
repeated in `dock::draw` and in the dock test. Public doc comments run past one line where AGENTS.md
says one (`FileDevice::open_head`, `read_prefix`, `levels`, `edit_bone`,
`BoneAddress`, `rebuild_rest_pose`, `keep_prefs_in`, `reads_typed_text`, `named` and
`MAX_ELEMENTS`), the ⚠ paragraphs among them stating real constraints; and comments narrate the next
block, as in `src/ui/timeline/the_marks_of_the_timeline_read_at_3_to_1_on_their_rows.rs`,
`src/state/the_skeleton_files_own_model_can_be_hidden.rs` and
`src/state/a_skeleton_is_seen_without_an_animation.rs`, where three say what the assertion under
them checks.
`every_rs_file_in_src_is_declared_by_a_module` reads a `mod` only at column 0 and rejects
`pub(in path) mod x;`; its `#[path]` branch and the main, cli_main and mod rule for a module's
directory are exercised by no test, it passed on its first run and was never seen failing, and the
`#[path]` module that `src/render/meshes.rs` declares into
`crates/domain/or-interchange/tests/baseline/mod.rs` leaves that directory unjudged;
`src/doc_counts.rs` and `src/state/tests.rs` read `src/state/mod.rs` as text, so another move out of
it can break them as `every_confirmable_action_can_be_applied` was broken;
`no_file_in_src_exceeds_the_code_line_gate` asserts only a count over the gate, so no test failed
before the move that gave `src/state/mod.rs` its room, which the ignored
`report_the_tree_by_code_lines` measures and asserts nothing about; and `docs/reference/Crate_Reference.md`'s
table of `src/state` is not exhaustive, `pending_prompts.rs` having a row where `delete_prompt.rs`
and `save_destination.rs` have none.

**Done when.** `cargo doc` finds no unresolved link in `src/`; the typo and the comments above say
what is so; a test builds a tree with a `#[path]` module, a `pub(in path) mod` and a module in a
directory of its own and finds the walk following each; the line gate has a test that fails on a
file over it; and the table of `src/state` lists every file there.

### R68g What the accessibility checks still cannot see

[↑ checklist](#phase-10--the-application)

**What.** The checks of R59q and R68a fail when what they hold is taken out, reach the Settings
window and the window's own wiring, and say what they measured.

**Where it stands.** Found while closing [R59q](#r59q-an-accessibility-baseline) (#555) and
[R68a](#r68a-what-the-accessibility-baseline-still-leaves) (#563): 13 of the 18 tests R59q lists
were never seen failing (the ring's scroll and menu tests, both glyph tests, both italics tests, the
gizmo's, the naming test, the disabled-fade test and the 10-point test), and the only failing-first
evidence for its Done-when test is a count of 112 pairs that cannot be run again; `ringed()` in
`src/ui/the_keyboard_is_always_seen.rs` accepts any `ACCENT_TEXT` stroke of width 2 or more whose
rect contains the widget and does not check the ring's own rect, and the dialog test takes the
dialog's frame for the first rect filled with `window_fill` that contains the button, though the
theme sets `window_fill` and `panel_fill` to one colour; every ring test presses Tab in the editor
window's `Shell::draw`, `Shell::settings` serves only the contrast and naming scans, so the Settings
window's focus ring and its `name_the_focus` call have no test and the Tab walk never visits it;
only the editor's window and the Settings window call `focus_ring::draw`, so a caller that draws
`ui::draw` alone gets no ring; the contrast scan skips every text row whose mesh starts with a
background (`glyph_vertex_range.start > 0`), though its doc says it judges every run of text; the
pages for an inactive tab under the pointer, a folder with a row picked and each menu depend on
hard-coded pointer coordinates and nothing checks that the click landed, where the palette page has
a guard; the boxes over the viewport "read at 4.5:1 by construction" through `overlay_bg` with no
test of the worst scene behind it, and "every match has an outline" (`SEARCH_MATCH_EDGE`) with no
test that draws a find; the production wiring of the AccessKit tree, `init_accesskit`, the hop from
`user_event` to `on_event` and the hidden-then-shown order of the windows, has no test, since the
tests call `accessibility::on_event` and read the tree egui builds; the Tab walk ends silently at
the first repeated rect, skips a stop whose rect is not positive and gives up after 120 Tabs a
workspace, with 20 stops over all workspaces its only guard; the naming test checks that a name has
a speakable character and not that it means something; the timeline test says "the timeline" but its
`literals()` lists only the clip source's colours and not the particle timeline's `CHANNEL_COLOURS`;
the failing-first claims for the window tree, the Tab walk, the fade exemption, the axis test, the
timeline marks and the focus ring rest on the lane's word, and the density graph, the curve handle
and the unsupported-badge tests have no recorded failure; and no before screens were taken.

**Done when.** A test takes each check's subject out and finds that check fail; a test draws the
Settings window and walks Tab through it; a test finds the ring's own rect; a test removes the
AccessKit wiring and fails; the Tab walk says when it stops before it came round; and the contrast
scan judges a row whose mesh starts with a background.

### R68h A test floor that a deletion crossed

[↑ checklist](#phase-10--the-application)

**What.** `just ready` passes on `main`, and the suppression check's guard that its scan looked
somewhere is not one that deleting what it judges, as its own rule asks, can turn red.

**Where it stands.** Done. `a_suppression_that_hides_unused_surface_says_why` no longer asks the tree
for a number of suppressions: its floors (more than 30 parsed, more than 8 of an unused-surface lint)
are gone. So is the floor of more than 5 in `a_too_many_arguments_suppression_sits_on_a_function`,
the same defect in shape, which this row did not name: 20 such suppressions stand against it at
`863d2703`, so a deletion was not about to cross it, and it goes as a precaution. The scan is held to
having found a `.rs` file under `src/`, so deleting a suppression with its item, as the check asks,
cannot turn it red and a tree with none passes; a `.rs` file it cannot read is still skipped without
an error. The parser is proved on fixture folders in a
`tempfile` directory removed on drop:
`a_scan_of_a_folder_with_no_rust_file_is_refused_rather_than_judged_clean` (an empty folder, a folder
that is not there and a folder of no `.rs` file),
`a_suppression_with_a_sentence_beside_it_is_found_in_a_subfolder_and_passes`,
`a_suppression_with_no_sentence_is_named_by_the_check` and
`a_tree_with_no_suppression_at_all_passes_the_scan_and_the_check`. The check failed on `main` at
`863d2703` with 8 found against a floor of more than 8, and the empty-folder test failed until the
scan refused such a folder; each of the other three failed when the scan was broken in the part it
covers (a walk that
stops at the top folder, a check that names nothing, a floor of one suppression) and passed with the
break taken out. Two floors of the same kind, for example, stand outside this check and are not
touched, and neither was measured against the tree: the panel count in `src/ui/dock_commands.rs` and
the walked-file count in `crates/io/or-io/tests/what_still_names_the_envelope.rs`.

**Done when.** The suppression check passes on `main` with the suppressions the tree holds, a test
finds its guard green with one fewer, and a test finds it failing when its scan is pointed at a
folder with no `.rs` file.

### R68i Count floors that a deletion could still cross

[↑ checklist](#phase-10--the-application)

**What.** A test that guards its scan with a count is held to what the scan found, not to how many,
as [R68h](#r68h-a-test-floor-that-a-deletion-crossed) left the suppression check, or states the
measurement its floor stands on beside it.

**Where it stands.** Found while closing [R68h](#r68h-a-test-floor-that-a-deletion-crossed) (#581),
which took the floors out of its own check and named two of the same kind that it neither touched
nor measured: `every_registered_panel_can_be_offered_by_name` in `src/ui/dock_commands.rs`, which
asks for more than 5 panels, and `the_walk_reaches_the_tree_it_claims_to` in
`crates/io/or-io/tests/what_still_names_the_envelope.rs`, which asks for more than 400 files. Read
off the tree at `6f032310`, not run, the first floor stands against the 18 panel ids that `ids` in
`src/ui/panels/registry.rs` declares and the second against the 1226 `.rs` files that `git ls-files`
lists under `crates` and `src`, before the walk drops the envelope's own folder and its own file, so
neither is about to be crossed, as R68h found of the floor it took out as a precaution. The same
shape stands in the app's document gates and was seen, not measured:
`the_roadmap_is_an_index_of_itself` in `src/doc_counts.rs` asks for more than 50 checkboxes and more
than 50 sections, and `every_count_a_document_states_is_asserted_or_a_named_measurement` in
`src/doc_counts/figures.rs` for more than 20 documents and more than 300 counts. The tree was not
searched for the rest.

**Done when.** Every floor of this shape in a test under `src/` and `crates/` is listed with the
figure it stands against and the revision it was measured at, and each is either replaced by a guard
on what its scan found, as R68h did, or kept with that figure beside it; and a test points each
guard that stays at a scan of nothing and finds it fail.

### R68j What the suppression scan still skips

[↑ checklist](#phase-10--the-application)

**What.** The scan of the suppression check refuses a file it cannot read, as it refuses a folder
with no `.rs` file, each branch of its parser is proved on a fixture, and its doc comment says what
the code holds.

**Where it stands.** Found while closing [R68h](#r68h-a-test-floor-that-a-deletion-crossed) (#581),
whose review read the merged check at `6f032310`: `scan` in `src/suppressions.rs` refuses a folder
that holds no `.rs` file, but a file that fails `read_to_string`, for a permission or for bytes that
are not UTF-8, is skipped, so a scan in which every file failed to read returns no suppression and
every check passes on it, the case the doc comment of `scan` says the guard rules out when it says
the scan is held to having read a `.rs` file, where the guard checks that one was found. No fixture
holds a multi-line attribute, which `scan` gathers until its brackets balance, or an
`#[expect(..., reason = "...")]`, which `states_a_reason` takes as a reason, so each branch is proved
on the real tree alone, where a regression leaves the check green wherever the tree holds none of
that shape. The second paragraph of the same doc comment ends "The parser is proved on fixtures
instead.", which records what the change replaced and states no constraint, as the Comments section
of AGENTS.md asks it to.

**Done when.** A test points the scan at a folder whose only `.rs` file holds bytes that are not
UTF-8 and finds it refused, naming the file; a fixture holds a multi-line attribute and a test finds
it judged as one suppression; a fixture holds an `#[expect]` with a `reason =` and one without, and a
test finds the first passing and the second named; and the doc comment of `scan` says the guard as
the code holds it and no more.

### R69 One window of tabs and a rail

[↑ checklist](#phase-10--the-application)

A programme that replaces the workspace strip with tabs and a rail, which the owner chose on
2026-11-21
([Project_Context.md](Project_Context.md#the-window-becomes-tabs-and-a-rail-in-ten-rows-2026-11-21)).
What is worked on is a tab, whether a file, a folder, the project's overview or a content of the
project, and the panels arrange themselves by the kind of the tab on screen; a rail of Home, Game,
Project and Automate says where things come from. The parts are in the order they are built.
[R69a](#r69a-the-start-page-leads-with-what-is-installed-and-what-was-open),
[R69b](#r69b-one-menu-bar-with-or-without-a-project),
[R69c](#r69c-one-build-control-and-a-save-that-offers-what-can-happen) and
[R69d](#r69d-every-panel-has-a-door-and-every-tile-a-menu) change no model and each stands alone.
[R69e](#r69e-a-tab-for-each-thing-and-no-workspace-strip) is the change of model, and
[R69f](#r69f-a-rail-of-rooms-and-one-primary-action-a-tab), [R69g](#r69g-one-details-panel) and
[R69h](#r69h-one-undo-for-the-tab-on-screen) follow it. [R69i](#r69i-the-automate-room) needs only
the command table, and
[R69j](#r69j-the-guide-the-tour-and-the-design-say-the-window-as-built) closes the words. A finding
that fits none takes the next letter here.

Read off the tree at `90eb685c` on 2026-11-21 and not tried in a window, the shortest path a
newcomer finds takes six clicks to see a model of the install on a first launch, six clicks and two
saves to replace a texture and save it into the mods overlay, and six to eight clicks from the start
page to a built clothing pack. The parts aim at two, three and three.

### R69a The start page leads with what is installed and what was open

[↑ checklist](#phase-10--the-application)

**What.** The start page shows, with no scroll in the smallest window, the installs with their Open
buttons and what was open lately, then one row that starts a project: New project… and a chip for
each template. It fills the window on a first launch, and New project is one step with the name
filled in.

**Where it stands.** Built on 2026-11-21 but for one clause. `draw_start_page` in
`src/ui/panels/game_picker.rs` draws the installs first, the row of the install in use saying how
reading its keys went, then the recent projects, files and folders, then New project… with a chip
for each template of `goals` in `src/ui/panels/start_page.rs`, then the ways in; with nothing open
`src/ui/dock.rs` draws the page over the whole window
(`a_first_launch_shows_every_install_and_what_starts_a_project_above_the_fold_with_no_panel_around`,
seen failing on the page before it). The two explaining lines and the block about keys are gone
(`the_page_leaves_what_a_drop_does_to_the_hint_of_a_held_drag_on_every_launch`). The dialog names the
project after its template, numbered past a folder in the way, and selects the name
(`a_chip_opens_the_dialog_on_its_details_and_back_reaches_the_gallery`). Kept as it is: New
project… opens the gallery a step before the details, since the gallery is where a template is read
and a chip is the one step (`src/ui/panels/new_project.rs`).

**Done when.** A `just shots` screen of a first launch at the smallest window shows every install's
Open and the row that starts a project, and no panel around the page; an install's row says the
state of its keys and the block is gone; a chip opens New project on its template with a name Create
accepts; the gallery and the details are one step; and the two explaining lines are gone, the hint
of a held drag saying what a drop does
(`the_hint_of_a_held_drag_is_on_a_plate_inside_the_window_and_says_what_the_drop_changes`).

### R69b One menu bar with or without a project

[↑ checklist](#phase-10--the-application)

**What.** The menu bar reads File, Edit, View, Project, Automate and Help whether or not a project
is open, a row that cannot run is greyed with its reason, and each thing has one name in one case.

**Where it stands.** Built on 2026-11-21.
`project_menu_of_the_window` in `src/ui/panels/menu.rs` draws the Project menu with and without a
project (`the_bar_has_the_same_menus_with_and_without_a_project`), and More Project Commands is
gone: Open project in new window… is a row of File, Make this folder a project a row of Project, and
Remove active document from project one of its Properties and sharing. Tools is Automate. New
Session reads Close all, since it also leaves the project, under the id `new_project`, and the
palette still answers to the words a command had (`was` in `src/ui/commands/table.rs`). Open recent
is one submenu with a heading for each kind, and View ▸ Panels lists a panel once. A garment's menu
and the report of an imported pack say Reveal in the file manager, as the command does. Every
command is named in sentence case, so File says Open project… as the start page does. Show the build
keeps a wording of its own.

**Done when.** A test draws the bar with and without a project and finds the same top-level entries;
More Project Commands is gone and its commands are rows of File and of Project; Tools is named
Automate; New Session reads Close all under the id it has, so a binding kept in `prefs.json` still
runs it;
Open recent is one submenu with a heading for each kind; View ▸ Panels lists a panel once; a renamed
command is still found in the palette by the words it had; `every_menu_item_is_in_the_palette` and
`the_project_menu_fits_the_smallest_window_and_keeps_the_rest_in_submenus` hold; and each new label
has its French.

### R69c One Build control and a save that offers what can happen

[↑ checklist](#phase-10--the-application)

**What.** Build is one control at the top right of the window, beside what the edit mode says, and
the question a save asks offers only what can happen, with a change of mode and the write as one
act.

**Where it stands.** Built on 2026-11-21 but for its screens. The window's Build button is at the
right of the menu bar (`src/ui/panels/menu.rs`), with the build it makes, a dot and a word for how
it stands; the status bar and the Wardrobe's list hold none, the list keeping the chooser and the
state (`the_window_has_one_build_button_in_the_menu_bar_and_the_chooser_stays_under_the_list`), and
a build that fails brings the Build tab to the front where the screen docks it. The question a save
asks (`src/ui/panels/save_destination.rs`) says under an answer the mode would refuse which mode it
changes to, and choosing it changes the mode and writes; a refused write that offers a copy into the
overlay raises that question alone and not the choice of modes with it.

**Done when.** A test draws the Wardrobe and finds one Build control outside the Build tab, and none
in the status bar; an answer of the save question that the mode does not allow is offered as the
change of mode it needs, named as such, and choosing it changes the mode and writes, with no second
save; no two questions are raised in one frame; a build that fails brings the Build
tab to the front; and a `just shots` script replaces a texture and saves it into the overlay from
Read only in four actions. Whether a button labelled Save to mods may write the overlay with no
question is the owner's ([Appendix B](#appendix-b--judgement-calls-to-confirm)).

### R69d Every panel has a door and every tile a menu

[↑ checklist](#phase-10--the-application)

**What.** One field finds a file of the install by name, hash or pattern, every registered panel is
in a layout or behind a command that shows it, and a texture tile and the viewport answer a
right-click and a dropped image.

**Where it stands.** Begun on 2026-11-21. Search and Installed mods are tabs beside Files in
Browse's layout (`src/ui/dock.rs`), and Go to file… is a row of File and a button, Find, of the
Files panel (`src/ui/panels/hierarchy/navigation.rs`). A panel no default layout docks is named
with what shows it, and a test fails on one that is neither
(`every_panel_is_docked_by_a_default_layout_or_shown_by_something_named` in
`src/ui/panels/registry.rs`): Statistics has Help ▸ Statistics. A right-click on a tile of the
gallery and on the picture of the texture preview offers Replace from image… and Export texture…
from the registry, and an image dropped on a tile replaces that texture
(`src/ui/panels/gallery.rs`, `src/state/os_drop.rs`). A right-click in the viewport offers its
framing and a model's glTF commands (`src/ui/panels/viewport.rs`), one on the timeline plays, goes
to the start and shows the whole length, and a Files row's menu opens its edits and what it copies
from Edit and Copy as. The key of Go to File puts the focus in the field of Files, and what the
words name is listed under it with the scopes, the arrows and Enter the box of the palette has
(`draw_matches_of`); while the words only filter the tree, the list is a count that opens. The gate
of context menus walks the viewport's menu, a tile's and the timeline's
(`every_context_menu_draws_from_the_registry`), and a tile is named for a screen reader. Kept as it
is: with nothing open there is no Files field, so Go to file is the box of the palette.

**Done when.** The Files panel's field lists matches from the whole install as it is typed in, with
the scopes Go to File has, and the key of Go to File puts the focus in it; Installed mods and the
search of contents are docked in Browse's layout; a test fails on a registered panel that no layout
docks and no command reveals, and Statistics passes it or is removed; a right-click on a tile and in
the viewport offers its commands from the registry (`every_context_menu_draws_from_the_registry`),
Replace from image… and Export… among them; an image dropped on a tile replaces that texture as one
step of undo; and a Files row's menu groups its extract, edit, copy and compare commands in
submenus.

### R69e A tab for each thing and no workspace strip

[↑ checklist](#phase-10--the-application)

**What.** A content of a project and the project's overview open as tabs beside files and folders,
the arrangement of panels follows the kind of the tab on screen, and the strip of workspace buttons,
its More and the pinned buttons are gone.

**Where it stands.** Begun on 2026-11-21. The overview of a project and the screen of each kind of
content it holds are tabs before the tabs of its files (`screens_as_tabs` in
`src/state/workbench/strip.rs`, drawn by `src/ui/panels/document_tabs.rs`), a file's tab is not the
one on show while such a screen is, and the strip draws only the workspaces that can show the file
in front while more than one can (`workspaces_in_the_strip`): it is the View as switch. Ctrl+1 to
Ctrl+9 go to the tab of that place under the ids they had (`tab_places`). Two contents of one kind
are a tab each, named for the content, and one is on show (`screen_tabs`,
`a_project_with_two_contents_of_one_kind_has_a_tab_for_each_and_one_is_on_show`). Kept as it is: the
tab of a screen sets the workspace, which no other control does; an arrangement is saved by
workspace (`src/ui/dock_layout.rs`), and the ids a saved layout holds are read through a table of
the ids that changed, so `LAYOUT_SCHEMA` is not raised and no layout is discarded; a project keeps
its pinned buttons, which choose its tabs.

**Done when.** Each row of the table of task screens opens its content as a tab, and the overview is
one; the workspace on screen is derived from the active tab and no control sets it, `Workspace`
staying the name of an arrangement; a tab whose file suits two editors, a model seen as its
textures, has a View as switch; no editor is drawn with no tab of its kind; `LAYOUT_SCHEMA` is
raised, and a test reads a layout of the version before and finds it discarded whole with the rest
of the preferences kept; a project's pinned buttons are carried unread; Ctrl+1 to Ctrl+9 go to the
tab of that place under the ids they have; `no_site_matches_on_the_kind_of_a_task_screen` and
`a_second_task_screen_is_one_row` hold on the tabs; and the `workspace` step of a `just shots`
script still shows the arrangement it names.

### R69f A rail of rooms and one primary action a tab

[↑ checklist](#phase-10--the-application)

**What.** A rail at the left edge switches the side column between Home, Game, Project and Automate,
with Help and Settings at its foot, and a bar under the tabs says where the tab's thing is and
offers one primary action for its kind and at most three more.

**Where it stands.** Begun on 2026-11-21. A rail at the left edge has Home, Game, Project and
Automate, with Help and Settings at its foot (`src/ui/panels/rail.rs`). A room brings its panel
forward in the side column of the screen in front, which stays in front (`reveal_in_the_side` in
`src/ui/dock_commands.rs`), and the room of the panel in front is lit. A bar under the tabs says
where the file in front is and offers its primary command, filled, with at most three more
(`context` in `src/ui/panels/document_tabs.rs`), and every kind of file has exactly one
(`primary_of`). The commands the headers of the Wardrobe, the Weapons and the Map screens held are
at the right end of the row of tabs while the screen is in front (`bar` of its row in
`src/ui/task_screens.rs`), and the Project panel is no longer docked on those screens. With no
project the Project room runs New project…. Kept as they are, by choices made before this row: the
Outline is a panel of the screens that have one and not the foot of every room, the project's name
is a popup of the status bar, and a content's problems are counted where each screen says them. It
follows
[R69e](#r69e-a-tab-for-each-thing-and-no-workspace-strip).

**Done when.** Each room shows its column: Game, the field that finds a file, the tree, the places
and the installed mods; Project, Content and Builds, or New project… and Open project… with no
project; Automate, what [R69i](#r69i-the-automate-room) builds; and Outline at the foot in every
room. One table gives each kind of tab its primary command, and a test finds exactly one for every
kind and finds it drawn filled in the bar; the header commands of the Wardrobe, the Weapons and the
Map screens are in the bar; the Project panel is not repeated in a content's tab; the problems of a
content are counted once, on the Problems tab; and the screens of each room at the smallest window
are looked at and described in the pull request.

### R69g One Details panel

[↑ checklist](#phase-10--the-application)

**What.** One panel on the right, Details, shows whatever is selected in the tab on screen, in place
of Inspector, Properties, Garment, Weapon and the map's Selection.

**Where it stands.** Begun on 2026-11-21. One panel with the id `details`, titled Details, draws
what the screen in front selects: what the row of a task screen names (`details` in
`src/ui/task_screens.rs`), a project's properties on its overview, else the fields of the open file
(`src/ui/panels/registry.rs`). A saved layout or a plugin that names one of the five ids gets it
(`current` in `src/ui/panel_registry.rs`), and a workspace whose saved arrangement held two of them
starts over (`src/ui/dock_layout.rs`). The Properties of a row of Files draw at the top of Details
and no longer in a window (`src/ui/panels/hierarchy/files_menu.rs`). The options of a playback are
behind a Preview options button at the viewport's top left and are no tab of the Animation
workspace (`src/ui/panels/viewport.rs`). The panel that drew them is gone, and a saved layout that
names it comes back without that tab
(`a_layout_naming_a_panel_that_is_gone_comes_back_without_its_tab`). It follows
[R69e](#r69e-a-tab-for-each-thing-and-no-workspace-strip).

**Done when.** A selection of any kind, a file's field, an asset, a garment, a weapon or a placed
entity, draws in the panel with the id `details`; a saved layout or a plugin that names one of the
five ids gets Details, by a table of old ids read before the registry; what the window for a row of
Files shows is in Details, and one command is named Properties; the playback options are in the
viewport's own bar and one panel is titled Preview; and the on-screen tests of the Wardrobe, the
Weapons and the Map screens find their fields in it.

### R69h One undo for the tab on screen

[↑ checklist](#phase-10--the-application)

**What.** Undo and Redo act on the tab on screen, whatever it holds, and the History panel lists
that tab's steps.

**Where it stands.** Read off the tree at `90eb685c` on 2026-11-21: the command table
(`src/ui/commands/table.rs`) holds Undo, Undo project change, Undo clothes change, Undo weapon
change, Undo map change and Undo import, each but the last with its Redo. The key of Undo reaches a
document's history, the project's on the Project workspace and the project's joined with the
clothes' on the Wardrobe. Since 2026-11-21 it also reaches the stack of the Weapons and of the Map
screen, whose rows of the table of task screens name their undo and redo
(`undo_and_redo_on_a_screen_that_names_its_own_are_that_screens`), and the Project menu's History
submenu holds no undo of a kind of content. The commands are still six, and the rest follows
[R69e](#r69e-a-tab-for-each-thing-and-no-workspace-strip).

**Done when.** A test edits on a Weapons tab and on a Map tab, presses the key of Undo and finds the
edit taken back, having been seen to fail before the change; Edit ▸ Undo names the step it would
take back; the Project menu's History submenu holds no undo of a kind of content; the ids of the
commands it replaces still run Undo, so a binding kept in `prefs.json` works; and Undo import stays
on its report and its toast.

### R69i The Automate room

[↑ checklist](#phase-10--the-application)

**What.** The app shows the command line of what was just done and runs one command over many
files: a log of the commands run, Copy as command, a batch, and the plugins in one place.

**Where it stands.** Begun on 2026-11-21. A command of the table states its command line through
one field (`cli` of `Spec` in `src/ui/commands/table.rs`): Convert edition…, Export to glTF…,
Replace from glTF…, Create DLC pack… and Build have one. Automate ▸ Command log lists each command
run in the window with Run again and, where it has a line, Copy as command
(`src/ui/panels/command_log.rs`). The help of `openrage-cli` lists every verb it runs, and
[CLI_Server_Protocol.md](../reference/CLI_Server_Protocol.md) every verb of `openrage-cli project`.
Three of the lines name the file in front, and a test has the CLI read each line with the parser
of its command (`accepts` of `or-cli`). The log says what was in front when a command ran.
Automate ▸ Batch… runs Extract and Convert edition over the rows selected in Files and exports
their models to glTF or their textures to PNG as one job (`src/state/batch.rs`), with the
`openrage-cli bulk` line beside them. The plugins are run, installed and set from a Plugins panel
Automate opens, and Settings has neither their page nor a page of a plugin's own
(`src/ui/panels/settings_plugins.rs`). A batch walks a folder selected down the rows Files lists
under it, and runs over the results selected in Search or in a search of every archive, each read
as its export reaches it (`batch_inputs`).

**Done when.** A command of the table states its command line through one field, and a test runs
the CLI's own parser on each line it writes; a log lists each command run with what it acted on, Run
again and, where it has a line, Copy as command; Batch runs Extract, Convert edition, Export to glTF
or a texture export over the selection, the matches of a find or a folder as one job, and gives the
`openrage-cli bulk` line that does the same; the plugins are run, set and installed from Automate,
and Settings has no Plugins page; the help lists every verb, and the protocol document every project
verb. The room is a menu until [R69f](#r69f-a-rail-of-rooms-and-one-primary-action-a-tab) gives it a
place on the rail.

### R69j The guide the tour and the design say the window as built

[↑ checklist](#phase-10--the-application)

**What.** The guide, the tour, the French and the design documents describe the window the parts
before this one built.

**Where it stands.** On 2026-11-21 the verdict of
[Workbench_UX_Design.md](Workbench_UX_Design.md#verdict) says the tabs of a project's screens, the
rail, the bar under the tabs, Details and the menus as the parts written that day leave them, and
its inventory names Details where it named the five panels;
[Workbench_Guide.md](../guides/Workbench_Guide.md) says Ctrl+1 to Ctrl+9 go to a tab; and a test
holds each step of the tour to a panel its screen docks
(`every_step_of_the_tour_shows_a_panel_its_screen_docks` in `src/state/front_door/sample.rs`).
The rules of the design say the tab row where they said the strip.

**Done when.** The verdict of the UX design states the rules of the window as built, its pages are
the rail's rooms and the kinds of tab, and its inventory names no control that is gone; a test finds
each step of the tour pointing at a control that is drawn; the guide's sections keep their anchors,
since tooltips name them; `src/i18n/languages/fr.po` has every new sentence; and the gates of
`just ready` pass.

### R70 The GTA V world editor

[↑ checklist](#phase-11--the-gta-v-world-editor)

A programme that brings every capability of NodeRunner Studio for GTA V into OpenRage, which the
owner asked for on 2026-11-25
([Project_Context.md](Project_Context.md#every-noderunner-studio-capability-for-gta-v-is-wanted-in-openrage-2026-11-25))
and said the same day to build in parallel lanes
([recorded](Project_Context.md#the-world-editor-plan-is-built-in-parallel-lanes-2026-11-25)).
[World_Editor_Plan.md](World_Editor_Plan.md) owns what it rests on, how it fits the tree, the order,
each part's brief and the rulings asked; [World_Editor_Parity.md](World_Editor_Parity.md) weighs
each capability; [World_Editor_UX_Design.md](World_Editor_UX_Design.md) says what a mapper would
meet. NodeRunner's licence forbids a work based on it, so a part is built from those documents and
from measurements of the install
([Licence_And_Provenance_Policy.md](../guides/Licence_And_Provenance_Policy.md#sources)).

A stage is a point at which the result can be told to a mapper, not a gate: a part starts when the
parts it depends on have merged. [R70a](#r70a-what-the-game-loads),
[R70b](#r70b-the-gta-v-world-index), [R70h](#r70h-one-vocabulary-of-world-edits) and
[R70w](#r70w-a-changed-bound-and-a-changed-path-region-as-sources) stand on nothing of this
programme, and [R70g](#r70g-collision-as-a-query) needs R70b only for the bounds at a position.
[R70c](#r70c-the-gta-v-world-streamed-and-drawn) is what the screen parts wait for, and with
[R70e](#r70e-find-anything-and-go-there), [R70i](#r70i-selecting-and-moving-in-the-world) and
[R70q](#r70q-a-build-a-server-and-a-story-install-take) it is the first thing to hand a mapper: open
the map, find a place, duplicate an object and swap its model, drop it to the ground, build a
resource. The data parts of [R70m](#r70m-interiors) and
[R70o](#r70o-parked-vehicles-zones-lights-and-occluders) wait for no screen part. A screen part and
an open part of [R69](#r69-one-window-of-tabs-and-a-rail) that edit the same shell file never run
together.

A part's Done when is its check; its brief says what to read and what to build. A part that writes a
kind of file adds it to the recipe and the sheet of R70a. Lanes began on 2026-11-25: parts of R70a,
R70b, R70d, R70e, R70g, R70h, R70q, R70r, R70w and R70y are built, with no part closed, no install
suite run and nothing loaded in a game. R70 has no letter left: a finding that fits no part opens
the next number.

### R70a What the game loads

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** A sheet of what to look for in a running game, a line for each kind of file a part of R70
writes, each with the date, the game's build and the result; a test that reads its format; the Build
tab's reading of it; a guide page with the steps to load a build on a FiveM server and in story
mode; and the sample as a recipe that makes its sources on the person's machine over their own
install, never a project in the repository ([brief](World_Editor_Plan.md#what-the-game-loads-r70a),
[In_Game_Acceptance.md](../guides/In_Game_Acceptance.md)).

**Where it stands.** The sheet, its reader, its command and its guide are built; the Build tab's
reading, a run of the recipe over an install and every result are not. The sheet is
`crates/domain/or-project/src/acceptance.json`, with the lines of a map changed, of a map made from
nothing ([R70h](#r70h-one-vocabulary-of-world-edits)) and of a story pack, and no result: who runs
the game is held back in [Workbench_Plan.md](Workbench_Plan.md#held-back). `or_project::acceptance` refuses a line or a
result that is wrong, with the line's id (`a_line_that_is_wrong_is_refused_with_its_id`,
`a_result_that_is_wrong_is_refused_with_the_id_of_its_line`), a row a line names has a heading
here (`every_line_names_a_row_of_the_roadmap`), and `Sheet::about` gives a build the lines about
the kinds of file it wrote, with the newest result of each for an edition
(`a_build_that_wrote_a_map_and_a_type_file_is_told_their_lines_and_that_nobody_checked_them`).
`openrage-cli project acceptance` prints them for a resource or a pack a build wrote, and refuses
a folder that is neither (`a_folder_no_build_wrote_is_refused`). The guide's table is the data
file's lines (`the_guide_lists_the_lines_of_the_sheet`), its example of a result is one the sheet
reads (`the_guide_writes_a_result_the_way_the_sheet_reads_one`), and the recipe's
`openrage-cli project` commands are read from the guide and run over synthetic files
(`the_recipe_builds_a_resource_and_the_command_names_the_lines_about_its_files`). A line whose kind that
recipe does not build fails `the_sheet_and_the_recipe_are_about_the_same_kinds_of_file`, and so
does a kind it builds, and a build reads back, that has no line.

Left: the Build tab's line, a change to the app; the recipe run headless over an install, in the
slow tier; a kind a part writes that is in neither the recipe nor the sheet, which no test fails
for, as today a bound and a path region from a source
([R70w](#r70w-a-changed-bound-and-a-changed-path-region-as-sources)) and the manifest of a build's
maps ([R70q](#r70q-a-build-a-server-and-a-story-install-take)); the step of the recipe that makes a
map from nothing; a map in a story pack
(`a_story_pack_refuses_the_sample_as_the_guide_says_it_does_today`); and the results.

**Depends on.** The plan's ruling 1, whose default is that the owner fills the sheet, once a stage.

**Done when.** The sample builds headless over an install and every file reads back, in the slow
tier and named in the skip ledger without an install; a kind of file a part writes and the recipe
lacks fails a test; a test reads the sheet's format; the Build tab says "not checked in game" for a
kind whose line is empty; and the guide page states the steps for a FiveM server and for story mode.
Filling the sheet is the owner's.

**Risk.** Without it no later part is known to load in game, the clause
[R62](#r62-the-rbm-layout-synthesiser) is still open on.

### R70b The GTA V world index

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** The index of a mounted GTA V install: every map with its extents, parent and the group
that requests it; every archetype with its type file and box; and two queries by position, the maps
and the bounds. The mount's cutoff at a level of DLC is passed from the screen, the choice of story
or multiplayer is added to `or-title`, and the index is keyed by what the install holds, the cutoff
and the choice, kept on disk and rebuilt only where the install changed. It adds
`openrage-cli world at` and `openrage-cli world index`
([brief](World_Editor_Plan.md#the-gta-v-world-index-r70b)).

**Where it stands.** Two parts are built on installs made by hand; no install suite has run.
`crates/domain/or-depends/src/world_index.rs` reads every map, type file, manifest and bound an
archive at a time, answers the maps and carried bounds at a position and is kept on disk by each
archive's stamp (`the_world_index_of_a_mount`), behind `openrage-cli world index` and `world at`.
`DlcCutoff::Level` stops the mount at a pack of the DLC list and `MountOptions::mode` takes the map
as story or multiplayer
(`a_mount_stops_at_a_level_of_dlc_and_takes_the_map_as_story_or_multiplayer`), set only by
`openrage-cli dlc-list`. The older cutoff by a pack's name hash, which `--dlc` is, keeps the
folders of the packs past it from the walk as a level does
(`a_pack_past_a_name_hash_is_left_out_of_the_walk_as_one_past_a_level_is`), and an archive one of
whose files did not open is kept with no stamp, so the next build reads it again
(`a_file_that_did_not_open_is_read_again_by_the_next_build_though_its_archive_stands`). Left: both in the index's key and from a screen; the group that requests a
map; the cost on an install.

**Done when.** Over a mounted install the index names every map and type file the mount lists, and a
position inside a known map's extents answers that map, in an install suite of the slow tier, named
in the skip ledger without one; in the same suite the bounds answered at a position are the ones
whose own boxes hold it; a second start opens no archive that did not change; an index kept at one
cutoff is not read at another; story hides what multiplayer adds, on a synthetic mount; and
`openrage-cli world at` answers a position.

### R70c The GTA V world streamed and drawn

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** A GTA V install drawn through the world renderer, with its models and interiors, for both
targets, in the World room and as a Map content's Scene. It lands in parts: the seam between what
the World screen takes from a title and what it shares; the viewer made one for each world tab; the
GTA V feed from R70b, with the content a project links as a layer; then the room, the tab, the stack
as a bar under the tabs, the World tab's primary command, the shared camera and the viewer's presets
and frame cap ([brief](World_Editor_Plan.md#the-gta-v-world-streamed-and-drawn-r70c)).

**Where it stands.** Not built. A GTA VI install alone feeds the world renderer. World is a
workspace shown while a GTA VI folder is set, with one viewer (`src/state/gta6_worldview/mod.rs`);
the Map screen draws boxes, a scanned GTA VI map body too.

**Depends on.** [R70b](#r70b-the-gta-v-world-index); its room, tab and bar edit what
[R69f](#r69f-a-rail-of-rooms-and-one-primary-action-a-tab) builds.

**Done when.** A synthetic world of parent and child levels draws the level the rules select at each
distance, in real frames with no window; an install's tab and a project's tab draw two stacks in one
session; the GTA VI screens and a scanned GTA VI map body draw the same frames as before the seam,
and [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md) states their new camera inputs; a
synthetic interior draws its objects; a linked project's map draws in the linking project's tab; the
French screens of the world hold at the smallest window; the bar names the stack and says when the
install shown is not of the build's edition; the World tab has its primary command in R69f's table;
a preset sets the distances and the frame cap it names; and a frame over the city is measured on the
owner's machine, its budget recorded.

### R70d The day sky water and shadows of the world

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** GTA V's timecycle, weather and water fed into the lighting, the shadow cascades, the sky
and the water the World screen has for GTA VI, behind the seam of R70c, at an hour and a weather the
person sets; a reader for GTA V's `water.xml` in `or-io` and the water it describes; the distance
shadows reach as a setting; and a measure of what drawing GTA V's materials through the game's own
shaders costs over the whole map, for the plan's ruling 9
([brief](World_Editor_Plan.md#the-day-sky-water-and-shadows-of-the-world-r70d)).

**Where it stands.** The water file reads and saves; nothing draws it, and nothing feeds GTA V's
day to the world. `CWaterData` in `crates/io/or-io/src/gta5/data/water.rs` opens a `water.xml` into
its water, calming and wave quads, keeps every element and attribute it does not type and writes
them back, so an unchanged file saves to the bytes it came as and a changed value is written among
them; a quad or a list the document did not hold is written with the document's own line endings
and indentation (`what_the_document_did_not_hold_is_laid_out_as_the_document_is`); `height_at`
answers the height of the water over a point, with a cut quad's half and a quad the game leaves
out. `openrage-cli convert` and the formats census take the file by its name.
Documents written by hand hold it, in `crates/io/or-io/src/gta5/data/water/tests.rs`:
`a_document_reads_to_its_quads_and_saves_to_its_fixed_point`,
`an_element_and_an_attribute_the_reader_does_not_name_survive_a_read_and_a_save`,
`the_height_at_a_point_inside_a_quad_on_its_edge_and_outside_every_quad` and
`each_refusal_names_its_element` among them. ⚠ No install's water file has been read: the list of
what the reader types awaits one, `crates/title/or-title/tests/the_water_file_of_an_install.rs`
holds the reader on a mount built by hand, and its test over a real install is in the slow tier and
has not run. For a GTA VI install the world has the install's day, the sun's shadow in cascades
(`crates/runtime/or-renderer/src/world/shadow.rs`), the game's lighting passes, lamps, a sky and
water ([the day](../features/GTA_VI_World_Viewer.md#the-day)). For GTA V the Shaded viewport lights
one model from the install's timecycle
([Viewport_Shading_Modes.md](../features/Viewport_Shading_Modes.md)). Left: the water drawn, the
feed of GTA V's timecycle and weather into the world's lighting, the shadow cascades and the sky,
the distance shadows reach as a setting, and the measure of the game's own shaders.

**Depends on.** [R70c](#r70c-the-gta-v-world-streamed-and-drawn).

**Done when.** The reader opens the water file of both installs and saves its fixed point; the same
hour and weather give the same sun on a model in Shaded and in the world; a GTA VI day draws the
same frames as before; the water of a synthetic file draws where the file puts it; the shadow
setting changes how far a shadow is cast in a drawn frame; and the cost of the game's own shaders
over the whole map is measured and recorded with its date and its machine.

### R70e Find anything and go there

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** Places in the palette (a position in the forms scripts write, a hash in each spelling, a
model, a map, a type), listed in the box and under the field of Files, a position or a hash first; a
model's row with its count of placements, stepped through from the nearest; Show in world on a
file's row, an archetype's row and a row of Uses and Placed by; bookmarks a person saves from the
box, each with a key to go to it; Copy for script in Lua, JSON and C#, as templates in data, the
button keeping the form used last; and `openrage-cli world find`
([brief](World_Editor_Plan.md#find-anything-and-go-there-r70e)).

**Where it stands.** Two modules are built in `or-maps`, which nothing calls yet:
`crates/domain/or-maps/src/place.rs` reads a pasted position, a hash in each spelling or text, and
`crates/domain/or-maps/src/script_text.rs` writes a position in Lua, JSON and C# from
`crates/domain/or-maps/src/script_forms.json`, each held by its own tests
(`just test-crate or-maps`). Left: every part on screen and `openrage-cli world find`. The palette,
Go to File and the name index exist ([R59j](#r59j-one-command-registry), [R24b](#r24b-go-to-file),
[R23](#r23-names-and-hashes)).

**Depends on.** [R70b](#r70b-the-gta-v-world-index);
[R70c](#r70c-the-gta-v-world-streamed-and-drawn) to go there; [R70g](#r70g-collision-as-a-query) for
the height of two numbers.

**Done when.** Each form of a position parses to the same point; a hash typed three ways finds one
model; a copy in each form parses back to the position and the heading it came from; two numbers
over a known road go to the road and not to sea level; a model placed twice steps from one placement
to the other; a pasted position is the first result with a file open; Show in world on a row moves
the camera to that row's place; a bookmark's key goes to it after a restart; and
`openrage-cli world find` prints the place a query names.

### R70f Layers of what the game hides

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** Layers, each drawn within its radius of the camera: road nodes and links by category,
navmesh by category, scenario points with their heading, collision by material and by version, rooms
and portals, occluder boxes and occlusion meshes with a preview of what they hide, the levels of
detail, and sound zones and fixed emitters, each sound played when selected; parked vehicles,
timecycle zones and far lights once the data part of R70o types them; presets; the pick order; and
the list of maps only a script requests, with a stated naming rule for two versions of one place
([brief](World_Editor_Plan.md#layers-of-what-the-game-hides-r70f)).

**Where it stands.** Not built. The viewport draws a `.ynv`'s polygons ([R33b](#r33b-navmesh-ynv)).
`crates/domain/or-world/src/map_data.rs` reads a box occluder, the stored box of an occlusion mesh
and a parked vehicle's position and one length; it only counts the timecycle zones and tests whether
far lights are present. The GTA VI World panel lists the script-managed groups, and no screen uses
`crates/domain/or-world/src/state.rs` for GTA V.

**Depends on.** [R70c](#r70c-the-gta-v-world-streamed-and-drawn) and
[R70g](#r70g-collision-as-a-query); the data part of
[R70o](#r70o-parked-vehicles-zones-lights-and-occluders) for parked vehicles, timecycle zones and
far lights.

**Done when.** Each layer draws a synthetic file's records at their places in real frames; a preset
shows the layers it names and no other; a layer off costs no draw; a click where two layers overlap
takes the thing the pick order puts first; selecting a synthetic emitter starts its sound; and the
list names the maps of a synthetic manifest that only a script requests, two versions of one place
under the stated rule.

### R70g Collision as a query

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** In `or-world`, over declared bounds handed in: a ray, a swept sphere and a box overlap,
with the hit's material, and the ground under a point that ignores walls. Then the set of bounds
near a position, read through the index of R70b and kept by the caller that has the mount,
`or-depends` for the CLI and the app's streamer for the screen. The particle collider becomes a
caller, and `openrage-cli world ground` answers a position
([brief](World_Editor_Plan.md#collision-as-a-query-r70g)).

**Where it stands.** The primitives are built in
`crates/domain/or-world/src/collision/`: `BoundCollider` is a bound's exact shapes under a tree, and
`CollisionSet` holds the colliders its caller hands it by key and answers a ray, a swept sphere, a
box overlap and the ground, held on bounds built by hand
(`crates/domain/or-world/tests/collision_queries.rs`,
`crates/domain/or-world/tests/collision_world.rs`). Of the staged corpus, 948 bounds become
colliders with none refused
(`every_staged_bound_becomes_a_collider_whose_ray_and_swept_sphere_agree`, 2026-11-25), and 897 of
971 geometries stop 40 mm short of the box they state
(`every_staged_geometry_lies_in_the_box_it_states`, 2026-11-25), so whether the game's surface is
the polygons is open. Left: a caller that hands the set the index's bounds;
`openrage-cli world ground`; the install check; and the particle collider, `BvhCollider` in
`crates/runtime/or-simulation/src/collision.rs`, unchanged because it makes triangles of the curved
shapes the query meets exactly, which is the owner's to rule on.

**Depends on.** Nothing for the primitives; [R70b](#r70b-the-gta-v-world-index) for the bounds near
a position.

**Done when.** Each primitive of a bound is hit where arithmetic says, on bounds built by hand; the
ground under a point beside a wall is the floor and not the wall's top, on bounds built by hand;
over an install, the ground under a position on a known road is within a step of the road's nodes
there, in the slow tier; `openrage-cli world ground` answers that position; and the particle
preview's collisions are unchanged.

### R70h One vocabulary of world edits

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** Named operations with inverses for what a world tool does, as data that Undo, History and
`openrage-cli maps apply` share; the working set's renumbering across files carried over the typed
roots; the first edit of a game object importing its map as a source with its base; a new map and a
new entity made from a donor of the mounted install; and the Map screen's Undo moved into the
project's History ([brief](World_Editor_Plan.md#one-vocabulary-of-world-edits-r70h)).

**Where it stands.** The vocabulary, for one pack: `Operation`, `apply`, `apply_all` and `Session`
in `crates/domain/or-maps/src/operation.rs`, which the Map screen's edits and
`openrage-cli maps apply` call (`an_operation_and_its_inverse_leave_the_sources_as_they_were`, on
synthetic packs). A session holds the sources as typed roots and writes them once at a flush, as
`apply` writes them one by one; an inverse states the digest or the session token it applies to,
or is refused; and the Map screen's Undo keeps the operation and its inverse
(`a_session_of_map_edits_writes_once_what_each_edit_writes`). On a synthetic map of 20,000 entities
a pose edit took 2.4 s one by one, and 0.83 ms with one flush of 2.2 s in a session
(`one_pose_edit_is_timed_on_synthetic_maps_of_two_thousand_and_twenty_thousand_entities`, release,
load average 70, 2026-11-25). And a map, a built entity, a type file and a base archetype made
with no donor and no snapshot (`crates/domain/or-world/src/create.rs`), behind
`openrage-cli world new-map` and `world archetypes`, which pass the rules of a pack
(`a_map_and_a_type_file_made_from_nothing_enter_a_pack_and_break_none_of_its_rules`).

⚠ They are written as PSO, and RBM is refused (`rbm_is_refused_with_the_word_nothing_derives_named`,
[R62](#r62-the-rbm-layout-synthesiser)); the game's own world files are RBM, so whether it loads
one written as PSO is a line of [R70a](#r70a-what-the-game-loads)'s sheet, and none is made from an
install's donor. The first entity or archetype of a file that holds none is built that way,
the last one taken out is put back by the XML, and an entity of an archetype the pack does not
declare is held by the extents where it stands, said in a note
(`crates/domain/or-maps/tests/an_operation_where_the_pack_holds_too_little.rs`). A deletion
planned by [R70k](#r70k-removing-what-the-game-placed) renumbers the links of the child maps; one
entity taken out alone leaves them, which the rule `parent-link-broken` names once one is past the
end. Left: the Map screen on a session, a create across files, the import, a new map and a built
entity as operations, the donor and the install's boxes, History, the install measure.

**Depends on.** Nothing in R70; it does the part of
[R69h](#r69h-one-undo-for-the-tab-on-screen) a Map tab needs below the screen.

**Done when.** Every operation followed by its inverse leaves the sources byte for byte, and a test
fails when an operation has no inverse; a list applied by the CLI and by the screen gives the same
sources; a deletion with its chain runs under `OPENRAGE_SCHEMA=none`; a new map reads back as its
root, passes the rules and carries no zero signature; an edit is taken back after the project is
opened again; History goes back over a first edit that imported; a first edit over the other
edition's install writes nothing, and a rule names a source whose base is the other title's; the
install suite counts the maps a first edit refuses; and one drag on the largest map of an install
and what History keeps for a run of edits are measured.

**Risk.** A large game map is a large XML written at each edit. A map made with no donor would carry
a zero signature word ([R62](#r62-the-rbm-layout-synthesiser), open).

### R70i Selecting and moving in the world

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** The pick by pixel with its second click; the rectangle; the group move; the choice of
axes; copy, paste, duplicate and its repeat; the snap to another object's box; the swap of a model;
the drop to the ground and the follow; the manipulator in the world; the tools' keys bound by where
a key sits; the line that offers a project, the first edit's toast and Edit in the project; each
tool's hints; an object's right-click menu and double click; and the safe fix of `guid-twice`
([brief](World_Editor_Plan.md#selecting-and-moving-in-the-world-r70i)).

**Where it stands.** Not built. The Map screen's manipulator, snaps and typed fields work over boxes
([built](Workbench_UX_Design.md#what-the-map-screen-builds)), and no command targets a placed thing
(`src/ui/commands/target.rs`).

**Depends on.** [R70c](#r70c-the-gta-v-world-streamed-and-drawn), [R70g](#r70g-collision-as-a-query)
and [R70h](#r70h-one-vocabulary-of-world-edits); it draws in [R69g](#r69g-one-details-panel)'s
Details and [R69h](#r69h-one-undo-for-the-tab-on-screen)'s Undo.

**Done when.** `the_maps_screen_on_screen` extended to a drag in the world on a model, as one step
of Undo; a pick through a hole in a fence takes what is behind it, and a wall stops it; a rectangle
leaves what a wall hides; a dropped object lands on the collision under it, not on a roof above; a
repeated duplicate leaves equal spacing; an object moved against another snaps to its box, and a
swapped model keeps its place; Delete on a game object writes nothing and says why; a moved object's
far model follows it, and a shared one is left and reported; a tool in a read-only World tab writes
nothing and offers Edit in the project; Escape that closes the palette keeps the selection; a tool's
key answers by its position on an AZERTY layout; each tool and a drag show their hints; an object's
menu draws from the registry (`every_context_menu_draws_from_the_registry`); a double click opens an
object's model; Fix all safe leaves every guid stated once.

### R70j A library of props and what comes in

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** The Library tab, a list by name with a search, over linked projects and the game's props
by a category derived from the install; the ghost and the placement by surface; a zip as a place to
adopt from; files an asset escrow locks recognised, kept whole and drawn as a box; and the map rules
in the import report. Two parts wait for the owner: the thumbnails for the plan's ruling 2, and for
ruling 5 the reader of the XML a Blender export writes and the watched folder
([brief](World_Editor_Plan.md#a-library-of-props-and-what-comes-in-r70j)).

**Where it stands.** Not built. Links, adoption of a folder, the thumbnail cache and the import
report exist ([R48](#r48-projects)). The ruling of 2026-09-22 keeps that XML out
([R37](#r37-metadata-and-xml-at-parity)), and [Workbench_Plan.md](Workbench_Plan.md#held-back) holds
its import back.

**Depends on.** [R70i](#r70i-selecting-and-moving-in-the-world); rulings 2 and 5 for the parts that
wait.

**Done when.** The Library lists a linked project's props and a synthetic install's by derived
category; a ghost lands on the surface under the pointer, on bounds built by hand; a prop dragged
from a linked project builds with only the files it needs; a locked file survives a build byte for
byte and is named; a zip that reaches outside its folder is refused; and an import that breaks a map
rule names the rule in its report. Once ruling 2 is answered yes, the Library draws a thumbnail for
each prop it lists; once ruling 5 is answered yes, an exported XML imports as sources that build,
and a file changed in the watched folder is imported again. A no closes that part in one line.

### R70k Removing what the game placed

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** A deletion of an object the game placed, over R70h: the chain across files, the parents'
counts, and a preview that names each file touched before the step; the copy of the script line that
hides a model, which changes no file; a flag for objects that cannot be told from a twin, which are
left; and the rule for a door left where a building was deleted. The building's collision is
[R70l](#r70l-editing-collision)'s and its occluders
[R70o](#r70o-parked-vehicles-zones-lights-and-occluders)'s: until they land, the preview says both
stay ([brief](World_Editor_Plan.md#removing-what-the-game-placed-r70k)).

**Where it stands.** The chain is built for a pack whose sources hold the maps: `plan` in
`crates/domain/or-maps/src/delete.rs` says what goes with the entities chosen, each far model that
stays with its count lowered, each twin left and every file written, as one list of operations
whose inverses put the files back, with the script line that hides a model and
`openrage-cli maps delete`
(`a_deletion_takes_the_chain_of_far_models_across_the_files_of_a_pack`, on packs built by hand,
with no schema snapshot). Not run: `deleting_an_entity_of_the_install_keeps_every_link`, which
needs an install; its body runs on maps built by hand
(`the_checks_of_the_install_suite_hold_on_a_tree_of_maps_built_by_hand`). The plan renumbers the
links of each child map among the sources and lowers the counts of the parents
(`renumber_for_removed`, `set_num_children`); a map that is not among
the sources keeps its links, and the rule `parent-link-broken` names one left past the end of its
parent map. Left: the rule for a door, the preview on a screen, and the import of the game's maps a
deletion reaches ([or-maps](../reference/Crate_Reference.md#domain-crates)).

**Depends on.** [R70h](#r70h-one-vocabulary-of-world-edits) and
[R70i](#r70i-selecting-and-moving-in-the-world).

**Done when.** Over an install, deleting any object with its chain leaves every link and count
valid, as `executing_a_plan_keeps_every_link` holds for a map; the preview names every file the step
then writes and no other; undo restores every file; twins are flagged and left; the copied script
line names the model deleted; and a door inside the box of a deleted building is reported.

### R70l Editing collision

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** A collision tool: pick polygons, a rectangle that takes the visible ones, delete, set a
material from the game's list, move with shared corners following; the three versions of an area
written together, to the source of R70w. In `or-world`, over R70g, the association of a building
with its polygons. From it, the proposal of polygons to remove with a deleted building, as data the
person edits before it applies, which leaves ground slabs and decals unless an option, off by
default, takes them; and the option that moves a building's collision with it, which takes the
polygons inside its box, leaves the ground and says what it moved
([brief](World_Editor_Plan.md#editing-collision-r70l)).

**Where it stands.** Not built. A bound is built, rebuilt and validated in `or-io`
([R41c](#r41c-bounds-and-physics-construction)) and opens in the app, and a changed bound is held as
a source ([R70w](#r70w-a-changed-bound-and-a-changed-path-region-as-sources)); no screen edits a
polygon.

**Depends on.** [R70f](#r70f-layers-of-what-the-game-hides),
[R70i](#r70i-selecting-and-moving-in-the-world), [R70k](#r70k-removing-what-the-game-placed) and
[R70w](#r70w-a-changed-bound-and-a-changed-path-region-as-sources).

**Done when.** An edited bound passes the validation that refuses what the game would misread, and
reads back with the polygons expected; a polygon moved takes the corners it shares; the three
versions stay alike in what was edited; on bounds built by hand the association takes a building's
walls and leaves the ground, and with the option on takes the slab too; a building moved with its
collision takes the polygons inside its box, leaves the ground and names what it moved; and undo
restores every file.

**Risk.** Which polygons belong to a building is a geometric judgement, so it proposes and never
applies unseen.

### R70m Interiors

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** The data part first: rooms, portals, entity sets and the interior's time cycle modifiers
declared in the typed world family of `or-io`, with the engine's names (the plan's ruling 14). Then
what needs no builder: the entity set switches and their script lines; placement inside an interior;
the sound passages and the room audio shown read only, with a notice on a game interior that was
moved or copied; and the health rules the brief names. The builder waits for ruling 2: the edit of a
room, a portal and an entity set, a new interior, a copy and a second instance
([brief](World_Editor_Plan.md#interiors-r70m)). The audio is [R70x](#r70x-the-audio-of-an-interior).

**Where it stands.** Not built. The four records are carried in the value tree and written back as
they came, and the members the install states for each are measured
([what wave P44 built for Maps](Workbench_Views_Design.md#what-wave-p44-built-for-maps)).
`interior-broken` in `crates/domain/or-maps/src/rules.rs` checks an interior today.

**Depends on.** Nothing for the data part; [R70f](#r70f-layers-of-what-the-game-hides) and
[R70i](#r70i-selecting-and-moving-in-the-world) for the rest; ruling 2 for the builder.

**Done when.** The declared members are the ones the install states, held by the install suite;
every interior of an install reads typed and saves to its fixed point; each health rule fires on an
interior built to break it; a prop placed in a room builds into the type file and into no map; a
switch's copied line names its entity set; and a moved game interior carries the notice. Once ruling
2 is answered yes: a room's box moved takes its portals with it, a fitted portal's corners are in
the order the game expects, and a new interior, a copy and a second instance each read back and pass
the health rules. A no closes the builder in one line.

### R70n Doors

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** The search of doors, first by the door extension an archetype carries, counted over an
install, then by model name, saying how many models had no name; the pairing of double doors; the
copy forms as templates in data, among them the door tuning entry and the line that gives it to a
model; the arc and the clearance over R70g; a door that opens in the view and stops where its arc is
blocked; the zone in which a door opens by itself drawn, with its tuning read; the fitted
replacement; and the door rules: a door model of the project whose archetype lacks what the game's
doors carry, a game door moved or deleted, two doors at one place, and a door whose arc is blocked
([brief](World_Editor_Plan.md#doors-r70n)).

**Where it stands.** Not built. The door extension is typed in
`crates/domain/or-world/src/extension.rs`, and the door tuning opens in the metadata editor.

**Depends on.** [R70g](#r70g-collision-as-a-query) and
[R70i](#r70i-selecting-and-moving-in-the-world).

**Done when.** Over an install the search counts the doors found by their extension, those found by
name and the models with no name, in the slow tier; two leaves that face each other pair and two
that do not stay single; a door behind furniture is reported blocked and the same door in an empty
room is not, and in the view it stops at the furniture; a door's zone draws at the size its tuning
states; each copy form names the door it was copied from; a fitted replacement stands inside the
frame of the door it replaces; and each door rule fires on a map built to break it.

### R70o Parked vehicles zones lights and occluders

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** The data part first: parked vehicles, timecycle zones, far lights and occluders declared
in the typed world family of `or-io`. Then their place, move, size and fields, the far lights' two
maps kept in step, and occlusion meshes drawn and removed as boxes are; the occluder fit as a
proposal, which for an interior puts thin boxes in its outer walls and leaves its portals to the
outside free; from the association of R70l, the proposal of the occluders over a deleted building;
the occluder rules the brief names; the copy of a zone's centre for a script; and a game vehicle
drawn on a selected parking spot
([brief](World_Editor_Plan.md#parked-vehicles-zones-lights-and-occluders-r70o)).

**Where it stands.** Not built. `crates/domain/or-world/src/map_data.rs` reads a box occluder, the
stored box of an occlusion mesh and a parked vehicle's position and one length, counts the timecycle
zones and tests whether far lights are present; a save carries each record as it came.

**Depends on.** Nothing for the data part; [R70f](#r70f-layers-of-what-the-game-hides),
[R70i](#r70i-selecting-and-moving-in-the-world), [R70k](#r70k-removing-what-the-game-placed) and
[R70l](#r70l-editing-collision) for the rest.

**Done when.** The declared members are the ones the install states, held by the install suite;
every record of an install reads typed and saves to its fixed point; a record placed, moved and
sized reads back so; the far lights' two maps agree after an edit; a zone's copied centre parses
back to it; a fitted box lies inside the collision it was fitted to, on a building built by hand,
and an interior's fitted boxes leave its outside portals free; the proposal over a deleted building
names the occluders inside its box and no other; each occluder rule fires on a box built to break
it; and a selected parking spot draws the vehicle its record names.

### R70p Roads scenario points and the navmesh

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** The edit of a road node's fields and lanes, and nodes and links added and removed.
Removing a node renumbers its region, so every region that names a renumbered node is written in the
same step and named before it, and a node another region names is disabled and not deleted. The edit
of a scenario point, in regions that exist, with its chain following. The rules the brief names, for
a node, a link, a scenario point, a building added on walkable ground and a deleted building whose
floors stay in the navmesh. No navmesh is authored
([brief](World_Editor_Plan.md#roads-scenario-points-and-the-navmesh-r70p)).

**Where it stands.** Not built. A `.ynd` opens owned with an outline and a save
([R33a](#r33a-path-nodes-ynd)), and a link names its far node by region and index
(`crates/io/or-io/src/gta5/path_region.rs`). A scenario region is typed
([R33d](#r33d-scenario-points)), a region made from nothing waits on
[R62](#r62-the-rbm-layout-synthesiser), and a changed `.ynd` is held as a source, whose form awaits
the owner's word ([R70w](#r70w-a-changed-bound-and-a-changed-path-region-as-sources)).

**Depends on.** [R70f](#r70f-layers-of-what-the-game-hides),
[R70i](#r70i-selecting-and-moving-in-the-world) and
[R70w](#r70w-a-changed-bound-and-a-changed-path-region-as-sources).

**Done when.** An edited region and every region that names one of its nodes read back with every
link resolving; a deleted node leaves no link to it in any region; a node another region names is
disabled and not deleted; a scenario point moved takes its chain, and its region reads back; and
each rule fires on a file built to break it.

**Risk.** A link names its far node by region and index, so renumbering one region breaks its
neighbours' links unless they are written in the same step.

### R70q A build a server and a story install take

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** In a map's contribution to a build, the manifest of its maps, the line that declares the
resource a map, and the line for the server's configuration shown after a build. For a story pack,
the maps and the type files handed over at a path through an archive's name: the row adds that path
in the Map content, the contents the install's packs state when they register an archive of maps,
and a type file named inside that archive, and the cell unlocks (the plan's ruling 15). A map past
its extents is split at the build, by area, its links carried across the parts
([brief](World_Editor_Plan.md#a-build-a-server-and-a-story-install-take-r70q)).

**Where it stands.** Part one is built; nothing ran against an install or a game. A Map content
hands a build `stream/_manifest.ymf`, stating for each map the type files `or-world`'s derivation
finds (`a_build_writes_a_manifest_that_reads_back_naming_the_type_file_its_map_needs`), and the
line `this_is_a_map`. The manifest is not made from nothing:
`crates/domain/or-world/src/manifest_build.rs` types it by the manifests of the game a content
keeps in its `manifests` folder (`openrage-cli maps keep-manifest`) and copies each entry from the
one they state for a map of that name, since nothing derives the flags of an entry. A map taken from the game keeps the entry the game states
(`a_map_taken_from_the_game_keeps_the_entry_the_game_states_where_the_derived_one_differs`); a map
none states, such as one made from nothing, gets no entry, and a note of the plan names it.
`openrage-cli maps verify` fails a build whose manifest is gone or is another document. Of the
1,801 `.ymf` staged in the metadata corpus, 1,798 read as a manifest a build writes from, and the
185 that state a map are written back stating what they state
(`every_staged_manifest_of_the_game_is_written_back_stating_what_it_states`, 2026-11-25).

⚠ A kept manifest is a compiled file of the game held whole in a project, which the ruling of
2026-10-09 says a project never holds
([rulings in force](Workbench_Plan.md#rulings-in-force)); whether it stays, or the
flags and the layout it is kept for are held another way, is the owner's to rule on. Until then it
is treated as a source taken from the game is: `openrage-cli project check-sharing` names each one
and exits 1, and the managed block of the project's `.gitignore` leaves the folder out
(`a_manifest_of_the_game_a_maps_content_keeps_fails_the_sharing_check_and_is_ignored_by_git`).

For a story pack the content hands its files over inside one nested archive, registered as map
data, each type file asked for by the `.ityp` name of the archive's folder
(`a_story_pack_holds_its_maps_in_the_nested_archive_its_content_document_registers`). The cell
stays locked: the folder of that archive, what a pack states of an archive of type files and the
change set that turns map data on are unmeasured
([maps in a pack](../features/Dlc_Pack_Creation.md#maps-in-a-pack)).

Left: the line for the server's configuration; the split, which
`crates/domain/or-world/src/partition.rs` plans by level of detail and does not apply; the lists a
derivation does not give, written empty; one manifest for two Map contents of a build, refused
today as one path; keeping a manifest from the editor; the three measurements; and the install
suite, written and never run ([the writer in full](../reference/Crate_Reference.md#domain-crates)).

**Depends on.** [R70h](#r70h-one-vocabulary-of-world-edits).

**Done when.** A project's build writes a `_manifest.ymf` that reads back and names what the
project's maps place, seen failing on a build that writes none; the resource's manifest declares it
a map, and the line shown after a build names the resource; a changed game map whose derived lists
differ from the shipped ones keeps the shipped entry; a built pack lists its maps where the
install's own packs list theirs; the cell says yes only with the archive written; every link of a
split map resolves; two builds are the same bytes, and one added object changes only the part that
holds it.

### R70r Other resources of the server

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** The scan of a folder of built resources into what each ships and claims, a server's or the
builds of a person's other projects; a server's folder mounted as the read-only layer of the world's
stack, in `or-title`; the finding that names two resources shipping one file, a rule of the Problems
panel raised by the edit that imports such a file and again before a build writes; the three-way
merge of a map with the game's file as base; a finding for a source whose base is no longer the
install's file when the index is rebuilt; `openrage-cli project check-server` and
`openrage-cli project check-resource`, with the findings the brief names; and a warning for a build
folder the server does not look in
([brief](World_Editor_Plan.md#other-resources-of-the-server-r70r)).

**Where it stands.** The check of one resource folder is built, and nothing else of the row.
`crates/domain/or-project/src/resource.rs` reads any resource folder as a server will, with no
project, and `openrage-cli project check-resource` prints what its rules find, as data with
`--json`, and exits 1 when a finding is an error. The manifest is read and never run
(`crates/domain/or-project/src/resource/manifest.rs`), the lines it cannot read are kept as notes,
and no manifest costs the reader its stack (`brackets_opened_to_any_depth_cost_no_stack`). The
rules are the rows of `RULES`, each firing on a folder built by hand to break it and on no other
(`each_rule_fires_on_the_folder_built_to_break_it_and_on_no_other`), and a sound folder reports
nothing. A streamed file is opened through the reader a build reloads its own files through; a
file an asset escrow locks is counted and never opened
(`a_locked_file_is_counted_named_and_not_opened`); and a path that leaves the resource is reported
and not followed. With `--game` the install is asked about a name the resource lacks, held on a
catalogue built by hand; the install suite has not run.

Open in what is built ([in full](../reference/Crate_Reference.md#domain-crates)): no file is
weighed without `--limits`, since the size at which a server warns was read from no server, and
neither were the types a server streams; nothing judges the game line; and a name is found whatever
its case where the machine that checks ignores case. Left: the scan of a server and
`openrage-cli project check-server`, the finding for two resources that ship one file, the
three-way merge, the server as a layer of the mount, the finding for a base that is no longer the
install's, and the warning for a build folder outside a category. Adoption reads a resource folder
(`crates/domain/or-project/src/adopt.rs`), and nothing here says the game loads a file.

**Depends on.** [R70c](#r70c-the-gta-v-world-streamed-and-drawn),
[R70h](#r70h-one-vocabulary-of-world-edits) and
[R70q](#r70q-a-build-a-server-and-a-story-install-take).

**Done when.** Two resources that change different objects of one map merge with no conflict, and
two that move the same object report it; two that ship one bound are named and not merged; each
finding of the resource check fires on a resource built to break it, and a sound one passes; a
synthetic resources folder that ships one map draws it in place of the install's; importing a map
another resource ships raises the finding before any build; a source whose base is no longer the
rebuilt index's file is a finding that offers the merge; and a build folder under a folder that is
not a category is warned about.

### R70s The model workshop

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** In parts, each on its own check. First, operations on a model source: list its parts, each
hidden or shown alone; remove a connected part in every level; move and copy a part; add a part
copied from another model; add a box or a plane with a material of the model; draw the neighbouring
objects around it. Second, for a game model, the count of placements that use it and Save as a new
prop: the model under a new name with a type file that holds a copy of its archetype, the entity the
workshop was opened from swapped to it. Third, after the plan's ruling 13, a prop made from an
image. Last and marked experimental, an opening cut into a wall
([brief](World_Editor_Plan.md#the-model-workshop-r70s)).

**Where it stands.** Not built. A model is a source of glTF and JSON that recooks
([Source_Format_Design.md](Source_Format_Design.md)), and a drawable made from nothing is held back
in [Workbench_Plan.md](Workbench_Plan.md#held-back).

**Depends on.** [R70i](#r70i-selecting-and-moving-in-the-world); ruling 13 for the prop from an
image.

**Done when.** A model with a part removed recooks, reads back and draws, the part gone from every
level; a part moved, a part copied in from another model and a box added each recook and read back
where they were put; a part hidden draws nothing and changes no source; the count of placements is
what Placed by lists; after Save as a new prop the game's archetype still resolves to the game's
file and one entity names the new one; and a wall with an opening cut recooks and reads back with
the opening. Once ruling 13 is answered yes, a prop made from an image reads back as a drawable with
its texture; a no closes that part in one line.

### R70t Peds cutscenes life and walking the map

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** In parts, each on its own check. First a browser of peds with outfits and clips over the
Animation workspace, which copies a ped's model name and the lines that play the clip shown. Then a
player of a cutscene's tracks in the world, with no sound. Then what a scenario point would spawn at
the hour shown. Last, after the plan's ruling 2, a character that walks, climbs and swims over the
collision query, labelled on screen as a check of the path and not as the game's own movement
([brief](World_Editor_Plan.md#peds-cutscenes-life-and-walking-the-map-r70t)).

**Where it stands.** Not built. The Animation workspace plays a clip on a skinned ped, the Wardrobe
dresses one, and a `.cut` opens as metadata ([R35](#r35-the-remaining-formats)).

**Depends on.** [R70c](#r70c-the-gta-v-world-streamed-and-drawn) and
[R70g](#r70g-collision-as-a-query); ruling 2 for the walk.

**Done when.** The browser lists the peds of a synthetic install, and the copied lines name the clip
shown; a cutscene's cuts fall at the times its file states; and a point lists what its type and
model set allow at the hour shown, on a region built by hand. Once ruling 2 is answered yes, the
character stops at a wall and steps a kerb on bounds built by hand; a no closes the walk in one
line.

### R70u Working together

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** Three parts, each usable alone. First, the merge of R70r wrapped as the driver Git calls
for a map source, so a shared repository merges by object; it merges a changed game map only where
the repository holds the sources taken from the game (the plan's ruling 17). Second, a session on a
direct connection: the host orders the operations of R70h and sends them, with each person's
selection and camera; what a person holds selected is locked to the others; the host admits each
person, a guest whose stack differs from the host's is refused with what differs, only the host
builds, and a person can go to another's view; a person's Undo takes back their own last step,
through the host; a guest who lost the session joins again; the host sets how many may join. Third,
with ruling 6, a relay that forwards bytes it cannot read
([brief](World_Editor_Plan.md#working-together-r70u)).

**Where it stands.** Not built. A source taken from the game is left out of a repository by default
([commit policy](Source_Format_Design.md#commit-policy)). Nothing reaches the network unless a
person asks: the one client in the tree is behind `openrage-cli script download-natives`.

**Depends on.** [R70h](#r70h-one-vocabulary-of-world-edits) and
[R70r](#r70r-other-resources-of-the-server); rulings 6 and 17.

**Done when.** The driver merges two branches that change different objects of one map; two edits
that cross are ordered the same for every person; a person who joins late has the same sources byte
for byte; an operation on a thing another person holds is refused and names who holds it; a guest
with a different stack is refused with what differs; a guest who asks for a build is refused; a
person's Undo leaves another's step in place; and the relay forwards a session's bytes between two
processes and decodes none.

### R70v The keys settings and guide of a mapper

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** Presets of keys, with one that matches CodeWalker's, and the place of a binding in
`src/ui/shortcuts.rs`; the settings a mapper changes on one page (mouse sensitivity, field of view,
an inverted vertical axis, the movement letters of the keyboard's layout, the ground grid, the snap
steps and the layer radii), with Restore defaults; a panel's title carrying a sentence on its game
term as its hover; the sentences of the map rules wrapped into the catalogs; a tour of the world; a
chapter of the guide; and a Map template that opens on the world
([brief](World_Editor_Plan.md#the-keys-settings-and-guide-of-a-mapper-r70v)).

**Where it stands.** Not built. A key is changed and kept in the preferences, where a conflict is
one key for two commands of the whole table and the matcher reads a press by its character first.
Keymap presets were left open on 2026-10-03
([Project_Context.md](Project_Context.md#the-ux-reviews-plan-is-the-apps-next-work-2026-10-03)), a
library crate's sentences are English, and `Template::Maps` opens on the Map screen.

**Depends on.** [R70i](#r70i-selecting-and-moving-in-the-world).

**Done when.** No preset binds one key to two commands of one place; every step of the tour points
at a control that is drawn; each setting of the page changes what it names in a frame drawn with no
window, and Restore defaults puts each back; a finding of a map rule draws in French; a panel's
title shows its sentence on hover; the guide's chapter names no control that is not drawn; and a
project made from the Map template opens on the world.

### R70w A changed bound and a changed path region as sources

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** The form in which a project holds a bound and a path region it changed: the source, the
base it was taken from, its cook and its compare against that base, in `or-source` beside the kinds
of `crates/domain/or-source/src/game_import.rs`. A project holds no compiled game file, so
[R70l](#r70l-editing-collision) and [R70p](#r70p-roads-scenario-points-and-the-navmesh) write to
this form. It draws nothing
([brief](World_Editor_Plan.md#a-changed-bound-and-a-changed-path-region-as-sources-r70w)).

**Where it stands.** The source, the base and the cook are built in `or-source`, with no content
kind and no screen; the compare against the base is not. `crates/domain/or-source/src/bounds.rs`
makes `<name>.bounds.src` of a `.ybn`, its triangles as a glb and what a glb cannot say as
`bounds.json`, and `crates/domain/or-source/src/paths.rs` makes `<name>.paths.src` of a `.ynd`, one
`paths.json` of its nodes, links and junctions. Each keeps the game's file as its base, is a row of
`or_source::game_import::KINDS`, cooks to the canonical save of its file while untouched and is left
out of a build until it changes
(`a_bound_taken_from_the_game_is_left_out_of_a_build_until_a_polygon_changes`,
`a_path_region_taken_from_the_game_is_left_out_of_a_build_until_its_document_is_edited`).
[Source_Format_Design.md](Source_Format_Design.md#bounds-and-path-regions) states both forms, what
each refuses and what a rebuilt bound keeps of its file.

On the staged corpus, with no install: 21 of 21 `.ybn` and 259 of 259 `.ynd` became a source that
cooks to its canonical save (`every_corpus_ybn_becomes_a_source_that_cooks_to_its_canonical_save`,
`every_corpus_ynd_becomes_a_source_that_cooks_to_its_canonical_save`, 2026-11-25), and each of the
77991 nodes is addressed by its place in the one region of its file
(`every_node_of_a_corpus_ynd_is_addressed_by_its_place_in_one_region`, 2026-11-25).

Left: the compare of a source against its base, as the polygon removed or the field changed; the
round trip over an install, which no suite walks; the form of a path region, which awaits the
owner's word; a child added to or removed from a composite, and the changed meshes the source
refuses by name; and nothing was run in the game.

**Done when.** Every `.ybn` and `.ynd` of an install becomes a source and cooks back to the
canonical save of its file, in the slow tier and named in the skip ledger without an install; an
untouched source is left out of a build; a source with one polygon removed compares to its base as
that polygon; and a path region with one node's field changed compares to its base as that field.

### R70x The audio of an interior

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** First, in `or-io`, the authoring of the game data items of an interior and its rooms in a
`.rel`, with a save that reaches a fixed point. Then the occlusion file written from the rooms and
portals of R70m, through a donor's type table. Then the audio lines in the resource's manifest. A
typed root for the occlusion metadata waits for the plan's ruling 16
([brief](World_Editor_Plan.md#the-audio-of-an-interior-r70x)).

**Where it stands.** Not built. A `.rel` item reads typed and no typed field is edited
([R34b](#r34b-the-audio-metadata-schemas)), and the members of the occlusion metadata are spelled as
hashes ([metadata](Source_Format_Design.md#metadata)). How the game finds an interior's occlusion
file is not measured.

**Depends on.** [R70m](#r70m-interiors).

**Done when.** A `.rel` with an item added reads back typed and saves to its fixed point; the
occlusion file of an install's interior, written again from its rooms and portals, reads back
through the reader that opens the game's; and a resource with an interior registers both files.

### R70y What a project changed and putting it back

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** The comparison of a map source with its base as data: entities added, moved, deleted and
changed in a field, and those that cannot be told from a twin, listed as undecided and never as
deleted; `openrage-cli maps changes`, which prints it; the panel that lists it, with counts, a
filter and a click that selects the object or goes to where a deleted one stood, a deleted object
drawn in the world as an outline while its row is selected; the operation that puts one object or
one deleted building back as the base has it, with the far models a deletion took, as one step; and
Discard changes to this map, after a question that counts them
([brief](World_Editor_Plan.md#what-a-project-changed-and-putting-it-back-r70y)).

**Where it stands.** The comparison and the verb are built, on synthetic maps, with no screen.
`crates/domain/or-maps/src/changes.rs` compares two typed roots of one map by object: the entities
added, the entities deleted and each one both hold with what it states differently, named (moved,
turned, scaled, another archetype, flags, parent, guid, the other typed members by the engine's
names). `types_changes` says the same of the archetypes of a type file, with the entities of an
interior, and what a file states of itself is compared member by member. An entity is found by its
guid, else by its archetype at the same pose, else by the nearest; a twin that moved is listed as
ambiguous, the data's word for the plan's undecided, and never as moved
(`two_twins_with_one_archetype_and_one_guid_are_ambiguous_when_one_moved_and_never_a_move`), and
the parent links are compared through the matching. `source_changes` cooks a source and the base
it keeps (`compile_base` in `crates/domain/or-source/src/metadata.rs`), and
`openrage-cli maps changes` prints it, as data with `--json`
(`crates/domain/or-maps/tests/what_a_map_changed_of_its_base.rs`;
[the matching in full](../reference/Crate_Reference.md#domain-crates)).

⚠ Two readings are the owner's to rule on. The nearest match believes a place over a guid
(`an_entity_taken_out_and_another_of_its_archetype_put_in_read_as_one_moved_with_another_guid`).
And a new entity takes one more than the largest guid of its map (`free_guid` in
`crates/domain/or-world/src/create.rs`), so one added after the entity with the largest guid was
deleted takes that guid and compares as one entity changed, held as it stands through the
operations
(`an_entity_added_after_the_one_with_the_largest_guid_went_takes_its_guid_and_compares_as_that_entity_changed`). What no typed
member names (a car generator, an occluder, a room, a portal, an extension) is said to differ and
is not listed. Left: the panel, the outline, the operation that puts an object back, Discard, and
a map of an install compared.

**Depends on.** [R70h](#r70h-one-vocabulary-of-world-edits) and
[R70i](#r70i-selecting-and-moving-in-the-world).

**Done when.** A source whose map moves one entity, deletes one and adds one compares to exactly
those three, and two twins swapped compare as undecided; `openrage-cli maps changes` prints the
three; the panel lists the three, and a click on the deleted one goes to where it stood and draws
its outline; putting each back leaves the source equal to its base; a deleted building put back
takes its far models with it in one step of Undo; and Discard changes to this map leaves the source
equal to its base.

### R70z A browser of models and vehicles

[↑ checklist](#phase-11--the-gta-v-world-editor)

**What.** The browser of every object type and every vehicle of the game: filters by class, by pack
and by size, favourites, the models opened last, back and forward; where the model shown is placed,
nearest first; a command that saves the view of a model as a PNG, with a transparent or a keying
background, a size and preset views; and for a vehicle, the install's vehicle colours and variations
read in `or-io`, the vehicle drawn in the combinations the game plans, the lines that paint it in
the colours shown, copied, and its sheet. The grid of thumbnails waits for the plan's ruling 2
([brief](World_Editor_Plan.md#a-browser-of-models-and-vehicles-r70z)).

**Where it stands.** Not built. The thumbnail cache exists (`src/render/thumbnails.rs`), Placed by
answers which maps place a model, and nothing in the tree interprets the install's vehicle colours
and variations.

**Depends on.** [R70e](#r70e-find-anything-and-go-there) and
[R70j](#r70j-a-library-of-props-and-what-comes-in); ruling 2 for the thumbnails.

**Done when.** A filter by class lists the vehicles a synthetic install's data puts in that class
and no other; a favourite and the models opened last are there after a restart; the placements of a
model are listed nearest first; a saved picture has the size asked and is transparent where nothing
is drawn; a synthetic install's vehicle draws in each combination its data lists, the copied lines
name the colours shown, and its sheet states what that data states. Once ruling 2 is answered yes,
the grid draws a thumbnail for each model listed; a no closes that part in one line.

---

## Appendix A — union coverage index

Every row of `internal/tool-feature-inventory.md` §10, the union list, against the item that owns it;
`NG` means [Non-goals](#non-goals). 217 numbered rows, each once.

- **A row stays with its owner whatever the outcome**, so `U-041` stays R36's and `U-203` R20's
  although both closed as measured refusals. The five rows that moved on 2026-09-21 moved because R25
  split: `U-009` and `U-028`–`U-031` went to R25b.
- **A decomposition does not move rows.** Forty rows sit against numbers that are programme
  headers — R40, R41, R44, R49, R50, R51, R59 and R60 — and each header's section says where its rows
  are answered; dealing them out needs the union list, which is not in this tree.
- **R44's six** (U-165, U-166, U-168, U-175, U-176, U-178) describe the process protocol that
  [R43](#r43-the-external-surface-and-open-with) owns. They stay against R44, and R43 says it
  answers them.

```
U-001 R19   U-002 R19   U-003 R19   U-004 R19   U-005 R20   U-006 R20   U-007 R20
U-008 R19   U-009 R25b  U-010 R53   U-011 R53   U-012 R22   U-013 R19   U-014 R49
U-015 R23   U-016 R23   U-017 R23   U-018 R10   U-019 R40   U-020 R55   U-021 R26
U-022 R23   U-023 R23   U-024 R24   U-025 R24   U-026 R24   U-027 R26   U-028 R25b
U-029 R25b  U-030 R25b  U-031 R25b  U-032 R20   U-033 R21   U-034 R21   U-035 R20
U-036 R20   U-037 R28   U-038 R45   U-039 R31   U-040 R31   U-041 R36   U-042 R47
U-043 R47   U-044 R31   U-045 R26   U-046 R26   U-047 R47   U-048 R27   U-049 R47
U-050 R47   U-051 R48   U-052 R48   U-053 R47   U-054 R48   U-055 R48   U-056 R26
U-057 R47   U-058 R32   U-059 R32   U-060 R37   U-061 R23   U-062 R37   U-063 R37
U-064 R37   U-065 R37   U-066 R37   U-067 R37   U-068 R37   U-069 R37   U-070 R29
U-071 R37   U-072 R32   U-073 R32   U-074 R32   U-075 R32   U-076 R32   U-077 R32
U-078 R47   U-079 R47   U-080 R32   U-081 R32   U-082 R32   U-083 R32   U-084 R32
U-085 R47   U-086 R39   U-087 R39   U-088 R39   U-089 R39   U-090 R39   U-091 R39
U-092 R39   U-093 R40   U-094 R40   U-095 R40   U-096 R42   U-097 R42   U-098 R40
U-099 R38   U-100 R26   U-101 R47   U-102 R38   U-103 R38   U-104 R39   U-105 R41
U-106 R30   U-107 R47   U-108 R33   U-109 R33   U-110 R47   U-111 R33   U-112 R33
U-113 R34   U-114 R34   U-115 R34   U-116 R47   U-117 R41   U-118 R34   U-119 R47
U-120 R47   U-121 R47   U-122 R47   U-123 R47   U-124 R47   U-125 R47   U-126 R47
U-127 R42   U-128 R47   U-129 R35   U-130 R28   U-131 R47   U-132 R47   U-133 R47
U-134 R47   U-135 R41   U-136 R47   U-137 R63   U-138 R35   U-139 R35   U-140 R35
U-141 R40   U-142 R46   U-143 R41   U-144 R46   U-145 R47   U-146 R47   U-147 R47
U-148 NG    U-149 NG    U-150 R49   U-151 R49   U-152 R49   U-153 R49   U-154 R50
U-155 R51   U-156 R50   U-157 R50   U-158 R50   U-159 R50   U-160 R52   U-161 R52
U-162 R52   U-163 R52   U-164 R48   U-165 R44   U-166 R44   U-167 R26   U-168 R44
U-169 R30   U-170 R43   U-171 R43   U-172 R26   U-173 R29   U-174 R59   U-175 R44
U-176 R44   U-177 R45   U-178 R44   U-179 R27   U-180 R29   U-181 R29   U-182 R29
U-183 R39   U-184 R39   U-185 R23   U-186 R55   U-187 R28   U-188 R29   U-189 R29
U-190 R41   U-191 R45   U-192 R29   U-193 R40   U-194 R29   U-195 R45   U-196 R41
U-197 R59   U-198 R59   U-199 R60   U-200 R25   U-201 R14   U-202 R14   U-203 R20
U-204 R20   U-205 R29   U-206 R29   U-207 R20   U-208 R39   U-209 R39   U-210 R40
U-211 R41   U-212 R41   U-213 R41   U-214 R41   U-215 R23   U-216 R41   U-217 R47
```

The eight unnumbered rows: mesh, skinning and LOD authoring → [R47](#r47-the-plugin-slate-after-the-first);
mod uninstall and DLC pack creation → [R51](#r51-package-authoring-uninstall-and-dlc-creation);
write-back targets → [R28](#r28-authoring-and-write-back-targets); dockable panels and performance
tuning → [R59](#r59-presentation-input-and-workspaces); onboarding permissions →
[R60](#r60-onboarding-permissions-and-crash-reporting); cross-platform → [R58](#r58-cross-platform).

Capabilities the inventory marks parity in §1 and §9 with no §10 row: audio playback and Bink video
→ [R34](#r34-audio-and-media); glTF export → [R38](#r38-mesh-interchange).

---

## Appendix B — judgement calls to confirm

Places where scoping this roadmap needed a decision the inputs did not make and no owner decision
has made since. A call the owner has settled is recorded with the decision, in
[Project_Context.md](Project_Context.md) or as D1–D18 in
[OR_IO_Specification.md](OR_IO_Specification.md#owner-decisions), and leaves this list. The owner's
answers of 2026-10-01 settled every call then open but the first three; the rest came with the
merges from #415 to #451.

1. **[R66g](#r66g-rewrite-the-history): the freeze and the branches.** When the rewrite comes,
   which the owner left for later: the open draft #131 and the agent branches on `origin`. D12 is
   taken: old revisions keep the engine's names.
2. **[R66h](#r66h-publish-the-crate-and-the-editor-together): the lawyer questions.** When the
   questions D14 parked are taken up; the owner keeps them parked.
3. **[R67](#r67-menyoo-spooner-xml-import): a spooner corpus.** Where a corpus of spooner files
   from more than one Menyoo version may come from; the row waits for one, unscheduled.
5. **[R58](#r58-cross-platform): may CI run a recipe?** #417 added a `just fmt` and `just lint`
   step to `ci.yml`'s Linux job, as the proof that the recipes start on Linux, reading AGENTS.md's
   old "CI calls cargo directly and runs no recipe" as the reason CI needs no nextest rather than
   a ban, and rewrote that bullet to match. If it was a ban, the step and the bullet are reverted
   and the Linux proof needs a Linux machine.
6. **[R58b](#r58b-ci-on-macos-and-linux): how often should the macOS job run?** `ci.yml`'s `macos`
   job runs weekly and on request, which the agent chose by analogy with the Windows job under the
   cost decision of 2026-09-23. That decision is about Windows. A hosted macOS minute bills at ten
   times a Linux one on GitHub's published rates, so every pull request would cost accordingly.
   If another cadence is wanted, the job's `if:` line changes; if the owner wants it on every pull
   request, the cost decision needs restating for macOS.
7. **[R69c](#r69c-one-build-control-and-a-save-that-offers-what-can-happen): may a button labelled
   Save to mods write the overlay from Read only?** The review of 2026-11-21 proposes that the label
   is the consent, with a write to the base game still behind its question and its backup. Until the
   owner says so a save asks, and replacing a texture and saving it for the game stays at four
   actions where the review's target is three.
8. **[R70](#r70-the-gta-v-world-editor): the rulings of the world editor plan.** The plan of
   2026-11-25 asks the owner for rulings and gives each a default
   ([the rulings asked](World_Editor_Plan.md#rulings-asked-of-the-owner), which owns the list). The
   owner said that day to build the plan
   ([recorded](Project_Context.md#the-world-editor-plan-is-built-in-parallel-lanes-2026-11-25)), so
   a row takes a default until a sentence reverses it, except where a default reopens a dated
   decision or builds what [Workbench_Plan.md](Workbench_Plan.md#held-back) holds back. Those parts
   wait for the owner: the interior builder, the walk mode and the thumbnail browser in the app and
   not in a plugin (ruling 2); the XML of a Blender export read on import (ruling 5); a story pack
   installed into the mods folder, whose default is no (ruling 7); a drawable made from nothing
   (ruling 13); and a typed root declared by hand outside the families the record names (ruling 16).
   Among the defaults in force, these fill what a dated entry or a decision sheet left open: key
   presets and the camera's pointer (ruling 8), the interior records declared in `or-io` and a story
   pack's nested archive of maps (rulings 14 and 15), a live session that reaches the network when a
   person starts one (ruling 6), and a changed game map left out of a repository by default (ruling
   17).

## Deferred and known gaps

Open work no open item schedules. An item that does own a gap says so in its own section: BC6H and
tiled Gen9 textures in [R39](#r39-textures), Enhanced material coverage in
[R40e](#r40e-the-enhanced-shader-container), the second container in
[R56](#r56-rdr2-the-second-container), discovering a launcher or Epic install and packaging off
Windows in [R58](#r58-cross-platform), `.ysc` in [R61](#r61-script-containers), the archived clothing work in
[R46](#r46-first-plugin-a-clothing-tool-on-openrage), and what a canonical save still loses
(`census::STILL_LOST`) and the bit ranges no resource declaration names yet (OD-BITS) in the list
[R66d](#r66d-declare-gta-v-format-by-format) closed with.

- **The drawable-upload baseline is unrecorded.** `every_corpus_drawable_uploads_as_recorded` in
  `src/render/meshes.rs` holds each corpus drawable's uploaded geometry and texture names, Legacy and Enhanced, to a
  record, and `openrage-corpus/baselines` holds neither of its two records, so it records a skip.
  `just baselines-record` cannot write them: its suites are `tests/` binaries of library crates and
  this test is in the app binary, which the baseline step of `.github/workflows/local-gate.yml`
  does not run either.
- **A check deleted with `or-asset` in #332 and not replaced.** can_remove refused deleting a
  texture another material still needs ([R39](#r39-textures)); no owned suite holds that yet.
- **`.ycd` version 43.** Four clip dictionaries in a Legacy install whose structures are not the v46
  layout (`YCD_FILE_VERSION_GEN8_ALT`). `FORMATS` declares `.ycd` at 46 only, so they are refused
  by name rather than read as v46 garbage (`a_v43_ycd_is_refused_rather_than_read_as_v46`); reading
  them needs the v43 layout worked out.
- **A `.nametable` entry the reader skips is lost on save.** The app opens a name table as its names
  one per line ([R23](#r23-names-and-hashes)), and an entry the reader cannot take as a name, such
  as one retail entry in `dlchei4_game.dat151.nametable`, is logged and not kept, so a save drops
  it and shifts every later index (#436).
- **`.ytd` version 2.** One file in an Enhanced install, `markers.ytd`, at a version neither target
  describes; refused.
- **Compact animation blocks.** A `crBlockStream` whose payload is `crAnimCache`-compressed is carried
  verbatim and refused on decode (`compact_block_is_refused`); the corpus holds none.
- **Expression clips.** `CrClip` declares no variant for an `AnimationExpression` clip, so one opens
  as `CrClipAny::Base`, its header alone: the `crExpressions` subtree it owns is neither read nor
  written back. The corpus holds none.
- **RBM writer shapes.** A populated `atBinaryMap` and a wide string are refused as
  `MetaError::UnsupportedShape`; the RBM corpus holds neither.
- **Gen9 particle simulation.** `or-simulation` takes `PtxParticleRule<Gen8>` and
  `PtxEffectRule<Gen8>` only, so a Gen9 `.ypt` opens, edits and saves but is not simulated.
- **Renderer.** GPU skinning (the viewport skins on the CPU; see
  [Animation_Preview.md](../features/Animation_Preview.md)), a sorted alpha-blend pass and offscreen image
  readback are not scheduled.
- **Lines the viewport draws and a click does not pick.** A `.yld`'s cloth edges and collision
  composite (Wireframe), a `.ynv`'s polygon outlines, a particle effect's gizmo lines and a plugin's
  overlay lines, and a `.ybn` in the shaded modes, whose meshes are not keyed by node; and the
  Outline lists a fragment's Medium and Low physics bounds, which the viewport does not draw
  ([R42h](#r42h-what-a-pick-still-misses-after-r42f)).
