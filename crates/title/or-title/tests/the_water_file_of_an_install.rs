//! The water files a GTA V mount lists, each read through the mount: the reader types every
//! element and attribute the file states, and an unchanged file saves to the bytes it came as.
//!
//! The first test holds the walk and the checks on an install built here. ⚠ The second is
//! install-gated: `OPENRAGE_GAME_ROOT` names the installs, `;`-separated, and without one it
//! records its skip. A skipped suite has not passed.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::container::rpf::crypto::{
    extract_keys, Gta5RetailCryptoProvider, NoOpCryptoProvider, RpfCryptoProvider,
};
use or_io::container::rpf::RpfBuilder;
use or_io::gta5::data::water::{is_water_file, CWaterData};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::{build_mounted_game, DlcCutoff, Game, GtaInstall, MountedGame};

/// The level's own water file, which every install holds.
const LEVEL_WATER: &str = "common:/data/levels/gta5/water.xml";

fn mount(root: &Path, crypto: Arc<dyn RpfCryptoProvider>) -> MountedGame {
    let install = GtaInstall {
        root: root.to_path_buf(),
    };
    build_mounted_game(&install, crypto, DlcCutoff::All).expect("the install mounts")
}

/// Every file of the mount that is named as a water file, by its path, with its bytes.
fn water_files(game: &MountedGame) -> Vec<(String, Vec<u8>)> {
    let mut paths = Vec::new();
    game.for_each_file(|logical, _, leaf| {
        if is_water_file(leaf) {
            paths.push(logical.to_string());
        }
    });
    paths
        .into_iter()
        .map(|path| {
            let bytes = (game.vfs.open(&path))
                .unwrap_or_else(|| panic!("{path}: the mount lists it and does not open it"));
            (path, bytes)
        })
        .collect()
}

/// How many water, calming and wave quads `bytes` hold, or why they do not pass.
fn checked(bytes: &[u8]) -> Result<[usize; 3], String> {
    let water = CWaterData::open(bytes).map_err(|e| e.to_string())?;
    let unnamed = water.unnamed();
    if !unnamed.is_empty() {
        return Err(format!("the reader does not type {unnamed:?}"));
    }
    let saved = water.save().map_err(|e| e.to_string())?;
    if saved != bytes {
        return Err("an unchanged file saves to other bytes".to_string());
    }
    let again = CWaterData::open(&saved).map_err(|e| format!("the save does not reopen: {e}"))?;
    if again != water || again.save().map_err(|e| e.to_string())? != saved {
        return Err("the save is not its own fixed point".to_string());
    }
    Ok([
        water.water_quads.len(),
        water.calming_quads.len(),
        water.wave_quads.len(),
    ])
}

fn write_archive(path: &Path, entries: &[(&str, &[u8])]) {
    let mut builder = RpfBuilder::default();
    for (inner, body) in entries {
        builder.insert_with(inner, *body, false).unwrap();
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    builder.save(path).unwrap();
}

/// The corners of an item, laid out as the documents below are.
const CORNERS: &str = r#"
      <minX value="-8" />
      <maxX value="8" />
      <minY value="0" />
      <maxY value="4" />"#;

#[test]
fn the_water_files_of_a_mount_built_here_are_found_read_and_checked() {
    let quad = format!("\n    <Item>{CORNERS}\n      <z value=\"1.50\" />\n    </Item>");
    let calm = format!("\n    <Item>{CORNERS}\n      <fDampening value=\"0.5\" />\n    </Item>");
    let level = format!("<WaterData>\n  <WaterQuads>{quad}\n  </WaterQuads>\n</WaterData>\n");
    let island = format!(
        "<WaterData>\n  <WaterQuads>{quad}{quad}\n  </WaterQuads>\n  \
         <CalmingQuads>{calm}\n  </CalmingQuads>\n</WaterData>\n"
    );
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("the_water_file_of_an_install");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("GTA5.exe"), b"").unwrap();
    write_archive(
        &root.join("common.rpf"),
        &[
            ("data/levels/gta5/water.xml", level.as_bytes()),
            ("data/watertune.xml", b"<CMainWaterTune />"),
        ],
    );
    write_archive(
        &root.join("update").join("update.rpf"),
        &[(
            "common/data/levels/gta5/water_island.xml",
            island.as_bytes(),
        )],
    );

    let game = mount(&root, Arc::new(NoOpCryptoProvider::new()));
    assert_eq!(
        game.vfs.open(LEVEL_WATER).as_deref(),
        Some(level.as_bytes())
    );
    let mut found: Vec<(String, Result<[usize; 3], String>)> = water_files(&game)
        .into_iter()
        .map(|(path, bytes)| (path, checked(&bytes)))
        .collect();
    found.sort();
    assert_eq!(
        found,
        [
            (LEVEL_WATER.to_string(), Ok([1, 0, 0])),
            (
                "update:/common/data/levels/gta5/water_island.xml".to_string(),
                Ok([2, 1, 0])
            ),
        ]
    );

    // A file the reader does not type whole, and one that is no water file, do not pass.
    let fogged = level.replace("<z value", "<fog value=\"1\" />\n      <z value");
    assert_eq!(
        checked(fogged.as_bytes()),
        Err("the reader does not type {\"WaterData/WaterQuads/Item/fog\"}".to_string())
    );
    assert_eq!(
        checked(b"<CMainWaterTune />"),
        Err("the document's root is <CMainWaterTune>, not <WaterData>".to_string())
    );
}

/// The slow tier names this module: it mounts whole installs.
mod a_real_install {
    use super::*;

    #[test]
    fn every_water_file_is_typed_whole_and_saves_to_its_fixed_point() {
        let what = "every water file of a real install typed whole, at its fixed point";
        let roots: Vec<PathBuf> = game_roots()
            .into_iter()
            .filter(|root| {
                let edition = Game::detect_from_root(root);
                edition == Some(Game::GtaVLegacy) || edition == Some(Game::GtaVEnhanced)
            })
            .collect();
        if roots.is_empty() {
            skip_install(what, format_args!("no GTA V install; set {GAME_ROOT_ENV}"));
            return;
        }
        for root in roots {
            let edition = Game::detect_from_root(&root).expect("a GTA V install");
            let exe = (edition.find_executable(&root)).expect("an install has its executable");
            let keys = extract_keys(&exe, &|_| {}).expect("keys from the install's executable");
            let game = mount(
                &root,
                Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))),
            );
            assert!(
                game.vfs.exists(LEVEL_WATER),
                "{}: no {LEVEL_WATER}",
                root.display()
            );
            let files = water_files(&game);
            assert!(
                !files.is_empty(),
                "{}: the mount lists no water file",
                root.display()
            );
            for (path, bytes) in files {
                let [water, calming, wave] = checked(&bytes)
                    .unwrap_or_else(|why| panic!("{}: {path}: {why}", root.display()));
                eprintln!(
                    "{}: {path}: {water} water, {calming} calming and {wave} wave quads",
                    root.display()
                );
                assert!(water > 0, "{}: {path}: no water quad", root.display());
            }
        }
    }
}
