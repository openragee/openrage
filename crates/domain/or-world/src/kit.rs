//! The conformance kit: the rules every title's world holds, each checked over a hand-built
//! install the title's own tests state as a [`Fixture`], and [`conformance_kit!`](crate::conformance_kit),
//! which writes one test for each rule under the rule's name. A title that breaks a rule fails the
//! test of that name.
//!
//! ⚠ A rule is one a title can break. What the driver guarantees whatever the title (a held map
//! read once, a body laid out once a selection) is held by the driver's own tests over the fake
//! title, which counts.
//!
//! ⚠ Every rule opens the world afresh, with no key unless the fixture names an executable, its index kept under a scratch folder the
//! caller makes and removes: the macro makes one a test with `tempfile`, which a crate that calls
//! it lists among its dev-dependencies.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::Vec3;
use or_job::{Cancel, Progress};

use crate::caps::Capability;
use crate::contract::{
    EditError, EditPlace, EditSession, EditTitle, Envelope, Keys, Opening, Worker, WorldInstall,
    WorldSpec, WorldTitle,
};
use crate::driver::{Driver, Selected, Streamed};
use crate::kept::{self, Verdict};
use crate::model::{
    is_interior_key, AssetKey, Element, Feed, FeedName, Index, MapId, Moment, SwitchId, Switched,
    TextureAsk,
};
use crate::streaming::{NodeId, NodeState};

/// A parent node and the children that draw in its place, in two maps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ladder {
    /// The map that holds the parent, by the title's name of it.
    pub parent_map: u64,
    /// The map that holds its children, which names [`Self::parent_map`] as a parent.
    pub child_map: u64,
    /// Where the parent draws and no child is in reach.
    pub far: Vec3,
    /// Where every child draws whole.
    pub near: Vec3,
}

/// A map the clock lets in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timed {
    pub map: u64,
    /// Where it streams when its hours let it in.
    pub eye: Vec3,
    /// An hour it streams at.
    pub on: f32,
    /// An hour it does not stream at.
    pub off: f32,
}

/// A map only a request by name or its switch lets in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeldBack {
    pub map: u64,
    pub eye: Vec3,
    pub switch: SwitchId,
}

/// An interior a map places.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InteriorAt {
    /// Where its instance is in reach and every node of it draws.
    pub eye: Vec3,
    /// How many instances its nodes draw there.
    pub instances: usize,
}

/// A list of instances.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListAt {
    pub eye: Vec3,
    /// How many instances the lists in reach lay out there.
    pub instances: usize,
}

/// The lights and occluders of the bodies streamed at one place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lit {
    pub eye: Vec3,
    pub lights: usize,
    pub occluders: usize,
}

/// A place with ground under it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grounded {
    /// A point above the ground, within ten metres of it.
    pub at: Vec3,
    /// The height of the ground under it.
    pub height: f32,
}

/// What a fixture's world holds, in the terms every title shares.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scene {
    /// The weather every rule is checked under.
    pub weather: u64,
    /// Where nothing streams.
    pub nowhere: Vec3,
    pub ladder: Ladder,
    pub timed: Timed,
    pub held: HeldBack,
    pub interior: InteriorAt,
    /// `None` where no body of the title holds a list.
    pub list: Option<ListAt>,
    pub lit: Lit,
    /// Read only where the world states a height of the ground or a collision query.
    pub ground: Grounded,
    /// A map and how many placement rows it gives, where the world has the capability.
    pub placed: (u64, usize),
}

/// What makes a fixture's operation, guarded by how the session it is handed holds its files.
pub type MakeOperation = dyn Fn(&dyn EditSession) -> Envelope;

/// One operation of a title's edit store that moves a node, for the edit rules.
pub struct EditFixture {
    pub title: Box<dyn EditTitle>,
    /// The folder the store is opened over, where it is one.
    pub folder: Option<PathBuf>,
    /// The map that holds the node the operation moves.
    pub map: u64,
    pub node: u32,
    /// Where the operation moves the node to.
    pub to: Vec3,
    /// Where the node draws from, before and after.
    pub eye: Vec3,
    /// The operation, guarded by how the session holds its files when asked.
    pub op: Box<MakeOperation>,
}

/// A title, a hand-built world of it, and what that world holds.
pub struct Fixture {
    pub title: Box<dyn WorldTitle>,
    pub world: WorldSpec,
    pub scene: Scene,
    /// `None` where the title has no edit store.
    pub edit: Option<EditFixture>,
    /// What lives as long as the fixture, such as the scratch folder a hand-built install is written to.
    pub keeps: Option<Box<dyn std::any::Any>>,
    /// The executable whose keys open the world; `None` for a hand-built install, which opens with no key.
    pub executable: Option<PathBuf>,
}

/// One rule of the kit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    KeptLinks,
    ChildNamesParent,
    ParentUntilChildren,
    TimedMap,
    HeldMap,
    Interior,
    List,
    Columns,
    Lit,
    Caps,
    ApplyShows,
    StaleGuard,
    Inverse,
    Slots,
    Streams,
    Policy,
    Models,
    Frame,
    Draws,
    SessionBody,
}

impl Rule {
    pub const ALL: [Self; 20] = [
        Self::KeptLinks,
        Self::ChildNamesParent,
        Self::ParentUntilChildren,
        Self::TimedMap,
        Self::HeldMap,
        Self::Interior,
        Self::List,
        Self::Columns,
        Self::Lit,
        Self::Caps,
        Self::ApplyShows,
        Self::StaleGuard,
        Self::Inverse,
        Self::Slots,
        Self::Streams,
        Self::Policy,
        Self::Models,
        Self::Frame,
        Self::Draws,
        Self::SessionBody,
    ];

    /// The rule as a sentence, which is the name of the test that holds it.
    pub fn name(self) -> &'static str {
        match self {
            Self::KeptLinks => "a_kept_index_links_to_the_index_it_was_built_as",
            Self::ChildNamesParent => "a_child_map_names_its_parent_map",
            Self::ParentUntilChildren => {
                "a_parent_draws_until_its_children_are_loaded_and_the_children_after"
            }
            Self::TimedMap => "a_map_by_the_clock_streams_inside_its_hours_alone",
            Self::HeldMap => "a_held_map_streams_only_when_it_is_asked_for",
            Self::Interior => "an_interior_comes_in_within_its_instance_distance",
            Self::List => "a_list_lays_out_an_instance_for_each_it_holds",
            Self::Columns => "every_instance_is_a_whole_row_of_the_columns",
            Self::Lit => "the_lights_and_occluders_of_a_body_ride_with_the_selection",
            Self::Caps => "the_capabilities_say_what_the_world_answers",
            Self::ApplyShows => {
                "after_an_apply_the_selection_shows_the_change_with_nothing_flushed"
            }
            Self::StaleGuard => "a_stale_guard_refuses_the_operation",
            Self::Inverse => "an_operation_and_its_inverse_leave_the_store_as_it_was",
            Self::Slots => "every_slot_and_switch_is_listed_once",
            Self::Streams => "a_map_streams_from_inside_its_box_and_nothing_streams_from_nowhere",
            Self::Policy => "the_policy_states_every_level",
            Self::Models => "every_model_a_selection_draws_with_reads_whole",
            Self::Frame => "the_look_states_a_whole_frame_at_every_hour",
            Self::Draws => "a_body_says_what_draws_each_of_its_nodes",
            Self::SessionBody => "a_sessions_body_stands_until_the_next_apply_and_no_longer",
        }
    }
}

/// A fixture's world, opened and linked.
struct Opened {
    install: Arc<dyn WorldInstall>,
    index: Arc<Index>,
}

type Held<T = ()> = Result<T, String>;

fn said<T, E: std::fmt::Display>(what: &str, result: Result<T, E>) -> Held<T> {
    result.map_err(|e| format!("{what}: {e}"))
}

/// Open the world of `fixture` through its title with no key, its index kept under `scratch`.
fn open_in(
    fixture: &Fixture,
    scratch: &Path,
) -> Held<(Arc<dyn WorldInstall>, Index, kept::Report)> {
    let (cancel, progress) = (Cancel::new(), Progress::new());
    let opening = Opening {
        config: scratch,
        keys: match &fixture.executable {
            Some(executable) => Keys::Executable(executable),
            None => Keys::None,
        },
        cancel: &cancel,
        progress: &progress,
    };
    let opened = fixture.title.open(&fixture.world, &opening);
    let install = said("the world does not open", opened)?;
    let file = kept::file_for(scratch, fixture.title.id(), &fixture.world);
    let refreshed = kept::refresh(fixture.title.id(), &fixture.world, &*install, &file);
    let (index, report) = said("the index does not build", refreshed)?;
    if let Some(e) = &report.not_kept {
        return Err(format!("the index was not kept: {e}"));
    }
    Ok((install, index, report))
}

fn open(fixture: &Fixture, scratch: &Path) -> Held<Opened> {
    let (install, index, _) = open_in(fixture, scratch)?;
    Ok(Opened {
        install,
        index: Arc::new(index),
    })
}

impl Opened {
    fn map(&self, name: u64) -> Held<MapId> {
        (self.index.named(name))
            .ok_or_else(|| format!("the fixture names the map {name:#x}, which the index lacks"))
    }

    fn driver(&self) -> Driver<'static> {
        Driver::new(Arc::clone(&self.install), Arc::clone(&self.index))
    }
}

/// Every model ready.
fn all_ready(_: AssetKey) -> NodeState {
    NodeState::Ready
}

/// One view of `driver` from `eye`: every wanted body read, then a selection.
fn look(
    driver: &mut Driver<'_>,
    eye: Vec3,
    at: &Moment,
    switched: &Switched,
    edits: Option<&dyn EditSession>,
    ready: &dyn Fn(AssetKey) -> NodeState,
) -> (Streamed, Selected) {
    // A field of view of 45 degrees scales no distance.
    let view = (driver.policy()).view(eye, Vec3::ZERO, std::f32::consts::FRAC_PI_4);
    let streamed = driver.stream(&view, at, switched, edits, usize::MAX);
    let selected = driver.select(&view, at, ready);
    (streamed, selected)
}

/// [`look`] from `eye` at noon, nothing switched, every model ready, through a driver of its own.
fn glance(opened: &Opened, eye: Vec3, weather: u64) -> (Streamed, Selected) {
    let (at, none) = (Moment::noon(weather), Switched::default());
    look(&mut opened.driver(), eye, &at, &none, None, &all_ready)
}

/// The instances of `selected` whose node stands under `key`, by their place.
fn under(selected: &Selected, key: u64) -> Vec<usize> {
    let nodes = selected.instances.node.iter().enumerate();
    nodes
        .filter(|(_, node)| node.0 == key)
        .map(|(at, _)| at)
        .collect()
}

/// The models the instances `of` of `selected` draw with.
fn models_of(selected: &Selected, of: &[usize]) -> BTreeSet<AssetKey> {
    of.iter()
        .filter_map(|at| selected.instances.model_of(*at))
        .collect()
}

/// An error where a map the view wanted gave no body.
fn unread(streamed: &Streamed) -> Held {
    match streamed.unread.first() {
        Some((map, why)) => Err(format!("the map {map:?} gave no body: {why}")),
        None => Ok(()),
    }
}

fn kept_links(fixture: &Fixture, scratch: &Path) -> Held {
    let (_, built, first) = open_in(fixture, scratch)?;
    if !matches!(first.verdict, Verdict::Absent) {
        return Err(format!(
            "a first open found a kept index: {:?}",
            first.verdict
        ));
    }
    let (_, kept, second) = open_in(fixture, scratch)?;
    if !matches!(second.verdict, Verdict::Current) || !second.read.is_empty() || second.derived {
        return Err(format!(
            "a second open of a world that did not change read the archives {:?}, derived again: {}, and judged the kept index {:?}",
            second.read, second.derived, second.verdict
        ));
    }
    if built.maps().len() != kept.maps().len() {
        return Err(format!(
            "the kept sections link to {} maps and {} were built",
            kept.maps().len(),
            built.maps().len()
        ));
    }
    match (built.maps().iter().zip(kept.maps())).find(|(a, b)| a != b) {
        Some((built, kept)) => Err(format!(
            "the map `{}` links from the kept sections as {kept:?} and was built as {built:?}",
            built.label
        )),
        None => Ok(()),
    }
}

fn child_names_parent(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let ladder = &fixture.scene.ladder;
    let (parent, child) = (
        opened.map(ladder.parent_map)?,
        opened.map(ladder.child_map)?,
    );
    let row = opened.index.map(child).ok_or("the child map is no row")?;
    match row.parents.contains(&parent) {
        true => Ok(()),
        false => Err(format!(
            "the map `{}` names the parents {:?}, and its nodes' parent stands in {parent:?}",
            row.label, row.parents
        )),
    }
}

fn parent_until_children(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let (scene, ladder) = (&fixture.scene, &fixture.scene.ladder);
    let (parent, child) = (
        opened.map(ladder.parent_map)?.key(),
        opened.map(ladder.child_map)?.key(),
    );

    let (streamed, far) = glance(&opened, ladder.far, scene.weather);
    unread(&streamed)?;
    let far_parents = under(&far, parent);
    if far_parents.is_empty() || !under(&far, child).is_empty() {
        return Err(format!(
            "from afar {} instance(s) of the parent map draw and {} of the child map: the parent alone should",
            far_parents.len(),
            under(&far, child).len()
        ));
    }

    let (streamed, near) = glance(&opened, ladder.near, scene.weather);
    unread(&streamed)?;
    let children = under(&near, child);
    if children.is_empty() || !under(&near, parent).is_empty() {
        return Err(format!(
            "from near, every model ready, {} instance(s) of the child map draw and {} of the parent map: the children alone should",
            children.len(),
            under(&near, parent).len()
        ));
    }

    let (theirs, its) = (models_of(&near, &children), models_of(&far, &far_parents));
    if !theirs.is_disjoint(&its) {
        return Err(
            "the fixture's parent and its children draw with one model, so none can load alone"
                .into(),
        );
    }
    let loading = |model: AssetKey| match theirs.contains(&model) {
        true => NodeState::Loading,
        false => NodeState::Ready,
    };
    let (at, none) = (Moment::noon(scene.weather), Switched::default());
    let (_, held) = look(
        &mut opened.driver(),
        ladder.near,
        &at,
        &none,
        None,
        &loading,
    );
    let parents = under(&held, parent);
    let whole = parents.iter().all(|at| held.instances.fade[*at] == 1.0);
    if parents.is_empty() || !whole || !under(&held, child).is_empty() {
        return Err(format!(
            "from near, the children loading, {} instance(s) of the parent map draw (whole: {whole}) and {} of the child map: the parent should, whole, alone",
            parents.len(),
            under(&held, child).len()
        ));
    }
    match theirs.iter().all(|model| held.waiting.contains(model)) {
        true => Ok(()),
        false => {
            Err("a child's model that is loading is not among those the selection waits for".into())
        }
    }
}

/// Whether a camera at `eye` has the map `map` in the forest at `at` under `switched`.
fn streams(opened: &Opened, map: MapId, eye: Vec3, at: &Moment, switched: &Switched) -> Held<bool> {
    let (streamed, _) = look(&mut opened.driver(), eye, at, switched, None, &all_ready);
    unread(&streamed)?;
    Ok(streamed.wanted.iter().any(|(wanted, _)| *wanted == map))
}

fn timed_map(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let (scene, timed) = (&fixture.scene, &fixture.scene.timed);
    let map = opened.map(timed.map)?;
    let at = |hour: f32| Moment {
        hour,
        weather: scene.weather,
    };
    let none = Switched::default();
    let (on, off) = (
        streams(&opened, map, timed.eye, &at(timed.on), &none)?,
        streams(&opened, map, timed.eye, &at(timed.off), &none)?,
    );
    match (on, off) {
        (true, false) => Ok(()),
        _ => Err(format!(
            "at {} h it streams: {on}; at {} h it streams: {off}",
            timed.on, timed.off
        )),
    }
}

fn held_map(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let (scene, held) = (&fixture.scene, &fixture.scene.held);
    let map = opened.map(held.map)?;
    let at = Moment::noon(scene.weather);
    let mut switched = Switched::default();
    let unasked = streams(&opened, map, held.eye, &at, &switched)?;
    switched.on.insert(held.switch);
    let by_switch = streams(&opened, map, held.eye, &at, &switched)?;
    let mut requested = Switched::default();
    requested.requested.insert(map);
    let by_name = streams(&opened, map, held.eye, &at, &requested)?;
    match (unasked, by_switch, by_name) {
        (false, true, true) => Ok(()),
        _ => Err(format!(
            "with nothing asking it streams: {unasked}; with its switch on: {by_switch}; asked for by name: {by_name}"
        )),
    }
}

fn interior(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let (scene, interior) = (&fixture.scene, &fixture.scene.interior);
    let (streamed, selected) = glance(&opened, interior.eye, scene.weather);
    unread(&streamed)?;
    if let Some((at, why)) = streamed.interiors_unread.first() {
        return Err(format!("the interior of {:?} gave no body: {why}", at.map));
    }
    let nodes = selected.instances.node.iter();
    let inside = nodes.filter(|node| is_interior_key(node.0)).count();
    match !streamed.interiors.is_empty() && inside == interior.instances {
        true => Ok(()),
        false => Err(format!(
            "{} interior(s) came in and {inside} instance(s) of an interior draw, where the fixture states {}",
            streamed.interiors.len(),
            interior.instances
        )),
    }
}

fn list(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let scene = &fixture.scene;
    let Some(list) = &scene.list else {
        return Ok(());
    };
    let (streamed, selected) = glance(&opened, list.eye, scene.weather);
    unread(&streamed)?;
    let lists: BTreeSet<NodeId> = selected.selection.lists.iter().map(|l| l.node).collect();
    let nodes = selected.instances.node.iter();
    let laid = nodes.filter(|node| lists.contains(node)).count();
    match !lists.is_empty() && laid == list.instances {
        true => Ok(()),
        false => Err(format!(
            "{} list(s) draw and lay out {laid} instance(s), where the fixture states {}",
            lists.len(),
            list.instances
        )),
    }
}

/// The places a fixture states something draws from.
fn eyes(scene: &Scene) -> Vec<Vec3> {
    let fixed = [
        scene.ladder.far,
        scene.ladder.near,
        scene.interior.eye,
        scene.lit.eye,
    ];
    let list = scene.list.map(|list| list.eye);
    fixed.into_iter().chain(list).collect()
}

/// An error where the world's answer about the ground of the scene is not what its capability `stated` says: the height where it is stated, a refusal where it is not.
fn grounded(
    what: &str,
    stated: bool,
    height: f32,
    answered: Result<Option<crate::collision::Hit>, crate::contract::WorldError>,
) -> Held {
    match (stated, answered) {
        (true, Ok(Some(hit))) if (hit.position.z - height).abs() <= 1e-2 => Ok(()),
        (true, Ok(found)) => Err(format!(
            "{what} is stated, and asking under the fixture's place gave {:?}, where the ground stands at {height}",
            found.map(|hit| hit.position.z)
        )),
        (true, Err(e)) => Err(format!("{what} is stated, and asking gave: {e}")),
        (false, Err(_)) => Ok(()),
        (false, Ok(_)) => Err(format!("no {what} is stated, and asking gave an answer")),
    }
}

fn columns(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let scene = &fixture.scene;
    let words = opened.install.caps().instance_words;
    let mut drawn = 0;
    for eye in eyes(scene) {
        let (_, selected) = glance(&opened, eye, scene.weather);
        let (of, count) = (&selected.instances, selected.instances.len());
        drawn += count;
        let lengths = [
            of.model.len(),
            of.rows.len(),
            of.sphere.len(),
            of.fade.len(),
        ];
        if lengths.iter().any(|len| *len != count) {
            return Err(format!(
                "{count} instances stand in columns of {lengths:?} entries"
            ));
        }
        if of.width() != words || of.words.len() != count * words {
            return Err(format!(
                "{count} instances carry {} word(s), where the world declares {words} an instance",
                of.words.len()
            ));
        }
        let mut selected_nodes: BTreeSet<NodeId> =
            (selected.selection.drawn.iter().map(|draw| draw.node)).collect();
        selected_nodes.extend(selected.selection.lists.iter().map(|list| list.node));
        for at in 0..count {
            let (fade, sphere) = (of.fade[at], of.sphere[at]);
            let numbers = of.rows[at].iter().flatten().chain(&sphere);
            if !numbers.copied().all(f32::is_finite) || sphere[3] < 0.0 {
                return Err(format!(
                    "instance {at} stands at no place: {:?}, {sphere:?}",
                    of.rows[at]
                ));
            }
            if !(fade.abs() > 0.0 && fade.abs() <= 1.0) {
                return Err(format!("instance {at} is drawn at the fade {fade}"));
            }
            if of.model_of(at).is_none() {
                return Err(format!(
                    "instance {at} names a model the selection does not list"
                ));
            }
            if !selected_nodes.contains(&of.node[at]) {
                return Err(format!(
                    "instance {at} is of the node {:?}, which the selection did not draw",
                    of.node[at]
                ));
            }
        }
    }
    match drawn {
        0 => Err("nothing draws from any place the fixture states".into()),
        _ => Ok(()),
    }
}

fn lit(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let (scene, lit) = (&fixture.scene, &fixture.scene.lit);
    let (streamed, selected) = glance(&opened, lit.eye, scene.weather);
    unread(&streamed)?;
    let bodies = selected.bodies.iter().map(|(_, body)| body);
    let (lights, occluders) = bodies.fold((0, 0), |(lights, occluders), body| {
        let stated = body.occluders();
        (
            lights + body.lights().len(),
            occluders + stated.boxes.len() + stated.meshes.len(),
        )
    });
    match (lights, occluders) == (lit.lights, lit.occluders) {
        true => Ok(()),
        false => Err(format!(
            "the bodies of the selection state {lights} light(s) and {occluders} occluder(s), where the fixture states {} and {}",
            lit.lights, lit.occluders
        )),
    }
}

/// An edit session over `opened`, where the fixture has an edit store.
fn session<'a>(
    fixture: &'a Fixture,
    opened: &Opened,
) -> Held<Option<(&'a EditFixture, Box<dyn EditSession>)>> {
    let Some(edit) = &fixture.edit else {
        return Ok(None);
    };
    let place = EditPlace {
        install: &opened.install,
        index: &opened.index,
        folder: edit.folder.as_deref(),
    };
    let session = said("the edit store does not open", edit.title.open(&place))?;
    Ok(Some((edit, session)))
}

fn caps(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let caps = opened.install.caps();
    if caps.has(Capability::Edits) != fixture.edit.is_some() {
        return Err(format!(
            "an edit store is stated: {}, and the fixture has one: {}",
            caps.has(Capability::Edits),
            fixture.edit.is_some()
        ));
    }
    if caps.has(Capability::Base) && !caps.has(Capability::Edits) {
        return Err("a base is stated of a world with no edit store".into());
    }
    let (map, rows) = fixture.scene.placed;
    let mut placed = Vec::new();
    let indexer = opened.install.indexer();
    let answered = indexer.placements(&opened.index, opened.map(map)?, &mut placed);
    match (caps.has(Capability::Placements), answered) {
        (true, Ok(())) if placed.len() == rows => {}
        (true, Ok(())) => {
            return Err(format!(
                "the map gives {} placement row(s), where the fixture states {rows}",
                placed.len()
            ))
        }
        (true, Err(e)) => return Err(format!("placement rows are stated, and asking gave: {e}")),
        (false, Err(_)) => {}
        (false, Ok(())) => return Err("no placement rows are stated, and asking gave rows".into()),
    }
    match (
        caps.has(Capability::Water),
        opened.install.water(&opened.index),
    ) {
        (true, Ok(quads)) => {
            let whole = |quad: &crate::model::WaterQuad| {
                let numbers = [
                    quad.min[0],
                    quad.min[1],
                    quad.max[0],
                    quad.max[1],
                    quad.height,
                ];
                numbers.iter().all(|n| n.is_finite())
                    && quad.min[0] <= quad.max[0]
                    && quad.min[1] <= quad.max[1]
            };
            if let Some(quad) = quads.iter().find(|quad| !whole(quad)) {
                return Err(format!(
                    "water is stated, and a sheet of it is no rectangle at a height: {quad:?}"
                ));
            }
        }
        (true, Err(e)) => return Err(format!("water is stated, and asking gave: {e}")),
        (false, Err(_)) => {}
        (false, Ok(_)) => return Err("no water is stated, and asking gave sheets of it".into()),
    }
    let (ground, mut worker) = (&fixture.scene.ground, Worker::new());
    let probe = crate::collision::GroundProbe {
        max_rise: 0.0,
        max_drop: 20.0,
        max_slope: std::f32::consts::FRAC_PI_4,
    };
    let under = (opened.install).ground(&opened.index, ground.at, &probe, &mut worker);
    grounded(
        "a height of the ground",
        caps.has(Capability::Ground),
        ground.height,
        under,
    )?;
    let down = (opened.install).ray(&opened.index, ground.at, Vec3::NEG_Z, 20.0, &mut worker);
    grounded(
        "a collision query",
        caps.has(Capability::Collision),
        ground.height,
        down,
    )?;
    if let Some((edit, session)) = session(fixture, &opened)? {
        let op = (edit.op)(&*session);
        let based = op.at.iter().all(|(at, _)| session.base(at).is_some());
        if based != caps.has(Capability::Base) {
            return Err(format!(
                "a base is stated: {}, and the store answers one for what the operation changes: {based}",
                caps.has(Capability::Base)
            ));
        }
    }
    Ok(())
}

/// Where the node `node` stands in what `driver` selects from `eye`, the session asked before the reader.
fn place_of(
    driver: &mut Driver<'_>,
    node: NodeId,
    eye: Vec3,
    weather: u64,
    edits: &dyn EditSession,
) -> Held<Vec3> {
    let (at, none) = (Moment::noon(weather), Switched::default());
    let (streamed, selected) = look(driver, eye, &at, &none, Some(edits), &all_ready);
    unread(&streamed)?;
    let found = selected.instances.node.iter().position(|of| *of == node);
    (found.and_then(|at| selected.instances.position(at)))
        .ok_or_else(|| format!("the node {node:?} the fixture moves does not draw from its eye"))
}

fn close(a: Vec3, b: Vec3) -> bool {
    a.distance(b) <= 1e-3
}

fn apply_shows(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let Some((edit, mut session)) = session(fixture, &opened)? else {
        return Ok(());
    };
    let map = opened.map(edit.map)?;
    let node = (map.key(), edit.node);
    let weather = fixture.scene.weather;
    let mut driver = opened.driver();
    let before = place_of(&mut driver, node, edit.eye, weather, &*session)?;
    if close(before, edit.to) {
        return Err("the fixture moves its node to where it stands".into());
    }
    let op = (edit.op)(&*session);
    let applied = said("the operation does not apply", session.apply(&op))?;
    if !applied.maps.contains(&map) {
        return Err(format!(
            "the apply names the maps {:?} as changed, and not {map:?}",
            applied.maps
        ));
    }
    let after = place_of(&mut driver, node, edit.eye, weather, &*session)?;
    match close(after, edit.to) {
        true => Ok(()),
        false => Err(format!(
            "the node stood at {before} and was moved to {}, and the next selection draws it at {after}",
            edit.to
        )),
    }
}

fn stale_guard(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let Some((edit, mut session)) = session(fixture, &opened)? else {
        return Ok(());
    };
    let op = (edit.op)(&*session);
    said("the operation does not apply", session.apply(&op))?;
    match session.apply(&op) {
        Err(EditError::Stale(_)) => Ok(()),
        Err(other) => Err(format!(
            "an operation whose guard is stale was refused otherwise: {other}"
        )),
        Ok(_) => Err("an operation applied again, its guard taken before the first apply".into()),
    }
}

fn inverse(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let Some((edit, mut session)) = session(fixture, &opened)? else {
        return Ok(());
    };
    let node = (opened.map(edit.map)?.key(), edit.node);
    let weather = fixture.scene.weather;
    let before = place_of(&mut opened.driver(), node, edit.eye, weather, &*session)?;
    let op = (edit.op)(&*session);
    let guards = |session: &dyn EditSession| -> Vec<_> {
        op.at.iter().map(|(at, _)| session.guard(at)).collect()
    };
    let stood = guards(&*session);
    let applied = said("the operation does not apply", session.apply(&op))?;
    said(
        "the inverse does not apply",
        session.apply(&applied.inverse),
    )?;
    if guards(&*session) != stood {
        return Err("after an operation and its inverse a guard of what it changed differs".into());
    }
    let after = place_of(&mut opened.driver(), node, edit.eye, weather, &*session)?;
    match close(after, before) {
        true => Ok(()),
        false => Err(format!(
            "the node stood at {before}, and after the operation and its inverse at {after}"
        )),
    }
}

/// An error where one of `ids` is listed twice.
fn once(what: &str, ids: impl IntoIterator<Item = u64>) -> Held {
    let mut seen = BTreeSet::new();
    match ids.into_iter().find(|id| !seen.insert(*id)) {
        Some(id) => Err(format!("{what} {id} is listed twice")),
        None => Ok(()),
    }
}

fn slots(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let options = fixture.title.options();
    once("the option", options.iter().map(|o| u64::from(o.id.0)))?;
    let switches = opened.install.look().switches(&opened.index);
    once("the switch", switches.iter().map(|s| u64::from(s.id.0)))?;
    if !switches.iter().any(|s| s.id == fixture.scene.held.switch) {
        return Err("the switch the fixture's held map waits for is none the look lists".into());
    }
    let weathers = opened.install.look().weathers(&opened.index);
    once("the weather", weathers.iter().map(|weather| weather.id))?;
    if weathers
        .iter()
        .any(|weather| weather.name.trim().is_empty())
    {
        return Err("a weather has a name of no words".into());
    }
    let checked = |weather: &crate::slots::WeatherKind| weather.id == fixture.scene.weather;
    if !weathers.is_empty() && !weathers.iter().any(checked) {
        return Err("the weather the fixture checks under is none the look lists".into());
    }
    if opened.install.caps().has(Capability::WeatherGate) && weathers.is_empty() {
        return Err("a weather gates what streams, and the look lists no weather".into());
    }
    let extras = opened.install.extras();
    for (name, slots) in [
        ("the layer", &extras.layers),
        ("the rule", &extras.rules),
        ("the panel section", &extras.sections),
        ("the streamed extra", &extras.streamed),
    ] {
        once(name, slots.iter().map(|slot| u64::from(slot.id)))?;
    }
    let texts = (options.iter().map(|option| option.label))
        .chain(switches.iter().map(|switch| switch.label))
        .chain(extras.all().map(|(_, slot)| slot.label));
    if texts.into_iter().any(|text| text.key().trim().is_empty()) {
        return Err("a slot shows a sentence of no words".into());
    }
    match &fixture.edit {
        Some(edit) => {
            let operations = edit.title.operations();
            once(
                "the operation",
                operations.iter().map(|op| u64::from(op.id.0)),
            )
        }
        None => Ok(()),
    }
}

fn streams_rule(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let scene = &fixture.scene;
    let (at, none) = (Moment::noon(scene.weather), Switched::default());
    let child = opened.map(scene.ladder.child_map)?;
    if !streams(&opened, child, scene.ladder.near, &at, &none)? {
        return Err(
            "the child map does not stream from where the fixture says its nodes draw".into(),
        );
    }
    let (streamed, _) = glance(&opened, scene.nowhere, scene.weather);
    match streamed.wanted.len() + streamed.waiting.len() + streamed.unread.len() {
        0 => Ok(()),
        maps => Err(format!(
            "{maps} map(s) are wanted from where the fixture says nothing streams"
        )),
    }
}

fn policy(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    match opened.install.policy().fault() {
        Some(fault) => Err(fault.into()),
        None => Ok(()),
    }
}

fn models(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let scene = &fixture.scene;
    let mut asks = BTreeSet::new();
    for eye in eyes(scene) {
        let (_, selected) = glance(&opened, eye, scene.weather);
        asks.extend(selected.instances.models.iter().copied());
    }
    let asks: Vec<AssetKey> = asks.into_iter().collect();
    let (mut worker, mut read) = (Worker::new(), Vec::new());
    let reader = opened.install.models();
    reader.models(&opened.index, &asks, &mut worker, &mut read);
    if asks.is_empty() {
        return Err("nothing drawn from the fixture's places draws with a model".into());
    }
    if let Some(unread) = (asks.iter()).find(|ask| !read.iter().any(|(key, _)| key == *ask)) {
        return Err(format!(
            "the model {unread:?}, which a place of the fixture draws with, does not read"
        ));
    }
    once("the model", read.iter().map(|(key, _)| key.0))?;
    let mut textures = BTreeSet::new();
    for (key, model) in &read {
        if !asks.contains(key) || model.lods.is_empty() {
            return Err(format!(
                "the model {key:?} was not asked for, or holds no level of detail"
            ));
        }
        for mesh in model.lods.iter().flat_map(|lod| &lod.meshes) {
            let (stride, count) = (mesh.layout.stride as usize, mesh.vertex_count());
            // An offset near the last number a `u32` holds must not wrap to one that fits.
            let fits = |element: &Element| {
                u64::from(element.offset) + u64::from(element.format.size()) <= stride as u64
            };
            let laid_out = mesh.layout.elements.iter().all(fits);
            if stride == 0 || mesh.vertices.len() % stride != 0 || !laid_out {
                return Err(format!(
                    "a mesh of the model {key:?} holds no whole vertices of its layout"
                ));
            }
            let past = mesh.indices.iter().any(|index| *index as usize >= count);
            if mesh.indices.len() % 3 != 0 || past {
                return Err(format!(
                    "a mesh of the model {key:?} names a vertex past its {count}, or no whole triangles"
                ));
            }
            textures.extend(mesh.surface.colour);
            textures.extend(mesh.surface.normal);
        }
    }
    let textures: Vec<TextureAsk> = textures.into_iter().collect();
    let mut read = Vec::new();
    reader.textures(&opened.index, &textures, &mut worker, &mut read);
    if let Some(unread) = (textures.iter()).find(|ask| !read.iter().any(|(key, _)| key == *ask)) {
        return Err(format!(
            "the texture {unread:?}, which a surface of a model the fixture draws with asks for, does not read"
        ));
    }
    for (ask, texture) in &read {
        let first = texture.levels.first().map_or(0, Vec::len);
        let whole = first == texture.format.level_bytes(texture.width, texture.height);
        if !textures.contains(ask) || texture.width == 0 || texture.height == 0 || !whole {
            return Err(format!(
                "the texture {ask:?} was not asked for, or its first level is not whole"
            ));
        }
    }
    Ok(())
}

/// A feed that keeps whether every value handed to it was a number.
struct Numbers(bool);

impl Feed for Numbers {
    fn set(&mut self, _: FeedName<'_>, value: &[f32]) {
        self.0 &= value.iter().all(|number| number.is_finite());
    }
}

fn frame(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let look = opened.install.look();
    for hour in 0..24u8 {
        let at = Moment {
            hour: f32::from(hour),
            weather: fixture.scene.weather,
        };
        let mut fed = Numbers(true);
        let facts = look.frame(&at, &mut fed);
        let colours = [facts.key, facts.ambient, facts.clear];
        let lit = (colours.iter().flatten()).all(|part| part.is_finite() && *part >= 0.0);
        let towards = (facts.sun.length() - 1.0).abs() <= 1e-3;
        let exposed = facts.exposure.is_finite() && facts.exposure > 0.0;
        if !(towards && lit && exposed && fed.0) {
            return Err(format!(
                "at {hour} h the frame is {facts:?}, and every value fed by name is a number: {}; the sun is to be a unit vector, a colour a number not below nought and the exposure above nought",
                fed.0
            ));
        }
    }
    Ok(())
}

fn draws(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let scene = &fixture.scene;
    let at = Moment::noon(scene.weather);
    let mut told = Vec::new();
    for eye in eyes(scene) {
        let (_, selected) = glance(&opened, eye, scene.weather);
        for (key, body) in &selected.bodies {
            told.clear();
            body.draws(&at, &mut told);
            if told.len() != body.nodes().len() {
                return Err(format!(
                    "the body under the key {key:#x} holds {} node(s) and says what draws {}: a node it says nothing of draws nothing",
                    body.nodes().len(),
                    told.len()
                ));
            }
        }
    }
    Ok(())
}

fn session_body(fixture: &Fixture, scratch: &Path) -> Held {
    let opened = open(fixture, scratch)?;
    let Some((edit, mut session)) = session(fixture, &opened)? else {
        return Ok(());
    };
    let map = opened.map(edit.map)?;
    let op = (edit.op)(&*session);
    let applied = said("the operation does not apply", session.apply(&op))?;
    // A session that answers no body breaks another rule, not this one.
    let (Some(first), Some(again)) = (session.body(map), session.body(map)) else {
        return Ok(());
    };
    if !Arc::ptr_eq(&first, &again) {
        return Err(
            "asked twice with no apply between, the session answers two bodies of the map: the driver would put the map in the forest afresh at every view".into(),
        );
    }
    said(
        "the inverse does not apply",
        session.apply(&applied.inverse),
    )?;
    match session.body(map) {
        Some(after) if Arc::ptr_eq(&first, &after) => Err(
            "an apply changed the map and the session answers the body it answered before: the driver would not see the change".into(),
        ),
        _ => Ok(()),
    }
}

/// Whether the world of `fixture` holds `rule`, and why not where it does not; its index is kept under `scratch`, which holds no file yet.
pub fn check(fixture: &Fixture, rule: Rule, scratch: &Path) -> Result<(), String> {
    let run: fn(&Fixture, &Path) -> Held = match rule {
        Rule::KeptLinks => kept_links,
        Rule::ChildNamesParent => child_names_parent,
        Rule::ParentUntilChildren => parent_until_children,
        Rule::TimedMap => timed_map,
        Rule::HeldMap => held_map,
        Rule::Interior => interior,
        Rule::List => list,
        Rule::Columns => columns,
        Rule::Lit => lit,
        Rule::Caps => caps,
        Rule::ApplyShows => apply_shows,
        Rule::StaleGuard => stale_guard,
        Rule::Inverse => inverse,
        Rule::Slots => slots,
        Rule::Streams => streams_rule,
        Rule::Policy => policy,
        Rule::Models => models,
        Rule::Frame => frame,
        Rule::Draws => draws,
        Rule::SessionBody => session_body,
    };
    run(fixture, scratch)
}

/// Every rule the world of `fixture` breaks, each with why, each rule's index kept in a folder of its own under `scratch`.
pub fn broken(fixture: &Fixture, scratch: &Path) -> Vec<(Rule, String)> {
    let rules = Rule::ALL.into_iter();
    rules
        .filter_map(|rule| {
            let broke = check(fixture, rule, &scratch.join(rule.name()));
            Some((rule, broke.err()?))
        })
        .collect()
}

/// Fail the test that holds `rule` where the world of `fixture` breaks it; its index is kept under `scratch`.
pub fn hold(fixture: &Fixture, rule: Rule, scratch: &Path) {
    if let Err(why) = check(fixture, rule, scratch) {
        panic!(
            "the title `{}` breaks the rule `{}`: {why}",
            fixture.title.id(),
            rule.name()
        );
    }
}

/// Write the kit's rules as tests over the fixture `$fixture` makes, each under its rule's name.
#[macro_export]
macro_rules! conformance_kit {
    ($fixture:expr) => {
        $crate::conformance_kit! { @rules $fixture;
            a_kept_index_links_to_the_index_it_was_built_as => KeptLinks,
            a_child_map_names_its_parent_map => ChildNamesParent,
            a_parent_draws_until_its_children_are_loaded_and_the_children_after => ParentUntilChildren,
            a_map_by_the_clock_streams_inside_its_hours_alone => TimedMap,
            a_held_map_streams_only_when_it_is_asked_for => HeldMap,
            an_interior_comes_in_within_its_instance_distance => Interior,
            a_list_lays_out_an_instance_for_each_it_holds => List,
            every_instance_is_a_whole_row_of_the_columns => Columns,
            the_lights_and_occluders_of_a_body_ride_with_the_selection => Lit,
            the_capabilities_say_what_the_world_answers => Caps,
            after_an_apply_the_selection_shows_the_change_with_nothing_flushed => ApplyShows,
            a_stale_guard_refuses_the_operation => StaleGuard,
            an_operation_and_its_inverse_leave_the_store_as_it_was => Inverse,
            every_slot_and_switch_is_listed_once => Slots,
            a_map_streams_from_inside_its_box_and_nothing_streams_from_nowhere => Streams,
            the_policy_states_every_level => Policy,
            every_model_a_selection_draws_with_reads_whole => Models,
            the_look_states_a_whole_frame_at_every_hour => Frame,
            a_body_says_what_draws_each_of_its_nodes => Draws,
            a_sessions_body_stands_until_the_next_apply_and_no_longer => SessionBody,
        }
    };
    (@rules $fixture:expr; $($name:ident => $rule:ident,)*) => {
        $(
            #[test]
            fn $name() {
                // A test named otherwise than its rule would fail under another rule's name.
                assert_eq!(stringify!($name), $crate::kit::Rule::$rule.name());
                let scratch = ::tempfile::tempdir().expect("a scratch folder");
                $crate::kit::hold(&$fixture, $crate::kit::Rule::$rule, scratch.path())
            }
        )*
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_macro_writes_a_test_for_every_rule_under_its_name() {
        let source = include_str!("kit.rs");
        for rule in Rule::ALL {
            let line = format!("{} => {rule:?},", rule.name());
            assert!(source.contains(&line), "the macro writes no test `{line}`");
        }
    }
}
