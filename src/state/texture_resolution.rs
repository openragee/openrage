//! Which open dictionary supplies a material's texture, and why.
//!
//! A drawable names its textures by string and carries none of them, and
//! nothing in the format points from a drawable to a dictionary. So [`resolve`]
//! ranks every open dictionary holding the name by authoring convention, a
//! per-document pin overrides the ranking, and [`TextureResolutionReport`] names
//! which document answered, by which rule, and every other one that held it.
//!
//! ⚠ A wrong dictionary looks like a correct result — plausible textures on the
//! wrong garment — so a name held by two open dictionaries is reported rather
//! than silently picked.
//!
//! ⚠ **The ranking is a convention, not a measurement**: the tests pin that each
//! rule outranks the next, which is a claim about this code, not about shipped
//! content.
//!
//! ⚠ Falsified against [`Candidate::sort_key`]: reduced to the MRU term alone it
//! failed 4 of the 10 tests then in the suite, and first-opened in place of
//! most-recently-opened failed 1 of 10.
//! `a_same_stem_dictionary_beats_the_first_one_opened` opens its winner in the
//! *middle* because, with the same-stem dictionary opened last, it passed under
//! a resolver with no stem rule.

#[path = "texture_resolution_lookup.rs"]
mod lookup;
use lookup::{fetch, holds};

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::path::Path;

use or_io::gta5::data::asset_name::AssetName;

use super::document::{DocPayload, Document, DocumentId};
use super::preview_scene::MeshSource;
use super::source_textures::texture_embedded_in;
use super::textures;
use super::{AppState, DocumentKind};
use crate::ui::dock::Workspace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResolutionRule {
    Embedded,

    /// The variant a garment is drawn with, whatever name its material states.
    Clothing,

    Pinned,

    /// A dictionary the user attached by hand, from disk or out of an archive.
    ///
    /// ⚠ Above every name heuristic below it and beneath [`Self::Pinned`],
    /// because the two say different things. An attach is session-wide — *also
    /// consult this* — where a pin names one dictionary for one document, so a
    /// pin has to survive attaching something else.
    Attached,

    SameStem,

    /// The active document's `_hi` / `+hi` companion, or its base.
    DetailVariant,

    SameFolder,

    SameArchive,

    AnyOpen,
}

impl ResolutionRule {
    pub fn label(self) -> &'static str {
        match self {
            Self::Embedded => "embedded",
            Self::Clothing => "clothing texture",
            Self::Pinned => "pinned",
            Self::Attached => "attached",
            Self::SameStem => "name match",
            Self::DetailVariant => "detail pair (_hi / +hi)",
            Self::SameFolder => "same folder",
            Self::SameArchive => "same archive",
            Self::AnyOpen => "open dictionary",
        }
    }

    pub fn is_a_guess(self) -> bool {
        self != Self::Embedded
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureOrigin {
    pub document: DocumentId,
    pub label: String,
    pub rule: ResolutionRule,
    pub implicitly_opened: bool,
}

impl TextureOrigin {
    pub fn describe(&self) -> String {
        if self.implicitly_opened {
            // ⚠ Not "from archive": the flag also covers a dictionary found
            // through a configured search path.
            format!("{} (opened automatically)", self.label)
        } else {
            format!("{} ({})", self.label, self.rule.label())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureOutcome {
    pub name: String,
    pub found: Option<TextureOrigin>,
    pub also_in: Vec<TextureOrigin>,
}

impl TextureOutcome {
    pub fn is_ambiguous(&self) -> bool {
        !self.also_in.is_empty()
            && self
                .found
                .as_ref()
                .is_some_and(|origin| origin.rule.is_a_guess())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextureResolutionReport {
    pub document: Option<DocumentId>,
    pub outcomes: Vec<TextureOutcome>,
    pub epoch: u64,
}

impl TextureResolutionReport {
    pub fn requested(&self) -> usize {
        self.outcomes.len()
    }

    pub fn resolved(&self) -> usize {
        self.outcomes.iter().filter(|o| o.found.is_some()).count()
    }

    pub fn conflicts(&self) -> usize {
        self.outcomes.iter().filter(|o| o.is_ambiguous()).count()
    }

    pub fn missing(&self) -> Vec<&str> {
        self.outcomes
            .iter()
            .filter(|o| o.found.is_none())
            .map(|o| o.name.as_str())
            .collect()
    }

    pub fn is_clean(&self) -> bool {
        self.resolved() == self.requested() && self.conflicts() == 0
    }

    pub fn is_empty(&self) -> bool {
        self.outcomes.is_empty()
    }

    pub fn primary_source(&self) -> Option<&TextureOrigin> {
        let mut best: Option<(&TextureOrigin, usize)> = None;
        for outcome in &self.outcomes {
            let Some(origin) = outcome.found.as_ref() else {
                continue;
            };
            let count = self
                .outcomes
                .iter()
                .filter(|o| {
                    o.found
                        .as_ref()
                        .is_some_and(|other| other.document == origin.document)
                })
                .count();
            if best.is_none_or(|(_, best_count)| count > best_count) {
                best = Some((origin, count));
            }
        }
        best.map(|(origin, _)| origin)
    }

    /// `"6 of 7 · 1 conflict"`.
    pub fn summary(&self) -> String {
        let mut text = format!("{} of {}", self.resolved(), self.requested());
        let conflicts = self.conflicts();
        if conflicts > 0 {
            text.push_str(&format!(
                " \u{00B7} {conflicts} conflict{}",
                if conflicts == 1 { "" } else { "s" }
            ));
        }
        text
    }
}

pub struct Resolved<'a> {
    pub texture: textures::Texture<'a>,
    pub origin: TextureOrigin,
    pub also_in: Vec<TextureOrigin>,
}

/// The document textures are asked for: the one a preview mesh came from (`from`), else the active one.
///
/// ⚠ The Animation workspace's active document is usually a clip dictionary, and the ranking,
/// the pins and the embedded textures are those of what the preview draws.
fn subject(state: &AppState, from: Option<MeshSource>) -> Option<&Document> {
    match from {
        Some(source) => state.document(source.document),
        None => state.active_document(),
    }
}

/// The document `name` is embedded in, with the texture: the one a preview mesh came from (`from`), else the active one.
fn embedded<'a>(
    state: &'a AppState,
    from: Option<MeshSource>,
    selection: Option<u64>,
    name: &str,
) -> Option<(&'a Document, textures::Texture<'a>)> {
    let doc = subject(state, from)?;
    let texture = match from {
        Some(source) => texture_embedded_in(&doc.payload, source.entry, name)?,
        None => textures::texture_named(&doc.payload, selection, name)?,
    };
    Some((doc, texture))
}

/// The variant dictionary the garment a preview mesh came from (`from`), else the active one, is drawn with, and its first texture.
///
/// ⚠ Asked of every name a mesh states, which is its first texture's alone: nothing else of a
/// material is drawn.
fn clothing<'a>(
    state: &'a AppState,
    from: Option<MeshSource>,
) -> Option<(&'a Document, textures::Texture<'a>)> {
    let garment = from
        .map(|source| source.document)
        .or(state.active_document)?;
    let dictionary = state.document(state.garment_dictionary(garment)?)?;
    let textures::TextureListing::Textures {
        textures, source, ..
    } = textures::textures_in(&dictionary.payload, None)
    else {
        return None;
    };
    let first = textures.first()?;
    Some((
        dictionary,
        textures::texture_at(&dictionary.payload, source, &first.name)?,
    ))
}

pub fn resolve<'a>(
    state: &'a AppState,
    from: Option<MeshSource>,
    selection: Option<u64>,
    name: &str,
) -> Option<Resolved<'a>> {
    if let Some((doc, texture)) = clothing(state, from) {
        return Some(Resolved {
            texture,
            origin: TextureOrigin {
                document: doc.id,
                label: document_label(doc),
                rule: ResolutionRule::Clothing,
                implicitly_opened: false,
            },
            also_in: Vec::new(),
        });
    }
    if let Some((doc, texture)) = embedded(state, from, selection, name) {
        let origin = TextureOrigin {
            document: doc.id,
            label: document_label(doc),
            rule: ResolutionRule::Embedded,
            implicitly_opened: false,
        };
        let also_in = rank_candidates(state, from, name)
            .into_iter()
            .filter(|c| c.document != doc.id)
            .map(|c| c.into_origin(state))
            .collect();
        return Some(Resolved {
            texture,
            origin,
            also_in,
        });
    }

    let candidates = rank_candidates(state, from, name);
    let mut origins: Vec<TextureOrigin> = candidates
        .iter()
        .map(|c| c.clone().into_origin(state))
        .collect();
    for (index, candidate) in candidates.iter().enumerate() {
        let doc = state.document(candidate.document)?;
        let Some(texture) = fetch(doc, name) else {
            continue;
        };
        let origin = origins.remove(index);
        return Some(Resolved {
            texture,
            origin,
            also_in: origins,
        });
    }
    None
}

pub fn probe(
    state: &AppState,
    from: Option<MeshSource>,
    selection: Option<u64>,
    name: &str,
) -> TextureOutcome {
    if let Some((doc, _)) = clothing(state, from) {
        return TextureOutcome {
            name: name.to_string(),
            found: Some(TextureOrigin {
                document: doc.id,
                label: document_label(doc),
                rule: ResolutionRule::Clothing,
                implicitly_opened: false,
            }),
            also_in: Vec::new(),
        };
    }
    let mut candidates = rank_candidates(state, from, name);

    if let Some((doc, _)) = embedded(state, from, selection, name) {
        let found = TextureOrigin {
            document: doc.id,
            label: document_label(doc),
            rule: ResolutionRule::Embedded,
            implicitly_opened: false,
        };
        candidates.retain(|c| c.document != doc.id);
        return TextureOutcome {
            name: name.to_string(),
            found: Some(found),
            also_in: candidates
                .into_iter()
                .map(|c| c.into_origin(state))
                .collect(),
        };
    }

    let mut origins = candidates.into_iter().map(|c| c.into_origin(state));
    let found = origins.next();
    TextureOutcome {
        name: name.to_string(),
        found,
        also_in: origins.collect(),
    }
}

/// What `wanted` names, each with the mesh source that asks for it, resolve to.
pub fn report(
    state: &AppState,
    selection: Option<u64>,
    wanted: impl IntoIterator<Item = (Option<MeshSource>, String)>,
    epoch: u64,
) -> TextureResolutionReport {
    let distinct: BTreeSet<(String, Option<MeshSource>)> = wanted
        .into_iter()
        .map(|(from, name)| (name.to_ascii_lowercase(), from))
        .collect();
    let mut outcomes: Vec<TextureOutcome> = Vec::new();
    for (name, from) in distinct {
        let outcome = probe(state, from, selection, &name);
        // A name two sources resolve alike is one line.
        if !outcomes.contains(&outcome) {
            outcomes.push(outcome);
        }
    }

    TextureResolutionReport {
        document: state.active_document,
        outcomes,
        epoch,
    }
}

#[derive(Debug, Clone)]
struct Candidate {
    document: DocumentId,
    rule: ResolutionRule,
    /// Position in whichever list the user ordered — the pins for this
    /// document, or the attachments. Zero for a candidate no list names.
    explicit_index: usize,
    mru: usize,
}

impl Candidate {
    fn sort_key(&self) -> (u8, usize, usize) {
        (self.rule as u8, self.explicit_index, self.mru)
    }

    fn into_origin(self, state: &AppState) -> TextureOrigin {
        TextureOrigin {
            label: state
                .document(self.document)
                .map(document_label)
                .unwrap_or_default(),
            implicitly_opened: state.auto_opened_texture_docs.contains(&self.document),
            document: self.document,
            rule: self.rule,
        }
    }
}

/// The dictionaries holding `name`, best first, ranked against the document the textures are asked for.
fn rank_candidates(state: &AppState, from: Option<MeshSource>, name: &str) -> Vec<Candidate> {
    let active = subject(state, from);
    let pins = active
        .map(|doc| state.pinned_texture_dictionaries(doc.id))
        .unwrap_or_default();
    let count = state.documents.len();

    let mut out: Vec<Candidate> = state
        .documents
        .iter()
        .enumerate()
        .filter(|(_, doc)| holds(active, doc, name))
        .filter_map(|(index, doc)| {
            let pin_index = pins.iter().position(|id| *id == doc.id);
            let attach_index = state
                .attached_texture_docs
                .iter()
                .position(|attached| attached.document == doc.id);
            let rule = if Some(doc.id) == active.map(|asked| asked.id) {
                ResolutionRule::Embedded
            } else if pin_index.is_some() {
                ResolutionRule::Pinned
            } else if attach_index.is_some() {
                ResolutionRule::Attached
            } else if same_stem(active, doc) {
                ResolutionRule::SameStem
            } else if detail_variant(active, doc) {
                ResolutionRule::DetailVariant
            } else if same_folder(active, doc) {
                ResolutionRule::SameFolder
            } else if same_archive(active, doc) {
                ResolutionRule::SameArchive
            } else {
                ResolutionRule::AnyOpen
            };
            // ⚠ A dictionary the app opened for one model serves that model alone: left in the
            // ranking, the first model opened would supply the textures of every later one that
            // states a name it holds, and the later one's own dictionary would never open.
            // The exceptions are what every vehicle shares, which serves each vehicle alike, and GTA VI
            // dictionaries and packs, whose 64-bit key names one texture.
            if rule > ResolutionRule::DetailVariant
                && state.auto_opened_texture_docs.contains(&doc.id)
                && !super::ttpf_document::is_gta6(Some(doc))
                && !(active.is_some_and(|a| a.kind == DocumentKind::Yft)
                    && is_shared_vehicle_dictionary(doc))
            {
                return None;
            }
            Some(Candidate {
                document: doc.id,
                rule,
                explicit_index: pin_index.or(attach_index).unwrap_or(0),
                mru: count - 1 - index,
            })
        })
        .collect();
    out.sort_by_key(Candidate::sort_key);
    out
}

fn document_label(doc: &Document) -> String {
    doc.path
        .as_ref()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| doc.name.clone())
}

pub(super) fn stem(doc: &Document) -> Option<String> {
    doc.path
        .as_ref()
        .and_then(|p| p.file_stem())
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .or_else(|| Some(doc.name.to_ascii_lowercase()))
        .filter(|s| !s.is_empty())
}

/// The dictionaries that hold the generic textures of every vehicle (the mud, the detail and the glass), in the order they are tried, beside the vehicle's own.
const SHARED_VEHICLE_DICTIONARIES: [&str; 4] = [
    "vehshare.ytd",
    "vehshare_worn.ytd",
    "vehshare_truck.ytd",
    "vehshare_army.ytd",
];

fn is_shared_vehicle_dictionary(doc: &Document) -> bool {
    let label = document_label(doc);
    SHARED_VEHICLE_DICTIONARIES
        .iter()
        .any(|shared| shared.eq_ignore_ascii_case(&label))
}

/// The dictionary file names that could serve a drawable called `stem`, best first.
///
/// ⚠ `prop_x_hi.yft` is served by `prop_x.ytd`, and a vehicle's
/// higher-resolution set is in `vehicle+hi.ytd`. The exact name comes first,
/// then the base set before the high-detail one, as `DetailVariant` ranks below
/// `SameStem`.
pub(crate) fn wanted_dictionaries(stem: &str) -> Vec<String> {
    let name = AssetName::parse(stem);
    let mut wanted = vec![format!("{stem}.ytd")];
    for candidate in [
        format!("{}.ytd", name.base),
        format!("{}.ytd", name.companion_with("+hi")),
        format!("{}.ytd", name.companion_with("_hi")),
    ] {
        if !wanted.contains(&candidate) {
            wanted.push(candidate);
        }
    }
    wanted
}

/// The active document's stem, when a texture report about it names a missing texture.
pub(super) fn stem_missing_textures(state: &AppState) -> Option<(DocumentId, String)> {
    if state.texture_report.missing().is_empty() {
        return None;
    }
    let doc = state.active_document()?;
    if state.texture_report.document != Some(doc.id) {
        return None;
    }
    Some((doc.id, stem(doc)?))
}

fn same_stem(active: Option<&Document>, candidate: &Document) -> bool {
    match (active.and_then(stem), stem(candidate)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// The candidate is the active document's `_hi` / `+hi` companion, or its
/// base.
///
/// ⚠ Ranked **below** an exact stem match on purpose. A `+hi` dictionary
/// holds the same texture names at higher resolution, so both answer and
/// picking the companion over the exact match would silently change which
/// bytes a preview shows. The report names the rule, and a pin overrides it.
fn detail_variant(active: Option<&Document>, candidate: &Document) -> bool {
    let (Some(a), Some(b)) = (active.and_then(stem), stem(candidate)) else {
        return false;
    };
    AssetName::parse(&a).is_companion_of(&AssetName::parse(&b))
}

fn same_folder(active: Option<&Document>, candidate: &Document) -> bool {
    let Some(a) = active
        .and_then(|d| d.path.as_deref())
        .and_then(Path::parent)
    else {
        return false;
    };
    let Some(b) = candidate.path.as_deref().and_then(Path::parent) else {
        return false;
    };
    a.as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
}

fn same_archive(active: Option<&Document>, candidate: &Document) -> bool {
    match (
        active.and_then(|d| d.rpf_entry_binding),
        candidate.rpf_entry_binding,
    ) {
        (Some(a), Some(b)) => a.parent_doc_id == b.parent_doc_id,
        _ => false,
    }
}

/// A texture dictionary the user attached by hand.
///
/// ⚠ Session state, not a preference. A path in `texture_search_paths` says
/// *look here for anything*; this names one dictionary that is open now, so it
/// cannot outlive the session the document ids belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachedDictionary {
    pub document: DocumentId,
    /// Attaching opened it, so detaching closes it again.
    pub opened_by_attach: bool,
}

impl AppState {
    pub fn attached_texture_dictionaries(&self) -> &[AttachedDictionary] {
        &self.attached_texture_docs
    }

    pub fn is_attached_texture_dictionary(&self, doc: DocumentId) -> bool {
        self.attached_texture_docs
            .iter()
            .any(|attached| attached.document == doc)
    }

    /// Consult an already-open dictionary for every document's textures.
    pub fn attach_texture_dictionary(&mut self, doc: DocumentId) -> Result<(), String> {
        if !self
            .document(doc)
            .is_some_and(|d| d.payload.kind() == super::DocumentKind::Ytd)
        {
            return Err("Only a texture dictionary can be attached".to_string());
        }
        self.record_attachment(doc, false);
        Ok(())
    }

    /// Open a `.ytd` from disk and attach it.
    pub fn attach_texture_dictionary_from_disk(
        &mut self,
        path: &Path,
    ) -> Result<DocumentId, String> {
        if let Some(existing) = self
            .documents
            .iter()
            .find(|d| d.path.as_deref() == Some(path))
            .map(|d| d.id)
        {
            self.attach_texture_dictionary(existing)?;
            return Ok(existing);
        }

        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let Some(kind) = super::DocumentKind::from_path(path) else {
            return Err(format!("{}: no loader for this file type", path.display()));
        };
        let provider = self.keys.provider();
        let payload = super::document_io::parse_payload(path, kind, &bytes, provider)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if payload.kind() != super::DocumentKind::Ytd {
            return Err(format!("{} is not a texture dictionary", path.display()));
        }

        let resource = super::document_io::resource_image_for(kind, &bytes);
        let id = self.alloc_doc_id();
        self.documents
            .push(Document::new(id, Some(path.to_path_buf()), payload).with_resource(resource));
        self.record_attachment(id, true);
        Ok(id)
    }

    /// Pull a `.ytd` out of an open archive and attach it.
    pub fn attach_texture_dictionary_from_archive(
        &mut self,
        parent: DocumentId,
        entry: super::RpfNodeId,
    ) -> Result<DocumentId, String> {
        if let Some(existing) = self.document(parent).and_then(|doc| match &doc.payload {
            DocPayload::Rpf(rpf) => rpf.entry_documents.get(&entry).copied(),
            _ => None,
        }) {
            self.attach_texture_dictionary(existing)?;
            return Ok(existing);
        }

        let binding = super::RpfEntryBinding {
            parent_doc_id: parent,
            entry_id: entry,
        };
        let (id, payload_is_a_dictionary) = {
            let id = self.open_archive_entry_now(binding)?;
            let is_dictionary = self
                .document(id)
                .is_some_and(|d| d.payload.kind() == super::DocumentKind::Ytd);
            (id, is_dictionary)
        };
        if !payload_is_a_dictionary {
            self.drop_document(id);
            return Err("That archive entry is not a texture dictionary".to_string());
        }
        self.record_attachment(id, true);
        Ok(id)
    }

    /// Stop consulting it, closing it again if attaching is what opened it.
    pub fn detach_texture_dictionary(&mut self, doc: DocumentId) {
        let Some(index) = self
            .attached_texture_docs
            .iter()
            .position(|attached| attached.document == doc)
        else {
            return;
        };
        let attached = self.attached_texture_docs[index];
        self.retry_texture_search();
        if !attached.opened_by_attach {
            self.attached_texture_docs.remove(index);
        } else if self.unsaved_documents(&[doc]).is_empty() {
            self.drop_document(doc);
        } else {
            // ⚠ The attachment ends with the document, so a Cancel keeps both.
            self.request_close(vec![doc], super::AfterClose::Nothing);
        }
    }

    fn record_attachment(&mut self, doc: DocumentId, opened_by_attach: bool) {
        if self.is_attached_texture_dictionary(doc) {
            return;
        }
        self.attached_texture_docs.push(AttachedDictionary {
            document: doc,
            opened_by_attach,
        });
        // ⚠ An attachment can supply a name the automatic routes gave up on,
        // and the giving up is remembered per document.
        self.retry_texture_search();
    }

    fn retry_texture_search(&mut self) {
        self.attempted_texture_siblings.clear();
        self.attempted_shared_dictionaries.clear();
        self.attempted_texture_search_paths.clear();
    }

    /// Read an archive entry into a document without leaving the frame.
    ///
    /// ⚠ Not [`Self::open_rpf_entry`]: that one may parse on a worker thread,
    /// so it cannot hand back the id a caller has to record against.
    pub(super) fn open_archive_entry_now(
        &mut self,
        binding: super::RpfEntryBinding,
    ) -> Result<DocumentId, String> {
        let (entry_path, bytes) = {
            let parent = self
                .document(binding.parent_doc_id)
                .ok_or("That archive is no longer open")?;
            let DocPayload::Rpf(rpf) = &parent.payload else {
                return Err("That document is not an archive".to_string());
            };
            let path = rpf
                .session
                .path_of(binding.entry_id)
                .ok_or("That archive entry is gone")?;
            let bytes = rpf
                .session
                .bytes_for(binding.entry_id)
                .ok_or_else(|| format!("No bytes for '{path}'"))?;
            (path, (*bytes).clone())
        };

        let mut display_path = self
            .document(binding.parent_doc_id)
            .and_then(|d| d.path.clone())
            .unwrap_or_else(|| std::path::PathBuf::from("(unsaved RPF)"));
        for part in entry_path.split('/') {
            display_path.push(part);
        }

        let kind = super::DocumentKind::from_path(&display_path)
            .ok_or_else(|| format!("No loader registered for {entry_path}"))?;
        let provider = self.keys.provider();
        let payload = super::document_io::parse_payload(&display_path, kind, &bytes, provider)
            .map_err(|e| format!("{entry_path}: {e}"))?;
        let resource = super::document_io::resource_image_for(kind, &bytes);
        Ok(self.attach_hidden_document(display_path, payload, binding, resource))
    }

    pub fn pinned_texture_dictionaries(&self, doc: DocumentId) -> &[DocumentId] {
        self.pinned_texture_docs
            .get(&doc)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn pin_texture_dictionary(&mut self, doc: DocumentId, dictionary: DocumentId) {
        let pins = self.pinned_texture_docs.entry(doc).or_default();
        if !pins.contains(&dictionary) {
            pins.push(dictionary);
        }
    }

    pub fn unpin_texture_dictionary(&mut self, doc: DocumentId, dictionary: DocumentId) {
        if let Some(pins) = self.pinned_texture_docs.get_mut(&doc) {
            pins.retain(|id| *id != dictionary);
            if pins.is_empty() {
                self.pinned_texture_docs.remove(&doc);
            }
        }
    }

    pub fn clear_texture_pins(&mut self, doc: DocumentId) {
        self.pinned_texture_docs.remove(&doc);
    }

    /// The documents the viewport's textures are asked for: what the Animation preview draws, else the active one.
    pub fn textured_documents(&self) -> Vec<DocumentId> {
        match self.active_workspace {
            Workspace::Animation => self.preview_scene.drawn_documents(),
            _ => self.active_document.into_iter().collect(),
        }
    }

    /// The dictionaries pinned for any of them.
    pub fn texture_pins_in_view(&self) -> Vec<DocumentId> {
        (self.textured_documents().into_iter())
            .flat_map(|doc| self.pinned_texture_dictionaries(doc).to_vec())
            .collect()
    }

    pub fn pin_texture_dictionary_in_view(&mut self, dictionary: DocumentId) {
        for doc in self.textured_documents() {
            self.pin_texture_dictionary(doc, dictionary);
        }
    }

    pub fn unpin_texture_dictionary_in_view(&mut self, dictionary: DocumentId) {
        for doc in self.textured_documents() {
            self.unpin_texture_dictionary(doc, dictionary);
        }
    }

    pub fn clear_texture_pins_in_view(&mut self) {
        for doc in self.textured_documents() {
            self.clear_texture_pins(doc);
        }
    }

    pub(super) fn forget_texture_resolution(&mut self, id: DocumentId) {
        self.pinned_texture_docs.remove(&id);
        self.pinned_texture_docs.retain(|_, pins| {
            pins.retain(|pinned| *pinned != id);
            !pins.is_empty()
        });
        self.auto_opened_texture_docs.remove(&id);
        self.attached_texture_docs
            .retain(|attached| attached.document != id);
        self.attempted_texture_siblings.remove(&id);
        self.attempted_shared_dictionaries
            .retain(|(doc, _)| *doc != id);
        self.attempted_texture_search_paths.remove(&id);
        self.attempted_texture_project.remove(&id);
        if self.texture_report.document == Some(id) {
            self.texture_report = TextureResolutionReport::default();
        }
    }

    pub fn try_auto_open_texture_sibling(&mut self) -> bool {
        let Some(doc) = self.active_document() else {
            return false;
        };
        let doc_id = doc.id;
        // ⚠ A model asks for its own dictionary whether or not an open one holds the names it
        // states: another model's dictionary does, and answering from that is the wrong texture.
        let is_model = matches!(
            doc.kind,
            DocumentKind::Ydr | DocumentKind::Ydd | DocumentKind::Yft
        );
        let misses = self.texture_report.document == Some(doc_id)
            && !self.texture_report.missing().is_empty();
        if !is_model && !misses {
            return false;
        }
        if self.attempted_texture_siblings.contains(&doc_id) {
            return false;
        }
        let Some(binding) = doc.rpf_entry_binding else {
            return false;
        };
        let Some(stem) = stem(doc) else {
            return false;
        };
        self.attempted_texture_siblings.insert(doc_id);

        let wanted = wanted_dictionaries(&stem);
        let Some((sibling_node, sibling_path)) = wanted
            .iter()
            .find_map(|want| self.sibling_archive_entry(binding, want))
        else {
            return false;
        };
        if self.rpf_entry_is_open(binding.parent_doc_id, sibling_node) {
            return false;
        }

        let id = match self.open_archive_entry_now(super::RpfEntryBinding {
            parent_doc_id: binding.parent_doc_id,
            entry_id: sibling_node,
        }) {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!(%error, sibling = %sibling_path, "texture sibling auto-open failed");
                return false;
            }
        };
        self.auto_opened_texture_docs.insert(id);
        tracing::info!(sibling = %sibling_path, "opened a texture sibling from the archive");
        true
    }

    /// Open, beside the active vehicle, the next dictionary every vehicle shares its generic textures through, while a name is still missing; whether one opened.
    ///
    /// ⚠ One a call, each at most once for a model, and only for a name nothing open holds: the
    /// vehicle's own dictionary does not carry the mud, the detail and the glass its materials name.
    pub fn try_auto_open_shared_dictionary(&mut self) -> bool {
        let Some(doc) = self.active_document() else {
            return false;
        };
        let doc_id = doc.id;
        let misses = self.texture_report.document == Some(doc_id)
            && !self.texture_report.missing().is_empty();
        if doc.kind != DocumentKind::Yft || !misses {
            return false;
        }
        let Some(binding) = doc.rpf_entry_binding else {
            return false;
        };
        for name in SHARED_VEHICLE_DICTIONARIES {
            if self.attempted_shared_dictionaries.contains(&(doc_id, name)) {
                continue;
            }
            let Some((node, path)) = self.sibling_archive_entry(binding, name) else {
                continue;
            };
            self.attempted_shared_dictionaries.insert((doc_id, name));
            if self.rpf_entry_is_open(binding.parent_doc_id, node) {
                continue;
            }
            let opened = self.open_archive_entry_now(super::RpfEntryBinding {
                parent_doc_id: binding.parent_doc_id,
                entry_id: node,
            });
            match opened {
                Ok(id) => {
                    self.auto_opened_texture_docs.insert(id);
                    tracing::info!(shared = %path, "opened a shared vehicle dictionary from the archive");
                    return true;
                }
                Err(error) => {
                    tracing::warn!(%error, shared = %path, "shared vehicle dictionary auto-open failed");
                }
            }
        }
        false
    }

    /// Open a `.ytd` from the configured search paths for a name nothing holds.
    ///
    /// ⚠ **First match wins, in the user's order**: a mod's folder and the base
    /// game's can hold dictionaries of the same name.
    ///
    /// ⚠ Only the companion names are tried, not every `.ytd` in the folder.
    /// Opening a folder's worth of dictionaries to find one name would make
    /// every later resolution ambiguous by flooding `also_in`.
    pub fn try_open_texture_from_search_paths(&mut self) -> bool {
        if self.texture_report.missing().is_empty() {
            return false;
        }
        let Some(doc) = self.active_document() else {
            return false;
        };
        let doc_id = doc.id;
        if self.texture_report.document != Some(doc_id) {
            return false;
        }
        if self.attempted_texture_search_paths.contains(&doc_id) {
            return false;
        }
        let Some(stem) = stem(doc) else {
            return false;
        };
        self.attempted_texture_search_paths.insert(doc_id);

        let wanted = wanted_dictionaries(&stem);
        let roots = self.prefs.texture_search_paths.clone();
        for root in roots {
            for file in &wanted {
                if self.open_texture_dictionary_file(&root.join(file)) {
                    return true;
                }
            }
        }
        false
    }

    /// Open a dictionary on disk as an auto-opened document. `false` when it
    /// is missing, already open, or does not parse.
    pub(super) fn open_texture_dictionary_file(&mut self, path: &Path) -> bool {
        let already_open = self
            .documents
            .iter()
            .any(|d| d.path.as_deref() == Some(path));
        if !path.is_file() || already_open {
            return false;
        }
        let Ok(bytes) = std::fs::read(path) else {
            return false;
        };
        let Some(kind) = super::DocumentKind::from_path(path) else {
            return false;
        };
        let provider = self.keys.provider();
        let payload = match super::document_io::parse_payload(path, kind, &bytes, provider) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "texture dictionary failed to open");
                return false;
            }
        };
        let resource = super::document_io::resource_image_for(kind, &bytes);
        let id = self.alloc_doc_id();
        self.documents
            .push(Document::new(id, Some(path.to_path_buf()), payload).with_resource(resource));
        self.auto_opened_texture_docs.insert(id);
        tracing::info!(path = %path.display(), "opened a texture dictionary");
        true
    }

    fn sibling_archive_entry(
        &self,
        binding: super::RpfEntryBinding,
        name: &str,
    ) -> Option<(super::RpfNodeId, String)> {
        let parent = self.document(binding.parent_doc_id)?;
        let DocPayload::Rpf(rpf) = &parent.payload else {
            return None;
        };
        let folder = rpf.session.node(binding.entry_id)?.parent?;
        let sibling = rpf.session.children_of(folder).into_iter().find(|id| {
            rpf.session
                .node(*id)
                .is_some_and(|n| !n.is_dir() && n.name.eq_ignore_ascii_case(name))
        })?;
        let path = rpf.session.path_of(sibling)?;
        Some((sibling, path))
    }

    fn rpf_entry_is_open(&self, parent: DocumentId, entry: super::RpfNodeId) -> bool {
        self.document(parent).is_some_and(|doc| match &doc.payload {
            DocPayload::Rpf(rpf) => rpf.entry_documents.contains_key(&entry),
            _ => false,
        })
    }

    fn attach_hidden_document(
        &mut self,
        path: std::path::PathBuf,
        payload: DocPayload,
        binding: super::RpfEntryBinding,
        resource: Option<std::sync::Arc<super::document::ResourceImage>>,
    ) -> DocumentId {
        let id = self.alloc_doc_id();
        let mut doc = Document::new(id, Some(path), payload).with_resource(resource);
        doc.rpf_entry_binding = Some(binding);
        self.documents.push(doc);
        if let Some(parent) = self.document_mut(binding.parent_doc_id) {
            if let DocPayload::Rpf(rpf) = &mut parent.payload {
                rpf.entry_documents.insert(binding.entry_id, id);
            }
        }
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Document;
    use or_io::codec::texture::GrcTextureFormat;
    use std::path::PathBuf;

    fn dictionary(texture: &str) -> DocPayload {
        let mut dict = or_io::gta5::PgDictionary::default();
        dict.insert(or_io::gta5::gen8::GrcTexture {
            name: texture.to_string(),
            width: 4,
            height: 4,
            format: GrcTextureFormat::A8R8G8B8.to_raw_format(),
            ..Default::default()
        });
        DocPayload::Resource(or_io::Asset::TextureDict(dict))
    }

    fn open(state: &mut AppState, path: &str, payload: DocPayload) -> DocumentId {
        let id = state.documents.len() as DocumentId + 1;
        state
            .documents
            .push(Document::new(id, Some(PathBuf::from(path)), payload));
        id
    }

    fn gta6_document(id: DocumentId, path: &str, payload: DocPayload) -> Document {
        let mut doc = Document::new(id, Some(PathBuf::from(path)), payload);
        doc.read_only = Some(crate::state::document::rpf9::entry::READ_ONLY);
        doc
    }

    #[test]
    fn a_gta6_dictionary_resolves_a_shader_key_before_its_name_is_learned() {
        let name = "table_texture_not_learned";
        let dict = gta6_document(1, "textures.rpf/defaults.ytd", dictionary(name));
        let model = gta6_document(
            2,
            "data.rpf/model.ydr",
            DocPayload::Resource(or_io::Asset::Drawable(Default::default())),
        );
        let key = or_io::gta6::hash::name_hash64(name.as_bytes());
        for wanted in [format!("hash_{key:016X}"), name.to_ascii_uppercase()] {
            assert!(holds(Some(&model), &dict, &wanted));
            assert_eq!(
                fetch(&dict, &wanted).expect("the key resolves").name(),
                name
            );
        }
        let missing = format!("hash_{:016X}", key ^ 1);
        assert!(!holds(Some(&model), &dict, &missing));
        assert!(fetch(&dict, &missing).is_none());
    }

    #[test]
    fn gta6_and_gta5_dictionaries_do_not_supply_each_others_models() {
        let gta5_dict = Document::new(1, None, dictionary("shared_name"));
        let gta6_dict = gta6_document(2, "texture.ytd", dictionary("shared_name"));
        let gta5_model = Document::new(
            3,
            None,
            DocPayload::Resource(or_io::Asset::Drawable(Default::default())),
        );
        let gta6_model = gta6_document(
            4,
            "model.ydr",
            DocPayload::Resource(or_io::Asset::Drawable(Default::default())),
        );
        assert!(holds(Some(&gta5_model), &gta5_dict, "shared_name"));
        assert!(holds(Some(&gta6_model), &gta6_dict, "shared_name"));
        assert!(!holds(Some(&gta5_model), &gta6_dict, "shared_name"));
        assert!(!holds(Some(&gta6_model), &gta5_dict, "shared_name"));
    }

    #[test]
    fn a_gta6_texture_does_not_match_a_32_bit_name_collision() {
        let stored = "rpf9_texture_41144";
        let other = "rpf9_texture_45400";
        assert_eq!(
            or_io::hash::string_hash_str(stored),
            or_io::hash::string_hash_str(other)
        );
        assert_ne!(
            or_io::gta6::hash::name_hash64(stored.as_bytes()),
            or_io::gta6::hash::name_hash64(other.as_bytes())
        );
        let dict = gta6_document(1, "textures.ytd", dictionary(stored));
        assert!(fetch(&dict, other).is_none());
        assert_eq!(fetch(&dict, stored).expect("the exact key").name(), stored);
    }

    #[test]
    fn an_automatically_opened_gta6_dictionary_serves_another_gta6_model() {
        let mut state = AppState::new();
        state.documents.push(gta6_document(
            1,
            "textures.rpf/defaults.ytd",
            dictionary("external_default"),
        ));
        state.documents.push(gta6_document(
            2,
            "data.rpf/model.ydr",
            DocPayload::Resource(or_io::Asset::Drawable(Default::default())),
        ));
        state.auto_opened_texture_docs.insert(1);
        state.active_document = Some(2);
        let key = or_io::gta6::hash::name_hash64(b"external_default");
        let name = format!("hash_{key:016X}");
        let outcome = probe(&state, None, None, &name);
        let resolved = resolve(&state, None, None, &name).expect("the dictionary answers");
        assert_eq!(outcome.found, Some(resolved.origin.clone()));
        assert_eq!(resolved.origin.document, 1);
        assert_eq!(resolved.origin.rule, ResolutionRule::AnyOpen);
        assert_eq!(resolved.texture.name(), "external_default");
    }

    #[test]
    fn a_same_stem_dictionary_beats_the_first_one_opened() {
        let mut state = AppState::new();
        let oldest = open(&mut state, "C:/a/also_wrong.ytd", dictionary("diffuse"));
        let right = open(&mut state, "C:/a/gamakichi.ytd", dictionary("diffuse"));
        let newest = open(&mut state, "C:/a/wrong_one.ytd", dictionary("diffuse"));
        let drawable = open(
            &mut state,
            "C:/a/gamakichi.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        state.active_document = Some(drawable);

        let resolved = resolve(&state, None, None, "diffuse").expect("some dictionary answers");
        assert_eq!(
            resolved.origin.document, right,
            "the same-stem dictionary has to win"
        );
        assert_ne!(
            resolved.origin.document, oldest,
            "first-opened is the bug that was reported"
        );
        assert_ne!(
            resolved.origin.document, newest,
            "most-recently-opened is the tie-break, not a rule of its own"
        );
        assert_eq!(resolved.origin.rule, ResolutionRule::SameStem);
    }

    /// ⚠ Reverse this ranking and a vehicle silently previews its high-detail
    /// set.
    #[test]
    fn a_detail_companion_answers_but_never_outranks_the_exact_name() {
        let mut state = AppState::new();
        let hi = open(&mut state, "C:/a/vehicle+hi.ytd", dictionary("paint"));
        let exact = open(&mut state, "C:/a/vehicle.ytd", dictionary("paint"));
        let drawable = open(
            &mut state,
            "C:/a/vehicle.ydr",
            DocPayload::Resource(or_io::Asset::Drawable(Default::default())),
        );
        state.active_document = Some(drawable);

        let resolved = resolve(&state, None, None, "paint").expect("both answer");
        assert_eq!(resolved.origin.document, exact);
        assert_eq!(resolved.origin.rule, ResolutionRule::SameStem);

        let report = report(&state, None, [(None, "paint".to_string())], 0);
        assert!(
            report.outcomes[0]
                .also_in
                .iter()
                .any(|other| other.document == hi),
            "the companion has to be visible, not discarded"
        );

        // With no exact match the companion is what answers, and says so.
        let mut only_hi = AppState::new();
        open(&mut only_hi, "C:/a/vehicle+hi.ytd", dictionary("paint"));
        let drawable = open(
            &mut only_hi,
            "C:/a/vehicle.ydr",
            DocPayload::Resource(or_io::Asset::Drawable(Default::default())),
        );
        only_hi.active_document = Some(drawable);
        let resolved = resolve(&only_hi, None, None, "paint").expect("the companion answers");
        assert_eq!(resolved.origin.rule, ResolutionRule::DetailVariant);
    }

    /// ⚠ Ranked under `SameStem`, an attachment would be invisible in the one
    /// case it is reached for: a drawable whose same-named dictionary holds the
    /// wrong bytes.
    #[test]
    fn an_attachment_outranks_every_name_heuristic() {
        let mut state = AppState::new();
        let attached = open(&mut state, "C:/mod/replacement.ytd", dictionary("diffuse"));
        open(&mut state, "C:/a/gamakichi.ytd", dictionary("diffuse"));
        let drawable = open(
            &mut state,
            "C:/a/gamakichi.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        state.active_document = Some(drawable);

        assert_eq!(
            resolve(&state, None, None, "diffuse").unwrap().origin.rule,
            ResolutionRule::SameStem,
            "the same-stem dictionary answers until something is attached"
        );

        state.attach_texture_dictionary(attached).expect("a .ytd");
        let resolved = resolve(&state, None, None, "diffuse").expect("resolves");
        assert_eq!(resolved.origin.document, attached);
        assert_eq!(resolved.origin.rule, ResolutionRule::Attached);

        state.detach_texture_dictionary(attached);
        assert_eq!(
            resolve(&state, None, None, "diffuse").unwrap().origin.rule,
            ResolutionRule::SameStem,
            "detaching puts the heuristics back"
        );
    }

    #[test]
    fn a_pin_survives_an_attachment() {
        let mut state = AppState::new();
        let pinned = open(&mut state, "C:/a/pinned.ytd", dictionary("diffuse"));
        let attached = open(&mut state, "C:/mod/attached.ytd", dictionary("diffuse"));
        let drawable = open(
            &mut state,
            "C:/a/gamakichi.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        state.active_document = Some(drawable);
        state.pin_texture_dictionary(drawable, pinned);
        state.attach_texture_dictionary(attached).expect("a .ytd");

        let resolved = resolve(&state, None, None, "diffuse").expect("resolves");
        assert_eq!(resolved.origin.document, pinned);
        assert_eq!(resolved.origin.rule, ResolutionRule::Pinned);
    }

    /// ⚠ Two attachments can hold the same name, so the list is ordered and
    /// never a set — the same reason the search paths in `Prefs` are.
    #[test]
    fn the_first_attached_dictionary_wins() {
        let mut state = AppState::new();
        let first = open(&mut state, "C:/mod/a.ytd", dictionary("diffuse"));
        let second = open(&mut state, "C:/mod/b.ytd", dictionary("diffuse"));
        let drawable = open(
            &mut state,
            "C:/a/unrelated.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        state.active_document = Some(drawable);
        state.attach_texture_dictionary(first).expect("a .ytd");
        state.attach_texture_dictionary(second).expect("a .ytd");

        assert_eq!(
            resolve(&state, None, None, "diffuse")
                .unwrap()
                .origin
                .document,
            first
        );

        state.detach_texture_dictionary(first);
        assert_eq!(
            resolve(&state, None, None, "diffuse")
                .unwrap()
                .origin
                .document,
            second,
            "detaching the winner promotes the next, not the heuristics"
        );
    }

    #[test]
    fn detaching_closes_only_what_attaching_opened() {
        let mut state = AppState::new();
        let already_open = open(&mut state, "C:/a/open.ytd", dictionary("diffuse"));
        state
            .attach_texture_dictionary(already_open)
            .expect("a .ytd");
        assert_eq!(
            state.attached_texture_dictionaries(),
            &[AttachedDictionary {
                document: already_open,
                opened_by_attach: false,
            }]
        );

        state.detach_texture_dictionary(already_open);
        assert!(state.attached_texture_dictionaries().is_empty());
        assert!(
            state.document(already_open).is_some(),
            "a document the user opened is not closed by detaching it"
        );
    }

    #[test]
    fn only_a_texture_dictionary_can_be_attached() {
        let mut state = AppState::new();
        let drawable = open(
            &mut state,
            "C:/a/gamakichi.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        assert!(state.attach_texture_dictionary(drawable).is_err());
        assert!(state.attached_texture_dictionaries().is_empty());
    }

    #[test]
    fn closing_an_attached_dictionary_drops_the_attachment() {
        let mut state = AppState::new();
        let attached = open(&mut state, "C:/mod/replacement.ytd", dictionary("diffuse"));
        state.attach_texture_dictionary(attached).expect("a .ytd");

        state.close_document(attached);
        assert!(
            state.attached_texture_dictionaries().is_empty(),
            "an attachment naming a closed document is a dangling id"
        );
    }

    #[test]
    fn a_pin_beats_every_automatic_rule() {
        let mut state = AppState::new();
        let other = open(&mut state, "C:/a/ped_generic.ytd", dictionary("diffuse"));
        open(&mut state, "C:/a/gamakichi.ytd", dictionary("diffuse"));
        let drawable = open(
            &mut state,
            "C:/a/gamakichi.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        state.active_document = Some(drawable);

        state.pin_texture_dictionary(drawable, other);
        let resolved = resolve(&state, None, None, "diffuse").expect("resolves");
        assert_eq!(resolved.origin.document, other);
        assert_eq!(resolved.origin.rule, ResolutionRule::Pinned);

        state.clear_texture_pins(drawable);
        let resolved = resolve(&state, None, None, "diffuse").expect("resolves");
        assert_eq!(resolved.origin.rule, ResolutionRule::SameStem);
    }

    #[test]
    fn the_most_recently_opened_wins_among_equals() {
        let mut state = AppState::new();
        open(&mut state, "C:/one/older.ytd", dictionary("diffuse"));
        let newer = open(&mut state, "C:/two/newer.ytd", dictionary("diffuse"));
        let drawable = open(
            &mut state,
            "C:/three/unrelated.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        state.active_document = Some(drawable);

        let resolved = resolve(&state, None, None, "diffuse").expect("resolves");
        assert_eq!(resolved.origin.document, newer);
        assert_eq!(resolved.origin.rule, ResolutionRule::AnyOpen);
    }

    #[test]
    fn the_same_folder_beats_an_unrelated_one() {
        let mut state = AppState::new();
        let neighbour = open(&mut state, "C:/peds/tex.ytd", dictionary("diffuse"));
        open(&mut state, "C:/elsewhere/newest.ytd", dictionary("diffuse"));
        let drawable = open(
            &mut state,
            "C:/peds/gamakichi.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        state.active_document = Some(drawable);

        let resolved = resolve(&state, None, None, "diffuse").expect("resolves");
        assert_eq!(resolved.origin.document, neighbour);
        assert_eq!(resolved.origin.rule, ResolutionRule::SameFolder);
    }

    #[test]
    fn a_second_dictionary_holding_the_name_is_listed() {
        let mut state = AppState::new();
        let right = open(&mut state, "C:/a/gamakichi.ytd", dictionary("eye_diff"));
        let rival = open(&mut state, "C:/a/ped_generic.ytd", dictionary("eye_diff"));
        let drawable = open(
            &mut state,
            "C:/a/gamakichi.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        state.active_document = Some(drawable);

        let outcome = probe(&state, None, None, "eye_diff");
        assert_eq!(outcome.found.as_ref().map(|o| o.document), Some(right));
        assert_eq!(
            outcome
                .also_in
                .iter()
                .map(|o| o.document)
                .collect::<Vec<_>>(),
            vec![rival],
            "the dictionary that did not win still has to be named"
        );
        assert!(outcome.is_ambiguous());
    }

    #[test]
    fn a_name_nothing_holds_is_reported_missing() {
        let mut state = AppState::new();
        open(&mut state, "C:/a/gamakichi.ytd", dictionary("diffuse"));
        let drawable = open(
            &mut state,
            "C:/a/gamakichi.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        state.active_document = Some(drawable);

        let built = report(
            &state,
            None,
            [(None, "diffuse".to_string()), (None, "fx_mask".to_string())],
            1,
        );
        assert_eq!(built.requested(), 2);
        assert_eq!(built.resolved(), 1);
        assert_eq!(built.missing(), vec!["fx_mask"]);
        assert!(!built.is_clean());
        assert_eq!(built.summary(), "1 of 2");
    }

    #[test]
    fn an_unambiguous_resolution_is_clean() {
        let mut state = AppState::new();
        open(&mut state, "C:/a/gamakichi.ytd", dictionary("diffuse"));
        let drawable = open(
            &mut state,
            "C:/a/gamakichi.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        state.active_document = Some(drawable);

        let built = report(&state, None, [(None, "diffuse".to_string())], 1);
        assert!(built.is_clean());
        assert_eq!(built.conflicts(), 0);
        assert_eq!(
            built.primary_source().map(|o| o.label.as_str()),
            Some("gamakichi.ytd")
        );
    }

    #[test]
    fn closing_a_pinned_dictionary_drops_the_pin() {
        let mut state = AppState::new();
        let dict = open(&mut state, "C:/a/ped_generic.ytd", dictionary("diffuse"));
        let drawable = open(
            &mut state,
            "C:/a/gamakichi.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        state.active_document = Some(drawable);
        state.pin_texture_dictionary(drawable, dict);
        assert_eq!(state.pinned_texture_dictionaries(drawable), &[dict]);

        state.close_document(dict);
        assert!(
            state.pinned_texture_dictionaries(drawable).is_empty(),
            "a pin naming a closed document is a dangling id"
        );
    }

    #[test]
    fn closing_the_pinning_document_drops_its_stack() {
        let mut state = AppState::new();
        let dict = open(&mut state, "C:/a/ped_generic.ytd", dictionary("diffuse"));
        let drawable = open(
            &mut state,
            "C:/a/gamakichi.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        state.active_document = Some(drawable);
        state.pin_texture_dictionary(drawable, dict);

        state.close_document(drawable);
        assert!(state.pinned_texture_docs.is_empty());
    }

    #[test]
    fn a_document_from_disk_never_auto_opens_a_sibling() {
        let mut state = AppState::new();
        let drawable = open(
            &mut state,
            "C:/a/gamakichi.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        state.active_document = Some(drawable);
        state.texture_report = report(&state, None, [(None, "fx_mask".to_string())], 1);
        assert_eq!(state.texture_report.missing(), vec!["fx_mask"]);

        assert!(!state.try_auto_open_texture_sibling());
        assert!(
            state.attempted_texture_siblings.is_empty(),
            "a document with no archive has nothing to attempt, so nothing to remember"
        );
    }

    #[test]
    fn a_report_about_another_document_triggers_nothing_for_what_is_no_model() {
        let mut state = AppState::new();
        let first = open(
            &mut state,
            "C:/a/first.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        let effects = open(
            &mut state,
            "C:/a/second.ypt",
            DocPayload::Resource(or_io::Asset::Particles(Default::default())),
        );
        let in_an_archive = |state: &mut AppState, id| {
            state.document_mut(id).expect("open").rpf_entry_binding =
                Some(super::super::RpfEntryBinding {
                    parent_doc_id: 99,
                    entry_id: 1,
                });
        };
        in_an_archive(&mut state, first);
        in_an_archive(&mut state, effects);
        state.active_document = Some(first);
        state.texture_report = report(&state, None, [(None, "fx_mask".to_string())], 1);

        state.active_document = Some(effects);
        assert!(!state.try_auto_open_texture_sibling());
        assert!(state.attempted_texture_siblings.is_empty());
    }

    #[test]
    fn a_model_in_an_archive_asks_for_its_own_dictionary_once_whatever_the_report_says() {
        let mut state = AppState::new();
        let model = open(
            &mut state,
            "C:/a/second.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        state.document_mut(model).expect("open").rpf_entry_binding =
            Some(super::super::RpfEntryBinding {
                parent_doc_id: 99,
                entry_id: 1,
            });
        state.active_document = Some(model);
        assert!(state.texture_report.is_empty());

        assert!(!state.try_auto_open_texture_sibling(), "no archive is open");
        assert!(
            state.attempted_texture_siblings.contains(&model),
            "it asked, though nothing was missing"
        );
    }

    #[test]
    fn closing_the_reported_document_drops_the_report() {
        let mut state = AppState::new();
        open(&mut state, "C:/a/gamakichi.ytd", dictionary("diffuse"));
        let drawable = open(
            &mut state,
            "C:/a/gamakichi.ydd",
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        );
        state.active_document = Some(drawable);
        state.texture_report = report(&state, None, [(None, "diffuse".to_string())], 1);
        state.attempted_texture_siblings.insert(drawable);
        state.auto_opened_texture_docs.insert(drawable);
        assert!(!state.texture_report.is_empty());

        state.close_document(drawable);
        assert!(state.texture_report.is_empty());
        assert!(state.attempted_texture_siblings.is_empty());
        assert!(state.auto_opened_texture_docs.is_empty());
    }

    #[test]
    fn a_dictionary_reading_itself_is_not_a_guess() {
        let mut state = AppState::new();
        open(&mut state, "C:/a/other.ytd", dictionary("diffuse"));
        let own = open(&mut state, "C:/a/gamakichi.ytd", dictionary("diffuse"));
        state.active_document = Some(own);

        let outcome = probe(&state, None, None, "diffuse");
        assert_eq!(
            outcome.found.as_ref().map(|o| o.rule),
            Some(ResolutionRule::Embedded)
        );
        assert!(!outcome.is_ambiguous(), "its own textures are not a guess");
    }
}
