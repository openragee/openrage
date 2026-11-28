//! What the preview pane says about the selected archive row, without opening
//! it as a document.

use std::sync::Arc;

use or_io::container::rpf::BodyForm;

use super::document::{DocumentId, RpfNodeId};
use super::extract_job::{plan, TypeFilter};
use super::files_view::pictures::{PictureKind, PICTURE_LIMIT};
use super::rpf_session::RpfEditSession;
use super::{AppState, DocPayload, RpfNodeBacking};

/// Entries larger than this are described but not read.
///
/// The read is on a worker, but a preview follows the selection, so arrowing
/// through a folder starts one read per row: 4 MiB inflates in a few
/// milliseconds, where a 200 MB `.rpf` would hold the worker and the drive.
pub const READ_LIMIT: u64 = 4 * 1024 * 1024;

/// The read of a row's bytes, which the pane makes on a worker; `None` is a read that failed.
pub type BodyRead = Box<dyn FnOnce() -> Option<Vec<u8>> + Send>;

const TEXT_CHARS: usize = 16 * 1024;
const HEX_BYTES: usize = 512;
const IMAGE_SIDE: u32 = 4096;

const TEXT_EXTENSIONS: &[&str] = &[
    "xml", "meta", "txt", "dat", "ini", "lua", "json", "cfg", "csv", "log", "ide", "ipl", "gxt",
    "psc", "sps", "md",
];
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp", "tga"];

#[derive(Clone)]
pub enum PreviewBody {
    Folder {
        files: usize,
        folders: usize,
        bytes: u64,
        census: Vec<(String, usize)>,
    },
    Text {
        text: String,
        truncated: bool,
    },
    Image {
        width: u32,
        height: u32,
        rgba: Arc<Vec<u8>>,
    },
    Hex {
        head: Vec<u8>,
        total: usize,
    },
    /// A texture or a model, which the pane draws as its picture, made off the UI thread.
    Picture,
    /// Its bytes are being read on a worker.
    Reading,
    TooLarge,
    Unreadable,
}

#[derive(Clone)]
pub struct EntryPreview {
    pub name: String,
    pub path: String,
    pub size: Option<u64>,
    pub kind: String,
    pub stored_in: String,
    pub body: PreviewBody,
}

/// How a file's bytes are best shown: as text, as a picture, or as bytes.
///
/// ⚠ **The extension proposes and the bytes decide.** A `.dat` holding a
/// binary table is not text however it is named, so a NUL in the first few
/// kilobytes sends it to the hex view; and an image that will not decode is
/// shown as the bytes it is rather than as nothing.
pub fn classify(name: &str, bytes: &[u8]) -> PreviewBody {
    let ext = super::extract_job::extension_of(name);
    if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        if let Ok(img) = image::load_from_memory(bytes) {
            if img.width() <= IMAGE_SIDE && img.height() <= IMAGE_SIDE {
                let rgba = img.to_rgba8();
                return PreviewBody::Image {
                    width: rgba.width(),
                    height: rgba.height(),
                    rgba: Arc::new(rgba.into_raw()),
                };
            }
        }
    }
    let sniff = &bytes[..bytes.len().min(4096)];
    let looks_textual = !sniff.contains(&0) && std::str::from_utf8(sniff).is_ok()
        || std::str::from_utf8(sniff).is_err_and(|e| e.error_len().is_none());
    if looks_textual && (TEXT_EXTENSIONS.contains(&ext.as_str()) || !sniff.is_empty()) {
        let text = String::from_utf8_lossy(bytes);
        let truncated = text.chars().count() > TEXT_CHARS;
        return PreviewBody::Text {
            text: text.chars().take(TEXT_CHARS).collect(),
            truncated,
        };
    }
    PreviewBody::Hex {
        head: bytes[..bytes.len().min(HEX_BYTES)].to_vec(),
        total: bytes.len(),
    }
}

/// Offset, `per_line` bytes and their printable characters, per line.
pub fn hex_lines(bytes: &[u8], per_line: usize) -> Vec<String> {
    let per_line = per_line.max(1);
    bytes
        .chunks(per_line)
        .enumerate()
        .map(|(i, row)| line(i * per_line, row, per_line))
        .collect()
}

/// The most of 16, 8 or 4 bytes whose line fits `width` at `glyph` per character; 4 when none does.
pub fn bytes_per_line(width: f32, glyph: f32) -> usize {
    let chars = if glyph > 0.0 { width / glyph } else { 0.0 };
    // The offset, two spaces, each byte's two digits and a space, two spaces and its character.
    let fits = |n: usize| (8 + 2 + 3 * n - 1 + 2 + n) as f32 <= chars;
    [16, 8].into_iter().find(|&n| fits(n)).unwrap_or(4)
}

/// One line of sixteen bytes: `row`'s bytes, at `offset` into the file.
pub fn hex_line(offset: usize, row: &[u8]) -> String {
    line(offset, row, 16)
}

fn line(offset: usize, row: &[u8], per_line: usize) -> String {
    let hex: Vec<String> = row.iter().map(|b| format!("{b:02X}")).collect();
    let text: String = row
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '.'
            }
        })
        .collect();
    let width = 3 * per_line - 1;
    format!("{offset:08X}  {:<width$}  {text}", hex.join(" "))
}

fn folder_body(session: &RpfEditSession, id: RpfNodeId) -> PreviewBody {
    let (mut files, mut folders, mut bytes) = (0, 0, 0u64);
    let mut stack = session.children_of(id);
    while let Some(at) = stack.pop() {
        let Some(node) = session.node(at) else {
            continue;
        };
        if node.is_dir() {
            folders += 1;
            stack.extend(session.children_of(at));
        } else {
            files += 1;
            bytes += session.size_for(at).unwrap_or(0);
        }
    }
    let mut census = super::extract_job::extension_census(session, id);
    census.truncate(6);
    PreviewBody::Folder {
        files,
        folders,
        bytes,
        census,
    }
}

fn stored_in(backing: &RpfNodeBacking) -> String {
    match backing {
        RpfNodeBacking::InMemoryFile { .. } => "Added in this session, not yet saved".to_string(),
        RpfNodeBacking::DiskFile { fs_path, .. } | RpfNodeBacking::DiskFolder { fs_path, .. } => {
            format!("On disk: {}", fs_path.display())
        }
        RpfNodeBacking::ArchiveFile { .. } => "Inside an archive".to_string(),
        RpfNodeBacking::Archive { .. } => "An archive".to_string(),
        _ => String::new(),
    }
}

/// Describe one row of an archive document from its tree, with the read its body waits on when it is small enough to show.
pub fn preview(
    session: &RpfEditSession,
    id: RpfNodeId,
) -> Option<(EntryPreview, Option<BodyRead>)> {
    let node = session.node(id)?;
    let path = session
        .path_of(id)
        .map(|p| format!("/{p}"))
        .unwrap_or_default();
    let held = (!node.is_dir()).then(|| session.size_for(id)).flatten();
    // A headerless row holding its loose form is listed by the sections it inflates to, as its Size column is.
    let size = held.and_then(|_| super::files_view::measured_bytes(session, id));
    let drawn = matches!(
        super::files_view::pictures::picture_kind(&node.name),
        Some(PictureKind::Texture | PictureKind::Model)
    );
    let mut read: Option<BodyRead> = None;
    let body = if node.is_dir() {
        folder_body(session, id)
    } else if drawn && held.is_some_and(|s| s <= PICTURE_LIMIT) {
        PreviewBody::Picture
    } else if held.is_some_and(|s| s > READ_LIMIT) {
        PreviewBody::TooLarge
    } else {
        match plan(session, id, &TypeFilter::All).into_iter().next() {
            Some(item) => {
                read = Some(Box::new(move || {
                    (item.read(BodyForm::Decoded))
                        .map_err(
                            |e| tracing::warn!(node_id = id, error = %e, "preview read failed"),
                        )
                        .ok()
                }));
                PreviewBody::Reading
            }
            None => PreviewBody::Unreadable,
        }
    };
    let preview = EntryPreview {
        name: node.name.clone(),
        path,
        size,
        kind: super::files_view::kind_in_words(session, node),
        stored_in: stored_in(&node.backing),
        body,
    };
    Some((preview, read))
}

/// `preview` with the body `bytes` show and the kind their head names; `None` is a read that failed.
pub fn with_bytes(mut preview: EntryPreview, bytes: Option<&[u8]>) -> EntryPreview {
    preview.body = match bytes {
        Some(bytes) => {
            preview.kind =
                super::files_view::kind_by_head(std::mem::take(&mut preview.kind), bytes);
            classify(&preview.name, bytes)
        }
        None => PreviewBody::Unreadable,
    };
    preview
}

/// What holds row `id`'s bytes, by address, and its entry: bytes put into it, or a saved archive under it, change it.
pub fn backing_revision(session: &RpfEditSession, id: RpfNodeId) -> (usize, u32) {
    let Some(node) = session.node(id) else {
        return (0, 0);
    };
    match &node.backing {
        RpfNodeBacking::InMemoryFile { bytes }
        | RpfNodeBacking::DiskFile {
            override_bytes: Some(bytes),
            ..
        } => (Arc::as_ptr(bytes).addr(), 0),
        RpfNodeBacking::ArchiveFile {
            override_bytes: Some(bytes),
            entry_index,
            ..
        } => (Arc::as_ptr(bytes).addr(), *entry_index),
        RpfNodeBacking::ArchiveFile {
            archive_id,
            entry_index,
            ..
        } => (
            (session.archive(*archive_id)).map_or(0, |slot| Arc::as_ptr(&slot.archive).addr()),
            *entry_index,
        ),
        _ => (0, 0),
    }
}

impl AppState {
    /// The archive row the preview pane follows: the focus of the active
    /// archive, or of the archive the active document was opened out of.
    pub fn preview_target(&self) -> Option<(DocumentId, RpfNodeId)> {
        let active = self.active_document?;
        let doc = self.document(active)?;
        let archive = match (&doc.payload, doc.rpf_entry_binding.as_ref()) {
            (DocPayload::Rpf(_), _) => active,
            (_, Some(binding)) => binding.parent_doc_id,
            _ => return None,
        };
        let node = self.selection.selected(archive)?;
        match &self.document(archive)?.payload {
            DocPayload::Rpf(rpf) if rpf.session.node(node).is_some() => Some((archive, node)),
            _ => None,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn text_is_text_and_a_nul_makes_it_bytes() {
        assert!(matches!(
            classify("a.meta", b"<CMapTypes>\n</CMapTypes>"),
            PreviewBody::Text {
                truncated: false,
                ..
            }
        ));
        assert!(matches!(
            classify("table.dat", b"RSC7\0\0\x01\x02"),
            PreviewBody::Hex { total: 8, .. }
        ));
        let long = "x".repeat(TEXT_CHARS + 10);
        match classify("log.txt", long.as_bytes()) {
            PreviewBody::Text { text, truncated } => {
                assert!(truncated);
                assert_eq!(text.chars().count(), TEXT_CHARS);
            }
            _ => panic!("a long text file is still text"),
        }
    }

    #[test]
    fn an_image_decodes_and_a_broken_one_is_shown_as_bytes() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        match classify("icon.png", &png) {
            PreviewBody::Image {
                width,
                height,
                rgba,
            } => {
                assert_eq!((width, height), (3, 2));
                assert_eq!(&rgba[..4], &[10, 20, 30, 255]);
            }
            _ => panic!("a PNG previews as a picture"),
        }
        assert!(matches!(
            classify("icon.png", &png[..20]),
            PreviewBody::Hex { .. }
        ));
    }

    #[test]
    fn a_hex_line_is_offset_bytes_and_printables() {
        let lines = hex_lines(b"RPF7\x00\x01abcdefghijklmnop", 16);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("00000000  52 50 46 37 00 01 61"));
        assert!(lines[0].ends_with("RPF7..abcdefghij"));
        assert!(lines[1].starts_with("00000010  6B"));
    }

    #[test]
    fn a_narrow_pane_takes_fewer_bytes_per_line_and_every_line_is_as_wide() {
        // 16 bytes take 75 characters, 8 take 43 and 4 take 27.
        assert_eq!(bytes_per_line(75.0 * 7.0, 7.0), 16);
        assert_eq!(bytes_per_line(74.0 * 7.0, 7.0), 8);
        assert_eq!(bytes_per_line(290.0, 6.0), 8);
        assert_eq!(bytes_per_line(42.0 * 7.0, 7.0), 4);
        assert_eq!(bytes_per_line(10.0, 0.0), 4);
        let lines = hex_lines(&[0x41; 20], 8);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].len(), 43);
        assert_eq!(
            lines[2].len(),
            lines[0].len() - 4,
            "a short line pads its hex column"
        );
        assert!(lines[1].starts_with("00000008  41"));
        assert_eq!(hex_lines(&[0x41; 16], 16)[0].len(), 75);
    }

    #[test]
    fn a_folder_counts_what_is_under_it() {
        let mut session = RpfEditSession::empty();
        let root = session.root_id();
        let models = session.add_folder(root, "models").unwrap();
        session
            .add_file_copy(models, "a.ydr", vec![0; 100])
            .unwrap();
        session.add_file_copy(models, "b.ydr", vec![0; 28]).unwrap();
        session.add_file_copy(root, "c.ytd", vec![0; 2]).unwrap();
        let (p, read) = preview(&session, root).unwrap();
        assert!(read.is_none(), "a folder is described from its tree");
        match p.body {
            PreviewBody::Folder {
                files,
                folders,
                bytes,
                census,
            } => {
                assert_eq!((files, folders, bytes), (3, 1, 130));
                assert_eq!(census[0], ("ydr".to_string(), 2));
            }
            _ => panic!("a folder previews as a folder"),
        }
        let (file, _) = preview(&session, session.children_of(models)[0]).unwrap();
        assert_eq!(file.path, "/models/a.ydr");
        assert_eq!(file.size, Some(100));
    }

    #[test]
    fn a_file_is_read_by_the_read_it_hands_back_and_a_failed_read_is_unreadable() {
        let mut session = RpfEditSession::empty();
        let root = session.root_id();
        let id = session
            .add_file_copy(root, "a.meta", b"<Item />".to_vec())
            .unwrap();
        let (head, read) = preview(&session, id).unwrap();
        assert!(matches!(head.body, PreviewBody::Reading));
        let bytes = read.expect("a small file waits on a read")();
        let read = with_bytes(head.clone(), bytes.as_deref());
        match read.body {
            PreviewBody::Text { text, .. } => assert_eq!(text, "<Item />"),
            _ => panic!("its bytes are text"),
        }
        assert!(matches!(
            with_bytes(head, None).body,
            PreviewBody::Unreadable
        ));
    }

    #[test]
    fn new_bytes_of_the_same_length_change_the_backing_revision() {
        let mut session = RpfEditSession::empty();
        let root = session.root_id();
        let id = session
            .add_file_copy(root, "a.txt", b"one".to_vec())
            .unwrap();
        let before = backing_revision(&session, id);
        session.set_bytes_for(id, b"two".to_vec()).unwrap();
        assert_ne!(backing_revision(&session, id), before);
    }

    /// A headerless GTA VI row holding its loose form previews at the size of the sections it inflates to.
    #[test]
    fn a_headerless_override_previews_at_its_section_size() {
        let _serial = crate::ui::theme::serial();
        let (session, model, _held) = headerless_override();
        // The sections are measured on a worker, never on the drawing thread.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let shown = loop {
            let (shown, _) = preview(&session, model).expect("a preview");
            if shown.size.is_some() || std::time::Instant::now() > deadline {
                break shown;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(shown.size, Some(0x200));
    }

    /// What `headerless_override` keeps alive: the archive's folder and the context it was read under.
    pub(crate) struct Held {
        given: Option<Arc<or_io::container::rpf::Gta6Context>>,
        _dir: tempfile::TempDir,
    }
    impl Drop for Held {
        fn drop(&mut self) {
            or_io::container::rpf::Gta6Context::swap_given(self.given.take());
        }
    }

    /// A synthetic GTA VI archive's headerless row `model.ydr` holding a loose form shorter than its 0x200 bytes of sections.
    pub(crate) fn headerless_override() -> (RpfEditSession, RpfNodeId, Held) {
        use std::io::Cursor;

        use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
        use or_io::container::rpf::{
            write_rpf9_resource, Gta6Context, NameResolver, Rpf9Form, Rpf9Sections,
        };
        use or_io::container::{NoOpCryptoProvider, RpfArchive};
        use or_io::gta6::hash::name_hash64;

        use crate::state::rpf_archive_open::tree_verbs::tests::{rbm_section, synthetic_keys};

        let loose = |virtual_section: Vec<u8>| {
            write_rpf9_resource(&Rpf9Sections {
                form: Rpf9Form::BlockMapped,
                virtual_section,
                physical: Vec::new(),
            })
            .expect("a loose resource")
        };

        let mut names = NameResolver::new();
        names.insert(name_hash64(b"data/model.ydr"), "data/model.ydr");
        let context = Gta6Context {
            keys: synthetic_keys().expect("the synthetic snapshot parses"),
            names: Arc::new(names),
        };
        let given = Gta6Context::swap_given(Some(Arc::new(context)));
        let entries = [(
            "data/model.ydr".to_owned(),
            Rpf9NewBody::Resource {
                type_code: 0x02,
                loose: loose(rbm_section()),
            },
        )];
        let key = Rpf9IndexKey {
            key_id: 0xDEAD_BEEF,
            iv: [0x33; 16],
        };
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let plan = build(&entries, or_io::platform::Platform::XboxSeries, &key, &keys)
            .expect("the archive builds");
        let mut bytes = Vec::new();
        (plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut bytes))
            .expect("the archive is laid out");
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("source.rpf");
        std::fs::write(&path, bytes).expect("the archive is written");
        let archive = RpfArchive::open_with(&path, Arc::new(NoOpCryptoProvider::new()))
            .expect("the archive opens");
        let mut session = RpfEditSession::from_archive(archive).expect("the tree lists");
        let model = (session.all_nodes().into_iter())
            .find(|&id| session.node(id).is_some_and(|n| n.name == "model.ydr"))
            .expect("the row is listed");

        let mut grown = rbm_section();
        grown.resize(0x200, 0);
        let held = loose(grown);
        assert!(
            held.len() < 0x200,
            "the loose form is shorter than its sections"
        );
        session.set_bytes_for(model, held).expect("held");
        (session, model, Held { given, _dir: dir })
    }

    /// Selecting a drawable in the tree previews it as a picture made off the
    /// UI thread and hands the viewport nothing: no document opens, no mesh is
    /// built for the active document, and the preview scene stays empty.
    #[test]
    fn browsing_a_drawable_loads_nothing_into_the_renderer() {
        use or_io::container::rpf::RpfBuilder;

        use crate::state::navigation::{resolve_path, Location, Resolved};
        use crate::state::DocumentKind;

        let drawable = [
            or_io::container::resource::Rsc7::header_from_entry_flags(0x10, 0x20).header_bytes(),
            vec![0u8; 32],
        ]
        .concat();
        let archive = RpfBuilder::default()
            .insert_with("models/props.ydr", drawable, false)
            .expect("add")
            .build()
            .expect("opens");

        let mut state = AppState::default();
        let doc = state.new_empty_document(DocumentKind::Rpf);
        let Some(DocPayload::Rpf(rpf)) = state.document_mut(doc).map(|d| &mut d.payload) else {
            panic!("an empty RPF document holds an RPF payload");
        };
        rpf.session = RpfEditSession::from_archive(archive).expect("session");
        let Resolved::Found(ydr) = resolve_path(&rpf.session, "models/props.ydr") else {
            panic!("the drawable is listed");
        };
        state.set_active_document(doc);

        state.navigate_to(Location { doc, node: ydr });

        assert_eq!(state.preview_target(), Some((doc, ydr)));
        let Some(DocPayload::Rpf(rpf)) = state.document(doc).map(|d| &d.payload) else {
            panic!("the archive is still open");
        };
        assert!(
            matches!(
                preview(&rpf.session, ydr).expect("a preview").0.body,
                PreviewBody::Picture
            ),
            "a drawable previews as its picture"
        );
        assert!(rpf.entry_documents.is_empty(), "no entry document opened");
        assert_eq!(state.documents.len(), 1, "browsing opened a document");
        assert_eq!(state.active_document, Some(doc));
        assert!(crate::render::meshes::build_meshes_for_active_document(&state).is_empty());
        assert!(state.preview_scene.mesh_sources.is_empty());
        assert!(state.preview_scene.texture_sources.is_empty());
        assert!(state.preview_scene.skeleton_source.is_none());
    }
}
