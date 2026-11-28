//! `update/update2.rpf` in the app's mount, for both editions: its `common/`
//! half over `common.rpf`, its `x64/` half over the platform archives, and
//! whole on `update2:/`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::container::rpf::crypto::{
    extract_keys, Gta5RetailCryptoProvider, NoOpCryptoProvider, RpfCryptoProvider,
};
use or_io::container::rpf::RpfBuilder;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::{build_mounted_game, DlcCutoff, Game, GtaInstall, MountedGame};

fn write_archive(path: &Path, entries: &[(&str, &[u8])]) {
    let mut builder = RpfBuilder::default();
    for (inner, body) in entries {
        builder.insert_with(inner, *body, false).unwrap();
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    builder.save(path).unwrap();
}

/// An install shaped like a retail one, with `exe` naming its edition.
fn synthetic_install(name: &str, exe: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("mounts_update2")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(exe), b"").unwrap();
    write_archive(&root.join("common.rpf"), &[("data/version.txt", b"base")]);
    write_archive(&root.join("x64a.rpf"), &[("textures/credits.ytd", b"base")]);
    write_archive(
        &root.join("update").join("update.rpf"),
        &[("common/data/tu.dat", b"update")],
    );
    write_archive(
        &root.join("update").join("update2.rpf"),
        &[
            ("common/data/version.txt", b"update2"),
            ("common/data/ugc/only2.ugc", b"update2"),
            ("x64/textures/credits.ytd", b"update2"),
            ("x64/levels/gta5/script/script_rel.rpf", b"update2"),
        ],
    );
    root
}

fn mount(root: &Path) -> MountedGame {
    let crypto: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
    let install = GtaInstall {
        root: root.to_path_buf(),
    };
    build_mounted_game(&install, crypto, DlcCutoff::BaseGame).expect("the install mounts")
}

fn read(game: &MountedGame, path: &str) -> Option<String> {
    game.vfs
        .open(path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn assert_layers_update2(game: &MountedGame) {
    for path in [
        "common:/data/ugc/only2.ugc",
        "common:/data/version.txt",
        "commoncrc:/data/version.txt",
        "platform:/textures/credits.ytd",
        "platformcrc:/textures/credits.ytd",
        "platform:/levels/gta5/script/script_rel.rpf",
        "update2:/common/data/ugc/only2.ugc",
    ] {
        assert_eq!(read(game, path).as_deref(), Some("update2"), "{path}");
    }
    assert_eq!(read(game, "common:/data/tu.dat").as_deref(), Some("update"));
    assert_eq!(
        game.vfs.locate("common:/data/version.txt").as_deref(),
        Some("update/update2.rpf/common/data/version.txt")
    );

    let mut listed = Vec::new();
    game.for_each_file(|logical, _, _| listed.push(logical.to_string()));
    assert!(
        listed
            .iter()
            .any(|l| l == "update2:/common/data/ugc/only2.ugc"),
        "the browser's walk does not reach update2.rpf: {listed:?}"
    );
}

#[test]
fn an_enhanced_mount_layers_update2_over_the_base_archives() {
    let game = mount(&synthetic_install("enhanced", "GTA5_Enhanced.exe"));
    assert_eq!(game.game, Game::GtaVEnhanced);
    assert_layers_update2(&game);
}

#[test]
fn a_legacy_mount_layers_update2_over_the_base_archives() {
    let game = mount(&synthetic_install("legacy", "GTA5.exe"));
    assert_eq!(game.game, Game::GtaVLegacy);
    assert_layers_update2(&game);
}

/// Mounts every retail install of `edition` under [`GAME_ROOT_ENV`].
fn retail_mounts(edition: Game, what: &str) -> Vec<(PathBuf, MountedGame)> {
    let roots: Vec<PathBuf> = game_roots()
        .into_iter()
        .filter(|root| Game::detect_from_root(root) == Some(edition))
        .collect();
    if roots.is_empty() {
        skip_install(
            what,
            format_args!("no {} install; set {GAME_ROOT_ENV}", edition.label()),
        );
    }
    roots
        .into_iter()
        .map(|root| {
            let exe = edition
                .find_executable(&root)
                .expect("an install has its executable");
            let keys = extract_keys(&exe, &|_| {}).expect("keys from the install's executable");
            let crypto: Arc<dyn RpfCryptoProvider> =
                Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
            let install = GtaInstall { root: root.clone() };
            let game = build_mounted_game(&install, crypto, DlcCutoff::BaseGame)
                .expect("the install mounts");
            (root, game)
        })
        .collect()
}

#[test]
fn a_retail_enhanced_install_resolves_what_only_update2_holds() {
    for (root, game) in retail_mounts(
        Game::GtaVEnhanced,
        "a_retail_enhanced_install_resolves_what_only_update2_holds",
    ) {
        assert!(
            game.vfs
                .exists("common:/shaders/fxdb/sga_win32_60_final.awc"),
            "{}: the shader container only update2.rpf holds does not resolve",
            root.display()
        );
        assert_eq!(
            game.vfs.locate("common:/data/version.txt").as_deref(),
            Some("update/update2.rpf/common/data/version.txt"),
            "{}",
            root.display()
        );
        assert_eq!(
            game.vfs
                .locate("platform:/data/lang/american_rel.rpf")
                .as_deref(),
            Some("update/update2.rpf/x64/data/lang/american_rel.rpf"),
            "{}",
            root.display()
        );
    }
}

#[test]
fn a_retail_legacy_install_resolves_what_only_update2_holds() {
    for (root, game) in retail_mounts(
        Game::GtaVLegacy,
        "a_retail_legacy_install_resolves_what_only_update2_holds",
    ) {
        assert_eq!(
            game.vfs
                .locate("platform:/levels/gta5/script/script_rel.rpf")
                .as_deref(),
            Some("update/update2.rpf/x64/levels/gta5/script/script_rel.rpf"),
            "{}: the script archive only update2.rpf holds does not resolve",
            root.display()
        );
        assert_eq!(
            game.vfs.locate("common:/data/version.txt").as_deref(),
            Some("update/update2.rpf/common/data/version.txt"),
            "{}",
            root.display()
        );
        assert_eq!(
            game.vfs
                .locate("platform:/data/lang/american_rel.rpf")
                .as_deref(),
            Some("update/update2.rpf/x64/data/lang/american_rel.rpf"),
            "{}",
            root.display()
        );
    }
}
