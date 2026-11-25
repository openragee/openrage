mod history;
mod image;
mod kind;
mod kind_name;
mod location;
mod material_baseline;
mod payload;
pub(crate) use crate::state::rpf9_scan::rpf9_table_document as rpf9;
mod text;

pub use kind::{DocumentKind, TEXT_DOCUMENT_EXTENSIONS};
pub use material_baseline::{
    drawable_mut, enhanced_drawable_mut, enhanced_drawables_of, BaselineValue, MaterialAddress,
    MaterialBaseline, Slot,
};
pub use payload::{DocPayload, FxcDocument};
pub use text::{LineEnding, TextDocument, TextEncoding};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::edit_history::Change;
use super::rpf_session::{RpfDocument, RpfEntryBinding};

pub type DocumentId = u64;
pub type RpfNodeId = u64;
pub type ArchiveId = u32;

mod script;

pub use script::{natives_dir, ScriptDocument};

#[allow(clippy::large_enum_variant)]
mod meta;

pub use meta::{meta_schema, MetaDocument};

mod gta6_meta_names;

mod gta6_meta_strings;

pub use gta6_meta_strings::{DocumentStrings, STRING_NAMES_NEED_KEYS};

pub use gta6_meta_names::{
    hash_name_or_text, hash_text_name, meta_names_from_the_text, meta_text_name,
};

mod rel;

pub use rel::RelDocument;

mod dat_table;

pub use dat_table::DatTable;

mod name_table;

pub use name_table::{name_table_bytes, name_table_text};

mod hex;

pub use hex::HexDocument;

#[derive(Debug, Clone)]
pub struct ResourceImage {
    pub data: Vec<u8>,
    pub virtual_size: usize,
    /// The RSC7 header the bytes were opened under, carried so a patched
    /// image goes back beneath the same one.
    pub seal: or_io::container::resource::Rsc7Seal,
}

impl ResourceImage {
    pub fn view(&self) -> or_io::desc::Image<'_> {
        or_io::desc::Image::new(&self.data, self.virtual_size)
    }

    pub fn view_mut(&mut self) -> or_io::desc::ImageMut<'_> {
        or_io::desc::ImageMut::new(&mut self.data, self.virtual_size)
    }

    pub fn to_resource_bytes(&self) -> Result<Vec<u8>, String> {
        use or_io::container::resource::{Compression, ResourceEnvelope, Rsc7};

        Rsc7::reseal(&self.data, &self.seal, Compression::default()).map_err(|e| e.to_string())
    }
}

pub struct Document {
    pub id: DocumentId,
    pub path: Option<PathBuf>,
    pub name: String,
    pub kind: DocumentKind,
    pub payload: DocPayload,
    pub rpf_entry_binding: Option<RpfEntryBinding>,
    pub rpf9_source: Option<rpf9::entry::EntrySource>,
    pub dirty: bool,
    /// The workspace this document was in front of last, which its tab brings back.
    pub seen_in: Option<crate::ui::dock::Workspace>,
    /// Why no view may change or save this document, when none may.
    pub read_only: Option<&'static str>,
    pub resource: Option<Arc<ResourceImage>>,
    patched: bool,
    /// The image holds an edit the payload has not been read back from: a drag writes the image each frame and is read once it ends.
    unread: bool,
    edits: u64,
    payload_epoch: u64,
    /// Captured on first ask and dropped on save; see [`MaterialBaseline`].
    material_baseline: Option<MaterialBaseline>,
    /// The size of the image the payload was decoded from, which stands for the payload once the image goes.
    decoded_bytes: usize,
    rebuild: image::Rebuild,
    pub history: super::EditHistory,
}

impl Document {
    pub fn new(id: DocumentId, path: Option<PathBuf>, payload: DocPayload) -> Self {
        let kind = payload.kind();
        let name = path
            .as_ref()
            .and_then(|p| p.file_stem().and_then(|s| s.to_str()))
            .unwrap_or(tr!("Untitled"))
            .to_string();

        let read_only = path
            .as_deref()
            .filter(|path| rpf9::entry::is_entry(path))
            .map(|_| rpf9::entry::READ_ONLY);
        Self {
            id,
            path,
            name,
            kind,
            payload,
            rpf_entry_binding: None,
            rpf9_source: None,
            dirty: false,
            seen_in: None,
            read_only,
            resource: None,
            patched: false,
            unread: false,
            edits: 0,
            payload_epoch: 0,
            material_baseline: None,
            decoded_bytes: 0,
            rebuild: Default::default(),
            history: Default::default(),
        }
    }

    pub fn is_from_rpf9_table(&self) -> bool {
        self.rpf9_source.is_some() || self.read_only == Some(rpf9::entry::READ_ONLY)
    }

    pub fn edit_epoch(&self) -> u64 {
        self.edits
    }

    /// The material baseline, captured on first ask.
    ///
    /// ⚠ **First ask is before any material edit and that is what makes it
    /// exact**: the editor is the only thing that mutates a material, and it
    /// calls this to draw before it can offer a change.
    pub fn material_baseline(&mut self) -> &MaterialBaseline {
        self.material_baseline
            .get_or_insert_with(|| MaterialBaseline::of(&self.payload))
    }

    pub fn with_resource(mut self, resource: Option<Arc<ResourceImage>>) -> Self {
        self.set_resource(resource);
        self
    }

    pub fn set_resource(&mut self, resource: Option<Arc<ResourceImage>>) {
        self.opened_as(resource.as_deref());
        self.decoded_bytes = resource.as_ref().map_or(0, |image| image.data.len());
        self.resource = resource;
        // ⚠ A new image comes with a payload decoded from it, as a live reload's does.
        self.payload_epoch = self.payload_epoch.wrapping_add(1);
    }

    /// The bytes this document holds on the CPU: the image it was read from, and its payload.
    pub fn held_bytes(&self) -> usize {
        let image = self.resource.as_ref().map_or(0, |image| image.data.len());
        let payload = match &self.payload {
            DocPayload::Text(text) | DocPayload::NameTable(text) => text.text.len_bytes(),
            DocPayload::Meta(meta) => meta.text.text.len_bytes() + meta.raw_bytes.len(),
            DocPayload::Bik(bik) => bik.bytes.len(),
            DocPayload::Hex(hex) => hex.bytes.len(),
            DocPayload::Compare(compared) => compared.held_bytes(),
            DocPayload::Awc(awc) => awc.held_bytes(),
            DocPayload::HeightMap(map) => map.max.len() + map.min.len() + map.water_mask.len(),
            DocPayload::DistantLights(lights) => {
                std::mem::size_of_val(lights.points.as_slice())
                    + std::mem::size_of_val(lights.groups.as_slice())
                    + 4 * 3 * lights.block_first_group.len()
            }
            DocPayload::WaterMap(map) => {
                std::mem::size_of_val(map.tiles.as_slice())
                    + std::mem::size_of_val(map.ref_ids.as_slice())
                    + std::mem::size_of_val(map.pools.as_slice())
                    + 16 * (map.river_points.len() + map.lake_boxes.len())
            }
            DocPayload::WorldBlanket(blanket) => 2 * blanket.cells.len(),
            DocPayload::NavPrecalc(precalc) => {
                precalc.bits.len()
                    + precalc
                        .cells
                        .iter()
                        .map(|c| 8 + 4 * c.entries())
                        .sum::<usize>()
            }
            DocPayload::DatTable(table) => table.held_bytes(),
            DocPayload::Rel(rel) => rel.held_bytes(),
            DocPayload::Gfx(movie) => movie.tags.iter().map(|t| t.body.len()).sum(),
            DocPayload::MoveNetwork(network) => {
                network.nodes.len() * std::mem::size_of::<or_io::mrf::MvNodeDef>()
            }
            DocPayload::Dds(texture) => texture.backing_store.len(),
            DocPayload::Ttpf(ttpf) => {
                std::mem::size_of_val(ttpf.pack.rows.as_slice())
                    + std::mem::size_of_val(ttpf.pack.entries.as_slice())
            }
            DocPayload::Fxc(fxc) => fxc.bytes.len(),
            DocPayload::Gxt2(gxt2) => gxt2
                .entries
                .iter()
                .map(|e| std::mem::size_of_val(e) + e.value.len())
                .sum(),
            DocPayload::Rpf(rpf) => rpf.session.held_bytes(),
            DocPayload::Rpf9(scan) => scan.held_bytes(),
            DocPayload::Rpf9Table(table) => table.held_bytes(),
            DocPayload::Resource(asset) => asset.heap_bytes(),
            // Counted as the image it was decoded from: nothing walks these values' heap.
            DocPayload::Ypdb(_)
            | DocPayload::Registered { .. }
            | DocPayload::Gta6Anim(_)
            | DocPayload::Gta6Drawable(_)
            | DocPayload::Gta6Expressions(_) => self.decoded_bytes,
            DocPayload::Gta6Text(text) => text.held_bytes(),
            DocPayload::Gta6AudioData(audio) => audio.held_bytes(),
            DocPayload::Gta6Save(save) => save.held_bytes(),
        };
        image + payload
    }

    /// Mark an edit no step of the history holds, which leaves nothing before it to undo.
    ///
    /// ⚠ Reads back an image edit the payload lacks before the image goes, so
    /// a payload changed before this call while one was unread is read over:
    /// catch up before changing it, as `bake_anim_edits` does.
    pub fn mark_edited(&mut self) {
        self.catch_up();
        self.history.clear();
        self.resource = None;
        self.touch(false);
    }

    /// Mark `archive` written, with every archive nested in it; an install or a folder is saved once no other archive is left.
    pub fn archive_written(&mut self, archive: ArchiveId) {
        if let DocPayload::Rpf(rpf) = &mut self.payload {
            rpf.session.clear_archive_dirty(archive);
        }
        // ⚠ A row put back after the write would read an entry the file no longer holds there.
        if self.history.reads_an_archive() {
            self.history.clear();
        }
        if self.unwritten_archives() == Some(false) {
            self.dirty = false;
            self.history.saved_at(self.history.at());
        }
    }

    /// Whether an install's or a folder's tree holds an archive not yet written; `None` for any other document.
    ///
    /// ⚠ Such a tree writes a disk edit as it happens, so a delete, rename or
    /// move on disk leaves it nothing to save.
    fn unwritten_archives(&self) -> Option<bool> {
        match &self.payload {
            DocPayload::Rpf(rpf)
                if matches!(self.kind, DocumentKind::GameRoot | DocumentKind::Folder) =>
            {
                Some(!rpf.session.dirty_archive_ids().is_empty())
            }
            _ => None,
        }
    }

    /// Hand the resource image to `edit`; `true` from it marks the document patched, its payload to be read back, and keeps the image as it was in a step called `label` that Undo puts back.
    ///
    /// ⚠ One call is one step, whatever `edit` changed in it: a run that patches several fields and replaces the file is undone whole, and a run that changed nothing makes none.
    pub fn patch_resource_image(
        &mut self,
        label: &str,
        edit: impl FnOnce(&mut ResourceImage) -> bool,
    ) -> bool {
        let Some(image) = self.resource.as_mut() else {
            return false;
        };
        let before = Arc::clone(image);
        if !edit(Arc::make_mut(image)) {
            return false;
        }
        self.record(label.to_string(), Change::Image { other: before });
        true
    }

    pub fn patched(&self) -> bool {
        self.patched && self.resource.is_some()
    }

    /// ⚠ Drops the material baseline: after a save the file is what the
    /// payload holds, and load-time values would make "reset to the file's
    /// value" undo work the user already committed.
    pub fn mark_saved(&mut self) {
        self.opened_as(None);
        self.dirty = false;
        self.patched = false;
        self.material_baseline = None;
        if self.history.reads_an_archive() {
            self.history.clear();
        }
        self.history.saved_at(self.history.at());
    }

    pub fn new_game_root(id: DocumentId, install_root: PathBuf, doc: RpfDocument) -> Self {
        let name = install_root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("GTA V")
            .to_string();
        Self {
            name,
            kind: DocumentKind::GameRoot,
            ..Self::new(id, Some(install_root), DocPayload::Rpf(doc))
        }
    }

    pub fn new_folder(id: DocumentId, folder: PathBuf, doc: RpfDocument) -> Self {
        let name = folder
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(tr!("Folder"))
            .to_string();
        Self {
            name,
            kind: DocumentKind::Folder,
            ..Self::new(id, Some(folder), DocPayload::Rpf(doc))
        }
    }

    pub fn title(&self) -> String {
        if self.dirty {
            format!("{} *", self.name)
        } else {
            self.name.clone()
        }
    }

    /// Its file's name, extension included, or its name while it has no file.
    pub fn file_name(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(|| self.name.clone(), |n| n.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_enhanced_payload_reports_gen9() {
        for payload in [
            DocPayload::Resource(or_io::Asset::TextureDictGen9(Default::default())),
            DocPayload::Resource(or_io::Asset::DrawableGen9(Default::default())),
            DocPayload::Resource(or_io::Asset::DrawableDictGen9(Default::default())),
            DocPayload::Resource(or_io::Asset::FragmentGen9(Default::default())),
        ] {
            assert!(
                payload.generation().is_gen9(),
                "{:?} holds Enhanced data and must say so",
                payload.kind()
            );
            assert!(payload.is_enhanced());
        }
    }

    /// A drawable document holding a zeroed resource image.
    fn probe_document() -> Document {
        let image = Arc::new(ResourceImage {
            data: vec![0u8; 64],
            virtual_size: 32,
            seal: {
                use or_io::container::resource::{ResourceEnvelope, Rsc7};
                Rsc7::peek(&Rsc7::bare_header(165)).expect("a bare header parses")
            },
        });
        Document::new(
            1,
            Some(PathBuf::from("probe.ydr")),
            DocPayload::Resource(or_io::Asset::Drawable(Default::default())),
        )
        .with_resource(Some(image))
    }

    #[test]
    fn a_document_counts_its_image_and_what_its_value_allocates() {
        let mut doc = probe_document();
        let DocPayload::Resource(asset) = &doc.payload else {
            unreachable!()
        };
        let value = asset.heap_bytes();
        assert_eq!(doc.held_bytes(), 64 + value);
        doc.mark_edited();
        assert_eq!(
            doc.held_bytes(),
            value,
            "the image goes on an edit and the value stays"
        );
    }

    #[test]
    fn an_untitled_resource_counts_its_runs_as_they_grow() {
        let dict = or_io::gta5::CrFrameFilterDictionary {
            codes: vec![0; 256],
            ..Default::default()
        };
        let mut doc = Document::new(
            1,
            None,
            DocPayload::Resource(or_io::Asset::FrameFilterDict(dict)),
        );
        let held = doc.held_bytes();
        assert!(held >= 256 * std::mem::size_of::<u32>(), "{held} bytes");
        if let DocPayload::Resource(or_io::Asset::FrameFilterDict(dict)) = &mut doc.payload {
            dict.codes.reserve_exact(1024);
        }
        assert!(doc.held_bytes() >= held + 1024 * std::mem::size_of::<u32>());
    }

    #[test]
    fn editing_a_document_drops_the_image_it_was_read_from() {
        let mut doc = probe_document();

        assert!(
            doc.resource.is_some(),
            "a freshly opened document keeps them"
        );
        assert!(!doc.dirty);

        doc.mark_edited();

        assert!(doc.dirty);
        assert!(
            doc.resource.is_none(),
            "the image describes the file on disk, which the payload no longer matches"
        );
    }

    #[test]
    fn a_patch_is_one_step_that_undo_takes_back_and_redo_makes_again() {
        use crate::state::edit_history::EditDirection;
        let mut doc = probe_document();
        let before = doc.resource.as_ref().expect("an image").data.clone();

        assert!(
            doc.patch_resource_image("A plugin changed the document", |image| {
                image.data[0] = 7;
                image.data.push(9);
                true
            })
        );
        let after = doc.resource.as_ref().expect("an image").data.clone();
        assert_ne!(before, after);
        let labels: Vec<&str> = doc
            .history
            .done()
            .iter()
            .map(|s| s.label.as_str())
            .collect();
        assert_eq!(
            labels,
            ["A plugin changed the document"],
            "one run is one step"
        );

        let mut step = doc
            .history
            .take(EditDirection::Undo)
            .expect("a step to undo");
        doc.flip(&mut step.change).expect("it flips");
        doc.stepped(step, EditDirection::Undo);
        assert_eq!(doc.resource.as_ref().unwrap().data, before);

        let mut step = doc
            .history
            .take(EditDirection::Redo)
            .expect("a step to redo");
        doc.flip(&mut step.change).expect("it flips back");
        doc.stepped(step, EditDirection::Redo);
        assert_eq!(doc.resource.as_ref().unwrap().data, after);
    }

    #[test]
    fn a_refused_patch_leaves_the_document_clean() {
        let mut doc = probe_document();
        let too_wide = or_io::desc::ScalarValue::Int(70_000);
        let u16 = or_io::desc::Scalar::Int(or_io::desc::IntWidth::U16);
        assert!(!doc.patch_resource_image("A plugin changed it", |image| {
            image.view_mut().put_scalar(4, u16, too_wide).is_ok()
        }));
        assert!(doc.history.done().is_empty(), "a refused write is no step");
        assert!(!doc.dirty, "a refused write marked the document dirty");
        assert!(
            !doc.patched(),
            "a refused write marked the document patched"
        );
        let bytes = &doc.resource.as_ref().expect("the image is kept").data;
        assert!(bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn mark_edited_is_the_only_way_a_document_becomes_dirty() {
        let root = or_test_support::workspace_root(env!("CARGO_MANIFEST_DIR"))
            .expect("the app crate sits in the workspace")
            .join("src");
        let mut offenders = Vec::new();
        walk_rust_files(&root, &mut |path, text| {
            for (n, line) in text.lines().enumerate() {
                let line = line.trim();
                if !line.ends_with(".dirty = true;") {
                    continue;
                }
                if line.starts_with("self.dirty") || line.starts_with("slot.dirty") {
                    continue;
                }
                offenders.push(format!("{}:{}: {line}", path.display(), n + 1));
            }
        });
        assert!(
            offenders.is_empty(),
            "these set a document dirty without dropping its resource image; \
             call `Document::mark_edited` instead:\n{}",
            offenders.join("\n")
        );
    }

    #[cfg(test)]
    fn walk_rust_files(dir: &std::path::Path, f: &mut impl FnMut(&std::path::Path, &str)) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_rust_files(&path, f);
            } else if path.extension().is_some_and(|e| e == "rs") {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    f(&path, &text);
                }
            }
        }
    }

    #[test]
    fn every_resource_round_trips_through_the_resource_type_mapping() {
        for resource in or_io::container::resource::ResourceType::ALL {
            let kind = DocumentKind::for_resource(resource);
            assert_eq!(
                kind.resource_type(),
                Some(resource),
                "{kind:?} came from {resource:?} and should map back to it"
            );
        }
    }

    #[test]
    fn every_owned_row_is_its_own_kind() {
        use or_io::container::resource::ResourceType as R;
        for (asset, resource) in [
            (or_io::Asset::FrameFilterDict(Default::default()), R::Yfd),
            (or_io::Asset::ExpressionDict(Default::default()), R::Yed),
            (or_io::Asset::PathRegion(Default::default()), R::Ynd),
            (or_io::Asset::Clips(Default::default()), R::Ycd),
            (
                or_io::Asset::Bounds(or_io::gta5::PhBoundAny::PhBoundSphere(Default::default())),
                R::Ybn,
            ),
            (or_io::Asset::BoundsDict(Default::default()), R::Ybd),
            (or_io::Asset::ClothDict(Default::default()), R::Yld),
            (or_io::Asset::NavMesh(Default::default()), R::Ynv),
        ] {
            let payload = DocPayload::Resource(asset);
            assert_eq!(payload.kind(), DocumentKind::for_resource(resource));
            assert_eq!(payload.kind().resource_type(), Some(resource));
            assert_eq!(payload.generation(), or_io::Generation::Gen8);
        }
    }

    #[test]
    fn a_builtin_kind_serializes_as_the_name_the_derive_wrote() {
        for kind in [
            DocumentKind::Ypt,
            DocumentKind::Meta,
            DocumentKind::GameRoot,
            DocumentKind::Gxt2,
        ] {
            let json = serde_json::to_string(&kind).expect("serialize");
            assert_eq!(json, format!("\"{}\"", kind.variant_name()));
            assert_eq!(
                serde_json::from_str::<DocumentKind>(&json).expect("deserialize"),
                kind
            );
        }
    }

    #[test]
    fn a_registered_kind_round_trips_under_its_own_prefix() {
        let kind = DocumentKind::Registered(or_io::container::resource::ResourceType::Yed);
        let json = serde_json::to_string(&kind).expect("serialize");
        assert_eq!(json, "\"registered:yed\"");
        assert_eq!(
            serde_json::from_str::<DocumentKind>(&json).expect("deserialize"),
            kind
        );

        assert!(DocumentKind::from_variant_name("registered:yed").is_none());
        assert!(serde_json::from_str::<DocumentKind>("\"registered:nope\"").is_err());
        assert!(serde_json::from_str::<DocumentKind>("\"Nonsense\"").is_err());
    }

    #[test]
    fn only_paged_resources_claim_a_resource_type() {
        // The app's own kinds are containers and text, not RSC7 resources, and
        // a pack cannot read any of them. Saying so here is what keeps
        // `resource_image_for` from trying.
        for kind in [
            DocumentKind::Gxt2,
            DocumentKind::Rpf,
            DocumentKind::Text,
            DocumentKind::Meta,
            DocumentKind::GameRoot,
            DocumentKind::Folder,
        ] {
            assert!(
                kind.resource_type().is_none(),
                "{kind:?} is not a paged resource"
            );
        }
    }

    #[test]
    fn a_non_resource_never_yields_an_image_however_convincing_the_bytes() {
        // `.rpf` opens with the same RSC7-shaped magic check further along, so
        // the guard has to be the kind rather than the bytes.
        let rsc7 = {
            let mut bytes = vec![0u8; 64];
            bytes[0..4].copy_from_slice(&0x3743_5352u32.to_le_bytes());
            bytes
        };
        for kind in [DocumentKind::Rpf, DocumentKind::Text, DocumentKind::Meta] {
            assert!(
                crate::state::document_io::resource_image_for(kind, &rsc7).is_none(),
                "{kind:?} should never keep a resource image"
            );
        }
    }
}
