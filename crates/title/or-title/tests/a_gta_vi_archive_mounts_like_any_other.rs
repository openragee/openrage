//! A GTA VI archive goes through the device every archive goes through: its root lists, a nested
//! archive lists as a folder under its own name, and a file inside it opens by its path.
//!
//! Install-gated: it skips cleanly without the keys and the build.
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot, or `OPENRAGE_GTA6_EXE` for the bundled one
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf`
//!
//! The crate's other suite that walks a whole install is here too: a GTA V install mounted at two
//! levels of DLC, which needs `OPENRAGE_GAME_ROOT` and skips without it; its checks also run on an
//! install built by hand.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use or_io::container::rpf::crypto::{
    extract_keys, Gta5RetailCryptoProvider, NoOpCryptoProvider, RpfCryptoProvider,
};
use or_io::container::rpf::{
    Gta6Context, GtaViKeys, NameResolver, RpfArchive, RpfBuilder, GTA6_EXE_ENV, GTA6_SNAPSHOT_ENV,
};
use or_test_support::skip_install;
use or_title::vfs::{
    build_mounted_game, ArchiveDevice, ArchiveEntry, ArchiveEntryKind, DlcCutoff, FileDevice,
};
use or_title::{
    ContentXml, DlcList, ExtraTitleUpdateData, Game, GtaInstall, ListedPackState, MountedGame,
    SetupData, TitleUpdateMount,
};

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";

/// The path of the first entry under `dir`, folders descended, that `wanted` accepts.
fn find(
    device: &ArchiveDevice,
    dir: &str,
    depth: usize,
    wanted: &dyn Fn(&ArchiveEntry) -> bool,
) -> Option<String> {
    let entries = device.entries(dir)?;
    let join = |name: &str| match dir {
        "" => name.to_owned(),
        dir => format!("{dir}/{name}"),
    };
    if let Some(hit) = entries.iter().find(|e| wanted(e)) {
        return Some(join(&hit.name));
    }
    if depth == 0 {
        return None;
    }
    (entries.iter())
        .filter(|e| e.kind == ArchiveEntryKind::Folder)
        .find_map(|e| find(device, &join(&e.name), depth - 1, wanted))
}

#[test]
fn a_nested_archive_lists_as_a_folder_and_a_file_in_it_opens_by_its_path() {
    let what = "a GTA VI archive through the archive device";
    let build = std::env::var_os(BUILD_ENV)
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    let (Some(build), Ok(Some(keys))) = (build, GtaViKeys::for_install(None)) else {
        skip_install(
            what,
            format_args!("set {BUILD_ENV} and {GTA6_SNAPSHOT_ENV} (or {GTA6_EXE_ENV})"),
        );
        return;
    };
    let mut archives: Vec<PathBuf> = (std::fs::read_dir(&build).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rpf"))
        .collect();
    archives.sort();
    assert!(!archives.is_empty(), "no .rpf under {}", build.display());

    // The keys travel into the open through the provider, as the mount hands one to every archive.
    let crypto: Arc<dyn RpfCryptoProvider> = Arc::new(Gta6Context {
        keys: Arc::new(keys),
        names: Arc::new(NameResolver::new()),
    });
    let (mut listed, mut nested_listed, mut opened) = (0, 0, 0);
    for path in &archives {
        let archive = RpfArchive::open_with(path, Arc::clone(&crypto))
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let device = ArchiveDevice::new(Arc::new(archive));
        let started = Instant::now();
        let root = device.entries("").expect("the root lists");
        let nested = root.iter().filter(|e| e.archive.is_some()).count();
        println!(
            "root of archive {listed}: {} entries, {nested} nested archives opened, in {:.1?}",
            root.len(),
            started.elapsed()
        );
        listed += 1;

        let is_archive = |e: &ArchiveEntry| e.archive.is_some();
        let Some(folder) = find(&device, "", 3, &is_archive) else {
            continue;
        };
        let inside = device
            .entries(&folder)
            .expect("a nested archive lists as a folder");
        nested_listed += 1;
        let small = |e: &ArchiveEntry| {
            e.kind == ArchiveEntryKind::File
                && e.archive.is_none()
                && (device.open_head(&format!("{folder}/{}", e.name), 16)).is_some()
        };
        if let Some(file) = inside.iter().find(|e| small(e)) {
            let rel = format!("{folder}/{}", file.name);
            assert!(device.exists(&rel), "a listed file exists by its path");
            opened += 1;
        }
    }
    println!("{listed} roots listed, {nested_listed} nested archives listed as folders, {opened} files opened by path");
    assert_eq!(listed, archives.len());
    assert!(nested_listed > 0, "a nested archive lists as a folder");
    assert!(
        opened > 0,
        "a file inside a nested archive opens by its path"
    );
}

/// Every logical path the walk of a mount meets, lowercase.
fn walked(game: &MountedGame) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    game.for_each_entry(|facts| {
        out.insert(facts.logical.to_ascii_lowercase());
    });
    out
}

/// The devices of the DLC packs a mount holds, lowercase, the title update's aside.
fn pack_devices(game: &MountedGame) -> BTreeSet<String> {
    (game.packs.iter())
        .map(|pack| pack.device_name.to_ascii_lowercase())
        .filter(|device| device != "update")
        .collect()
}

/// What mounting `install` whole and at the pack in the middle of its list shows: the level's mount
/// reports the later packs as left out and walks nothing of theirs, their patches included.
/// `Err` is why an install cannot show it.
fn at_the_level_in_the_middle(
    install: &GtaInstall,
    crypto: Arc<dyn RpfCryptoProvider>,
) -> Result<String, String> {
    let all = build_mounted_game(install, Arc::clone(&crypto), DlcCutoff::All)
        .expect("the install mounts");

    let mounted: Vec<usize> = (0..all.listed.len())
        .filter(|&at| all.listed[at].state == ListedPackState::Mounted)
        .collect();
    if mounted.len() < 2 {
        return Err("it mounts fewer than two listed packs".into());
    }
    let stop = mounted[mounted.len() / 2 - 1];
    let level = all.listed[stop].name.clone();
    let at_level = build_mounted_game(install, crypto, DlcCutoff::Level(level.clone()))
        .expect("the install mounts at a pack of its own list");

    assert_eq!(at_level.listed.len(), all.listed.len());
    for (at, (pack, whole)) in at_level.listed.iter().zip(&all.listed).enumerate() {
        assert_eq!(pack.name, whole.name);
        let expected = if at > stop {
            ListedPackState::LeftOut
        } else {
            whole.state
        };
        assert_eq!(pack.state, expected, "{} at level {level}", pack.name);
    }

    let kept = pack_devices(&at_level);
    let dropped: BTreeSet<String> = pack_devices(&all).difference(&kept).cloned().collect();
    assert!(!dropped.is_empty(), "the level {level} leaves a pack out");
    let of_a_dropped_pack = |logical: &str| {
        (logical.split_once(":/")).is_some_and(|(device, _)| dropped.contains(device))
    };
    let folders: Vec<String> = (at_level.roots.iter())
        .flat_map(|root| {
            let device = root.device.to_ascii_lowercase();
            (root.left_out.iter()).map(move |folder| format!("{device}:/{folder}"))
        })
        .collect();
    let in_a_folder_left_out = |logical: &str| folders.iter().any(|f| logical.starts_with(f));

    let (whole, level_walk) = (walked(&all), walked(&at_level));
    let beyond: Vec<&String> = level_walk.difference(&whole).collect();
    assert!(
        beyond.is_empty(),
        "the level walks what the whole mount does not: {:?}",
        &beyond[..beyond.len().min(5)]
    );
    let leaked: Vec<&String> = (level_walk.iter())
        .filter(|l| of_a_dropped_pack(l) || in_a_folder_left_out(l))
        .collect();
    assert!(
        leaked.is_empty(),
        "a left-out pack or a folder that holds or patches one is walked: {:?}",
        &leaked[..leaked.len().min(5)]
    );
    let unexplained: Vec<&String> = (whole.difference(&level_walk))
        .filter(|l| !of_a_dropped_pack(l) && !in_a_folder_left_out(l))
        .collect();
    assert!(
        unexplained.is_empty(),
        "the level hides what no left-out pack or patch of one holds: {:?}",
        &unexplained[..unexplained.len().min(5)]
    );
    Ok(format!(
        "level {level} keeps {} of {} packs and {} of {} walked files, {} folders skipped",
        kept.len(),
        kept.len() + dropped.len(),
        level_walk.len(),
        whole.len(),
        folders.len()
    ))
}

/// A real GTA V install mounted whole and at the pack in the middle of its list.
#[test]
fn a_real_install_at_a_level_of_dlc_walks_nothing_of_the_packs_listed_after_it() {
    let what = "a real GTA V install mounted at two levels of DLC";
    let roots = or_test_support::game_roots();
    if roots.is_empty() {
        skip_install(
            what,
            format_args!("set {} to an install root", or_test_support::GAME_ROOT_ENV),
        );
        return;
    }
    for root in &roots {
        let Some(exe) = Game::detect_from_root(root).and_then(|game| game.find_executable(root))
        else {
            skip_install(what, format_args!("{} has no executable", root.display()));
            continue;
        };
        let Ok(keys) = extract_keys(&exe, &|_| {}) else {
            skip_install(what, format_args!("no keys from {}", exe.display()));
            continue;
        };
        let crypto: Arc<dyn RpfCryptoProvider> =
            Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
        match at_the_level_in_the_middle(&GtaInstall { root: root.clone() }, crypto) {
            Ok(shown) => println!("{}: {shown}", root.display()),
            Err(why) => skip_install(what, format_args!("{}: {why}", root.display())),
        }
    }
}

/// ⚠ The install suite above skips with no install, so its checks run here on an install built
/// by hand: four packs, the last inside the title update's archive, each patched by a folder of it.
#[test]
fn the_checks_of_the_install_suite_hold_on_an_install_built_by_hand() {
    let scratch = tempfile::Builder::new()
        .prefix("or-title-two-levels-")
        .tempdir()
        .expect("scratch");
    let root = scratch.path();
    let archive = |entries: &[(&str, &[u8])]| {
        let mut builder = RpfBuilder::default();
        for (path, body) in entries {
            builder.insert_with(path, *body, false).expect("an entry");
        }
        builder.to_bytes().expect("save")
    };
    let write = |rel: &str, bytes: &[u8]| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    };
    let names = ["one", "two", "three", "four"];
    let pack = |name: &str, order: usize| {
        let setup = SetupData {
            name_hash: name.into(),
            device_name: format!("dlc_{name}"),
            dat_file: "content.xml".into(),
            order: order as i32,
            ..Default::default()
        };
        archive(&[
            ("setup2.xml", setup.to_xml().as_bytes()),
            ("content.xml", ContentXml::default().to_xml().as_bytes()),
            (&format!("{name}.ydr"), name.as_bytes()),
        ])
    };
    let list = DlcList {
        paths: (names.iter())
            .map(|name| format!("dlcpacks:/{name}/").into())
            .collect(),
    };
    write(
        "common.rpf",
        &archive(&[("data/dlclist.xml", list.to_xml().as_bytes())]),
    );
    for (order, name) in names.iter().enumerate().take(3) {
        write(&format!("x64/dlcpacks/{name}/dlc.rpf"), &pack(name, order));
    }
    let table = ExtraTitleUpdateData {
        mounts: (names.iter())
            .map(|name| TitleUpdateMount {
                device_name: format!("dlc_{name}"),
                path: format!("update:/dlc_patch/{name}/"),
            })
            .collect(),
    };
    let patches = names.map(|name| format!("dlc_patch/{name}/{name}_patch.ydr"));
    let (table, four) = (table.to_xml(), pack("four", 3));
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("common/data/ExtraTitleUpdateData.meta", table.as_bytes()),
        ("dlcpacks/four/dlc.rpf", &four),
    ];
    entries.extend(patches.iter().map(|path| (path.as_str(), &b"patch"[..])));
    write("update/update.rpf", &archive(&entries));

    let crypto: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
    let install = GtaInstall {
        root: root.to_path_buf(),
    };
    let shown = at_the_level_in_the_middle(&install, crypto).expect("four packs mount");
    // Each pack holds its two documents and one file.
    let of_a_pack = 3;
    // Whole: the list, the table, four patches, the archive of `four` and its files through the
    // title update, and the files of four packs.
    let whole = 1 + 1 + 4 + (1 + of_a_pack) + 4 * of_a_pack;
    // At `two`: the list, the table, two patches and the files of two packs.
    let at_two = 1 + 1 + 2 + 2 * of_a_pack;
    // Skipped: the patches of `three` and `four`, and the folder that holds `four`.
    assert_eq!(
        shown,
        format!(
            "level two keeps 2 of 4 packs and {at_two} of {whole} walked files, 3 folders skipped"
        )
    );
}
