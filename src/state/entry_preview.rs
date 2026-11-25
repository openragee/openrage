//! What the preview pane says about the selected archive row, without opening
//! it as a document.

use std::sync::Arc;

use super::document::{DocumentId, RpfNodeId};
use super::files_view::pictures::{PictureKind, PICTURE_LIMIT};
use super::rpf_session::RpfEditSession;
use super::{AppState, DocPayload, RpfNodeBacking};

/// Entries larger than this are described but not read.
///
/// ⚠ **The read is on the UI thread**, because a preview follows the
/// selection frame by frame; 4 MiB inflates in a few milliseconds and keeps
/// arrowing through a folder of textures responsive, where a 200 MB `.rpf`
/// would freeze the window for the length of a decompress.
pub const READ_LIMIT: u64 = 4 * 1024 * 1024;

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
    /// Its contents are read on a worker only when explicitly opened.
    Open,
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

/// Offset, sixteen bytes and their printable characters, per line.
pub fn hex_lines(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(i, row)| hex_line(i * 16, row))
        .collect()
}

/// One line of [`hex_lines`]: `row`'s bytes, at `offset` into the file.
pub fn hex_line(offset: usize, row: &[u8]) -> String {
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
    format!("{offset:08X}  {:<47}  {text}", hex.join(" "))
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
        RpfNodeBacking::ArchiveFile { .. } | RpfNodeBacking::Rpf9File { .. } => {
            "Inside an archive".to_string()
        }
        RpfNodeBacking::Archive { .. } | RpfNodeBacking::Rpf9Archive { .. } => {
            "An archive".to_string()
        }
        _ => String::new(),
    }
}

/// Describe one row of an archive document, reading it if it is small enough.
pub fn preview(session: &RpfEditSession, id: RpfNodeId) -> Option<EntryPreview> {
    let node = session.node(id)?;
    let path = session
        .path_of(id)
        .map(|p| format!("/{p}"))
        .unwrap_or_default();
    let size = (!node.is_dir()).then(|| session.size_for(id)).flatten();
    let kind = super::files_view::type_in_words(node);
    let drawn = matches!(
        super::files_view::pictures::picture_kind_for(node),
        Some(PictureKind::Texture | PictureKind::Model)
    );
    let body = if node.is_dir() {
        folder_body(session, id)
    } else if drawn && size.is_some_and(|s| s <= PICTURE_LIMIT) {
        PreviewBody::Picture
    } else if session.is_rpf9(id) {
        PreviewBody::Open
    } else if size.is_some_and(|s| s > READ_LIMIT) {
        PreviewBody::TooLarge
    } else {
        match session.bytes_for(id) {
            Some(bytes) => classify(&node.name, &bytes),
            None => PreviewBody::Unreadable,
        }
    };
    Some(EntryPreview {
        name: node.name.clone(),
        path,
        size,
        kind,
        stored_in: stored_in(&node.backing),
        body,
    })
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
mod tests {
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
        let lines = hex_lines(b"RPF7\x00\x01abcdefghijklmnop");
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("00000000  52 50 46 37 00 01 61"));
        assert!(lines[0].ends_with("RPF7..abcdefghij"));
        assert!(lines[1].starts_with("00000010  6B"));
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
        let p = preview(&session, root).unwrap();
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
        let file = preview(&session, session.children_of(models)[0]).unwrap();
        assert_eq!(file.path, "/models/a.ydr");
        assert_eq!(file.size, Some(100));
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
                preview(&rpf.session, ydr).expect("a preview").body,
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
