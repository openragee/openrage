//! Where the user has been in the explorer, and getting them back there:
//! back/forward, breadcrumbs, and opening a favourite.

use std::path::{Path, PathBuf};

use or_io::container::rpf::BodyForm;

use super::document::{DocumentId, RpfNodeId};
use super::explorer::Place;
use super::extract_job::TypeFilter;
use super::rpf_session::RpfEditSession;
use super::{AppState, DocPayload, LoadingState};
use crate::ui::panels::selection_ids::DOC_ROOT_SEL;

/// A row in one document: an archive entry, or the document itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub doc: DocumentId,
    pub node: u64,
}

impl Location {
    pub fn document(doc: DocumentId) -> Self {
        Self {
            doc,
            node: DOC_ROOT_SEL,
        }
    }
}

const MAX_HISTORY: usize = 100;

/// What a walk does once it has reached its rows.
#[derive(Debug)]
pub enum Arrival {
    /// Select the first row and scroll the tree to it.
    Reveal,
    /// Open the first row's file as a double click on its row does; a folder or archive is revealed.
    Open,
    /// Extract every row reached through the extraction job.
    Extract {
        dest: PathBuf,
        form: BodyForm,
        filter: TypeFilter,
    },
    /// Write what a batch makes of every row reached into a folder, as one job.
    Batch {
        kind: super::batch::Batch,
        folder: PathBuf,
    },
}

/// Places in one source a walk is on its way to, each with the row it reached.
#[derive(Debug)]
struct Walk {
    to: Vec<(Place, Option<RpfNodeId>)>,
    then: Arrival,
}

#[derive(Debug, Default)]
pub struct Navigation {
    back: Vec<Location>,
    forward: Vec<Location>,
    current: Option<Location>,
    /// A row the tree should open its way down to and scroll to on the next
    /// frame it draws.
    pub reveal: Option<Location>,
    /// Places whose document is still loading, or whose path runs through an
    /// archive nobody has expanded yet.
    pending: Option<Walk>,
}

impl Navigation {
    /// Record arriving somewhere. Arriving where the user already is records
    /// nothing, so a double-click is one step and not two.
    pub fn visit(&mut self, to: Location) {
        if self.current == Some(to) {
            return;
        }
        if let Some(from) = self.current.replace(to) {
            self.back.push(from);
            if self.back.len() > MAX_HISTORY {
                self.back.remove(0);
            }
        }
        self.forward.clear();
    }

    /// Where the user is now: what was last visited, gone back to or gone forward to.
    pub fn current(&self) -> Option<Location> {
        self.current
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Step back to the most recent location `exists` still accepts.
    ///
    /// ⚠ **A location can stop existing** — its archive entry deleted, its
    /// document closed — and history has no business sending the user there.
    /// Those are dropped on the way past rather than stopping at a row that
    /// is not in the tree.
    pub fn back(&mut self, exists: impl Fn(Location) -> bool) -> Option<Location> {
        while let Some(to) = self.back.pop() {
            if exists(to) {
                if let Some(from) = self.current.replace(to) {
                    self.forward.push(from);
                }
                return Some(to);
            }
        }
        None
    }

    pub fn forward(&mut self, exists: impl Fn(Location) -> bool) -> Option<Location> {
        while let Some(to) = self.forward.pop() {
            if exists(to) {
                if let Some(from) = self.current.replace(to) {
                    self.back.push(from);
                }
                return Some(to);
            }
        }
        None
    }

    /// Drop every location in a closed document.
    pub fn forget(&mut self, doc: DocumentId) {
        self.back.retain(|l| l.doc != doc);
        self.forward.retain(|l| l.doc != doc);
        if self.current.is_some_and(|l| l.doc == doc) {
            self.current = None;
        }
        if self.reveal.is_some_and(|l| l.doc == doc) {
            self.reveal = None;
        }
    }

    pub fn pending_place(&self) -> Option<&Place> {
        let walk = self.pending.as_ref()?;
        walk.to.first().map(|(place, _)| place)
    }
}

/// What walking a path through a session found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    Found(RpfNodeId),
    /// The walk reached a folder or archive whose children are not loaded;
    /// expanding it may let the walk continue.
    Unloaded(RpfNodeId),
    Missing,
}

/// Follow a `/`-separated path from the root, matching names without regard
/// to case the way the game's own file system does.
pub fn resolve_path(session: &RpfEditSession, inner: &str) -> Resolved {
    let mut at = session.root_id();
    for segment in inner.split('/').filter(|s| !s.is_empty()) {
        let next = session.children_of(at).into_iter().find(|&id| {
            session
                .node(id)
                .is_some_and(|n| n.name.eq_ignore_ascii_case(segment))
        });
        match next {
            Some(id) => at = id,
            None if session.needs_expansion(at) => return Resolved::Unloaded(at),
            None => return Resolved::Missing,
        }
    }
    Resolved::Found(at)
}

/// One step of a breadcrumb trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub to: Location,
}

impl AppState {
    /// Go somewhere as though the user had clicked it, and remember it.
    pub fn navigate_to(&mut self, to: Location) {
        if self.show_location(to) {
            self.navigation.visit(to);
        }
    }

    pub fn navigate_back(&mut self) -> bool {
        let mut nav = std::mem::take(&mut self.navigation);
        let to = nav.back(|l| self.location_exists(l));
        self.navigation = nav;
        to.is_some_and(|l| self.show_location(l))
    }

    pub fn navigate_forward(&mut self) -> bool {
        let mut nav = std::mem::take(&mut self.navigation);
        let to = nav.forward(|l| self.location_exists(l));
        self.navigation = nav;
        to.is_some_and(|l| self.show_location(l))
    }

    fn location_exists(&self, at: Location) -> bool {
        let Some(doc) = self.document(at.doc) else {
            return false;
        };
        if at.node == DOC_ROOT_SEL {
            return true;
        }
        match &doc.payload {
            DocPayload::Rpf(rpf) => rpf.session.node(at.node).is_some(),
            _ => true,
        }
    }

    /// Select a location and ask the tree to open its way down to it.
    ///
    /// ⚠ An archive entry that is already open as its own document hands the
    /// focus to that document, because that is what clicking the row does.
    fn show_location(&mut self, at: Location) -> bool {
        if !self.location_exists(at) {
            return false;
        }
        let bound = self.document(at.doc).and_then(|d| match &d.payload {
            DocPayload::Rpf(rpf) => rpf.entry_documents.get(&at.node).copied(),
            _ => None,
        });
        self.activate_if_shown(at.doc);
        if let Some(bound) = bound {
            self.activate_if_shown(bound);
        }
        self.selection.select(at.doc, at.node);
        self.navigation.reveal = Some(at);
        // A row of a scan's list shows its body, as a click on it does: its archive's own tab
        // is of a workspace the body's is not, so the row alone would change nothing on screen.
        if at.node != DOC_ROOT_SEL {
            self.show_scanned_row(at.doc, at.node);
        }
        true
    }

    /// Make `doc` the active document if the workspace on screen shows its kind.
    pub fn activate_if_shown(&mut self, doc: DocumentId) {
        // ⚠ Moving through Files never switches the workspace: Files sits beside
        // every workspace, and where this one cannot show `doc` the document on screen stays.
        if self
            .document(doc)
            .is_some_and(|d| self.active_workspace.handles_kind(d.kind))
        {
            self.set_active_document(doc);
        }
    }

    /// The path to what is selected in the active document, each step a place
    /// to go.
    ///
    /// ⚠ **A document opened out of an archive is a step inside that archive,
    /// not a root of its own**, so its trail starts at the archive — which is
    /// the one thing the user cannot otherwise see once the entry has its own
    /// tab.
    pub fn breadcrumb_trail(&self) -> Vec<Crumb> {
        let Some(active) = self.active_document() else {
            return Vec::new();
        };
        let (doc, node) = match active.rpf_entry_binding {
            Some(binding) => (binding.parent_doc_id, Some(binding.entry_id)),
            None => (active.id, self.selection.selected(active.id)),
        };
        let Some(document) = self.document(doc) else {
            return Vec::new();
        };
        let mut trail = vec![Crumb {
            label: document.title(),
            to: Location::document(doc),
        }];
        let DocPayload::Rpf(rpf) = &document.payload else {
            return trail;
        };
        let Some(mut at) = node.filter(|&n| rpf.session.node(n).is_some()) else {
            return trail;
        };
        let mut steps = Vec::new();
        while let Some(n) = rpf.session.node(at) {
            let Some(parent) = n.parent else {
                break;
            };
            steps.push(Crumb {
                label: n.display_name().into_owned(),
                to: Location { doc, node: at },
            });
            at = parent;
        }
        steps.reverse();
        trail.extend(steps);
        trail
    }

    /// The place a row is, if it can be named well enough to come back to.
    pub fn place_of(&self, at: Location) -> Option<Place> {
        let doc = self.document(at.doc)?;
        if let Some(binding) = doc.rpf_entry_binding {
            return self.place_of(Location {
                doc: binding.parent_doc_id,
                node: binding.entry_id,
            });
        }
        let source = doc.path.as_deref()?;
        let inner = match (&doc.payload, at.node) {
            (_, DOC_ROOT_SEL) => String::new(),
            (DocPayload::Rpf(rpf), node) => rpf.session.path_of(node)?,
            _ => String::new(),
        };
        Place::new(source, &inner)
    }

    /// Select a file on disk in the tree that holds it, or open it as a document of its own.
    pub fn reveal_disk_path(&mut self, path: &Path) {
        let held = self
            .documents
            .iter()
            .filter(|d| d.rpf_entry_binding.is_none() && matches!(d.payload, DocPayload::Rpf(_)))
            .filter_map(|d| d.path.as_deref())
            .find_map(|root| {
                let inner = path.strip_prefix(root).ok()?.to_str()?.replace('\\', "/");
                Place::new(root, &inner)
            });
        if let Some(place) = held.or_else(|| Place::new(path, "")) {
            self.open_place(place);
        }
    }

    /// Open a favourite: its document if that is not open, and then the path
    /// inside it as far as it has been loaded.
    pub fn open_place(&mut self, place: Place) {
        let source = PathBuf::from(&place.source);
        self.walk_to(&source, vec![place.inner], Arrival::Reveal);
    }

    /// Go to `inners` in `source`, opening it and every archive on the way as the tree does, then do what `then` says.
    pub fn walk_to(&mut self, source: &Path, inners: Vec<String>, then: Arrival) {
        let to: Vec<(Place, Option<RpfNodeId>)> = inners
            .iter()
            .filter_map(|inner| Place::new(source, inner))
            .map(|place| (place, None))
            .collect();
        if to.is_empty() {
            return;
        }
        if self.document_for_source(source).is_none() {
            if let Some(message) = self.open_source(source) {
                self.show_error_toast(message);
                return;
            }
        }
        self.navigation.pending = Some(Walk { to, then });
        self.tick_navigation();
    }

    /// Move a pending walk one step closer: find its document, walk each path,
    /// and expand whatever a path stopped at; once no path waits on an
    /// expansion, do what the walk is for with the rows it reached.
    ///
    /// Called once a frame, so a row three nested archives down arrives as
    /// each expansion lands rather than needing the user to click again.
    pub fn tick_navigation(&mut self) {
        let Some(mut walk) = self.navigation.pending.take() else {
            return;
        };
        // ⚠ An archive read while the keys are still being read fails for want of a key, and the place inside it then reads as gone: the walk waits for them.
        if self.keys.extraction.is_running && !self.keys.has_keys() {
            self.navigation.pending = Some(walk);
            return;
        }
        let Some(source) = walk
            .to
            .first()
            .map(|(place, _)| PathBuf::from(&place.source))
        else {
            return;
        };
        let Some(doc_id) = self.document_for_source(&source) else {
            if matches!(self.loading, LoadingState::Loading) {
                self.navigation.pending = Some(walk);
            }
            return;
        };
        let mut waiting = false;
        walk.to.retain_mut(|(place, reached)| {
            if reached.is_some() {
                return true;
            }
            let resolved = match self.document(doc_id).map(|d| &d.payload) {
                Some(DocPayload::Rpf(rpf)) => resolve_path(&rpf.session, &place.inner),
                Some(_) if place.inner.is_empty() => Resolved::Found(DOC_ROOT_SEL),
                _ => Resolved::Missing,
            };
            match resolved {
                Resolved::Found(node) => {
                    *reached = Some(if place.inner.is_empty() {
                        DOC_ROOT_SEL
                    } else {
                        node
                    });
                    true
                }
                Resolved::Unloaded(node) => {
                    if !self.expander.is_in_flight(doc_id, node) {
                        self.request_expand(doc_id, node);
                    }
                    let expanding = self.expander.is_in_flight(doc_id, node);
                    if !expanding {
                        self.show_warning_toast(format!("{} could not be opened", place.label()));
                    }
                    waiting |= expanding;
                    expanding
                }
                Resolved::Missing => {
                    self.show_warning_toast(format!("{} no longer exists", place.label()));
                    false
                }
            }
        });
        if waiting {
            self.navigation.pending = Some(walk);
            return;
        }
        let rows: Vec<RpfNodeId> = walk.to.iter().filter_map(|(_, row)| *row).collect();
        match walk.then {
            Arrival::Reveal => {
                let node = rows.first().copied().unwrap_or(DOC_ROOT_SEL);
                let archive = matches!(
                    self.document(doc_id).map(|d| &d.payload),
                    Some(DocPayload::Rpf(_))
                );
                // ⚠ A file of its own has no row in Files, so revealing it shows it.
                if node == DOC_ROOT_SEL && !archive {
                    self.set_active_document(doc_id);
                }
                self.navigate_to(Location { doc: doc_id, node });
            }
            Arrival::Open => {
                let Some(&node) = rows.first() else {
                    return;
                };
                // ⚠ The source itself, such as a closed tab reopened, has no row to open.
                if node == DOC_ROOT_SEL {
                    self.set_active_document(doc_id);
                    return;
                }
                let at = Location { doc: doc_id, node };
                let folder = match self.document(doc_id).map(|d| &d.payload) {
                    Some(DocPayload::Rpf(rpf)) => {
                        rpf.session.node(node).is_some_and(|n| n.is_dir())
                    }
                    _ => false,
                };
                if folder {
                    self.navigate_to(at);
                } else {
                    // ⚠ As the tree's double click does, and not `navigate_to`: making the
                    // archive active first switches the workspace away and back while the file loads.
                    self.selection.select(doc_id, node);
                    self.navigation.visit(at);
                    self.open_rpf_entry(doc_id, node);
                }
            }
            Arrival::Extract { dest, form, filter } => {
                if !rows.is_empty() {
                    self.extract_rpf_nodes(doc_id, rows, dest, form, filter);
                }
            }
            Arrival::Batch { kind, folder } => {
                if !rows.is_empty() {
                    self.batch_export(doc_id, &rows, kind, folder);
                }
            }
        }
    }

    fn document_for_source(&self, source: &Path) -> Option<DocumentId> {
        self.documents
            .iter()
            .find(|d| d.rpf_entry_binding.is_none() && d.path.as_deref() == Some(source))
            .map(|d| d.id)
    }

    /// Start opening whatever `source` is. `Some` is why it could not be.
    fn open_source(&mut self, source: &Path) -> Option<String> {
        if self.prefs.active_game_root().map(PathBuf::as_path) == Some(source) {
            self.open_game_root(source.to_path_buf());
            return None;
        }
        if source.is_dir() {
            self.open_folder(source.to_path_buf());
            return None;
        }
        if !source.is_file() {
            return Some(format!("{} is not on disk any more", source.display()));
        }
        self.load_file(source.to_path_buf());
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DocumentKind;

    fn at(doc: DocumentId, node: u64) -> Location {
        Location { doc, node }
    }

    #[test]
    fn back_restores_the_prior_location_and_forward_undoes_it() {
        let mut nav = Navigation::default();
        nav.visit(at(1, 10));
        nav.visit(at(1, 20));
        nav.visit(at(2, 30));

        assert_eq!(nav.back(|_| true), Some(at(1, 20)));
        assert_eq!(nav.back(|_| true), Some(at(1, 10)));
        assert_eq!(
            nav.back(|_| true),
            None,
            "there is nothing before the first"
        );
        assert_eq!(nav.current(), Some(at(1, 10)));

        assert_eq!(nav.forward(|_| true), Some(at(1, 20)));
        assert_eq!(nav.forward(|_| true), Some(at(2, 30)));
        assert_eq!(nav.forward(|_| true), None);
    }

    #[test]
    fn going_somewhere_new_after_going_back_drops_the_forward_branch() {
        let mut nav = Navigation::default();
        nav.visit(at(1, 10));
        nav.visit(at(1, 20));
        nav.back(|_| true);
        nav.visit(at(1, 99));
        assert!(!nav.can_go_forward());
        assert_eq!(nav.back(|_| true), Some(at(1, 10)));
    }

    #[test]
    fn revisiting_where_you_are_is_not_a_step() {
        let mut nav = Navigation::default();
        nav.visit(at(1, 10));
        nav.visit(at(1, 10));
        assert!(!nav.can_go_back());
    }

    #[test]
    fn a_location_that_stopped_existing_is_stepped_over() {
        let mut nav = Navigation::default();
        nav.visit(at(1, 10));
        nav.visit(at(1, 20));
        nav.visit(at(1, 30));
        assert_eq!(nav.back(|l| l.node != 20), Some(at(1, 10)));
    }

    #[test]
    fn closing_a_document_forgets_every_location_in_it() {
        let mut nav = Navigation::default();
        nav.visit(at(1, 10));
        nav.visit(at(2, 20));
        nav.visit(at(1, 30));
        nav.forget(1);
        assert_eq!(nav.current(), None);
        assert_eq!(nav.back(|_| true), Some(at(2, 20)));
        assert_eq!(nav.back(|_| true), None);
    }

    #[test]
    fn history_is_bounded() {
        let mut nav = Navigation::default();
        for node in 0..(MAX_HISTORY as u64 + 50) {
            nav.visit(at(1, node));
        }
        let mut steps = 0;
        while nav.back(|_| true).is_some() {
            steps += 1;
        }
        assert_eq!(steps, MAX_HISTORY);
    }

    fn session() -> (RpfEditSession, RpfNodeId, RpfNodeId) {
        let mut s = RpfEditSession::empty();
        let root = s.root_id();
        let levels = s.add_folder(root, "levels").unwrap();
        let gta5 = s.add_folder(levels, "gta5").unwrap();
        let file = s.add_file_copy(gta5, "Props.ydr", vec![1]).unwrap();
        (s, gta5, file)
    }

    #[test]
    fn a_path_resolves_without_regard_to_case() {
        let (s, gta5, file) = session();
        assert_eq!(resolve_path(&s, "LEVELS/gta5"), Resolved::Found(gta5));
        assert_eq!(
            resolve_path(&s, "/levels/gta5/props.YDR"),
            Resolved::Found(file)
        );
        assert_eq!(resolve_path(&s, ""), Resolved::Found(s.root_id()));
        assert_eq!(resolve_path(&s, "levels/nope"), Resolved::Missing);
    }

    fn state_with_archive() -> (AppState, DocumentId, RpfNodeId, RpfNodeId) {
        let (session, gta5, file) = session();
        let mut state = AppState::default();
        let id = state.new_empty_document(DocumentKind::Rpf);
        let doc = state.document_mut(id).unwrap();
        doc.path = Some(PathBuf::from("C:/game/x64a.rpf"));
        let DocPayload::Rpf(rpf) = &mut doc.payload else {
            panic!("an empty RPF document holds an RPF payload");
        };
        rpf.session = session;
        (state, id, gta5, file)
    }

    #[test]
    fn back_and_forward_move_the_selection_and_ask_the_tree_to_reveal() {
        let (mut state, doc, gta5, file) = state_with_archive();
        state.navigate_to(at(doc, gta5));
        state.navigate_to(at(doc, file));
        assert_eq!(state.selection.selected(doc), Some(file));

        assert!(state.navigate_back());
        assert_eq!(state.selection.selected(doc), Some(gta5));
        assert_eq!(state.navigation.reveal, Some(at(doc, gta5)));
        assert_eq!(state.active_document, Some(doc));

        assert!(state.navigate_forward());
        assert_eq!(state.selection.selected(doc), Some(file));
    }

    /// ⚠ Files is beside every workspace now, so going somewhere in it must not take the screen away.
    #[test]
    fn back_in_files_keeps_the_workspace() {
        use crate::ui::dock::Workspace;
        let (mut state, doc, gta5, file) = state_with_archive();
        state.navigate_to(at(doc, gta5));
        state.navigate_to(at(doc, file));
        let model = state.new_empty_document(DocumentKind::Ydr);
        assert_eq!(state.active_workspace, Workspace::Model);

        assert!(state.navigate_back());
        assert_eq!(
            (state.active_workspace, state.active_document),
            (Workspace::Model, Some(model)),
            "Back took the model off screen"
        );
        assert_eq!(state.selection.selected(doc), Some(gta5));
        assert_eq!(state.navigation.reveal, Some(at(doc, gta5)));

        state.navigate_to(Location::document(doc));
        assert_eq!(
            (state.active_workspace, state.active_document),
            (Workspace::Model, Some(model)),
            "a breadcrumb took the model off screen"
        );

        state.active_workspace = Workspace::Browse;
        assert!(state.navigate_back());
        assert_eq!(
            state.active_document,
            Some(doc),
            "in Browse, which shows archives, going back to a row previews it"
        );
    }

    #[test]
    fn a_place_that_is_a_file_of_its_own_shows_it_from_another_workspace() {
        use crate::ui::dock::Workspace;
        let (mut state, doc, _gta5, _file) = state_with_archive();
        let lamp = state.new_empty_document(DocumentKind::Ydr);
        let path = PathBuf::from("C:/mods/lamp.ydr");
        state.document_mut(lamp).expect("just made").path = Some(path.clone());
        state.set_active_document(doc);
        assert_eq!(state.active_workspace, Workspace::Browse);

        state.open_place(Place::new(&path, "").expect("a nameable place"));
        assert_eq!(
            (state.active_workspace, state.active_document),
            (Workspace::Model, Some(lamp))
        );
    }

    #[test]
    fn the_breadcrumbs_are_the_path_and_each_step_goes_there() {
        let (mut state, doc, gta5, file) = state_with_archive();
        state.navigate_to(at(doc, file));
        let trail = state.breadcrumb_trail();
        let labels: Vec<&str> = trail.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels[1..], ["levels", "gta5", "Props.ydr"]);
        assert_eq!(trail[0].to, Location::document(doc));
        assert_eq!(trail[2].to, at(doc, gta5));

        state.navigate_to(trail[2].to);
        assert_eq!(state.selection.selected(doc), Some(gta5));
        assert!(state.navigation.can_go_back());
    }

    /// "Show in mods overlay" asks this of the overlay's copy of an archive.
    #[test]
    fn a_file_on_disk_is_revealed_in_the_tree_that_holds_it() {
        use std::time::{Duration, Instant};

        let tmp = tempfile::tempdir().expect("tempdir");
        let copy = tmp.path().join("mods").join("update").join("update.rpf");
        std::fs::create_dir_all(copy.parent().expect("a parent")).expect("the overlay");
        std::fs::write(&copy, b"an overlay copy").expect("the copy");
        let mut state = AppState::new();
        state.open_folder(tmp.path().to_path_buf());
        let doc = state
            .active_document
            .expect("the folder opens as a document");

        state.reveal_disk_path(&copy);
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.navigation.pending_place().is_some() {
            state.poll_expansions();
            state.tick_navigation();
            assert!(Instant::now() < deadline, "the reveal never arrived");
            std::thread::sleep(Duration::from_millis(2));
        }

        let row = state
            .selection
            .selected(doc)
            .expect("the copy's row is selected");
        let Some(DocPayload::Rpf(rpf)) = state.document(doc).map(|d| &d.payload) else {
            panic!("a folder opens as a tree");
        };
        assert_eq!(
            rpf.session.path_of(row).as_deref(),
            Some("mods/update/update.rpf")
        );
        assert_eq!(state.navigation.reveal, Some(at(doc, row)));
    }

    #[test]
    fn a_favourite_names_its_archive_and_path_and_comes_back_to_the_same_row() {
        let (mut state, doc, _gta5, file) = state_with_archive();
        let place = state.place_of(at(doc, file)).expect("a nameable place");
        assert_eq!(place.source, "C:/game/x64a.rpf");
        assert_eq!(place.inner, "levels/gta5/Props.ydr");

        state.selection.clear_all();
        state.open_place(place);
        assert_eq!(state.selection.selected(doc), Some(file));
        assert!(state.navigation.pending_place().is_none());
    }
}
