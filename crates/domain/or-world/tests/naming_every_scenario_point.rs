//! Every scenario region's points named, and a point re-typed by name saved
//! back to its region's own layout.
//!
//! The staged regions name each point's type through their own look-ups; an
//! install, under `OPENRAGE_GAME_ROOT` (`;`-separated roots), also names each
//! type and model set against the scenario and model-set manifests it ships.
//!
//! The install suite of an interior's typed records is here too, beside the
//! other suite of this crate that mounts an install and reads every file of a
//! kind: every `.ytyp` opened as its typed root, the members the four records
//! of an interior type held to the ones the install states, and each type file
//! with an interior saved to its fixed point.

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_io::decl::File;
use or_io::hash::literal_string_hash;
use or_io::meta::{
    CBaseArchetypeDefAny, CMapTypes, CMloArchetypeDef, CMloEntitySet, CMloPortalDef, CMloRoomDef,
    CMloTimeCycleModifier, CScenarioPointRegion, MetaDocument, MetaInstance, Value,
};
use or_io::Error;
use or_test_support::{game_roots, skip_install, Corpus, CorpusOrigin, GAME_ROOT_ENV};
use or_title::game::Game;
use or_world::field;
use or_world::scenario::{set_scenario_type, PointAt, ScenarioManifests};

/// The staged regions: `.ymt` in the metadata corpus whose root is a scenario region.
fn staged_regions(what: &str) -> Option<Vec<(String, Vec<u8>, CScenarioPointRegion)>> {
    support::schema();
    let origin = or_test_support::owned_corpus_origin(
        "or-meta",
        env!("CARGO_MANIFEST_DIR"),
        "OPENRAGE_META_CORPUS",
        "binary",
    );
    if matches!(origin, CorpusOrigin::Absent(_)) {
        or_test_support::skip_corpus(what, "no metadata corpus is staged");
        return None;
    }
    let files: Vec<PathBuf> = Corpus::with_root(origin.path(), "ymt")
        .all_targets()
        .find_or_skip(what)?;
    let regions: Vec<_> = files
        .iter()
        .filter_map(|path| {
            let bytes = std::fs::read(path).ok()?;
            match CScenarioPointRegion::from_bytes(&bytes) {
                Err(Error::WrongType { .. }) => None,
                region => {
                    let name = path.file_name()?.to_string_lossy().into_owned();
                    Some((name, bytes, region.unwrap_or_else(|e| panic!("{e}"))))
                }
            }
        })
        .collect();
    if regions.is_empty() {
        or_test_support::skip_corpus(what, "no staged .ymt is a scenario region");
        return None;
    }
    Some(regions)
}

/// Gives the region's first own point type `index`, in the value tree.
fn retype_in_tree(root: &mut MetaInstance, index: u64) -> bool {
    let key = |name: &[u8]| literal_string_hash(name);
    let point = field::get_mut(root, key(b"Points"))
        .and_then(as_struct_mut)
        .and_then(|c| field::get_mut(c, key(b"MyPoints")))
        .and_then(|items| match items {
            Value::Array(items) => items.first_mut().and_then(as_struct_mut),
            _ => None,
        });
    match point.and_then(|p| field::get_mut(p, key(b"iType"))) {
        Some(Value::UInt(v)) => *v = index,
        Some(Value::Int(v)) => *v = index as i64,
        _ => return false,
    }
    true
}

fn as_struct_mut(value: &mut Value) -> Option<&mut MetaInstance> {
    match value {
        Value::Struct(inst) | Value::Pointer(Some(inst)) => Some(inst),
        _ => None,
    }
}

#[test]
fn every_staged_points_type_is_named_and_a_retyped_point_keeps_its_layout() {
    const WHAT: &str = "every_staged_points_type_is_named_and_a_retyped_point_keeps_its_layout";
    let Some(regions) = staged_regions(WHAT) else {
        return;
    };
    let mut failures = Vec::new();
    let mut retyped = 0usize;
    for (file, bytes, region) in &regions {
        for (cluster, point) in region.all_points() {
            if region.scenario_type(point).is_none() {
                failures.push(format!(
                    "{file}: a point of cluster {cluster:?} names type {} of {}",
                    point.i_type,
                    region.look_ups.type_names.len()
                ));
            }
        }
        let Some(first) = region.points.my_points.first() else {
            continue;
        };
        let names = &region.look_ups.type_names;
        let Some(other) = names.iter().find(|n| {
            region
                .scenario_type(first)
                .is_some_and(|t| t.hash != n.hash)
        }) else {
            continue;
        };
        let at = PointAt {
            cluster: None,
            index: 0,
        };
        let mut edited = region.clone();
        set_scenario_type(&mut edited, at, other.clone()).expect("the point exists");
        retyped += 1;
        let index = names
            .iter()
            .position(|n| n.hash == other.hash)
            .expect("named");
        let mut tree = MetaDocument::parse(support::schema(), bytes.clone()).expect("reads");
        assert!(retype_in_tree(&mut tree.instance, index as u64), "{file}");
        let (saved, want) = match (edited.to_bytes(), tree.to_bytes()) {
            (Ok(saved), Ok(want)) => (saved, want),
            (Err(e), _) => {
                failures.push(format!("{file}: the re-typed region does not save: {e}"));
                continue;
            }
            (_, Err(e)) => {
                failures.push(format!("{file}: the re-typed tree does not save: {e}"));
                continue;
            }
        };
        if saved != want {
            failures.push(format!(
                "{file}: re-typing a point saved otherwise than the tree's own layout does"
            ));
        }
        match CScenarioPointRegion::from_bytes(&saved) {
            Ok(back) if back == edited => {
                let named = back.scenario_type(&back.points.my_points[0]);
                if named.map(|n| n.hash) != Some(other.hash) {
                    failures.push(format!("{file}: the point reopens as another type"));
                }
            }
            Ok(_) => failures.push(format!(
                "{file}: the re-typed region does not reopen as saved"
            )),
            Err(e) => failures.push(format!("{file}: the re-typed region does not reopen: {e}")),
        }
    }
    assert!(retyped > 0, "no staged region has a point to re-type");
    assert!(
        failures.is_empty(),
        "{} problems over {} regions:\n  {}",
        failures.len(),
        regions.len(),
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

fn crypto_for(root: &Path) -> Option<Arc<dyn RpfCryptoProvider>> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    Some(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))))
}

/// The base game's scenario-info and model-set manifests; a DLC pack names its own by `fileType`.
const BASE_MANIFESTS: [&str; 3] = [
    "common:/data/ai/scenarios.meta",
    "common:/data/ai/ambientpedmodelsets.meta",
    "common:/data/ai/vehiclemodelsets.meta",
];
const MANIFEST_FILE_TYPES: [&str; 3] = [
    "SCENARIO_INFO_FILE",
    "AMBIENT_PED_MODEL_SET_FILE",
    "AMBIENT_VEHICLE_MODEL_SET_FILE",
];

struct Named {
    label: String,
    regions: usize,
    points: usize,
    with_model_set: usize,
    problems: Vec<String>,
}

fn name_install(root: &Path) -> Option<Named> {
    let library = support::schema();
    let install = or_title::GtaInstall::from_path(root)?;
    let game =
        or_title::build_mounted_game(&install, crypto_for(root)?, or_title::DlcCutoff::All).ok()?;
    let pack_manifests = game
        .packs
        .iter()
        .flat_map(|pack| &pack.data_files)
        .filter(|df| {
            MANIFEST_FILE_TYPES
                .iter()
                .any(|t| df.file_type.eq_ignore_ascii_case(t))
        })
        .map(|df| df.filename.clone());
    let mut manifests = ScenarioManifests::default();
    for path in BASE_MANIFESTS
        .map(String::from)
        .into_iter()
        .chain(pack_manifests)
    {
        let Some(bytes) = game.vfs.open(&path) else {
            continue;
        };
        if let Ok(doc) = MetaDocument::parse(library, bytes) {
            manifests.add(&doc.instance);
        }
    }
    let mut ymt: BTreeMap<String, String> = BTreeMap::new();
    game.for_each_file(|logical, _container, leaf| {
        if leaf.to_ascii_lowercase().ends_with(".ymt") {
            ymt.insert(logical.to_ascii_lowercase(), logical.to_string());
        }
    });
    let mut out = Named {
        label: root.display().to_string(),
        regions: 0,
        points: 0,
        with_model_set: 0,
        problems: Vec::new(),
    };
    for path in ymt.values() {
        let Some(bytes) = game.vfs.open(path) else {
            continue;
        };
        let region = match CScenarioPointRegion::from_bytes(&bytes) {
            Ok(region) => region,
            Err(Error::WrongType { .. }) => continue,
            Err(e) => {
                out.problems.push(format!("{path}: {e}"));
                continue;
            }
        };
        out.regions += 1;
        match manifests.name_points(&region) {
            Ok(points) => {
                out.points += points.len();
                out.with_model_set += points.iter().filter(|p| p.model_set.is_some()).count();
            }
            Err(e) => out.problems.push(format!("{path}: {e}")),
        }
    }
    Some(out)
}

#[test]
fn every_installed_points_type_and_model_set_is_named_by_a_manifest() {
    const WHAT: &str = "every_installed_points_type_and_model_set_is_named_by_a_manifest";
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(
            WHAT,
            format_args!(
                "no install root; set {GAME_ROOT_ENV} to a ;-separated list of install directories"
            ),
        );
        return;
    }
    let installs: Vec<Named> = roots.iter().filter_map(|r| name_install(r)).collect();
    if installs.is_empty() {
        skip_install(WHAT, "no install root mounted");
        return;
    }
    for install in &installs {
        let report = format!(
            "{}: {} regions, {} points named, {} with a model set, {} problems",
            install.label,
            install.regions,
            install.points,
            install.with_model_set,
            install.problems.len()
        );
        eprintln!("{report}");
        assert!(install.regions >= 100, "{report}");
        assert!(
            install.problems.is_empty(),
            "{report}\n  {}",
            install
                .problems
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n  ")
        );
    }
}

/// The four records of an interior, by the array of the interior that holds each and the members its record types.
const INTERIOR_RECORDS: [(&str, &str, &[&str]); 4] = [
    ("CMloRoomDef", "rooms", CMloRoomDef::MEMBERS),
    ("CMloPortalDef", "portals", CMloPortalDef::MEMBERS),
    ("CMloEntitySet", "entitySets", CMloEntitySet::MEMBERS),
    (
        "CMloTimeCycleModifier",
        "timeCycleModifiers",
        CMloTimeCycleModifier::MEMBERS,
    ),
];

/// What the type files of one install hold of interiors.
#[derive(Default)]
struct Interiors {
    label: String,
    type_files: usize,
    interiors: usize,
    /// How many records of each class of [`INTERIOR_RECORDS`] were read typed.
    records: [usize; 4],
    /// The member names each class states in the install, by name hash, with one spelling of each.
    stated: [BTreeMap<u32, String>; 4],
    saved: usize,
    problems: Vec<String>,
}

fn typed_interiors(types: &CMapTypes) -> impl Iterator<Item = &CMloArchetypeDef> {
    types.archetypes.iter().filter_map(|a| match a {
        CBaseArchetypeDefAny::CMloArchetypeDef(mlo) => Some(mlo),
        _ => None,
    })
}

impl Interiors {
    fn fold(&mut self, path: &str, bytes: &[u8]) {
        self.type_files += 1;
        let types = match CMapTypes::from_bytes(bytes) {
            Ok(types) => types,
            Err(e) => return self.problems.push(format!("{path}: opens typed: {e}")),
        };
        if typed_interiors(&types).next().is_none() {
            return;
        }
        let root = match MetaDocument::parse(support::schema(), bytes.to_vec()) {
            Ok(document) => document.instance,
            Err(e) => return self.problems.push(format!("{path}: reads: {e}")),
        };
        let viewed: Vec<_> = or_world::MapTypes::new(&root)
            .into_iter()
            .flat_map(|types| types.archetypes())
            .filter_map(|a| a.interior().map(|interior| (a.instance(), interior)))
            .collect();
        let typed: Vec<_> = typed_interiors(&types).collect();
        if viewed.len() != typed.len() {
            return self.problems.push(format!(
                "{path}: {} interiors read typed and {} in the tree",
                typed.len(),
                viewed.len()
            ));
        }
        for ((inst, view), mlo) in viewed.iter().zip(&typed) {
            self.interiors += 1;
            let held = [
                mlo.rooms.len(),
                mlo.portals.len(),
                mlo.entity_sets.len(),
                mlo.time_cycle_modifiers.len(),
            ];
            let in_tree = [
                view.room_count(),
                view.portal_count(),
                view.entity_set_count(),
                view.time_cycle_modifier_count(),
            ];
            if held != in_tree {
                self.problems.push(format!(
                    "{path}: an interior holds {in_tree:?} rooms, portals, entity sets and modifiers, and {held:?} read typed"
                ));
            }
            for (class, (_, array, _)) in INTERIOR_RECORDS.iter().enumerate() {
                self.records[class] += held[class];
                let key = literal_string_hash(array.as_bytes());
                for record in field::structs_at(inst, key) {
                    for (member, _) in &record.fields {
                        self.stated[class]
                            .entry(field::name_hash(member))
                            .or_insert_with(|| member.clone());
                    }
                }
            }
            let names: Vec<Option<String>> = view.rooms().map(|room| room.name()).collect();
            let typed_names: Vec<Option<String>> = (mlo.rooms.iter())
                .map(|room| Some(room.name.clone()))
                .collect();
            if names != typed_names {
                self.problems
                    .push(format!("{path}: the rooms are named {names:?} in the tree"));
            }
            let broken = or_world::validate::check_interior(view);
            if !broken.is_empty() {
                self.problems.push(format!("{path}: {}", broken[0]));
            }
        }
        // ⚠ An RBM goes back under a fresh DEFLATE, so the fixed point is the save of what the save reads as.
        let fixed = types.to_bytes().and_then(|saved| {
            let back = CMapTypes::from_bytes(&saved)?;
            Ok((back == types, back.to_bytes()? == saved))
        });
        match fixed {
            Ok((true, true)) => self.saved += 1,
            Ok((equal, fixed)) => self.problems.push(format!(
                "{path}: saved, it reads back equal: {equal}, and saves again to the same bytes: {fixed}"
            )),
            Err(e) => self.problems.push(format!("{path}: saves: {e}")),
        }
    }

    /// The members a record types that no record of the install states, and the members the install states that no record types.
    fn members_that_differ(&self) -> Vec<String> {
        let mut differ = Vec::new();
        for (class, (name, _, typed)) in INTERIOR_RECORDS.iter().enumerate() {
            let keys: Vec<u32> = (typed.iter())
                .map(|m| literal_string_hash(m.as_bytes()))
                .collect();
            for (member, key) in typed.iter().zip(&keys) {
                if !self.stated[class].contains_key(key) {
                    differ.push(format!("{name}::{member} is typed and never stated"));
                }
            }
            for (key, spelt) in &self.stated[class] {
                if !keys.contains(key) {
                    differ.push(format!("{name}::{spelt} is stated and not typed"));
                }
            }
        }
        differ
    }

    fn report(&self) -> String {
        format!(
            "{}: {} type files, {} interiors ({} rooms, {} portals, {} entity sets, {} time cycle modifiers), {} type files with an interior saved to their fixed point, {} problems",
            self.label,
            self.type_files,
            self.interiors,
            self.records[0],
            self.records[1],
            self.records[2],
            self.records[3],
            self.saved,
            self.problems.len()
        )
    }
}

fn interiors_of_install(root: &Path) -> Option<Interiors> {
    support::schema();
    let install = or_title::GtaInstall::from_path(root)?;
    let game =
        or_title::build_mounted_game(&install, crypto_for(root)?, or_title::DlcCutoff::All).ok()?;
    let mut ytyp: BTreeMap<String, String> = BTreeMap::new();
    game.for_each_file(|logical, _container, leaf| {
        if leaf.to_ascii_lowercase().ends_with(".ytyp") {
            ytyp.insert(logical.to_ascii_lowercase(), logical.to_string());
        }
    });
    let mut out = Interiors {
        label: root.display().to_string(),
        ..Interiors::default()
    };
    for path in ytyp.values() {
        match game.vfs.open(path) {
            Some(bytes) => out.fold(path, &bytes),
            None => out.problems.push(format!("{path}: does not open")),
        }
    }
    Some(out)
}

/// ⚠ The install is opened read only and nothing here writes into one.
#[test]
fn every_interior_of_an_install_reads_typed_states_the_members_typed_and_saves_to_its_fixed_point()
{
    const WHAT: &str =
        "every_interior_of_an_install_reads_typed_states_the_members_typed_and_saves_to_its_fixed_point";
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(
            WHAT,
            format_args!(
                "no install root; set {GAME_ROOT_ENV} to a ;-separated list of install directories"
            ),
        );
        return;
    }
    let installs: Vec<Interiors> = (roots.iter())
        .filter_map(|r| interiors_of_install(r))
        .collect();
    if installs.is_empty() {
        skip_install(WHAT, "no install root mounted");
        return;
    }
    for install in &installs {
        let report = install.report();
        eprintln!("{report}");
        // A census that read nothing would hold every member to nothing and pass.
        assert!(install.interiors >= 100, "{report}");
        assert!(
            install.records.iter().all(|&records| records >= 100),
            "{report}"
        );
        assert!(
            install.problems.is_empty(),
            "{report}\n  {}",
            install
                .problems
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n  ")
        );
        let differ = install.members_that_differ();
        assert!(differ.is_empty(), "{report}\n  {}", differ.join("\n  "));
    }
}

/// The census of an interior's members over trees built here, so the install suite's own judgement is seen to fail.
#[test]
fn the_census_of_an_interiors_members_names_one_typed_and_never_stated_and_one_stated_and_not_typed(
) {
    let mut census = Interiors::default();
    for (class, (_, _, typed)) in INTERIOR_RECORDS.iter().enumerate() {
        for member in *typed {
            census.stated[class].insert(literal_string_hash(member.as_bytes()), member.to_string());
        }
    }
    assert_eq!(census.members_that_differ(), Vec::<String>::new());

    let blend = literal_string_hash(b"blend");
    census.stated[0].remove(&blend);
    let extra = literal_string_hash(b"aMemberNoRecordTypes");
    census.stated[1].insert(extra, "aMemberNoRecordTypes".to_string());
    assert_eq!(
        census.members_that_differ(),
        [
            "CMloRoomDef::blend is typed and never stated",
            "CMloPortalDef::aMemberNoRecordTypes is stated and not typed"
        ]
    );
}
