//! A GTA VI drawable's texture key opens the pack that holds it from an index of its folder, which
//! is kept and reads again only the packs that changed; a
//! pack saves over its own file; an sRGB texture keeps its flag; the pack inspector states what
//! the `.ytd` one does; a scanned dictionary lists a texture that does not read, shows each
//! slice, serves no GTA V model and takes no texture of another size; and a drawable's tile
//! replaces and exports the texture its key resolves to in the document that holds it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use or_io::codec::texture::{
    decode_level, ChannelView, GrcTextureFormat, RgbaImage, TextureSource,
};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::headerless::{Dictionary, DictionaryTexture, Entry, HeaderlessError};

use super::a_gta_vi_archive_s_scan_is_kept::{modified, rewritten};
use super::enhanced_controls_say_so::{control, frame};
use super::jobs::JobKind;
use super::opening_a_ttpf::{
    cube, drawable_keyed, drawn, named_by_the_rules, open, pack, pack_of, rgba, save_as, written,
    BODY, SIDE,
};
use super::textures::{self, TextureListing, TextureSource as Source};
use super::toasts::ToastAction;
use super::{AppState, DocPayload, DocumentId};
use crate::platform::file_dialogs;
use crate::ui::commands::{self, Action, Target};
use crate::ui::dock::Workspace;

/// A pack of [`pack`]'s two textures whose first is keyed `key`.
fn keyed(key: u64, rgb565: u16) -> Vec<u8> {
    let mut bytes = pack(rgb565);
    let keys = 0x100 + 16 + 16 * 2;
    bytes[keys..keys + 8].copy_from_slice(&key.to_le_bytes());
    bytes
}

fn blue() -> RgbaImage {
    RgbaImage {
        width: SIDE.0,
        height: SIDE.1,
        data: [0, 0, 0xFF, 0xFF].repeat((SIDE.0 * SIDE.1) as usize),
        is_srgb: false,
    }
}

#[test]
fn a_drawable_s_key_opens_the_pack_its_folder_s_index_finds_and_a_key_no_pack_holds_opens_none() {
    let key = 0x0BAD_F00D_1234_5678u64;
    let dir = tempfile::tempdir().expect("a scratch directory");
    std::fs::create_dir_all(dir.path().join("packs").join("deep")).expect("folders");
    std::fs::create_dir(dir.path().join("models")).expect("a folder");
    let other = dir.path().join("packs").join("other.ttpf");
    std::fs::write(&other, keyed(key ^ 0xFF, 0x07E0)).expect("written");
    let holder = dir.path().join("packs").join("deep").join("holder.ttpf");
    std::fs::write(&holder, keyed(key, 0xF800)).expect("written");
    let mut state = AppState::new();
    state.open_folder(dir.path().to_path_buf());

    let (doc, _) = drawn(&mut state, &dir.path().join("models"), drawable_keyed(key));
    let name = format!("hash_{key:016x}");
    let ask = |state: &mut AppState, name: &str| {
        let wanted = [(None, name.to_string())];
        state.texture_report = super::texture_resolution::report(state, None, wanted, 1);
        opened_once_read(state)
    };
    assert!(super::texture_resolution::resolve(&state, None, None, &name).is_none());
    assert!(!ask(&mut state, &name), "the first ask starts the index");
    let indexing = state.running_jobs();
    assert!(
        (indexing.iter()).any(|j| j.doing.contains("Indexing")),
        "{indexing:?}"
    );
    named_by_the_rules(&mut state);
    let wanted = [(None, name.clone())];
    state.texture_report = super::texture_resolution::report(&state, None, wanted, 1);
    assert!(
        !state.try_open_texture_pack() && state.texture_packs_opening(),
        "the pack is read on a worker, never in the frame that asks for it"
    );
    assert!(ask(&mut state, &name), "the pack that holds the key opens");
    assert_eq!(
        state.active_document,
        Some(doc),
        "the drawable stays active"
    );
    let opened: Vec<_> = (state.documents.iter())
        .filter(|d| matches!(d.payload, DocPayload::Ttpf(_)))
        .filter_map(|d| d.path.clone())
        .collect();
    assert_eq!(opened, vec![holder], "only the pack that holds the key");
    let found = super::texture_resolution::resolve(&state, None, None, &name)
        .expect("the drawable's key resolves to the pack opened for it");
    assert_eq!(found.origin.label, "holder.ttpf");
    let decoded = decode_level(&found.texture, 0).expect("decodes");
    assert!(decoded.data.chunks(4).all(|pixel| pixel == rgba(0xF800)));
    let listed = textures::textures_in(&state.document(doc).expect("open").payload, None);
    let TextureListing::Textures { textures, .. } = textures::sized_for_gta6(&state, doc, listed)
    else {
        panic!("the drawable lists its key");
    };
    let shape = |t: &textures::DocumentTexture| (t.width, t.height, t.format);
    assert_eq!(
        textures.iter().map(shape).collect::<Vec<_>>(),
        [(SIDE.0, SIDE.1, GrcTextureFormat::DXT1)],
        "the size and format the open pack's directory states"
    );

    let unheld = format!("hash_{:016x}", key ^ 1);
    assert!(
        !ask(&mut state, &unheld),
        "a key no pack holds opens nothing"
    );

    // A pack the app writes under the folder is indexed at the next ask.
    let added = dir.path().join("packs").join("added.ttpf");
    std::fs::write(&added, keyed(key ^ 1, 0x001F)).expect("written");
    state.file_replaced(&added);
    assert!(!ask(&mut state, &unheld), "the folder is indexed again");
    named_by_the_rules(&mut state);
    assert!(ask(&mut state, &unheld), "the pack added opens");
}

#[test]
fn a_key_no_pack_of_the_folder_holds_is_said_so_shown_by_no_tile_and_asked_for_by_no_mesh() {
    use super::gta6_resource::{material_rows, texture_name};
    let (held, unheld) = (0x0BAD_F00D_1234_5678u64, 0x0BAD_F00D_1234_0000u64);
    // A material of no `DiffuseTex` record: a linear texture a pack holds, then a key none holds.
    let drawable = || {
        let mut drawable = drawable_keyed(held);
        drawable.shaders[0].parameters.clear();
        drawable.shaders[0].texture_keys = vec![Some(held), Some(unheld)];
        drawable
    };
    // The row of slot 1 after the pack that holds slot 0 has opened, the mesh's texture and the tiles.
    let opened = |second_key: u64| {
        let mut bytes = keyed(held, 0xF800);
        let keys = 0x100 + 16 + 16 * 2 + 8;
        bytes[keys..keys + 8].copy_from_slice(&second_key.to_le_bytes());
        let dir = tempfile::tempdir().expect("a scratch directory");
        written(dir.path(), &bytes);
        let mut state = AppState::new();
        state.open_folder(dir.path().to_path_buf());
        let (doc, meshes) = drawn(&mut state, dir.path(), drawable());
        let name = texture_name(held);
        assert_eq!(meshes[0].texture_name.as_ref(), Some(&name), "asked for");
        let unheld_row = |state: &AppState| material_rows(state, doc, &drawable())[0].1[1].clone();
        let (_, before, warned) = unheld_row(&state);
        assert!(before.contains("unresolved") && warned, "{before}");
        assert_eq!(state.texture_packs_indexed(), 0);
        for opens in [false, true] {
            let wanted = [(None, name.clone())];
            state.texture_report = super::texture_resolution::report(&state, None, wanted, 1);
            assert_eq!(opened_once_read(&mut state), opens);
            named_by_the_rules(&mut state);
        }
        assert_eq!(state.texture_packs_indexed(), 2, "one pack, indexed");
        let request = super::capabilities::GeometryRequest {
            state: &state,
            doc: state.document(doc).expect("open"),
            selection: None,
            entry: None,
        };
        let drawn_with = crate::render::gta6_drawable::meshes(&request).remove(0);
        let payload = &state.document(doc).expect("open").payload;
        let tiles = state.without_unheld_textures(doc, textures::textures_in(payload, None));
        let TextureListing::Textures { textures, .. } = tiles else {
            panic!("a listing");
        };
        let tiles: Vec<String> = textures.into_iter().map(|t| t.name).collect();
        (unheld_row(&state), drawn_with.texture_name, tiles)
    };

    let ((_, row, warned), texture, tiles) = opened(held ^ 0xFF);
    assert!(
        row.contains("is in no texture pack under") && !warned,
        "{row}"
    );
    assert_eq!(
        texture, None,
        "a linear map and a key nothing holds: untextured"
    );
    assert_eq!(tiles, [texture_name(held)]);

    // A pack with a key missing from disk may hold any key, so nothing is said of one it lacks.
    let ((_, row, warned), texture, tiles) = opened(0);
    assert!(row.contains("unresolved") && warned, "{row}");
    assert_eq!(texture, Some(texture_name(unheld)), "still asked for");
    assert_eq!(tiles, [texture_name(held), texture_name(unheld)]);
}

/// What the index's job says once it read each of two packs, and once it read `read` of them.
const BOTH_READ: &str = "2 GTA VI texture packs indexed by their keys";
fn read_of_two(read: usize) -> String {
    format!("{BOTH_READ}, {read} read and the rest from the kept index")
}

/// A folder of two packs: `a.ttpf`, whose first texture is keyed `keys[0]`, and `deep/b.ttpf`, keyed `keys[1]`.
fn two_packs(keys: [u64; 2]) -> (tempfile::TempDir, [PathBuf; 2]) {
    let dir = tempfile::tempdir().expect("a scratch directory");
    std::fs::create_dir(dir.path().join("deep")).expect("a folder");
    let packs = [
        dir.path().join("a.ttpf"),
        dir.path().join("deep").join("b.ttpf"),
    ];
    std::fs::write(&packs[0], keyed(keys[0], 0xF800)).expect("written");
    std::fs::write(&packs[1], keyed(keys[1], 0x07E0)).expect("written");
    (dir, packs)
}

/// A fresh app whose configuration folder is `config`, with the packs under `dir` indexed for a
/// drawable in it, and what the index's job said as it ended.
fn indexed(config: &Path, dir: &Path) -> (AppState, String) {
    super::prefs::keep_prefs_in(config);
    let mut state = AppState::new();
    state.open_folder(dir.to_path_buf());
    drawn(&mut state, dir, drawable_keyed(1));
    assert!(!holds(&mut state, 1), "the first ask starts the index");
    named_by_the_rules(&mut state);
    let ended = state.jobs.ended().iter();
    let said = (ended.filter(|job| job.kind == JobKind::TextureNames))
        .map(|job| job.said.clone())
        .next();
    (state, said.expect("the index's job ended"))
}

/// Whether the index finds `key`, which the active drawable asks for, in a pack, and that pack opens.
fn holds(state: &mut AppState, key: u64) -> bool {
    let wanted = [(None, format!("hash_{key:016x}"))];
    state.texture_report = super::texture_resolution::report(state, None, wanted, 1);
    opened_once_read(state)
}

/// [`AppState::try_open_texture_pack`], asked again until the packs it has a worker open are in.
fn opened_once_read(state: &mut AppState) -> bool {
    let until = Instant::now() + Duration::from_secs(10);
    let mut opened = state.try_open_texture_pack();
    while !opened && state.texture_packs_opening() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(2));
        opened = state.try_open_texture_pack();
    }
    opened
}

/// The files the texture pack indexes of `config` are kept in.
fn kept_indexes(config: &Path) -> Vec<PathBuf> {
    let Ok(folder) = std::fs::read_dir(config.join("gta6_texture_indexes")) else {
        return Vec::new();
    };
    folder.map(|entry| entry.expect("listed").path()).collect()
}

#[test]
fn a_folder_s_index_is_kept_and_the_next_one_finds_the_same_keys_and_opens_no_pack() {
    let (a, b, c) = (0xA1u64, 0xB2, 0xC3);
    let config = tempfile::tempdir().expect("a configuration folder");
    let (dir, [_, pack_b]) = two_packs([a, b]);
    let (mut state, said) = indexed(config.path(), dir.path());
    assert_eq!(said, BOTH_READ);
    assert!(holds(&mut state, a) && holds(&mut state, b));
    let [file] = kept_indexes(config.path()).try_into().expect("one file");
    let kept = std::fs::read(&file).expect("read");
    rewritten(&file, &kept, modified(&file) - Duration::from_secs(3600));
    let kept_at = modified(&file);

    // Another key under the pack's size and time: only the kept index states the key it held.
    rewritten(&pack_b, &keyed(c, 0x07E0), modified(&pack_b));
    let (mut state, said) = indexed(config.path(), dir.path());
    assert_eq!(said, read_of_two(0));
    assert!(!holds(&mut state, c), "the pack was read");
    assert!(holds(&mut state, a) && holds(&mut state, b));
    assert_eq!(kept_indexes(config.path()).len(), 1);
    assert_eq!(modified(&file), kept_at, "the same index was kept again");
}

#[test]
fn a_pack_of_another_size_or_time_is_the_only_one_the_next_index_reads() {
    let (a, b, c, d) = (0xA1u64, 0xB2, 0xC3, 0xD4);
    let config = tempfile::tempdir().expect("a configuration folder");
    let (dir, [_, pack_b]) = two_packs([a, b]);
    indexed(config.path(), dir.path());
    let written_at = modified(&pack_b);
    // A pack of three textures, so longer than one of two, whose first is keyed `key`.
    let longer = |key: u64| {
        let mut bytes = pack_of(&vec![(71 << 3, 0, vec![0; BODY]); 3]);
        let keys = 0x100 + 16 + 16 * 3;
        bytes[keys..keys + 8].copy_from_slice(&key.to_le_bytes());
        bytes
    };

    // Longer and written when it was, holding c where it held b.
    rewritten(&pack_b, &longer(c), written_at);
    let (mut state, said) = indexed(config.path(), dir.path());
    assert_eq!(said, read_of_two(1));
    assert!(!holds(&mut state, b), "the key the pack lost is found");
    assert!(holds(&mut state, c) && holds(&mut state, a));

    // As long and written ten seconds later, holding d where it held c.
    rewritten(&pack_b, &longer(d), written_at + Duration::from_secs(10));
    let (mut state, said) = indexed(config.path(), dir.path());
    assert_eq!(said, read_of_two(1));
    assert!(!holds(&mut state, c), "the key the pack lost is found");
    assert!(holds(&mut state, d) && holds(&mut state, a));
    let (_, said) = indexed(config.path(), dir.path());
    assert_eq!(said, read_of_two(0), "not kept anew");
}

#[test]
fn a_pack_removed_leaves_the_kept_index_and_a_pack_added_enters_it() {
    let (a, b, c) = (0xA1u64, 0xB2, 0xC3);
    let config = tempfile::tempdir().expect("a configuration folder");
    let (dir, [_, pack_b]) = two_packs([a, b]);
    indexed(config.path(), dir.path());
    let [file] = kept_indexes(config.path()).try_into().expect("one file");
    let with_b = std::fs::read(&file).expect("read");

    std::fs::remove_file(&pack_b).expect("removed");
    let (mut state, said) = indexed(config.path(), dir.path());
    let one = "1 GTA VI texture pack indexed by its keys, 0 read and the rest from the kept index";
    assert_eq!(said, one);
    assert!(!holds(&mut state, b) && holds(&mut state, a));
    let without_b = std::fs::read(&file).expect("read");
    assert!(without_b != with_b, "the pack removed is still kept");

    std::fs::write(dir.path().join("c.ttpf"), keyed(c, 0x001F)).expect("written");
    let (mut state, said) = indexed(config.path(), dir.path());
    assert_eq!(said, read_of_two(1));
    assert!(holds(&mut state, c) && holds(&mut state, a));
    let (_, said) = indexed(config.path(), dir.path());
    assert_eq!(said, read_of_two(0), "not kept anew");
}

#[test]
fn a_kept_index_cut_in_half_or_of_another_version_or_folder_is_ignored_and_every_pack_read() {
    use or_io::container::rpf::{seal_rpf9_scan, unseal_rpf9_scan};
    let (a, b) = (0xA1u64, 0xB2);
    let config = tempfile::tempdir().expect("a configuration folder");
    let (dir, packs) = two_packs([a, b]);
    indexed(config.path(), dir.path());
    let [file] = kept_indexes(config.path()).try_into().expect("one file");
    let whole = std::fs::read(&file).expect("read");

    // The version is the first word of what a kept index seals.
    let mut plain = unseal_rpf9_scan(&whole, 1 << 20).expect("unsealed");
    plain[0] += 1;
    let another_version = seal_rpf9_scan(&plain).expect("sealed");

    // Another folder's packs of the same names, sizes and times, which hold other keys.
    let (other, other_packs) = two_packs([a ^ 0xFF, b ^ 0xFF]);
    for (pack, other_pack) in packs.iter().zip(&other_packs) {
        let bytes = std::fs::read(other_pack).expect("read");
        rewritten(other_pack, &bytes, modified(pack));
    }
    indexed(config.path(), other.path());
    let mut kept = kept_indexes(config.path());
    kept.retain(|kept| *kept != file);
    let another_folder = std::fs::read(&kept[0]).expect("read");

    for damaged in [&whole[..whole.len() / 2], &another_version, &another_folder] {
        std::fs::write(&file, damaged).expect("written");
        let (mut state, said) = indexed(config.path(), dir.path());
        assert_eq!(said, BOTH_READ);
        assert!(holds(&mut state, a) && holds(&mut state, b));
        let (_, said) = indexed(config.path(), dir.path());
        assert_eq!(said, read_of_two(0), "not kept anew");
    }
}

#[test]
fn a_pack_saved_over_its_own_file_streams_through_a_file_beside_it_and_reads_its_replacement_back()
{
    let dir = tempfile::tempdir().expect("a scratch directory");
    let source = written(dir.path(), &pack(0xF800));
    std::fs::create_dir(dir.path().join("sub")).expect("a sub folder");
    let same_file = dir.path().join("sub").join("..").join("textures.ttpf");
    let mut state = AppState::new();
    let doc = open(&mut state, source.clone());
    let name = match &state.document(doc).expect("open").payload {
        DocPayload::Ttpf(ttpf) => ttpf.name(0),
        _ => panic!("a .ttpf payload"),
    };
    let options = or_io::authoring::TextureBuildOptions::default();
    let choice = or_io::authoring::FormatChoice::Automatic;
    (state.import_texture_over(doc, Source::Document, &name, &blue(), choice, &options))
        .expect("the pack's texture is replaced");
    assert!(
        save_as(&mut state, doc, &same_file),
        "{}",
        state.error_dialog_message
    );

    let saved = or_io::gta6::TexturePack::open(&source).expect("the saved pack opens");
    let decoded = decode_level(&saved.texture(0).expect("reads"), 0).expect("decodes");
    assert!(decoded
        .data
        .chunks(4)
        .all(|pixel| pixel == [0, 0, 0xFF, 0xFF]));
    assert!(!dir.path().join("textures.ttpf.tmp").exists());
    let reopened = match &state.document(doc).expect("still open").payload {
        DocPayload::Ttpf(ttpf) => ttpf.pack.clone(),
        _ => panic!("a .ttpf payload"),
    };
    assert_eq!(
        reopened.rows, saved.rows,
        "the document reads the rows the save wrote"
    );
    let shown = decode_level(&reopened.texture(0).expect("reads"), 0).expect("decodes");
    assert_eq!(shown.data, decoded.data);
}

#[test]
fn a_pack_saved_over_another_tab_s_file_leaves_that_tab_s_edits_to_ask_first() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let target = written(dir.path(), &pack(0xF800));
    let other = dir.path().join("other.ttpf");
    std::fs::write(&other, pack(0x07E0)).expect("written");
    let mut state = AppState::new();
    let edited = open(&mut state, target.clone());
    let name = match &state.document(edited).expect("open").payload {
        DocPayload::Ttpf(ttpf) => ttpf.name(0),
        _ => panic!("a .ttpf payload"),
    };
    let options = or_io::authoring::TextureBuildOptions::default();
    let choice = or_io::authoring::FormatChoice::Automatic;
    (state.import_texture_over(edited, Source::Document, &name, &blue(), choice, &options))
        .expect("the pack's texture is replaced");
    let saved = open(&mut state, other);
    assert!(
        save_as(&mut state, saved, &target),
        "{}",
        state.error_dialog_message
    );
    assert!(
        state.document(edited).expect("open").dirty,
        "its replacement is kept"
    );
    assert!(state.reload_conflicts.iter().any(|p| p == &target));
}

#[test]
fn a_pack_save_stopped_before_its_rename_leaves_the_file_as_it_was() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let source = written(dir.path(), &pack(0xF800));
    let ttpf = super::ttpf_document::TtpfDocument::open(&source).expect("opens");
    let cancel = or_job::Cancel::new();
    cancel.set();
    let stopped = super::ttpf_document::saved(&ttpf, &source, &cancel, 0, 0);
    assert!(matches!(stopped, super::SaveResult::Stopped));
    assert!(std::fs::read(&source).expect("read") == pack(0xF800));
    assert!(!dir.path().join("textures.ttpf.tmp").exists());
}

#[test]
fn an_srgb_pack_texture_is_shown_and_exported_as_srgb_and_a_linear_one_as_linear() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let solid = [0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0].repeat(BODY / 8);
    // DXGI 72 is BC1 sRGB, 71 BC1.
    let textures = [(72 << 3, 0, solid.clone()), (71 << 3, 0, solid)];
    let mut state = AppState::new();
    let doc = open(&mut state, written(dir.path(), &pack_of(&textures)));
    for (index, srgb) in [(0usize, true), (1, false)] {
        let payload = &state.document(doc).expect("open").payload;
        let DocPayload::Ttpf(ttpf) = payload else {
            panic!("a .ttpf payload");
        };
        let texture =
            textures::texture_at(payload, Source::Document, &ttpf.name(index)).expect("reads");
        assert_eq!(texture.is_srgb(), srgb, "texture {index}");
        let path = dir.path().join(format!("{index}.dds"));
        textures::export_texture(&texture, 0, &path).expect("exports");
        let dds = or_io::dds::Dds::open(&path).expect("a DDS");
        let dxgi = dds.dxt10.map(|x| x.dxgi_format);
        assert_eq!(dxgi == Some(72), srgb, "texture {index}: {dxgi:?}");
    }
}

/// The text of every widget `draw` draws for `state`'s active document.
fn drawn_text(state: &mut AppState, draw: fn(&mut egui::Ui, &mut AppState)) -> Vec<String> {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1200.0, 1600.0),
        )),
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| draw(ui, state));
    let update = output.platform_output.accesskit_update.expect("an update");
    (update.nodes.iter())
        .flat_map(|(_, node)| [node.label(), node.value()])
        .flatten()
        .map(str::to_string)
        .collect()
}

fn written_as(path: PathBuf, bytes: &[u8]) -> PathBuf {
    std::fs::write(&path, bytes).expect("written");
    path
}

#[test]
fn the_pack_inspector_states_srgb_shape_key_memory_mips_read_and_row_codecs() {
    let key = 0x0BAD_F00D_1234_5678u64;
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut bytes = pack_of(&[(72 << 3, 0, [0x31; BODY].to_vec())]);
    let keys = 0x100 + 16 + 16;
    bytes[keys..keys + 8].copy_from_slice(&key.to_le_bytes());
    let mut state = AppState::new();
    let doc = open(&mut state, written(dir.path(), &bytes));
    state.selection.select(doc, 0);
    let shown = drawn_text(&mut state, crate::ui::panels::inspector::draw);
    for text in [
        "sRGB",
        "yes",
        "2D",
        "0BADF00D12345678",
        "1",
        "4 KiB",
        "stored",
        "Row codecs",
    ] {
        assert!(
            shown.iter().any(|t| t == text),
            "{text} is not shown: {shown:?}"
        );
    }

    let doc = open(
        &mut state,
        written_as(dir.path().join("cube.ttpf"), &cube()),
    );
    state.selection.select(doc, 0);
    let shown = drawn_text(&mut state, crate::ui::panels::inspector::draw);
    assert!(shown.iter().any(|t| t == "cube"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "no"), "BC7 linear: {shown:?}");
}

/// A scanned dictionary's texture `name` of `slices` 4x4 BC1 slices, slice s all `0x10 + s`, or one whose format does not read.
pub(super) fn dictionary_texture(
    name: &str,
    slices: u8,
    reads: bool,
) -> (u32, Result<Entry, HeaderlessError>) {
    let format = or_io::gta6::ttpf::Format::of_dxgi(71).expect("BC1");
    let texture = or_io::gta6::Texture {
        width: 4,
        height: 4,
        format,
        levels: 1,
        faces: slices.into(),
        pixels: (0..slices).flat_map(|s| [0x10 + s; 8]).collect(),
    };
    let entry = DictionaryTexture {
        name: name.to_string(),
        width: 4,
        height: 4,
        depth: slices.into(),
        dimension: 1,
        format: if reads { 71 } else { 34 },
        mip_levels: 1,
        pixels_at: 0,
        pixels_len: 8 * u64::from(slices),
        texture: match reads {
            true => Ok(texture),
            false => Err(HeaderlessError::TextureFormat(34)),
        },
    };
    let code = or_io::gta6::hash::name_hash32(name.as_bytes());
    (code, Ok(Entry::Texture(entry)))
}

/// `entries` as a scanned body of an archive in `dir`, opened read-only.
fn scanned(
    state: &mut AppState,
    dir: &Path,
    entries: Vec<(u32, Result<Entry, HeaderlessError>)>,
) -> DocumentId {
    // An archive whose head is not on disk yet is all zeros.
    let archive = written_as(dir.join("data.rpf"), &[0; 0x1000]);
    let path = archive.join("0x10.ytd");
    let payload = super::ttpf_document::scanned_textures(&path, Dictionary { entries })
        .expect("a texture dictionary");
    let doc = state.insert_document(path, payload, None, None);
    assert!(state.document(doc).expect("open").read_only.is_some());
    doc
}

#[test]
fn a_scanned_dictionary_lists_a_texture_that_does_not_read_with_its_reason_beside_those_that_do() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let entries = vec![
        dictionary_texture("zq_glow", 1, true),
        dictionary_texture("zq_dark", 1, false),
    ];
    let doc = scanned(&mut state, dir.path(), entries);
    let doc_ref = state.document(doc).expect("open");
    let TextureListing::Textures { textures, source } =
        textures::textures_in(&doc_ref.payload, None)
    else {
        panic!("a dictionary lists its textures");
    };
    let mut names: Vec<&str> = textures.iter().map(|t| t.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["zq_dark", "zq_glow"]);
    let why = super::ttpf_document::refused(doc_ref, "ZQ_DARK").expect("a reason");
    assert!(why.contains("34"), "{why}");
    assert!(super::ttpf_document::refused(doc_ref, "zq_glow").is_none());
    let glow = textures::texture_at(&doc_ref.payload, source, "zq_glow").expect("reads");
    assert_eq!(glow.format(), GrcTextureFormat::DXT1);

    let dark = (textures.iter())
        .find(|t| t.name == "zq_dark")
        .expect("listed");
    state.selection.select(doc, dark.selection_id);
    let shown = drawn_text(&mut state, crate::ui::panels::texture_preview::draw);
    assert!(
        (shown.iter()).any(|t| t.contains("zq_dark is not shown") && t.contains("34")),
        "{shown:?}"
    );
    // A dictionary none of whose textures reads opens as its reader's facts.
    let none = Dictionary {
        entries: vec![dictionary_texture("zq_dark", 1, false)],
    };
    assert!(super::ttpf_document::scanned_textures(Path::new("0x20.ytd"), none).is_none());

    // The same path read again, where zq_dark now reads, keeps no reason of the old read.
    let path = state
        .document(doc)
        .and_then(|d| d.path.clone())
        .expect("a path");
    let again = Dictionary {
        entries: vec![dictionary_texture("zq_dark", 1, true)],
    };
    assert!(super::ttpf_document::scanned_textures(&path, again).is_some());
    let doc_ref = state.document(doc).expect("open");
    assert!(super::ttpf_document::refused(doc_ref, "zq_dark").is_none());
}

#[test]
fn a_scanned_dictionary_s_array_offers_the_slice_picker_and_shows_the_slice_chosen() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let entries = vec![dictionary_texture("zq_array", 3, true)];
    let doc = scanned(&mut state, dir.path(), entries);
    let id = crate::ui::panels::selection_ids::name_hash("zq_array");
    state.selection.select(doc, id);
    let shown = drawn_text(&mut state, crate::ui::panels::texture_preview::draw);
    assert!(shown.iter().any(|t| t == "Slice"), "{shown:?}");

    let doc_ref = state.document(doc).expect("open");
    let texture = textures::texture_at(&doc_ref.payload, Source::Document, "zq_array")
        .expect("reads")
        .into_owned();
    let textures::Texture::Owned(texture) = texture else {
        panic!("a Legacy texture");
    };
    let slices = super::ttpf_document::scanned_slices(doc_ref, &texture);
    assert_eq!(slices, 3);
    let second = super::ttpf_document::scanned_slice(texture.into_owned(), 1, slices);
    assert_eq!(second.payload(), [0x11; 8], "slice 1 alone");
}

#[test]
fn a_scanned_dictionary_s_cube_names_the_cube_and_face_its_slice_shows() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let (code, entry) = dictionary_texture("zq_cubes", 12, true);
    let Ok(Entry::Texture(mut cubes)) = entry else {
        panic!("a texture");
    };
    cubes.dimension = 2;
    let entries = vec![(code, Ok(Entry::Texture(cubes)))];
    let doc = scanned(&mut state, dir.path(), entries);
    let id = crate::ui::panels::selection_ids::name_hash("zq_cubes");
    state.selection.select(doc, id);
    let shown = drawn_text(&mut state, crate::ui::panels::texture_preview::draw);
    assert!(shown.iter().any(|t| t == "cube 0, face +X"), "{shown:?}");
}

#[test]
fn a_scanned_dictionary_serves_a_gta_vi_drawable_and_no_gta_v_model_and_takes_another_size() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let entries = vec![dictionary_texture("zq_glow", 1, true)];
    let doc = scanned(&mut state, dir.path(), entries);
    assert!(state.texture_dictionary_is_editable(doc, Source::Document));
    let options = or_io::authoring::TextureBuildOptions::default();
    let choice = or_io::authoring::FormatChoice::Automatic;
    let image = RgbaImage {
        width: 8,
        height: 8,
        data: vec![0xFF; 256],
        is_srgb: false,
    };
    (state.import_texture_over(doc, Source::Document, "zq_glow", &image, choice, &options))
        .expect("a scanned body takes a texture of another size, which its save plans anew");

    let gta_v = or_io::Asset::TextureDict(or_io::gta5::shared::PgDictionary::default());
    let model = dir.path().join("model.ytd");
    state.insert_document(model, DocPayload::Resource(gta_v), None, None);
    assert!(
        super::texture_resolution::resolve(&state, None, None, "zq_glow").is_none(),
        "a GTA V document is drawn with no GTA VI texture"
    );
    drawn(
        &mut state,
        dir.path(),
        drawable_keyed(name_hash64(b"zq_glow")),
    );
    let found = super::texture_resolution::resolve(&state, None, None, "zq_glow")
        .expect("a GTA VI drawable is drawn with the dictionary's texture");
    assert_eq!(found.origin.document, doc);
}

/// The Gallery tile of the one texture the drawable `doc` binds: what its menu acts on, and its selection id.
fn tile(state: &AppState, doc: DocumentId) -> (Target, u64) {
    let payload = &state.document(doc).expect("open").payload;
    let TextureListing::Textures { textures, .. } = textures::textures_in(payload, None) else {
        panic!("a drawable lists the textures it binds");
    };
    let name = textures[0].name.clone();
    (Target::Texture { doc, name }, textures[0].selection_id)
}

/// `action`'s command as the tile's menu offers it.
fn offered(state: &AppState, action: Action, tile: &Target) -> commands::Command {
    commands::on(state, action, tile.clone()).expect("a command that acts on a texture")
}

fn is_blue(texture: &impl TextureSource) -> bool {
    let decoded = decode_level(texture, 0).expect("decodes");
    decoded
        .data
        .chunks(4)
        .all(|pixel| pixel == [0, 0, 0xFF, 0xFF])
}

#[test]
fn a_drawable_s_tile_replaces_its_texture_in_the_pack_that_holds_it_and_offers_that_pack_s_save() {
    let key = 0x0BAD_F00D_1234_5678u64;
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = written(dir.path(), &keyed(key, 0xF800));
    let mut state = AppState::new();
    let pack = open(&mut state, path.clone());
    let (doc, _) = drawn(&mut state, dir.path(), drawable_keyed(key));
    let (on, selection) = tile(&state, doc);
    let Target::Texture { name, .. } = &on else {
        panic!("a texture");
    };

    // The tile's picture, as the Gallery caches it under the drawable.
    let ctx = egui::Context::default();
    let cached = (doc, name.clone(), ChannelView::Rgba);
    let red = textures::resolved_for_gta6(&state, doc, None, name).expect("the key resolves");
    let shown = state
        .texture_cache
        .get_or_decode(&ctx, cached.clone(), &red);
    assert!(shown.is_some(), "the tile shows the pack's texture");

    // The palette and a key act on the tile selected, the menu on the tile clicked.
    let by_key =
        |state: &AppState| commands::of(state, Action::ReplaceTextureFromImage).map(|c| c.enabled);
    assert!(
        matches!(by_key(&state), Some(Err(_))),
        "no tile is selected"
    );
    state.selection.select(doc, selection);
    assert_eq!(by_key(&state), Some(Ok(())));

    let image = dir.path().join("blue.png");
    or_io::codec::texture::save_png(&image, &blue()).expect("an image");
    file_dialogs::answer_next(image);
    let replace = offered(&state, Action::ReplaceTextureFromImage, &on);
    assert_eq!(replace.enabled, Ok(()));
    commands::run(&mut state, &ctx, &replace);

    assert_eq!(state.active_document, Some(doc), "the model stays in front");
    let held = &state.document(pack).expect("open").payload;
    let held = textures::texture_at(held, Source::Document, name).expect("the pack's texture");
    assert!(is_blue(&held), "{}", state.error_dialog_message);
    let drawn_with = textures::resolved_for_gta6(&state, doc, None, name).expect("resolves");
    assert!(
        is_blue(&drawn_with),
        "the model is drawn with the replacement"
    );
    assert!(
        state.texture_cache.get(&cached).is_none(),
        "the tile keeps the picture it had"
    );

    let toast = (state.toasts.iter())
        .find(|toast| toast.action == Some(ToastAction::SavePack(pack)))
        .unwrap_or_else(|| panic!("no toast offers the pack's save: {:?}", state.toasts));
    assert!(
        toast.message.contains(name.as_str()) && toast.message.contains("textures.ttpf"),
        "{}",
        toast.message
    );
    state.run_toast_action(ToastAction::SavePack(pack));
    let until = Instant::now() + Duration::from_secs(60);
    while state.save_receiver.is_some() {
        assert!(Instant::now() < until, "the save did not finish");
        state.poll_save_result();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(state.active_document, Some(doc), "the model stays in front");
    assert!(!state.document(pack).expect("open").dirty);
    let saved = or_io::gta6::TexturePack::open(&path).expect("the saved pack opens");
    assert!(
        is_blue(&saved.texture(0).expect("reads")),
        "{}",
        state.error_dialog_message
    );
}

#[test]
fn a_texture_replaced_undone_or_redone_in_its_pack_s_tab_leaves_no_drawable_s_tile_as_it_was() {
    use super::EditDirection::{Redo, Undo};
    let key = 0x0BAD_F00D_1234_5678u64;
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let pack = open(&mut state, written(dir.path(), &keyed(key, 0xF800)));
    let (doc, _) = drawn(&mut state, dir.path(), drawable_keyed(key));
    let (Target::Texture { name, .. }, _) = tile(&state, doc) else {
        panic!("a texture");
    };
    // The tile's picture, as the Gallery caches it under the drawable.
    let ctx = egui::Context::default();
    let cached = (doc, name.clone(), ChannelView::Rgba);
    let show = |state: &mut AppState| {
        let held = textures::resolved_for_gta6(state, doc, None, &name).expect("it resolves");
        let shown = (state.texture_cache).get_or_decode(&ctx, cached.clone(), &held);
        assert!(shown.is_some(), "the tile shows the pack's texture");
    };

    show(&mut state);
    let options = or_io::authoring::TextureBuildOptions::default();
    let choice = or_io::authoring::FormatChoice::Automatic;
    state
        .import_texture_over(pack, Source::Document, &name, &blue(), choice, &options)
        .expect("the pack takes the image");
    let stale = |state: &mut AppState| state.texture_cache.get(&cached).is_some();
    assert!(!stale(&mut state), "replaced in the pack's tab");
    for direction in [Undo, Redo] {
        show(&mut state);
        assert!(state.step(pack, direction).is_some(), "{direction:?}");
        assert!(!stale(&mut state), "{direction:?} in the pack's tab");
    }
}

/// A 4x4 image all blue, the size of [`dictionary_texture`]'s.
fn small_blue() -> RgbaImage {
    RgbaImage {
        width: 4,
        height: 4,
        data: [0, 0, 0xFF, 0xFF].repeat(16),
        is_srgb: false,
    }
}

#[test]
fn a_drawable_s_tile_exports_a_scanned_dictionary_s_texture_and_replaces_it_as_its_own_tab_does() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let entries = vec![dictionary_texture("zq_glow", 1, true)];
    let dictionary = scanned(&mut state, dir.path(), entries);
    let key = name_hash64(b"zq_glow");
    let (doc, _) = drawn(&mut state, dir.path(), drawable_keyed(key));
    let (on, _) = tile(&state, doc);

    let replace = offered(&state, Action::ReplaceTextureFromImage, &on);
    assert_eq!(replace.enabled, Ok(()));
    let image = dir.path().join("blue.png");
    or_io::codec::texture::save_png(&image, &small_blue()).expect("an image");
    file_dialogs::answer_next(image);
    commands::run(&mut state, &egui::Context::default(), &replace);
    assert_eq!(state.active_document, Some(doc), "the model stays in front");
    let held = state.document(dictionary).expect("open");
    let texture = textures::texture_at(&held.payload, Source::Document, "zq_glow").expect("held");
    assert!(is_blue(&texture), "{:?}", state.toasts);
    assert!(held.dirty, "the dictionary's tab holds the replacement");
    assert!(
        super::gta6_edit::edited_save(held).is_some(),
        "its Save As writes it"
    );
    let said = (state.toasts.iter()).find(|t| t.message.contains("Replaced zq_glow"));
    let said = said.unwrap_or_else(|| panic!("no toast: {:?}", state.toasts));
    assert!(
        said.message.contains("Save As") && said.action.is_none(),
        "{}",
        said.message
    );

    let export = offered(&state, Action::ExportTexture, &on);
    assert_eq!(export.enabled, Ok(()));
    let to = dir.path().join("glow.png");
    file_dialogs::answer_next(to.clone());
    commands::run(&mut state, &egui::Context::default(), &export);
    let exported = or_io::codec::texture::load_png(&to).expect("the export is a PNG");
    assert_eq!((exported.width, exported.height), (4, 4));
}

#[test]
fn a_drawable_s_tile_offers_neither_command_for_a_key_no_open_document_holds() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let (doc, _) = drawn(
        &mut state,
        dir.path(),
        drawable_keyed(0x0BAD_F00D_1234_5678),
    );
    let (on, _) = tile(&state, doc);
    let unresolved = crate::i18n::tr(super::gta6_textures::UNRESOLVED);
    assert!(unresolved.contains("is unresolved: no open texture pack"));
    for action in [Action::ReplaceTextureFromImage, Action::ExportTexture] {
        let enabled = offered(&state, action, &on).enabled;
        assert_eq!(enabled, Err(unresolved), "{action:?}");
    }
}

/// The Texture preview's button, as the bar names it.
const REPLACE: &str = "Replace from image…";
const PREVIEW: fn(&mut egui::Ui, &mut AppState) = crate::ui::panels::texture_preview::draw;

#[test]
fn the_texture_workspace_shows_a_drawable_s_texture_and_its_bar_replaces_it_in_the_pack_that_holds_it(
) {
    let key = 0x0BAD_F00D_1234_5678u64;
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let pack = open(&mut state, written(dir.path(), &keyed(key, 0xF800)));
    let (doc, _) = drawn(&mut state, dir.path(), drawable_keyed(key));
    let (on, selection) = tile(&state, doc);
    state.selection.select(doc, selection);
    state.active_workspace = Workspace::Texture;
    let shown = drawn_text(&mut state, |ui, state| {
        crate::ui::panels::gallery::draw(ui, state, None)
    });
    assert!(shown.iter().any(|t| t == "1 texture"), "{shown:?}");

    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, PREVIEW, &mut state, REPLACE, 0.0, Vec::new());
    let (bar, _) = frame(&ctx, PREVIEW, &mut state, REPLACE, 0.1, Vec::new());
    let (at, greyed) = bar.expect("the preview draws the texture under its bar");
    assert!(!greyed, "the pack takes a replacement from the model's tab");

    let image = dir.path().join("blue.png");
    or_io::codec::texture::save_png(&image, &blue()).expect("an image");
    file_dialogs::answer_next(image);
    let press = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let down = vec![egui::Event::PointerMoved(at), press(true)];
    frame(&ctx, PREVIEW, &mut state, REPLACE, 0.2, down);
    frame(&ctx, PREVIEW, &mut state, REPLACE, 0.3, vec![press(false)]);

    let Target::Texture { name, .. } = &on else {
        panic!("a texture");
    };
    let held = &state.document(pack).expect("open").payload;
    let held = textures::texture_at(held, Source::Document, name).expect("the pack's texture");
    assert!(is_blue(&held), "{:?}", state.toasts);
    let offers_the_save =
        |toast: &super::toasts::Toast| toast.action == Some(ToastAction::SavePack(pack));
    assert!(
        state.toasts.iter().any(offers_the_save),
        "{:?}",
        state.toasts
    );
}

#[test]
fn the_texture_preview_s_bar_replaces_a_scanned_dictionary_s_texture_a_drawable_binds() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let entries = vec![dictionary_texture("zq_glow", 1, true)];
    scanned(&mut state, dir.path(), entries);
    let key = name_hash64(b"zq_glow");
    let (doc, _) = drawn(&mut state, dir.path(), drawable_keyed(key));
    let (_, selection) = tile(&state, doc);
    state.selection.select(doc, selection);

    let replace = control(PREVIEW, &mut state, REPLACE);
    assert!(
        !replace.disabled,
        "the dictionary's tab takes the replacement"
    );
    assert!(!replace.says(crate::i18n::tr(super::rpf9_scan::READ_ONLY_BODY)));
}
