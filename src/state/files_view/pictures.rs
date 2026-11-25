//! A picture of a row for the Files view, read and decoded off the UI thread:
//! a texture's image, an image file's, or a model's geometry to render.

use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use or_io::codec::texture::{decode_level, ChannelView, RgbaImage};
use or_io::container::rpf::BodyForm;
use or_io::container::RpfCryptoProvider;

use super::super::document::{DocumentId, RpfNodeId};
use super::super::document_io::parse_payload;
use super::super::extract_job::{plan, ExtractItem, TypeFilter};
use super::super::rpf_session::RpfEditSession;
use super::super::textures::{self, TextureListing};
use super::super::{DocPayload, DocumentKind};

/// What a row's picture is made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureKind {
    /// The first texture of a dictionary or a `.dds`.
    Texture,
    /// A picture file the `image` crate reads.
    Image,
    /// A drawable's geometry, which the thumbnail renderer draws.
    Model,
}

pub fn picture_kind(name: &str) -> Option<PictureKind> {
    match DocumentKind::from_path(Path::new(name)) {
        Some(DocumentKind::Ytd | DocumentKind::Dds) => Some(PictureKind::Texture),
        Some(DocumentKind::Ydr | DocumentKind::Yft | DocumentKind::Ydd) => Some(PictureKind::Model),
        _ => {
            let ext = super::super::extract_job::extension_of(name);
            super::super::entry_preview::IMAGE_EXTENSIONS
                .contains(&ext.as_str())
                .then_some(PictureKind::Image)
        }
    }
}

/// A row's picture family, with the RPF9 table taking precedence over a filename.
pub fn picture_kind_for(node: &super::super::RpfSessionNode) -> Option<PictureKind> {
    if let Some(kind) = node.rpf9_type() {
        return (kind == or_io::container::rpf::Gta6EntryType::TextureDictionary)
            .then_some(PictureKind::Texture);
    }
    picture_kind(&node.name)
}

/// Entries larger than this are shown by their glyph: the whole file is read to make one.
pub const PICTURE_LIMIT: u64 = 32 * 1024 * 1024;

/// How many pictures are made at once, and how many may wait.
const RUNNING: usize = 2;
const WAITING: usize = 96;

/// One picture: a row, the longest side it is made for, and the row's size, so an edit makes it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PictureKey {
    pub doc: DocumentId,
    pub node: RpfNodeId,
    pub side: u32,
    pub size: u64,
}

pub enum Made {
    Image(RgbaImage),
    /// ⚠ Its geometry is built on the UI thread, whose state the builders read.
    Model {
        payload: Box<DocPayload>,
        kind: DocumentKind,
    },
    Failed(String),
}

/// Row `name`'s picture out of its bytes, at most `side` pixels on its longer side.
pub fn make(
    kind: PictureKind,
    name: &str,
    bytes: &[u8],
    side: u32,
    crypto: Arc<dyn RpfCryptoProvider>,
) -> Made {
    match kind {
        PictureKind::Image => match image::load_from_memory(bytes) {
            Ok(img) => {
                let rgba = img.thumbnail(side, side).to_rgba8();
                Made::Image(RgbaImage {
                    width: rgba.width(),
                    height: rgba.height(),
                    data: rgba.into_raw(),
                    is_srgb: true,
                })
            }
            Err(e) => Made::Failed(tr!("Not a picture: {e}", e = e)),
        },
        PictureKind::Texture | PictureKind::Model => {
            let Some(doc_kind) = DocumentKind::from_path(Path::new(name)) else {
                return Made::Failed(tr!("No reader names it").to_string());
            };
            let doc_kind = doc_kind.refine_with_bytes(bytes);
            let payload = match parse_payload(Path::new(name), doc_kind, bytes, crypto) {
                Ok(payload) => payload,
                Err(e) => return Made::Failed(e),
            };
            if kind == PictureKind::Model {
                return Made::Model {
                    payload: Box::new(payload),
                    kind: doc_kind,
                };
            }
            first_texture(&payload, name, side)
        }
    }
}

fn first_texture(payload: &DocPayload, name: &str, side: u32) -> Made {
    let TextureListing::Textures {
        textures, source, ..
    } = textures::textures_in(payload, None)
    else {
        return Made::Failed(tr!("It holds no texture").to_string());
    };
    let Some(texture) = textures
        .first()
        .and_then(|first| textures::texture_at(payload, source, &first.name))
    else {
        return Made::Failed(tr!("It holds no texture").to_string());
    };
    if !texture.has_pixel_data() || texture.width() == 0 || texture.height() == 0 {
        return Made::Failed(tr!("Its texture has no pixels").to_string());
    }
    let level = super::super::texture_cache::fitting_level(&texture, side);
    match decode_level(&texture, level) {
        Ok(rgba) => {
            let view = ChannelView::preview_default(texture.format());
            match or_io::authoring::fit_within(rgba.with_channel_view(view), side, name) {
                Some(fitted) => Made::Image(fitted),
                None => Made::Failed(tr!("It could not be scaled down").to_string()),
            }
        }
        Err(e) => Made::Failed(tr!("Its texture did not decode: {e}", e = e)),
    }
}

struct Job {
    key: PictureKey,
    kind: PictureKind,
    name: String,
    source: Source,
    crypto: Arc<dyn RpfCryptoProvider>,
}

enum Source {
    Entry(ExtractItem),
    Rpf9(Box<super::super::document::rpf9::entry::EntrySource>),
}

/// The pictures being made, newest asked first.
pub struct Pictures {
    tx: Sender<(PictureKey, Made)>,
    /// ⚠ Behind a lock only because a panel's state is shared across threads; one thread reads it.
    rx: Mutex<Receiver<(PictureKey, Made)>>,
    waiting: VecDeque<Job>,
    asked: HashSet<PictureKey>,
    running: usize,
}

impl Default for Pictures {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx: Mutex::new(rx),
            waiting: VecDeque::new(),
            asked: HashSet::new(),
            running: 0,
        }
    }
}

impl Pictures {
    /// Ask for a row's picture; `false` when it has none to make.
    pub fn ask(
        &mut self,
        session: &RpfEditSession,
        key: PictureKey,
        crypto: Arc<dyn RpfCryptoProvider>,
    ) -> bool {
        if self.asked.contains(&key) {
            return true;
        }
        let Some(node) = session.node(key.node).filter(|n| !n.is_dir()) else {
            return false;
        };
        let Some(kind) = picture_kind_for(node).filter(|_| key.size <= PICTURE_LIMIT) else {
            return false;
        };
        let source = match &node.backing {
            super::super::RpfNodeBacking::Rpf9File { table, base, index } => {
                let Ok(source) = table.entry(*base, *index) else {
                    return false;
                };
                Source::Rpf9(Box::new(source))
            }
            _ => {
                let Some(item) = plan(session, key.node, &TypeFilter::All).into_iter().next()
                else {
                    return false;
                };
                Source::Entry(item)
            }
        };
        // ⚠ The rows asked for last are the ones on screen; the oldest asked
        // were scrolled past, and are asked again if they come back.
        if self.waiting.len() >= WAITING {
            if let Some(dropped) = self.waiting.pop_front() {
                self.asked.remove(&dropped.key);
            }
        }
        self.waiting.push_back(Job {
            key,
            kind,
            name: node.name.clone(),
            source,
            crypto,
        });
        self.asked.insert(key);
        self.start();
        true
    }

    /// The pictures finished since the last call.
    pub fn finished(&mut self) -> Vec<(PictureKey, Made)> {
        let done: Vec<(PictureKey, Made)> = match self.rx.lock() {
            Ok(rx) => rx.try_iter().collect(),
            Err(_) => Vec::new(),
        };
        for (key, _) in &done {
            self.asked.remove(key);
            self.running = self.running.saturating_sub(1);
        }
        self.start();
        done
    }

    fn start(&mut self) {
        while self.running < RUNNING {
            let Some(job) = self.waiting.pop_back() else {
                return;
            };
            self.running += 1;
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let made = match job.source {
                    Source::Rpf9(source) => match source.payload_limited(PICTURE_LIMIT) {
                        Ok(payload) => first_texture(&payload, &job.name, job.key.side),
                        Err(error) => Made::Failed(error),
                    },
                    Source::Entry(item) => match item.read(BodyForm::Decoded) {
                        Ok(bytes) => make(job.kind, &job.name, &bytes, job.key.side, job.crypto),
                        Err(e) => Made::Failed(tr!("Its bytes could not be read: {e}", e = e)),
                    },
                };
                let _ = tx.send((job.key, made));
            });
        }
    }
}
