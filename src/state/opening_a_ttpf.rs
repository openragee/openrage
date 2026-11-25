//! A GTA VI `.ttpf` opens as a document that lists its textures from the directory, reads one
//! from the file only when it is shown, replaces one from an image and saves; a
//! scanned texture dictionary lists its textures by name, and names a pack's texture its name's
//! hash keys.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use or_io::codec::texture::{decode_level, GrcTextureFormat};
use or_io::container::rpf::{Rpf9Role, Rpf9Signature};
use or_io::gta6::hash::{name_hash32, name_hash64};

use super::textures::{self, TextureListing, TextureResolution, TextureSource};
use super::{AppState, DocPayload, DocumentId, DocumentKind, LoadingState};
use crate::ui::dock::Workspace;

/// A 128x64 BC1 texture is 32x16 blocks: one 4 KiB tile, so a solid colour reads whatever the tiling.
pub(super) const SIDE: (u32, u32) = (128, 64);
/// Each texture's decoded size.
pub(super) const BODY: usize = 0x1000;
/// A format code whose DXGI number, 34, has no measured element.
const NOT_BC: u16 = 0x110;
/// A codec kind that is not identified, whose stored bytes are never read.
const UNIDENTIFIED: u8 = 1;

/// A pack laid out as the container is measured to be, one body per texture: its format code, its
/// codec kind and its stored bytes, which a stored body decodes to and any other kind to [`BODY`]
/// bytes.
pub(super) fn pack_of(textures: &[(u16, u8, Vec<u8>)]) -> Vec<u8> {
    let count = textures.len() as u64;
    let directory = (16 + 8 * (count + 2)).next_multiple_of(0x100);
    let directory_len = (16 + 32 * count).next_multiple_of(0x1000);
    let mut file = vec![0u8; (directory + directory_len) as usize];
    let put = |file: &mut Vec<u8>, at: u64, word: u32| {
        let at = at as usize;
        file[at..at + 4].copy_from_slice(&word.to_le_bytes());
    };
    put(&mut file, directory, u32::from_le_bytes(*b"ETPF"));
    put(&mut file, directory + 4, 0x27A);
    put(&mut file, directory + 8, count as u32);

    // (offset, kind, virtual address) of each row, the sentinel last.
    let mut rows = vec![(directory, 0, 0)];
    let mut va = directory_len;
    for (k, (code, kind, stored)) in textures.iter().enumerate() {
        let entry = directory + 16 + 16 * k as u64;
        put(
            &mut file,
            entry,
            (SIDE.0 - 1) | (SIDE.1 - 1) << 14 | 1 << 28,
        );
        put(&mut file, entry + 4, u32::from(*code) | 1 << 21 | 1 << 24);
        put(&mut file, entry + 8, (va >> 12) as u32);
        rows.push((file.len() as u64, *kind, va));
        file.extend(stored);
        va += if *kind == 0 { stored.len() } else { BODY } as u64;
    }
    let (_, last_kind, _) = rows[rows.len() - 1];
    let virtual_size = va;
    rows.push((file.len() as u64, last_kind, virtual_size));

    put(&mut file, 0, u32::from_le_bytes(*b"RDS5"));
    put(&mut file, 4, rows.len() as u32);
    put(&mut file, 8, virtual_size as u32);
    for (i, &(offset, kind, va)) in rows.iter().enumerate() {
        let page = va >> 12;
        let at = 16 + 8 * i as u64;
        let lo = (offset >> 8) as u32 | u32::from(kind) << 26 | ((page & 3) as u32) << 30;
        put(&mut file, at, lo);
        put(&mut file, at + 4, (page >> 2) as u32);
    }
    file
}

/// Two stored textures: a BC1 one of solid `rgb565`, then one whose format is not BC.
pub(super) fn pack(rgb565: u16) -> Vec<u8> {
    let [lo, hi] = rgb565.to_le_bytes();
    let solid = [lo, hi, lo, hi, 0, 0, 0, 0].repeat(BODY / 8);
    pack_of(&[(0x238, 0, solid.clone()), (NOT_BC, 0, solid)])
}

/// What a 565 colour decodes to, each channel widened to eight bits.
pub(super) fn rgba(rgb565: u16) -> [u8; 4] {
    let widen = |value: u16, max: u16| ((u32::from(value) * 255 + 7) / u32::from(max)) as u8;
    [
        widen(rgb565 >> 11, 31),
        widen((rgb565 >> 5) & 63, 63),
        widen(rgb565 & 31, 31),
        255,
    ]
}

pub(super) fn open(state: &mut AppState, path: PathBuf) -> DocumentId {
    state.load_file(path.clone());
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.load_receiver.is_some() && Instant::now() < deadline {
        state.poll_load_result();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        matches!(state.loading, LoadingState::Idle),
        "{}: {}",
        path.display(),
        state.error_dialog_message
    );
    state
        .active_document
        .expect("the file opened as a document")
}

pub(super) fn written(dir: &Path, bytes: &[u8]) -> PathBuf {
    let path = dir.join("textures.ttpf");
    std::fs::write(&path, bytes).expect("written");
    path
}

#[test]
fn a_ttpf_clicked_in_an_open_folders_tree_opens_from_its_file() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = written(dir.path(), &pack(0xF800));
    let mut state = AppState::new();
    state.open_folder(dir.path().to_path_buf());
    let folder = (state.documents.iter())
        .find(|d| d.path.as_deref() == Some(dir.path()))
        .expect("the folder opens as a document")
        .id;
    let row = super::destructive_file_operations_ask::node_at(&mut state, folder, "textures.ttpf");
    state.open_rpf_entry(folder, row);
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.load_receiver.is_some() && Instant::now() < deadline {
        state.poll_load_result();
        std::thread::sleep(Duration::from_millis(5));
    }
    let opened = (state.documents.iter()).find(|d| d.path.as_deref() == Some(path.as_path()));
    assert_eq!(
        opened.map(|d| d.kind),
        Some(DocumentKind::Ttpf),
        "{}",
        state.error_dialog_message
    );
}

#[test]
fn a_ttpf_lists_its_textures_and_shows_a_readable_one_from_the_file_when_shown() {
    const RED: u16 = 0xF800;
    const GREEN: u16 = 0x07E0;
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = written(dir.path(), &pack(RED));
    let mut state = AppState::new();
    let doc = open(&mut state, path.clone());
    assert_eq!(
        state.document(doc).map(|d| d.kind),
        Some(DocumentKind::Ttpf)
    );
    assert_eq!(state.active_workspace, Workspace::Texture);

    // Written after the open: what is shown is what the file holds when it is shown.
    std::fs::write(&path, pack(GREEN)).expect("rewritten");

    let payload = &state.document(doc).expect("open").payload;
    let TextureListing::Textures { textures, source } = textures::textures_in(payload, None) else {
        panic!("a pack lists its textures");
    };
    assert_eq!(textures.len(), 2);
    assert_eq!((textures[0].width, textures[0].height), SIDE);
    assert_eq!(textures[0].format, GrcTextureFormat::DXT1);
    assert_eq!(textures[1].format, GrcTextureFormat::None);

    let TextureResolution::Found { name, .. } =
        textures::texture_for_selection(payload, textures[0].selection_id)
    else {
        panic!("the first row names a texture");
    };
    let texture = textures::texture_at(payload, source, &name).expect("texture 0 reads");
    let image = decode_level(&texture, 0).expect("decodes");
    assert_eq!((image.width, image.height), SIDE);
    assert!(
        image.data.chunks(4).all(|pixel| pixel == rgba(GREEN)),
        "texture 0 is the file's pixels when it was shown"
    );

    let png = dir.path().join("shown.png");
    textures::export_texture(&texture, 0, &png).expect("a shown texture exports");
    let exported = or_io::codec::texture::load_png(&png).expect("the export is a PNG");
    assert_eq!(&exported.data[..4], rgba(GREEN));

    let DocPayload::Ttpf(ttpf) = payload else {
        panic!("a .ttpf payload");
    };
    let why = ttpf.refusal(1).expect("texture 1 is not readable");
    assert!(why.contains(&format!("{NOT_BC:#05x}")), "{why}");
    assert!(textures::texture_at(payload, source, &textures[1].name).is_none());
}

/// Save As to `to`, waited for; whether the save was started.
pub(super) fn save_as(state: &mut AppState, doc: DocumentId, to: &Path) -> bool {
    state.set_active_document(doc);
    let started = state.save_active_document_as(to.to_path_buf());
    let until = Instant::now() + Duration::from_secs(60);
    while state.save_receiver.is_some() {
        assert!(Instant::now() < until, "the save did not finish");
        state.poll_save_result();
        std::thread::sleep(Duration::from_millis(5));
    }
    started
}

fn ttpf_pack(state: &AppState, doc: DocumentId) -> or_io::gta6::TexturePack {
    let Some(DocPayload::Ttpf(ttpf)) = state.document(doc).map(|d| &d.payload) else {
        panic!("a .ttpf payload");
    };
    ttpf.pack.clone()
}

#[test]
fn a_ttpf_texture_replaced_from_an_image_saves_as_a_new_pack_that_reads_it_back() {
    const BLUE: [u8; 4] = [0, 0, 0xFF, 0xFF];
    let dir = tempfile::tempdir().expect("a scratch directory");
    let source = written(dir.path(), &pack(0xF800));
    let mut state = AppState::new();
    let doc = open(&mut state, source.clone());
    assert!(state.texture_dictionary_is_editable(doc, TextureSource::Document));
    let Some(DocPayload::Ttpf(ttpf)) = state.document(doc).map(|d| &d.payload) else {
        panic!("a .ttpf payload");
    };
    let name = ttpf.name(0);
    let image = or_io::codec::texture::RgbaImage {
        width: SIDE.0,
        height: SIDE.1,
        data: BLUE.repeat((SIDE.0 * SIDE.1) as usize),
        is_srgb: false,
    };
    let options = or_io::authoring::TextureBuildOptions::default();
    let choice = or_io::authoring::FormatChoice::Automatic;
    let source_of = TextureSource::Document;
    let edit = (state.import_texture_over(doc, source_of, &name, &image, choice, &options))
        .expect("the pack's texture is replaced");
    assert!(edit.describe().contains(&name), "{}", edit.describe());
    // A one-mip texture under 128 KiB takes a chain of mips, its flags from its tiled pieces.
    (state.reencode_texture_in_place(doc, source_of, &name, choice, &options))
        .expect("the pack's texture is re-encoded with its mips");

    let payload = &state.document(doc).expect("open").payload;
    let shown = textures::texture_at(payload, source_of, &name).expect("shown");
    let decoded = decode_level(&shown, 0).expect("decodes");
    assert!(
        decoded.data.chunks(4).all(|pixel| pixel == BLUE),
        "the view shows the replacement"
    );

    let saved = dir.path().join("saved.ttpf");
    assert!(
        save_as(&mut state, doc, &saved),
        "{}",
        state.error_dialog_message
    );
    let reopened = or_io::gta6::TexturePack::open(&saved).expect("the saved pack opens");
    let texture = reopened
        .texture(0)
        .expect("the replacement reads from the new file");
    let decoded = decode_level(&texture, 0).expect("decodes");
    assert!(decoded.data.chunks(4).all(|pixel| pixel == BLUE));
    assert_eq!(
        reopened.entries,
        ttpf_pack(&state, doc).entries,
        "keys and entries are kept"
    );
    assert!(
        std::fs::read(&source).expect("read") == pack(0xF800),
        "the source is untouched"
    );
    assert!(super::document_io::why_not_authorable(DocumentKind::Ttpf).is_some());
}

#[test]
fn an_open_gta_vi_pack_supplies_no_texture_to_a_gta_v_document_and_one_to_a_gta_vi_drawable() {
    let key = 0x0BAD_F00D_1234_5678u64;
    let mut bytes = pack(0xF800);
    let keys = 0x100 + 16 + 16 * 2;
    bytes[keys..keys + 8].copy_from_slice(&key.to_le_bytes());
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = written(dir.path(), &bytes);
    let ttpf = super::ttpf_document::TtpfDocument::open(&path).expect("the pack opens");
    let mut state = AppState::new();
    state.insert_document(path, DocPayload::Ttpf(Arc::new(ttpf)), None, None);
    let name = format!("hash_{key:016x}");
    let gta_v = or_io::Asset::TextureDict(or_io::gta5::shared::PgDictionary::default());
    state.insert_document(
        dir.path().join("model.ytd"),
        DocPayload::Resource(gta_v),
        None,
        None,
    );
    assert!(
        super::texture_resolution::resolve(&state, None, None, &name).is_none(),
        "a GTA V document is drawn with no GTA VI texture"
    );
    drawn(&mut state, dir.path(), drawable_keyed(key));
    assert!(
        super::texture_resolution::resolve(&state, None, None, &name).is_some(),
        "a GTA VI drawable is drawn with the pack's texture"
    );
}

#[test]
fn a_ttpf_whose_bodies_are_not_on_disk_yet_opens_and_names_why() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let whole = pack(0xF800);
    // Inside texture 0's body, which the last body follows.
    let cut = whole.len() - BODY - BODY / 2;
    let mut state = AppState::new();
    let doc = open(&mut state, written(dir.path(), &whole[..cut]));
    let Some(DocPayload::Ttpf(ttpf)) = state.document(doc).map(|d| &d.payload) else {
        panic!("a .ttpf payload");
    };
    assert_eq!(ttpf.pack.entries.len(), 2);
    let why = ttpf.refusal(0).expect("texture 0's body is cut");
    assert!(why.contains(&format!("{cut} bytes on disk")), "{why}");
}

/// A pack of one 16×16 BC7 cube in 4 KiB tiles: six faces of one tail tile each, in one stored body, face f's bytes all 0x50 + f.
/// Its format flags are 0, as a cube of one row states.
pub(super) fn cube() -> Vec<u8> {
    let faces: Vec<u8> = (0..6u8).flat_map(|face| [0x50 + face; 0x1000]).collect();
    let mut bytes = pack_of(&[(0x310, 0, faces)]);
    let entry = 0x110;
    let words = [15 | 15 << 14 | 5 << 28, 0x310 | 5 << 10 | 2 << 21 | 1 << 24];
    for (i, word) in words.into_iter().enumerate() {
        bytes[entry + 4 * i..entry + 4 * i + 4].copy_from_slice(&u32::to_le_bytes(word));
    }
    bytes
}

#[test]
fn a_cube_shows_the_face_its_slice_picker_chose() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let doc = open(&mut state, written(dir.path(), &cube()));
    let payload = &state.document(doc).expect("open").payload;
    let DocPayload::Ttpf(ttpf) = payload else {
        panic!("a .ttpf payload");
    };
    assert_eq!((ttpf.refusal(0), ttpf.slices(0)), (None, 6));
    let source = TextureSource::Document;
    let name = ttpf.name(0);
    let shown = |payload: &DocPayload| {
        let texture = textures::texture_at(payload, source, &name).expect("a face is shown");
        let texture = texture.into_owned();
        let bytes = or_io::codec::texture::TextureSource::payload(&texture).to_vec();
        assert!(bytes.iter().all(|&b| b == bytes[0]), "one face");
        bytes[0]
    };
    assert_eq!(shown(payload), 0x50);
    ttpf.show_slice(0, 3);
    assert_eq!(shown(payload), 0x53, "-Y, the face chosen");
    ttpf.show_slice(0, 9);
    assert_eq!(shown(payload), 0x55, "past the last face, the last");
}

#[test]
fn the_texture_view_offers_a_slice_picker_for_a_cube_and_none_for_a_2d_texture() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let labels = |state: &mut AppState| -> Vec<String> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| {
            crate::ui::panels::texture_preview::draw(ui, state)
        });
        let update = output.platform_output.accesskit_update.expect("an update");
        let nodes = update.nodes.iter();
        nodes
            .filter_map(|(_, node)| node.label().map(str::to_string))
            .collect()
    };
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    for (name, bytes, picker) in [
        ("cube.ttpf", cube(), true),
        ("flat.ttpf", pack(0xF800), false),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).expect("written");
        let doc = open(&mut state, path);
        state.selection.select(doc, 0);
        let shown = labels(&mut state);
        assert_eq!(
            shown.iter().any(|l| l == "Slice"),
            picker,
            "{name}: {shown:?}"
        );
        assert_eq!(
            shown.iter().any(|l| l == "face +X"),
            picker,
            "the cube's slice names its face: {name}: {shown:?}"
        );
    }
}

/// The text of every shape `shapes` paints that starts with `#`: a texture's name.
fn names_painted(shapes: &[egui::epaint::ClippedShape]) -> usize {
    fn count(shape: &egui::Shape) -> usize {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().map(count).sum(),
            egui::Shape::Text(text) => usize::from(text.galley.text().starts_with('#')),
            _ => 0,
        }
    }
    shapes.iter().map(|clipped| count(&clipped.shape)).sum()
}

#[test]
fn ten_thousand_textures_lay_out_only_the_rows_in_view() {
    use super::nothing_draws_over_what_it_belongs_to::Panel;

    let _serial = crate::ui::theme::serial();
    const COUNT: usize = 10_000;
    let unread = (NOT_BC, UNIDENTIFIED, vec![0x31; 0x100]);
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let doc = open(
        &mut state,
        written(dir.path(), &pack_of(&vec![unread; COUNT])),
    );
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 720.0));
    type Draw = fn(&mut egui::Ui, &mut AppState);
    let views: [(&str, Draw); 2] = [
        ("gallery", |ui, state| {
            crate::ui::panels::gallery::draw(ui, state, None)
        }),
        ("outline", crate::ui::panels::hierarchy::draw_outline),
    ];
    for (view, draw) in views {
        let mut panel = Panel::new(rect);
        let mut output = None;
        for _ in 0..3 {
            output = Some(panel.frame(Vec::new(), |ui| draw(ui, &mut state)));
        }
        let output = output.expect("drawn");
        let nodes = output
            .platform_output
            .accesskit_update
            .as_ref()
            .map_or(0, |update| update.nodes.len());
        let painted = names_painted(&output.shapes);
        assert!(painted > 0, "the {view} paints no texture of {doc}");
        assert!(
            nodes < COUNT / 10,
            "the {view} lays out {nodes} widgets for {COUNT} textures, {painted} of them painted"
        );
    }
}

/// The virtual and physical sections of a headerless dictionary of one texture named `name`:
/// 128x64 BC1 of one level, one 4 KiB tile of solid `rgb565` in the physical section.
fn texture_dictionary(name: &str, rgb565: u16) -> (Vec<u8>, Vec<u8>) {
    const VIRTUAL: u64 = 0x5000_0000;
    let mut v = vec![0u8; 0x200];
    let mut put = |at: usize, bytes: &[u8]| v[at..at + bytes.len()].copy_from_slice(bytes);
    // The root: its block map, a reference count, then one code and one entry.
    put(0x08, &(VIRTUAL + 0x1F0).to_le_bytes());
    put(0x18, &1u32.to_le_bytes());
    put(0x20, &(VIRTUAL + 0x180).to_le_bytes());
    put(0x28, &[1, 0, 1, 0]);
    put(0x30, &(VIRTUAL + 0x190).to_le_bytes());
    put(0x38, &[1, 0, 1, 0]);
    put(0x180, &name_hash32(name.as_bytes()).to_le_bytes());
    put(0x190, &(VIRTUAL + 0x40).to_le_bytes());
    // The texture object at 0x40, its view at 0x90, its name at 0x100 behind 8 zero bytes.
    let t = 0x40;
    put(t + 0x08, &(BODY as u32 / 8).to_le_bytes());
    put(t + 0x0C, &[8, 0, 8, 0]);
    put(t + 0x18, &0x6000_0000u64.to_le_bytes());
    put(t + 0x28, &(VIRTUAL + t as u64 + 0x50).to_le_bytes());
    put(t + 0x30, &(VIRTUAL + 0x100).to_le_bytes());
    put(t + 0x38, &[SIDE.0 as u8, 0, SIDE.1 as u8, 0, 1, 0]);
    // Dimension 2D, DXGI 71 (BC1), tile mode 5, one level.
    put(t + 0x3E, &[1, 71, 5, 0, 1]);
    put(t + 0x46, &1u16.to_le_bytes());
    put(t + 0x4C, &2u32.to_le_bytes());
    put(0x108, name.as_bytes());
    // One virtual block and one physical.
    put(0x1F8, &[1, 1]);
    let [lo, hi] = rgb565.to_le_bytes();
    (v, [lo, hi, lo, hi, 0, 0, 0, 0].repeat(BODY / 8))
}

#[test]
fn a_scanned_texture_dictionary_lists_its_textures_by_name_and_names_a_pack_s_texture() {
    const BLUE: u16 = 0x001F;
    let (v, p) = texture_dictionary("Lamp_Glow", BLUE);
    let signature = Rpf9Signature::of(&v, v.len() as u64).expect("a signature");
    assert_eq!(signature.role(), Some(Rpf9Role::Dictionary));
    let dictionary = |v: &[u8]| match or_io::gta6::headerless::read(signature, v, Some(&p)) {
        Ok(or_io::gta6::headerless::Resource::Dictionary(d)) => d,
        other => panic!("not a dictionary: {other:?}"),
    };
    let payload = super::ttpf_document::scanned_textures(Path::new("0x10.ytd"), dictionary(&v))
        .expect("a dictionary of textures");
    let TextureListing::Textures { textures, source } = textures::textures_in(&payload, None)
    else {
        panic!("a dictionary lists its textures");
    };
    assert_eq!(textures.len(), 1);
    assert_eq!(textures[0].name, "Lamp_Glow");
    let texture = textures::texture_at(&payload, source, "Lamp_Glow").expect("it reads");
    let image = decode_level(&texture, 0).expect("decodes");
    assert!(image.data.chunks(4).all(|pixel| pixel == rgba(BLUE)));

    // A pack whose first texture is keyed by the name's 64-bit hash then shows that name.
    let mut bytes = pack(0xF800);
    let keys = 0x100 + 16 + 16 * 2;
    bytes[keys..keys + 8].copy_from_slice(&name_hash64(b"lamp_glow").to_le_bytes());
    bytes[keys + 8..keys + 16].copy_from_slice(&0x1234u64.to_le_bytes());
    let dir = tempfile::tempdir().expect("a scratch directory");
    let ttpf = super::ttpf_document::TtpfDocument::open(&written(dir.path(), &bytes))
        .expect("the pack opens");
    assert_eq!(ttpf.name(0), "Lamp_Glow");
    assert_eq!(ttpf.name(1), "hash_0000000000001234");
    assert_eq!(ttpf.index_of("hash_0000000000001234"), Some(1));
    assert_eq!(ttpf.index_of("LAMP_GLOW"), Some(0));
    assert_eq!(ttpf.learn("a name no key is the hash of"), None);

    // An entry that is no texture: no texture view, so the body opens as its reader's facts.
    let mut other = v.clone();
    other[0x40..0x48].fill(0xFF);
    let other = dictionary(&other);
    assert!(super::ttpf_document::scanned_textures(Path::new("0x10.ytd"), other).is_none());
}

/// A GTA VI drawable of one triangle whose one shader binds `key` in its second slot, as its base colour.
pub(super) fn drawable_keyed(key: u64) -> or_io::gta6::headerless::Drawable {
    use or_io::gta6::headerless::{
        Drawable, Geometry, Model, Parameter, ParameterValue, Shader, Vertices,
    };
    let geometry = Geometry {
        vertex_data: Vec::new(),
        index_data: Vec::new(),
        shader: 0,
        vertex_count: 3,
        channels: Vec::new(),
        triangles: vec![[0, 1, 2]],
        positions: Ok(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
        vertices: Vertices::default(),
    };
    Drawable {
        name: "lamp.#dr".to_string(),
        box_min: [0.0; 3],
        box_max: [1.0, 1.0, 0.0],
        shaders: vec![Shader {
            name_hash: 1,
            texture_keys: vec![None, Some(key)],
            parameters: vec![Parameter {
                hash: or_io::gta6::hash::name_hash32(b"DiffuseTex"),
                value: ParameterValue::Texture { slot: 1 },
            }],
        }],
        levels: [
            Some(vec![Model {
                geometries: vec![Ok(geometry)],
                palette: Ok(or_io::gta6::headerless::Palette::Own),
                matrix_index: 0,
            }]),
            None,
            None,
            None,
        ],
        lod_distances: [9998.0; 4],
        bound: None,
        embedded: None,
        extra_skeletons: Vec::new(),
    }
}

/// Wait for the naming runs an open started to end, and say how they went as a frame does.
pub(super) fn named_by_the_rules(state: &mut AppState) {
    let until = Instant::now() + Duration::from_secs(30);
    while state.texture_naming_jobs().next().is_some() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(2));
    }
    state.poll_jobs();
    assert!(
        state.texture_naming.is_empty(),
        "a naming run is still running"
    );
}

/// Insert `drawable` as a scanned body, the active document, and the meshes the model view draws of it.
pub(super) fn drawn(
    state: &mut AppState,
    dir: &Path,
    drawable: or_io::gta6::headerless::Drawable,
) -> (DocumentId, Vec<crate::render::meshes::MeshUpload>) {
    use or_io::gta6::headerless::Resource;
    let payload = DocPayload::Gta6Drawable(Arc::new(Resource::Drawable(drawable)));
    let doc = state.insert_document(dir.join("0x10.drawable"), payload, None, None);
    named_by_the_rules(state);
    let state = &*state;
    let request = super::capabilities::GeometryRequest {
        state,
        doc: state.document(doc).expect("open"),
        selection: None,
        entry: None,
    };
    (doc, crate::render::gta6_drawable::meshes(&request))
}

#[test]
fn a_gta_vi_drawable_s_texture_key_resolves_to_a_scanned_dictionary_s_texture_by_its_name() {
    let (v, p) = texture_dictionary("Lamp_Glow", 0x001F);
    let signature = Rpf9Signature::of(&v, v.len() as u64).expect("a signature");
    let Ok(or_io::gta6::headerless::Resource::Dictionary(dict)) =
        or_io::gta6::headerless::read(signature, &v, Some(&p))
    else {
        panic!("not a dictionary");
    };
    let textures = super::ttpf_document::scanned_textures(Path::new("0x10.ytd"), dict)
        .expect("a dictionary of textures");
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let dictionary = state.insert_document(dir.path().join("lamps.ytd"), textures, None, None);
    // Injecting the decoded payload also needs the provenance a scanned path sets on open.
    state
        .document_mut(dictionary)
        .expect("the dictionary")
        .read_only = Some(super::rpf9_scan::READ_ONLY_BODY);
    let key = name_hash64(b"lamp_glow");
    let (doc, meshes) = drawn(&mut state, dir.path(), drawable_keyed(key));
    assert_eq!(meshes[0].texture_name.as_deref(), Some("lamp_glow"));
    let found = super::texture_resolution::resolve(&state, None, None, "lamp_glow")
        .expect("the dictionary's texture resolves");
    assert_eq!(u32::from(found.texture.width()), SIDE.0);
    let listed = super::textures::textures_in(&state.document(doc).expect("open").payload, None);
    let super::textures::TextureListing::Textures { textures, .. } =
        super::textures::sized_for_gta6(&state, doc, listed)
    else {
        panic!("the drawable lists its key");
    };
    let size = |t: &super::textures::DocumentTexture| (t.width, t.height);
    assert_eq!(
        textures.iter().map(size).collect::<Vec<_>>(),
        [SIDE],
        "listed in the size of the texture it resolves to"
    );
    let materials = super::gta6_resource::material_rows(&state, 0, &drawable_keyed(key));
    let bound = &materials[0].1[1].1;
    assert!(bound.contains("lamps.ytd"), "{bound}");
}

#[test]
fn a_gta_vi_drawable_s_texture_key_resolves_to_an_open_pack_and_an_unknown_one_says_unresolved() {
    let key = 0x0BAD_F00D_1234_5678u64;
    let mut bytes = pack(0xF800);
    let keys = 0x100 + 16 + 16 * 2;
    bytes[keys..keys + 8].copy_from_slice(&key.to_le_bytes());
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = written(dir.path(), &bytes);
    let ttpf = super::ttpf_document::TtpfDocument::open(&path).expect("the pack opens");
    let mut state = AppState::new();
    state.insert_document(path, DocPayload::Ttpf(Arc::new(ttpf)), None, None);
    let (_, meshes) = drawn(&mut state, dir.path(), drawable_keyed(key));
    let name = meshes[0].texture_name.clone().expect("the key is bound");
    assert_eq!(name, format!("hash_{key:016X}"));
    let found = super::texture_resolution::resolve(&state, None, None, &name.to_ascii_lowercase())
        .expect("the pack's texture resolves, whatever the case of its hash name");
    assert_eq!(found.origin.label, "textures.ttpf");

    let unknown = drawable_keyed(key ^ 1);
    let materials = super::gta6_resource::material_rows(&state, 0, &unknown);
    let bound = &materials[0].1[1].1;
    assert!(bound.contains("unresolved"), "{bound}");
}

#[test]
fn a_texture_replaced_in_an_open_pack_is_drawn_on_the_gta_vi_drawable_that_keys_it() {
    const BLUE: [u8; 4] = [0, 0, 0xFF, 0xFF];
    let key = 0x0BAD_F00D_1234_5678u64;
    let mut bytes = pack(0xF800);
    let keys = 0x100 + 16 + 16 * 2;
    bytes[keys..keys + 8].copy_from_slice(&key.to_le_bytes());
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let pack_doc = open(&mut state, written(dir.path(), &bytes));
    let (doc, meshes) = drawn(&mut state, dir.path(), drawable_keyed(key));
    let name = meshes[0].texture_name.clone().expect("the key is bound");
    let before = crate::render::viewport::MeshCacheKey::of(&state, doc, 0);

    let image = or_io::codec::texture::RgbaImage {
        width: SIDE.0,
        height: SIDE.1,
        data: BLUE.repeat((SIDE.0 * SIDE.1) as usize),
        is_srgb: false,
    };
    let options = or_io::authoring::TextureBuildOptions::default();
    let choice = or_io::authoring::FormatChoice::Automatic;
    (state.import_texture_over(
        pack_doc,
        TextureSource::Document,
        &name,
        &image,
        choice,
        &options,
    ))
    .expect("the pack's texture is replaced");
    assert_ne!(
        crate::render::viewport::MeshCacheKey::of(&state, doc, 0),
        before,
        "the drawable's meshes are built again, so its textures are bound again"
    );
    let found = super::texture_resolution::resolve(&state, None, None, &name.to_ascii_lowercase())
        .expect("the pack's texture still resolves");
    let decoded = decode_level(&found.texture, 0).expect("decodes");
    assert!(
        decoded.data.chunks(4).all(|pixel| pixel == BLUE),
        "the drawable is drawn with the replacement"
    );
}

#[test]
fn a_pack_s_key_is_named_by_the_texture_rules_from_a_drawable_opened_before_or_after_it() {
    use or_io::gta6::headerless::Drawable;
    let hash = |name: &str| name_hash64(name.as_bytes());
    let keyed = |at: [u64; 2]| {
        let mut bytes = pack(0xF800);
        let keys = 0x100 + 16 + 16 * 2;
        for (i, key) in at.iter().enumerate() {
            bytes[keys + 8 * i..keys + 8 * i + 8].copy_from_slice(&key.to_le_bytes());
        }
        bytes
    };
    let named = |state: &AppState, doc: DocumentId, index: usize| match &state
        .document(doc)
        .expect("open")
        .payload
    {
        DocPayload::Ttpf(pack) => pack.name(index),
        _ => panic!("not a pack"),
    };
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let path = written(
        dir.path(),
        &keyed([hash("zq_lamp_ab"), hash("zq_lamp_glowing")]),
    );
    let ttpf = super::ttpf_document::TtpfDocument::open(&path).expect("the pack opens");
    let first = state.insert_document(path, DocPayload::Ttpf(Arc::new(ttpf)), None, None);

    // The drawable's name names a key of the open pack and the key it binds itself.
    let own = hash("zq_lamp_slod_nm");
    let drawable = Drawable {
        name: "zq_lamp.#dr".to_string(),
        ..drawable_keyed(own)
    };
    let (_, meshes) = drawn(&mut state, dir.path(), drawable);
    assert_eq!(meshes[0].texture_name.as_deref(), Some("zq_lamp_slod_nm"));
    assert_eq!(named(&state, first, 0), "zq_lamp_ab");
    let unnamed = hash("zq_lamp_glowing");
    assert_eq!(
        named(&state, first, 1),
        format!("hash_{unnamed:016X}"),
        "no rule gives that name"
    );

    // A pack opened after the drawable has its keys named from the drawable's name.
    let later = dir.path().join("later.ttpf");
    std::fs::write(
        &later,
        keyed([hash("zq_lamp_lod_ma"), hash("zq_lantern_ab")]),
    )
    .expect("written");
    let ttpf = super::ttpf_document::TtpfDocument::open(&later).expect("the pack opens");
    let second = state.insert_document(later, DocPayload::Ttpf(Arc::new(ttpf)), None, None);
    // The rules run on a worker the job list shows, not on the frame that opens the pack.
    assert_eq!(state.texture_naming.len(), 1);
    named_by_the_rules(&mut state);
    let ended = (state.jobs.ended().iter()).find(|j| j.kind == super::jobs::JobKind::TextureNames);
    assert!(
        matches!(
            ended.map(|j| &j.outcome),
            Some(super::jobs::Outcome::Done(_))
        ),
        "{:?}",
        state.jobs.ended()
    );
    assert_eq!(named(&state, second, 0), "zq_lamp_lod_ma");
    assert!(named(&state, second, 1).starts_with("hash_"));
}

#[test]
fn a_drawable_s_texture_named_by_rule_resolves_to_the_slice_a_pack_s_cube_shows() {
    let key = name_hash64(b"zq_lamp_ab");
    let mut bytes = cube();
    let keys = 0x100 + 16 + 16;
    bytes[keys..keys + 8].copy_from_slice(&key.to_le_bytes());
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = written(dir.path(), &bytes);
    let ttpf = super::ttpf_document::TtpfDocument::open(&path).expect("the pack opens");
    let mut state = AppState::new();
    let pack = state.insert_document(path, DocPayload::Ttpf(Arc::new(ttpf)), None, None);

    let drawable = or_io::gta6::headerless::Drawable {
        name: "zq_lamp.#dr".to_string(),
        ..drawable_keyed(key)
    };
    let (_, meshes) = drawn(&mut state, dir.path(), drawable);
    assert_eq!(meshes[0].texture_name.as_deref(), Some("zq_lamp_ab"));
    let face = |state: &AppState| {
        let found = super::texture_resolution::resolve(state, None, None, "zq_lamp_ab")
            .expect("the pack's cube resolves");
        assert_eq!(found.origin.document, pack);
        let texture = found.texture.into_owned();
        let bytes = or_io::codec::texture::TextureSource::payload(&texture).to_vec();
        assert!(bytes.iter().all(|&b| b == bytes[0]), "one face");
        bytes[0]
    };
    assert_eq!(face(&state), 0x50);
    let DocPayload::Ttpf(ttpf) = &state.document(pack).expect("open").payload else {
        panic!("a .ttpf payload");
    };
    ttpf.show_slice(0, 4);
    assert_eq!(face(&state), 0x54, "the face the picker chose");
}
