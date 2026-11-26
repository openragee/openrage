use std::path::{Path, PathBuf};
use std::sync::Once;

use or_io::Generation;
use or_title::game::{
    registry, Brand, ContainerFamily, DlcPolicy, GameProfile, InstallLayout, Platform,
};
use or_title::Game;

#[derive(Debug, Clone, Copy, Default)]
struct Testopolis;

impl GameProfile for Testopolis {
    fn id(&self) -> &'static str {
        "testopolis"
    }

    fn label(&self) -> &'static str {
        "Testopolis"
    }

    fn description(&self) -> &'static str {
        "A title invented by a test, to find out what inventing one costs."
    }

    fn container(&self) -> ContainerFamily {
        ContainerFamily::Rpf7
    }

    fn native_target(&self) -> Generation {
        Generation::Gen8
    }

    fn asset_targets(&self) -> &'static [Generation] {
        &[Generation::Gen8]
    }

    fn executable_candidates(&self) -> &'static [&'static str] {
        &["Testopolis.exe"]
    }

    fn install_layout(&self) -> InstallLayout {
        InstallLayout {
            base_archive: "base.rpf",
            update_archive: Some("update/update.rpf"),
            dlc_directories: &["dlc"],
            platform_archive_prefix: "x64",
        }
    }

    fn dlc_policy(&self) -> DlcPolicy {
        DlcPolicy::MountAll
    }
}

fn register_testopolis() -> Game {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        static PROFILE: Testopolis = Testopolis;
        registry::register(&PROFILE);
    });
    Game::by_id("testopolis").expect("registered above")
}

/// A folder removed when the first of the pair drops, so hold it as long as the second is used.
fn scratch(name: &str) -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::Builder::new()
        .prefix(&format!("or-title-{name}-"))
        .tempdir()
        .expect("scratch");
    let dir = scratch.path().to_path_buf();
    (scratch, dir)
}

fn touch(dir: &Path, name: &str) {
    if let Some(parent) = Path::new(name).parent() {
        std::fs::create_dir_all(dir.join(parent)).expect("parents");
    }
    std::fs::write(dir.join(name), b"").expect("touch");
}

#[test]
fn the_trait_is_implementable_without_touching_anything_else() {
    let profile: &dyn GameProfile = &Testopolis;

    assert_eq!(profile.id(), "testopolis");
    assert_eq!(profile.container(), ContainerFamily::Rpf7);
    assert_eq!(profile.native_target(), Generation::Gen8);
    assert!(!profile.description().is_empty());
}

#[test]
fn a_new_title_gets_detection_for_free() {
    let (_scratch, dir) = scratch("testopolis-detect");
    touch(&dir, "Testopolis.exe");

    let profile = Testopolis;
    assert!(profile.matches_install(&dir));

    let found = profile.find_executable(&dir).expect("executable");
    assert_eq!(found, dir.join("Testopolis.exe"));
}

#[test]
fn detection_does_not_claim_a_directory_that_is_not_its_own() {
    let (_scratch, dir) = scratch("testopolis-other");
    touch(&dir, "GTA5.exe");

    assert!(
        !Testopolis.matches_install(&dir),
        "a title must not claim an install belonging to another"
    );
}

#[test]
fn everything_the_mount_path_asks_for_is_answered_by_the_profile() {
    let profile: &dyn GameProfile = &Testopolis;
    let layout = profile.install_layout();

    let (_scratch, dir) = scratch("testopolis-mount");
    touch(&dir, "Testopolis.exe");
    touch(&dir, layout.base_archive);
    touch(&dir, layout.update_archive.expect("an update archive"));
    for dlc in layout.dlc_directories {
        std::fs::create_dir_all(dir.join(dlc)).expect("dlc dir");
    }

    assert!(dir.join(layout.base_archive).is_file());
    assert!(dir.join(layout.update_archive.unwrap()).is_file());
    assert!(dir.join(layout.dlc_directories[0]).is_dir());
}

#[test]
fn a_title_that_cannot_extract_keys_says_so_and_names_no_cache() {
    let profile: &dyn GameProfile = &Testopolis;
    assert!(!profile.supports_key_extraction());
    assert!(profile.keys_cache_name().is_none());
}

#[test]
fn support_follows_the_container_and_needs_no_declaration() {
    assert!(Testopolis.is_supported());
}

#[test]
fn a_title_on_gta_vs_target_reads_gta_vs_declarations_with_no_pack() {
    let target = Testopolis.native_target();

    for format in or_io::gta5::formats::FORMATS {
        assert!(
            format.targets().any(|t| t == target),
            "`.{}` is not declared for {target:?}",
            format.ext
        );
    }
}

#[test]
fn a_registered_title_is_a_first_class_one() {
    let game = register_testopolis();

    assert_eq!(game.asset_slug(), "testopolis");
    assert_eq!(game.label(), "Testopolis");
    assert_eq!(game.generation(), Generation::Gen8);
    assert!(game.profile().is_supported());

    assert!(Game::all().any(|g| g == game));
    assert!(registry::supported().any(|p| p.id() == "testopolis"));
}

#[test]
fn registering_a_title_does_not_reorder_detection() {
    register_testopolis();

    let ids: Vec<&str> = registry::all().iter().map(|p| p.id()).collect();
    let enhanced = ids.iter().position(|id| *id == "gta-v-enhanced");
    let legacy = ids.iter().position(|id| *id == "gta-v-legacy");
    let added = ids.iter().position(|id| *id == "testopolis");

    assert!(
        enhanced < legacy,
        "Enhanced must still be asked first: {ids:?}"
    );
    assert!(added > legacy, "a registered title is asked last: {ids:?}");

    let (_scratch, dir) = scratch("order-enhanced");
    touch(&dir, "GTA5_Enhanced.exe");
    touch(&dir, "GTA5.exe");
    assert_eq!(
        registry::detect(&dir).expect("detect").id(),
        "gta-v-enhanced"
    );
}

#[test]
fn registration_is_idempotent_and_cannot_shadow_a_builtin() {
    register_testopolis();
    static AGAIN: Testopolis = Testopolis;
    assert!(
        !registry::register(&AGAIN),
        "a second registration of the same id should be refused"
    );

    static IMPOSTOR: Impostor = Impostor;
    assert!(!registry::register(&IMPOSTOR));
    assert_eq!(
        registry::by_id("gta-v-legacy").expect("legacy").label(),
        "GTA V (Legacy)"
    );
}

#[derive(Debug, Clone, Copy, Default)]
struct Impostor;

impl GameProfile for Impostor {
    fn id(&self) -> &'static str {
        "gta-v-legacy"
    }
    fn label(&self) -> &'static str {
        "Definitely GTA V"
    }
    fn description(&self) -> &'static str {
        "Should never be reachable."
    }
    fn container(&self) -> ContainerFamily {
        ContainerFamily::Rpf7
    }
    fn native_target(&self) -> Generation {
        Generation::Gen8
    }
    fn asset_targets(&self) -> &'static [Generation] {
        &[Generation::Gen8]
    }
    fn executable_candidates(&self) -> &'static [&'static str] {
        &["Impostor.exe"]
    }
    fn install_layout(&self) -> InstallLayout {
        InstallLayout {
            base_archive: "base.rpf",
            update_archive: None,
            dlc_directories: &[],
            platform_archive_prefix: "",
        }
    }
    fn dlc_policy(&self) -> DlcPolicy {
        DlcPolicy::MountAll
    }
}

#[test]
fn a_title_that_declares_no_store_names_is_simply_not_auto_discovered() {
    let game = register_testopolis();
    let hints = game.profile().store_hints();

    assert!(hints.steam_folders.is_empty());
    assert!(hints.launcher_registry_keys.is_empty());
    assert!(hints.epic_display_names.is_empty());

    let legacy = registry::by_id("gta-v-legacy").expect("legacy");
    assert!(!legacy.store_hints().steam_folders.is_empty());
}

#[test]
fn a_title_gets_a_plate_to_be_drawn_with_whether_or_not_it_asked_for_one() {
    let game = register_testopolis();
    let default_brand = Brand::default();
    let brand = game.profile().brand();

    assert_eq!(brand.caption, default_brand.caption);
    assert!(brand.caption_scale > 0.0, "a zero scale paints nothing");

    let enhanced = registry::by_id("gta-v-enhanced").expect("enhanced");
    assert_eq!(enhanced.brand().caption, "V+");
}

#[test]
fn the_icon_list_falls_back_to_the_detection_list() {
    let game = register_testopolis();
    assert_eq!(
        game.profile().icon_executable_candidates(),
        game.profile().executable_candidates()
    );
    assert_ne!(
        Game::GtaVLegacy.profile().icon_executable_candidates(),
        Game::GtaVLegacy.profile().executable_candidates()
    );
}

#[test]
fn a_registered_title_round_trips_through_preferences() {
    let game = register_testopolis();

    let json = serde_json::to_string(&game).expect("serializes");
    assert_eq!(json, "\"testopolis\"");

    let back: Game = serde_json::from_str(&json).expect("parses");
    assert_eq!(back, game);
}

#[test]
fn the_enum_variant_names_prefs_used_to_store_still_load() {
    for (old, expected) in [
        ("\"GtaVLegacy\"", Game::GtaVLegacy),
        ("\"GtaVEnhanced\"", Game::GtaVEnhanced),
        ("\"Rdr2\"", Game::Rdr2),
    ] {
        let parsed: Game = serde_json::from_str(old).expect("parses");
        assert_eq!(parsed, expected, "{old} should still load");
    }
}

#[test]
fn a_title_nothing_registers_is_refused_rather_than_invented() {
    assert!(Game::by_id("never-registered").is_none());
    assert!(serde_json::from_str::<Game>("\"never-registered\"").is_err());
}

#[test]
fn every_shipped_title_took_the_same_two_steps() {
    for profile in registry::all() {
        assert!(!profile.id().is_empty());
        // A console package has no executable on disk, and is known by the title id it states.
        assert!(
            !profile.executable_candidates().is_empty()
                || profile.gdk_title_id().is_some()
                || !profile.playstation_title_ids().is_empty(),
            "{} names nothing an install is known by",
            profile.id()
        );
    }
}

// Enhanced lists `GTA5.exe` among its executables for key extraction, so the
// default `matches_install` would claim every Legacy install; it overrides it.
#[test]
fn one_shipped_title_needs_behaviour_and_the_rest_are_data() {
    let (_scratch, dir) = scratch("marker-superset");
    touch(&dir, "GTA5.exe");

    let mut overriding: Vec<&str> = Vec::new();
    let mut claiming: Vec<&str> = Vec::new();
    for profile in registry::all() {
        let a_candidate_is_present = profile
            .executable_candidates()
            .iter()
            .any(|name| dir.join(name).is_file());
        if !a_candidate_is_present {
            continue;
        }
        if profile.matches_install(&dir) {
            claiming.push(profile.id());
        } else {
            overriding.push(profile.id());
        }
    }

    println!(
        "with only GTA5.exe on disk: {claiming:?} claim the install by the \
         derivation, {overriding:?} name it and refuse it anyway"
    );

    assert_eq!(
        overriding,
        vec!["gta-v-enhanced"],
        "exactly one shipped title overrides detection; if a second appears, a \
         marker distinct from the executable list has become the thing to add \
         to GameProfile"
    );
    assert_eq!(claiming, vec!["gta-v-legacy"]);
}

/// A console build of Testopolis: no executable on disk, known by the title id its package states.
#[derive(Debug, Clone, Copy, Default)]
struct TestopolisPs4;

impl GameProfile for TestopolisPs4 {
    fn id(&self) -> &'static str {
        "testopolis-ps4"
    }

    fn label(&self) -> &'static str {
        "Testopolis (PlayStation 4)"
    }

    fn title(&self) -> &'static str {
        Testopolis.title()
    }

    fn platform(&self) -> Platform {
        Platform::Ps4
    }

    fn playstation_title_ids(&self) -> &'static [&'static str] {
        &["CUSA99999"]
    }

    fn description(&self) -> &'static str {
        Testopolis.description()
    }

    fn container(&self) -> ContainerFamily {
        ContainerFamily::Rpf7
    }

    fn native_target(&self) -> Generation {
        Generation::Gen8
    }

    fn asset_targets(&self) -> &'static [Generation] {
        &[Generation::Gen8]
    }

    fn executable_candidates(&self) -> &'static [&'static str] {
        &[]
    }

    fn install_layout(&self) -> InstallLayout {
        InstallLayout {
            platform_archive_prefix: "ps4",
            ..Testopolis.install_layout()
        }
    }

    fn dlc_policy(&self) -> DlcPolicy {
        DlcPolicy::MountAll
    }
}

#[test]
fn a_console_build_is_one_more_profile_found_by_its_title_id() {
    static PROFILE: TestopolisPs4 = TestopolisPs4;
    registry::register(&PROFILE);

    let (_scratch, dir) = scratch("testopolis-ps4");
    std::fs::create_dir_all(dir.join("sce_sys")).expect("sce_sys");
    std::fs::write(
        dir.join("sce_sys/param.sfo"),
        b"\0PSF\x01\x01\0\0TITLE_ID\0CUSA99999\0",
    )
    .expect("param.sfo");
    let found = registry::detect(&dir).expect("the package is a build");
    assert_eq!(found.id(), "testopolis-ps4");
    assert_eq!(found.platform(), Platform::Ps4);
    assert_eq!(
        found.title(),
        Testopolis.title(),
        "both builds are the same game"
    );

    let (_other, other) = scratch("another-ps4");
    std::fs::create_dir_all(other.join("sce_sys")).expect("sce_sys");
    std::fs::write(other.join("sce_sys/param.sfo"), b"TITLE_ID\0CUSA00001\0").expect("param.sfo");
    assert!(
        registry::detect(&other).is_none(),
        "another title's package was claimed"
    );
}
