//! GTA V's rows through the kept file every title shares: a world at a level of DLC keeps a file
//! and an index of its own, a second start reads no archive, an archive that changed is the only
//! one read again, and the rows joined from the sections are the index `or_depends` builds itself.

#[path = "../../or-depends/tests/forest_fixture/mod.rs"]
mod forest_fixture;
mod hand_built;
#[path = "../../or-depends/tests/world_fixture/mod.rs"]
mod world_fixture;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_depends::{MountedWorld, WorldIndex};
use or_job::{Cancel, Progress};
use or_world::contract::{Keys, Opening, WorldInstall, WorldSpec, WorldTitle};
use or_world::kept::{self, Report, Verdict};
use or_world::model::Index;
use or_world::slots::OptionValue;
use or_world_gta5::{Gta5Title, Gta5World, Linked, LEGACY, LEVEL, MODE, TITLE};

use forest_fixture::{arch, ent, map_around, map_file, types_file};
use world_fixture::{cube, dlc_list_bytes, hash, mount_all, write_archive, write_pack};

/// A map of one entity around `x`, as a file of an archive.
fn town(name: &str, x: f32) -> (String, Vec<u8>) {
    let map = map_around(
        cube([x, 0.0, 0.0], 50.0),
        vec![ent("bench", [x + 5.0, 5.0, 5.0])],
    );
    map_file(name, &map)
}

/// The archive `name` of the install at `root`, holding `files`.
fn archive(root: &Path, name: &str, files: &[(String, Vec<u8>)]) {
    let files: Vec<(&str, Vec<u8>)> = (files.iter())
        .map(|(path, bytes)| (path.as_str(), bytes.clone()))
        .collect();
    write_archive(root, name, &files);
}

/// An install of a base map and two packs, the second of which adds a map.
fn install_of_two_packs() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let root = dir.path();
    let list = dlc_list_bytes(&["alpha", "beta"]);
    write_archive(root, "common.rpf", &[("data/dlclist.xml", list)]);
    let types = types_file("props", &[arch("bench", 80.0)]);
    archive(root, "x64a.rpf", &[town("m_town", 0.0), types]);
    write_pack(root, "alpha", 10, Vec::new(), Vec::new());
    let (path, bytes) = town("m_beta", 1000.0);
    write_pack(root, "beta", 20, Vec::new(), vec![(path.as_str(), bytes)]);
    dir
}

/// `world` opened through the title with no key, its index from the kept file under `config`.
fn opened(world: &WorldSpec, config: &Path) -> (Arc<dyn WorldInstall>, Index, Report, PathBuf) {
    let (cancel, progress) = (Cancel::new(), Progress::new());
    let opening = Opening {
        config,
        keys: Keys::None,
        cancel: &cancel,
        progress: &progress,
    };
    let install = Gta5Title.open(world, &opening).expect("the world opens");
    let file = kept::file_for(config, TITLE, world);
    let (index, report) = kept::refresh(TITLE, world, &*install, &file).expect("the index builds");
    assert!(report.not_kept.is_none(), "{:?}", report.not_kept);
    (install, index, report, file)
}

fn at_level(root: &Path, pack: &str) -> WorldSpec {
    WorldSpec {
        options: vec![(LEVEL, OptionValue::Named(pack.to_owned()))],
        ..WorldSpec::of(LEGACY, root)
    }
}

fn has(index: &Index, map: &str) -> bool {
    index.named(u64::from(hash(map))).is_some()
}

#[test]
fn two_levels_of_one_install_share_no_index_and_no_kept_file() {
    let (dir, config) = (
        install_of_two_packs(),
        tempfile::tempdir().expect("a folder"),
    );
    let (whole, alpha) = (
        WorldSpec::of(LEGACY, dir.path()),
        at_level(dir.path(), "alpha"),
    );
    let (_, every, first, every_file) = opened(&whole, config.path());
    let (_, level, second, level_file) = opened(&alpha, config.path());
    assert_ne!(every_file, level_file);
    assert!(every_file.is_file() && level_file.is_file());
    // The second world took nothing of the first's file: it found none of its own.
    assert!(matches!(first.verdict, Verdict::Absent) && matches!(second.verdict, Verdict::Absent));
    assert!(has(&every, "m_town") && has(&every, "m_beta"));
    assert!(has(&level, "m_town") && !has(&level, "m_beta"));
    // Each stands as kept after the other was opened and written.
    for (world, with_beta) in [(&whole, true), (&alpha, false)] {
        let (_, index, report, _) = opened(world, config.path());
        assert!(matches!(report.verdict, Verdict::Current), "{report:?}");
        assert_eq!(
            (report.read.len(), report.derived),
            (0, false),
            "{report:?}"
        );
        assert_eq!(has(&index, "m_beta"), with_beta);
    }
}

#[test]
fn a_second_start_reads_no_archive_and_an_archive_that_changed_is_the_one_read_again() {
    let (dir, config) = (
        install_of_two_packs(),
        tempfile::tempdir().expect("a folder"),
    );
    for world in [
        WorldSpec::of(LEGACY, dir.path()),
        at_level(dir.path(), "beta"),
    ] {
        let (install, _, first, _) = opened(&world, config.path());
        let archives = install.inputs().archives;
        assert_eq!(first.read, (0..archives.len()).collect::<Vec<_>>());
        assert!(
            archives.iter().all(|archive| archive.stamp.is_some()),
            "{archives:?}"
        );
        let (_, _, second, _) = opened(&world, config.path());
        assert!(matches!(second.verdict, Verdict::Current), "{second:?}");
        assert_eq!(second.read, [] as [usize; 0]);
    }
    // The base archive gains a map: it alone is read again, in each world.
    let types = types_file("props", &[arch("bench", 80.0)]);
    let more = [town("m_town", 0.0), town("m_more", -1000.0), types];
    archive(dir.path(), "x64a.rpf", &more);
    for world in [
        WorldSpec::of(LEGACY, dir.path()),
        at_level(dir.path(), "beta"),
    ] {
        let (install, index, third, _) = opened(&world, config.path());
        let archives = install.inputs().archives;
        let changed: Vec<usize> = (0..archives.len())
            .filter(|at| archives[*at].name.ends_with("x64a.rpf"))
            .collect();
        assert_eq!(changed.len(), 1, "{archives:?}");
        assert!(matches!(third.verdict, Verdict::Changed), "{third:?}");
        assert_eq!(third.read, changed);
        assert!(has(&index, "m_more") && has(&index, "m_beta"));
    }
}

#[test]
fn the_rows_joined_from_the_kept_sections_are_the_index_or_depends_builds_of_the_mount() {
    let (dir, config) = (
        install_of_two_packs(),
        tempfile::tempdir().expect("a folder"),
    );
    for spare in [false, true] {
        if spare {
            archive(dir.path(), "x64b.rpf", &hand_built::files());
        }
        let world = WorldSpec::of(LEGACY, dir.path());
        // Built, and then linked again from the kept sections alone.
        for _ in 0..2 {
            let (_, index, _, _) = opened(&world, config.path());
            let linked = index.title::<Linked>().expect("a GTA V link");
            let game = mount_all(dir.path());
            let (reference, _) = WorldIndex::build(&MountedWorld::new(&game, dir.path()), None);
            assert_eq!(linked.world.seal().ok(), reference.seal().ok());
            assert_eq!(index.maps().len(), reference.maps().len());
        }
    }
}

#[test]
fn a_world_of_another_game_an_option_it_does_not_take_and_a_layer_it_cannot_mount_are_refused() {
    let (dir, config) = (
        install_of_two_packs(),
        tempfile::tempdir().expect("a folder"),
    );
    let (cancel, progress) = (Cancel::new(), Progress::new());
    let opening = Opening {
        config: config.path(),
        keys: Keys::None,
        cancel: &cancel,
        progress: &progress,
    };
    let refused = |world: &WorldSpec| match Gta5Title.open(world, &opening) {
        Ok(_) => "it opened".to_string(),
        Err(e) => e.to_string(),
    };
    let root = dir.path();
    let other = WorldSpec::of(or_world::fake::GAME, root);
    assert_eq!(
        refused(&other),
        "the title opens no world of the game `fake-world`"
    );
    let mut odd = WorldSpec::of(LEGACY, root);
    odd.options = vec![(MODE, OptionValue::Choice(7))];
    assert!(
        refused(&odd).starts_with("the option OptionId(2)"),
        "{}",
        refused(&odd)
    );
    odd.options = vec![(LEVEL, OptionValue::On(true))];
    assert!(
        refused(&odd).starts_with("the option OptionId(1)"),
        "{}",
        refused(&odd)
    );
    let mut layered = WorldSpec::of(LEGACY, root);
    layered.layers = vec![or_world::contract::Layer {
        path: root.join("elsewhere"),
    }];
    assert!(
        refused(&layered).contains("reads one layer"),
        "{}",
        refused(&layered)
    );
    // A level the list lacks is the mount's own refusal.
    assert_ne!(refused(&at_level(root, "no_such_pack")), "it opened");
    // The install's own overlay folder is the one layer it takes.
    layered.layers = vec![or_world::contract::Layer {
        path: root.join("mods"),
    }];
    assert_eq!(refused(&layered), "it opened");
    let _ = Gta5World::over;
}
