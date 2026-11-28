//! The vehicles of a mount built by hand as a browser lists them: which data files are read, which pack each vehicle
//! comes from, and each filter and page.
//!
//! ⚠ Every expected value is computed from the fixture's own lists: a vehicle's pack is the last file to state its model
//! in the order the fixture gives its files, and a filter's answer is the fixture's rows that pass it.

mod world_fixture;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use or_title::install::vehicles::{vehicles, LEVEL_VEHICLES, VEHICLE_FILE_TYPE};
use or_title::DataFileEntry;
use or_world_gta5::find::Rank;
use or_world_gta5::vehicles::{VehiclePage, VehicleQuery};
use or_world_gta5::VehicleCatalogue;
use world_fixture::{dlc_list_bytes, mount_all, write_archive, write_pack};

/// One vehicle of the fixture: its model, its game name, its class and its type, as a file spells them.
type Stated = (&'static str, &'static str, &'static str, &'static str);

/// A vehicle data file stating `stated`.
fn file(stated: &[Stated]) -> Vec<u8> {
    let items: String = (stated.iter())
        .map(|(model, game, class, kind)| {
            format!(
                "<Item><modelName>{model}</modelName><gameName>{game}</gameName>\
                 <type>{kind}</type><vehicleClass>{class}</vehicleClass>\
                 <drivers><Item><driverName>someone</driverName></Item></drivers></Item>"
            )
        })
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<CVehicleModelInfo__InitDataList>\
         <residentTxd>shared</residentTxd><InitDatas>{items}</InitDatas>\
         </CVehicleModelInfo__InitDataList>"
    )
    .into_bytes()
}

/// The level's vehicles as the base game states them.
const LEVEL: &[Stated] = &[
    ("carone", "CARONE", "VC_SPORTS", "VEHICLE_TYPE_CAR"),
    ("boatone", "BOATONE", "VC_BOAT", "VEHICLE_TYPE_BOAT"),
    ("truck", "HAULER", "VC_COMMERCIAL", "VEHICLE_TYPE_CAR"),
    ("truck2", "HAULER", "VC_COMMERCIAL", "VEHICLE_TYPE_CAR"),
];

/// The level's vehicles as the title update states them: the first car in another class, and one more.
const LEVEL_UPDATED: &[Stated] = &[
    ("carone", "CARONE", "VC_SUPER", "VEHICLE_TYPE_CAR"),
    ("boatone", "BOATONE", "VC_BOAT", "VEHICLE_TYPE_BOAT"),
    ("truck", "HAULER", "VC_COMMERCIAL", "VEHICLE_TYPE_CAR"),
    ("truck2", "HAULER", "VC_COMMERCIAL", "VEHICLE_TYPE_CAR"),
    ("bikeone", "BIKEONE", "VC_MOTORCYCLE", "VEHICLE_TYPE_BIKE"),
];

/// What the first pack states: a car of its own, and the level's boat again, its model in another case.
const ALPHA: &[Stated] = &[
    ("alphacar", "ALPHA", "VC_SPORTS", "VEHICLE_TYPE_CAR"),
    ("BoatOne", "BOATTWO", "VC_BOAT", "VEHICLE_TYPE_BOAT"),
];

/// What the second pack states in the file it leaves enabled, and in the one it registers disabled.
///
/// Its two helicopters state one class in two cases, which the list counts as one, as its filter takes a class in any case.
const BETA: &[Stated] = &[
    ("betaheli", "BHELI", "VC_HELICOPTER", "VEHICLE_TYPE_HELI"),
    ("betaheli2", "BHELI2", "vc_helicopter", "VEHICLE_TYPE_HELI"),
];
const BETA_DISABLED: &[Stated] = &[("betaoff", "BOFF", "VC_SPORTS", "VEHICLE_TYPE_CAR")];

fn registered(filename: &str, file_type: &str, disabled: bool) -> DataFileEntry {
    DataFileEntry {
        filename: filename.into(),
        file_type: file_type.into(),
        disabled,
        ..Default::default()
    }
}

/// An install whose level states vehicles, with three packs after it; with `update`, a title update that states the
/// level's vehicles again.
fn install(update: bool) -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::Builder::new()
        .prefix("or-depends-vehicles-")
        .tempdir()
        .expect("mkdir");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let level = LEVEL_VEHICLES.trim_start_matches("common:/");
    write_archive(
        &root,
        "common.rpf",
        &[
            (
                "data/dlclist.xml",
                dlc_list_bytes(&["alpha", "beta", "gamma"]),
            ),
            (level, file(LEVEL)),
        ],
    );
    if update {
        std::fs::create_dir(root.join("update")).expect("mkdir");
        let inside = format!("common/{level}");
        write_archive(
            &root,
            "update/update.rpf",
            &[(inside.as_str(), file(LEVEL_UPDATED))],
        );
    }
    write_pack(
        &root,
        "alpha",
        10,
        vec![registered(
            "dlc_alpha:/data/vehicles.meta",
            VEHICLE_FILE_TYPE,
            false,
        )],
        vec![("data/vehicles.meta", file(ALPHA))],
    );
    write_pack(
        &root,
        "beta",
        20,
        vec![
            registered("dlc_beta:/data/vehicles.meta", VEHICLE_FILE_TYPE, false),
            registered("dlc_beta:/data/off.meta", VEHICLE_FILE_TYPE, true),
            // A file of another type is no vehicle data file, whatever it holds.
            registered("dlc_beta:/data/handling.meta", "HANDLING_FILE", false),
            // The level's own file again, in another case: it is read once, as the level's.
            registered(
                &LEVEL_VEHICLES.to_ascii_uppercase(),
                VEHICLE_FILE_TYPE,
                false,
            ),
        ],
        vec![
            ("data/vehicles.meta", file(BETA)),
            ("data/off.meta", file(BETA_DISABLED)),
            ("data/handling.meta", file(BETA_DISABLED)),
        ],
    );
    write_pack(
        &root,
        "gamma",
        30,
        vec![
            registered("dlc_gamma:/data/missing.meta", VEHICLE_FILE_TYPE, false),
            registered("dlc_gamma:/data/peds.meta", VEHICLE_FILE_TYPE, false),
        ],
        vec![(
            "data/peds.meta",
            b"<CPedModelInfo__InitDataList><InitDatas /></CPedModelInfo__InitDataList>".to_vec(),
        )],
    );
    (scratch, root)
}

/// The files of the fixture that state vehicles, in the mount's order, each with the device of its pack.
fn stated(update: bool) -> Vec<(&'static str, &'static [Stated])> {
    let level = if update {
        ("update", LEVEL_UPDATED)
    } else {
        ("common", LEVEL)
    };
    vec![level, ("dlc_alpha", ALPHA), ("dlc_beta", BETA)]
}

/// One vehicle as the fixture expects it listed: what the last file to state its model states, that file's pack, and
/// the packs of the files before it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Expected {
    stated: Stated,
    pack: &'static str,
    replaced: Vec<&'static str>,
}

/// The vehicles of the fixture, by model in lower case.
fn expected(update: bool) -> BTreeMap<String, Expected> {
    let mut all: BTreeMap<String, Expected> = BTreeMap::new();
    for (pack, vehicles) in stated(update) {
        for vehicle in vehicles {
            let earlier = all.remove(&vehicle.0.to_ascii_lowercase());
            let replaced = earlier.map_or_else(Vec::new, |earlier| {
                let mut replaced = earlier.replaced;
                replaced.push(earlier.pack);
                replaced
            });
            all.insert(
                vehicle.0.to_ascii_lowercase(),
                Expected {
                    stated: *vehicle,
                    pack,
                    replaced,
                },
            );
        }
    }
    all
}

/// How many vehicles `catalogue` counts in each class it lists that is the helicopters', whatever its case.
fn class_of_both_cases(catalogue: &VehicleCatalogue) -> Vec<usize> {
    let classes = catalogue.classes().into_iter();
    let helicopters = classes.filter(|(class, _)| class.eq_ignore_ascii_case("VC_HELICOPTER"));
    helicopters.map(|(_, count)| count).collect()
}

fn catalogue(root: &Path) -> VehicleCatalogue {
    VehicleCatalogue::new(vehicles(&mount_all(root)))
}

/// The models of `page`, in its order, as their files spell them.
fn listed(page: &VehiclePage<'_>) -> Vec<String> {
    let rows = page.rows.iter();
    rows.map(|row| row.vehicle.vehicle.model_name.clone())
        .collect()
}

/// The models of the fixture `keep` keeps, as their last file spells them, by model in lower case.
fn those(update: bool, keep: impl Fn(&Expected) -> bool) -> Vec<String> {
    (expected(update).values())
        .filter(|vehicle| keep(vehicle))
        .map(|vehicle| vehicle.stated.0.to_string())
        .collect()
}

#[test]
fn each_vehicle_is_listed_once_with_what_the_last_file_to_state_it_states_and_that_files_pack() {
    for update in [false, true] {
        let (_scratch, root) = install(update);
        let catalogue = catalogue(&root);
        let list = catalogue.list();
        let expected = expected(update);
        assert_eq!(list.vehicles.len(), expected.len(), "update: {update}");
        assert!(expected.len() >= 6);
        let mut replaced = 0;
        for vehicle in &list.vehicles {
            let model = vehicle.vehicle.model_name.to_ascii_lowercase();
            let want = &expected[&model];
            let said = &vehicle.vehicle;
            assert_eq!(
                (
                    said.model_name.as_str(),
                    said.game_name.as_str(),
                    said.class.as_str(),
                    said.vehicle_type.as_str()
                ),
                want.stated,
                "{model}"
            );
            assert_eq!(list.file_of(vehicle).pack, want.pack, "{model}");
            let before: Vec<&str> = (vehicle.replaced.iter())
                .map(|file| list.files[*file].pack.as_str())
                .collect();
            assert_eq!(before, want.replaced, "{model}");
            replaced += before.len();
        }
        assert_eq!(replaced, 1, "the boat the first pack states again");
        let boat = &expected["boatone"];
        assert_eq!((boat.stated.1, boat.pack), ("BOATTWO", "dlc_alpha"));

        // The level's own file is the title update's copy where there is one, and the base game's where there is none.
        let level = &list.files[0];
        assert_eq!(level.path, LEVEL_VEHICLES);
        let (pack, vehicles) = stated(update)[0];
        assert_eq!(
            (level.pack.as_str(), &level.read),
            (pack, &Ok(vehicles.len()))
        );
        let car = &expected["carone"];
        assert_eq!(car.stated.2, if update { "VC_SUPER" } else { "VC_SPORTS" });
        assert_eq!(expected.contains_key("bikeone"), update);
    }
}

#[test]
fn a_disabled_file_and_a_file_of_another_type_add_nothing_and_a_file_that_does_not_read_says_why() {
    let (_scratch, root) = install(false);
    let catalogue = catalogue(&root);
    let list = catalogue.list();
    let files: Vec<(&str, &str)> = (list.files.iter())
        .map(|file| (file.pack.as_str(), file.path.as_str()))
        .collect();
    assert_eq!(
        files,
        [
            ("common", LEVEL_VEHICLES),
            ("dlc_alpha", "dlc_alpha:/data/vehicles.meta"),
            ("dlc_beta", "dlc_beta:/data/vehicles.meta"),
            ("dlc_gamma", "dlc_gamma:/data/missing.meta"),
            ("dlc_gamma", "dlc_gamma:/data/peds.meta"),
        ]
    );
    let read: Vec<Option<usize>> = (list.files.iter())
        .map(|file| file.read.as_ref().ok().copied())
        .collect();
    assert_eq!(
        read,
        [
            Some(LEVEL.len()),
            Some(ALPHA.len()),
            Some(BETA.len()),
            None,
            None
        ]
    );
    let why = |at: usize| list.files[at].read.clone().expect_err("it gave no vehicle");
    assert!(why(3).contains("no such file"), "{}", why(3));
    assert!(
        why(4).contains("CVehicleModelInfo__InitDataList"),
        "{}",
        why(4)
    );
    let models: BTreeSet<String> = (list.vehicles.iter())
        .map(|vehicle| vehicle.vehicle.model_name.to_ascii_lowercase())
        .collect();
    assert!(!models.contains("betaoff"), "{models:?}");
    assert!(models.contains("betaheli"));
}

#[test]
fn a_filter_by_class_lists_the_vehicles_the_data_puts_in_that_class_and_no_other() {
    let (_scratch, root) = install(true);
    let catalogue = catalogue(&root);
    // A class is one whatever its case, so the fixture's classes are told apart in upper case.
    let classes: BTreeSet<String> = (expected(true).values())
        .map(|vehicle| vehicle.stated.2.to_ascii_uppercase())
        .collect();
    assert!(classes.len() >= 5, "{classes:?}");
    let mut seen = 0;
    for class in &classes {
        let of_class = |vehicle: &Expected| vehicle.stated.2.eq_ignore_ascii_case(class);
        for asked in [class.to_string(), class.to_ascii_lowercase()] {
            let page = catalogue.page(&VehicleQuery {
                class: Some(&asked),
                ..VehicleQuery::default()
            });
            let wanted = those(true, of_class);
            assert_eq!(listed(&page), wanted, "{asked}");
            assert_eq!(page.total, wanted.len());
            assert!(page.rows.iter().all(|row| row
                .vehicle
                .vehicle
                .class
                .eq_ignore_ascii_case(class)));
        }
        seen += those(true, of_class).len();
    }
    assert_eq!(seen, expected(true).len(), "each vehicle is in one class");
    assert_eq!(
        class_of_both_cases(&catalogue),
        vec![2],
        "the helicopters' class, stated in two cases, is counted once"
    );
    // The car the title update moved is in its class and no longer in the base game's.
    let class = |class: &'static str| {
        listed(&catalogue.page(&VehicleQuery {
            class: Some(class),
            ..VehicleQuery::default()
        }))
    };
    assert_eq!(class("VC_SUPER"), ["carone"]);
    assert_eq!(class("VC_SPORTS"), ["alphacar"]);
    assert!(class("VC_NOWHERE").is_empty());
    // A class is the file's text and nothing shorter.
    assert!(class("SPORTS").is_empty());

    let tally = |all: Vec<(&str, usize)>| -> BTreeMap<String, usize> {
        let all = all.into_iter();
        let said: Vec<(String, usize)> =
            (all.map(|(text, count)| (text.to_ascii_uppercase(), count))).collect();
        let tally: BTreeMap<String, usize> = said.iter().cloned().collect();
        assert_eq!(tally.len(), said.len(), "a text is counted twice: {said:?}");
        tally
    };
    let count_by = |pick: fn(&Expected) -> &'static str| {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for vehicle in expected(true).values() {
            *counts
                .entry(pick(vehicle).to_ascii_uppercase())
                .or_default() += 1;
        }
        counts
    };
    assert_eq!(
        tally(catalogue.classes()),
        count_by(|vehicle| vehicle.stated.2)
    );
    assert_eq!(
        tally(catalogue.types()),
        count_by(|vehicle| vehicle.stated.3)
    );
    assert_eq!(tally(catalogue.packs()), count_by(|vehicle| vehicle.pack));
}

#[test]
fn a_type_a_pack_and_a_text_each_list_their_vehicles_and_a_page_is_a_slice_of_the_answer() {
    let (_scratch, root) = install(true);
    let catalogue = catalogue(&root);
    let page = |query: VehicleQuery<'_>| catalogue.page(&query);
    let all = page(VehicleQuery::default());
    assert_eq!(listed(&all), those(true, |_| true), "by model name");
    assert_eq!(all.total, expected(true).len());

    for kind in ["VEHICLE_TYPE_CAR", "VEHICLE_TYPE_BOAT", "vehicle_type_heli"] {
        let said = listed(&page(VehicleQuery {
            vehicle_type: Some(kind),
            ..VehicleQuery::default()
        }));
        let wanted = those(true, |vehicle| vehicle.stated.3.eq_ignore_ascii_case(kind));
        assert_eq!(said, wanted, "{kind}");
        assert!(!said.is_empty());
    }
    for pack in ["update", "dlc_alpha", "DLC_BETA", "common"] {
        let said = listed(&page(VehicleQuery {
            pack: Some(pack),
            ..VehicleQuery::default()
        }));
        let wanted = those(true, |vehicle| vehicle.pack.eq_ignore_ascii_case(pack));
        assert_eq!(said, wanted, "{pack}");
    }
    let of = |pack: &'static str| {
        listed(&page(VehicleQuery {
            pack: Some(pack),
            ..VehicleQuery::default()
        }))
    };
    assert_eq!(of("dlc_alpha"), ["alphacar", "BoatOne"]);
    assert!(
        of("common").is_empty(),
        "the title update's copy is the one read"
    );

    // A text ranks a model's name as a search does, and then lists the vehicles whose game name holds it.
    let ranked = |text: &'static str| {
        let page = page(VehicleQuery {
            text: Some(text),
            ..VehicleQuery::default()
        });
        assert_eq!(page.total, page.rows.len());
        let rows = page.rows.iter();
        rows.map(|row| {
            let rank = row.rank.expect("a text ranks its rows");
            (row.vehicle.vehicle.model_name.clone(), rank)
        })
        .collect::<Vec<_>>()
    };
    let row = |model: &str, rank: Rank| (model.to_string(), rank);
    assert_eq!(
        ranked("truck"),
        [row("truck", Rank::Exact), row("truck2", Rank::Prefix)]
    );
    assert_eq!(
        ranked("one"),
        [
            row("bikeone", Rank::Contained),
            row("BoatOne", Rank::Contained),
            row("carone", Rank::Contained)
        ]
    );
    assert_eq!(
        ranked("hauler"),
        [
            row("truck", Rank::Contained),
            row("truck2", Rank::Contained)
        ],
        "two models share a game name, and both are listed"
    );
    assert_eq!(ranked("BOATTWO"), [row("BoatOne", Rank::Contained)]);
    assert!(ranked("zzz").is_empty());
    let cars = page(VehicleQuery {
        text: Some("one"),
        vehicle_type: Some("VEHICLE_TYPE_CAR"),
        ..VehicleQuery::default()
    });
    assert_eq!(listed(&cars), ["carone"]);

    for count in [1, 2, 4, all.total + 3] {
        let mut rows = Vec::new();
        for offset in (0..all.total + count).step_by(count) {
            let page = page(VehicleQuery {
                offset,
                count,
                ..VehicleQuery::default()
            });
            assert_eq!(page.total, all.total);
            assert_eq!(page.rows.len(), all.total.saturating_sub(offset).min(count));
            rows.extend(page.rows);
        }
        assert_eq!(rows, all.rows, "{count} a page");
    }
}

/// ⚠ An install's level need not state vehicles: a file the mount does not hold and no pack registers is no file of the list.
#[test]
fn a_level_that_states_no_vehicles_lists_the_files_the_packs_register_and_no_other() {
    let scratch = tempfile::Builder::new()
        .prefix("or-depends-vehicles-no-level-")
        .tempdir()
        .expect("mkdir");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    write_archive(
        &root,
        "common.rpf",
        &[("data/dlclist.xml", dlc_list_bytes(&["alpha"]))],
    );
    let own = "dlc_alpha:/data/vehicles.meta";
    write_pack(
        &root,
        "alpha",
        10,
        vec![registered(own, VEHICLE_FILE_TYPE, false)],
        vec![("data/vehicles.meta", file(ALPHA))],
    );
    let catalogue = catalogue(&root);
    let list = catalogue.list();
    let files: Vec<(&str, &str, Option<usize>)> = (list.files.iter())
        .map(|file| {
            let read = file.read.as_ref().ok().copied();
            (file.pack.as_str(), file.path.as_str(), read)
        })
        .collect();
    assert_eq!(files, [("dlc_alpha", own, Some(ALPHA.len()))]);
    let models: Vec<&str> = (list.vehicles.iter())
        .map(|vehicle| vehicle.vehicle.model_name.as_str())
        .collect();
    assert_eq!(
        models,
        ALPHA.iter().map(|stated| stated.0).collect::<Vec<_>>()
    );
}
