//! `dlc-list` asked for a level of DLC and a mode, over an install of unencrypted archives built here.

use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::crypto::{NoOpCryptoProvider, RpfCryptoProvider};
use or_io::container::rpf::RpfBuilder;
use or_title::dlc::{ContentChangeSet, ContentChangeSetGroup};
use or_title::{ContentXml, DataFileEntry, DlcList, SetupData};

use super::*;

fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = RpfBuilder::default();
    for (path, body) in entries {
        builder
            .insert_with(path, *body, false)
            .expect("add an entry");
    }
    builder.to_bytes().expect("save")
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(path, bytes).expect("write");
}

fn write_pack(root: &Path, setup: &SetupData, content: &ContentXml) {
    write(
        &root
            .join("x64/dlcpacks")
            .join(&setup.name_hash)
            .join("dlc.rpf"),
        &archive(&[
            ("setup2.xml", setup.to_xml().as_bytes()),
            ("content.xml", content.to_xml().as_bytes()),
        ]),
    );
}

fn setup(name: &str, order: i32, groups: &[(&str, &str)]) -> SetupData {
    SetupData {
        name_hash: name.into(),
        device_name: format!("dlc_{name}"),
        dat_file: "content.xml".into(),
        order,
        content_change_set_groups: groups
            .iter()
            .map(|(group, change_set)| ContentChangeSetGroup {
                name_hash: group.to_string(),
                content_change_sets: vec![change_set.to_string().into()],
            })
            .collect(),
        ..Default::default()
    }
}

/// A content document of one disabled data file for each change set, which that change set enables.
fn content(change_sets: &[&str]) -> ContentXml {
    let file = |name: &str| format!("dlc_alpha:/{}.rpf", name.to_ascii_lowercase());
    ContentXml {
        data_files: (change_sets.iter())
            .map(|name| DataFileEntry {
                filename: file(name),
                file_type: "RPF_FILE".into(),
                disabled: true,
                ..Default::default()
            })
            .collect(),
        content_change_sets: (change_sets.iter())
            .map(|name| ContentChangeSet {
                name: name.to_string(),
                files_to_enable: vec![file(name).into()],
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

/// An install listing `alpha`, whose setup states a map group for each mode, then `beta`.
fn install() -> (tempfile::TempDir, GtaInstall) {
    let scratch = tempfile::Builder::new()
        .prefix("or-cli-dlc-list-")
        .tempdir()
        .expect("scratch");
    let root = scratch.path().to_path_buf();
    let list = DlcList {
        paths: ["alpha", "beta"]
            .iter()
            .map(|pack| format!("dlcpacks:/{pack}/").into())
            .collect(),
    };
    write(
        &root.join("common.rpf"),
        &archive(&[("data/dlclist.xml", list.to_xml().as_bytes())]),
    );
    write_pack(
        &root,
        &setup(
            "alpha",
            10,
            &[("GROUP_MAP", "ALPHA_MAP"), ("GROUP_MAP_SP", "ALPHA_SP")],
        ),
        &content(&["ALPHA_MAP", "ALPHA_SP", "ALPHA_LOOSE"]),
    );
    write_pack(&root, &setup("beta", 20, &[]), &ContentXml::default());
    (scratch, GtaInstall { root })
}

fn mounted(install: &GtaInstall, level: Option<&str>, mode: Option<&str>) -> MountedGame {
    let (cutoff, options) = dlc_list_mount(None, level, mode).expect("the flags read");
    let crypto: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
    build_mounted_game_with(install, crypto, cutoff, options).expect("the synthetic install mounts")
}

fn mode_of(word: Option<&str>) -> Option<MapMode> {
    word.map(|word| MapMode::from_label(word).expect("a mode"))
}

fn notes(install: &GtaInstall, level: Option<&str>, mode: Option<&str>) -> Vec<String> {
    dlc_list_notes(&mounted(install, level, mode), mode_of(mode))
}

#[test]
fn a_level_and_a_mode_are_read_from_the_flags_and_a_level_beside_a_cutoff_is_refused() {
    let (cutoff, options) = dlc_list_mount(None, None, None).expect("no flag reads");
    assert_eq!(cutoff, DlcCutoff::All);
    assert_eq!(options, MountOptions::default());

    let (cutoff, options) =
        dlc_list_mount(None, Some(" mpsum2 "), Some("Multiplayer")).expect("both read");
    assert_eq!(cutoff, DlcCutoff::Level("mpsum2".into()));
    assert_eq!(options.mode, Some(MapMode::Multiplayer));
    assert_eq!(options.overlay, MountOptions::default().overlay);

    let (cutoff, options) = dlc_list_mount(Some("mpsum2"), None, Some("story")).expect("reads");
    assert_eq!(
        cutoff,
        resolve_cutoff(Some("mpsum2")),
        "--dlc is the cutoff it was"
    );
    assert_eq!(options.mode, Some(MapMode::Story));

    let both = dlc_list_mount(Some("2699"), Some("mpsum2"), None).expect_err("two places to stop");
    assert!(both.contains("--dlc") && both.contains("--level"), "{both}");
    let unknown = dlc_list_mount(None, None, Some("online")).expect_err("not a mode");
    assert!(unknown.contains("`online`"), "{unknown}");
}

#[test]
fn the_notes_name_the_packs_a_level_left_out_and_what_the_mode_made_of_each_change_set() {
    let (_scratch, install) = install();

    assert_eq!(
        notes(&install, Some("alpha"), Some("story")),
        [
            "left out, 1 of 2 listed pack(s): beta",
            "as story: 2 of 3 data file(s) enabled",
            "  alpha: left out, stated for the other mode: ALPHA_MAP",
            "  alpha: no mode stated, applied as without --mode: ALPHA_LOOSE",
        ]
    );
    assert_eq!(
        notes(&install, None, Some("multiplayer")),
        [
            "as multiplayer: 2 of 3 data file(s) enabled",
            "  alpha: left out, stated for the other mode: ALPHA_SP",
            "  alpha: no mode stated, applied as without --mode: ALPHA_LOOSE",
        ]
    );
    assert_eq!(
        notes(&install, Some("alpha"), None),
        ["left out, 1 of 2 listed pack(s): beta"]
    );
}

#[test]
fn with_neither_flag_the_table_is_followed_by_nothing() {
    let (_scratch, install) = install();
    assert!(notes(&install, None, None).is_empty());
}

#[test]
fn a_level_the_list_lacks_fails_the_mount_with_its_name() {
    let (_scratch, install) = install();
    let (cutoff, options) = dlc_list_mount(None, Some("gamma"), None).expect("the flag reads");
    let crypto: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
    let Err(error) = build_mounted_game_with(&install, crypto, cutoff, options) else {
        panic!("a level the list lacks mounted")
    };
    assert!(error.to_string().contains("`gamma`"), "{error}");
}

#[test]
fn the_text_is_a_row_for_each_mounted_pack_their_count_and_then_the_notes() {
    let (_scratch, install) = install();
    let text = dlc_list_text(
        &mounted(&install, Some("alpha"), Some("story")),
        mode_of(Some("story")),
    );
    let [row, blank, count, below @ ..] = text.as_slice() else {
        panic!("a row, a blank line and a count: {text:?}")
    };
    assert!(row.starts_with("  0  alpha "), "{row}");
    let pack = Path::new("x64")
        .join("dlcpacks")
        .join("alpha")
        .join("dlc.rpf");
    assert!(row.ends_with(&*pack.to_string_lossy()), "{row}");
    assert_eq!((blank.as_str(), count.as_str()), ("", "1 pack(s)"));
    assert_eq!(below, notes(&install, Some("alpha"), Some("story")));
    assert!(!below.is_empty());

    let whole = dlc_list_text(&mounted(&install, None, None), None);
    assert_eq!(
        whole.len(),
        4,
        "two rows, a blank line and a count: {whole:?}"
    );
    assert_eq!(whole[3], "2 pack(s)");
}

#[test]
fn the_json_names_the_packs_left_out_and_with_a_mode_what_it_made_of_each_change_set() {
    let (_scratch, install) = install();
    let warnings = ["a note".to_string()];

    let doc = dlc_list_json(
        &mounted(&install, Some("alpha"), Some("story")),
        &warnings,
        mode_of(Some("story")),
    );
    assert_eq!(doc["count"], 1);
    assert_eq!(doc["packs"][0]["nameHash"], "alpha");
    assert_eq!(doc["warnings"], serde_json::json!(["a note"]));
    assert_eq!(doc["leftOut"], serde_json::json!(["beta"]));
    assert_eq!(doc["mode"], "story");
    assert_eq!(
        doc["changeSets"],
        serde_json::json!([{
            "pack": "alpha",
            "leftOut": ["ALPHA_MAP"],
            "unstated": ["ALPHA_LOOSE"],
        }])
    );

    let doc = dlc_list_json(&mounted(&install, None, None), &[], None);
    assert_eq!(doc["count"], 2);
    assert_eq!(
        doc["leftOut"],
        serde_json::json!([]),
        "the key is always there"
    );
    let keys: Vec<&str> = (doc.as_object().expect("an object").keys())
        .map(String::as_str)
        .collect();
    assert!(
        !keys.contains(&"mode") && !keys.contains(&"changeSets"),
        "with no mode the document says nothing of one: {keys:?}"
    );
}

/// ⚠ A pack the list names and the install lacks is skipped, which is not left out.
#[test]
fn a_listed_pack_that_did_not_mount_is_not_named_as_left_out() {
    let (_scratch, install) = install();
    std::fs::remove_file(install.root.join("x64/dlcpacks/alpha/dlc.rpf")).expect("alpha leaves");
    let game = mounted(&install, Some("alpha"), None);
    assert_eq!(packs_left_out(&game), ["beta"]);
    assert_eq!(packs_left_out(&mounted(&install, None, None)), [""; 0]);
}

fn dlc_list(install: &GtaInstall, args: &[&str]) -> ExitCode {
    let root = install.root.to_str().expect("a scratch path is UTF-8");
    // A keys file that is not there: the archives are not encrypted, and no cache of this machine is read.
    let keys = install.root.join("no-keys.json");
    let mut all = vec!["--game", root, "--keys", keys.to_str().expect("UTF-8")];
    all.extend_from_slice(args);
    run_dlc_list(&all.iter().map(|a| a.to_string()).collect::<Vec<_>>())
}

#[test]
fn the_verb_reads_its_flags_in_both_spellings_and_fails_on_a_level_the_list_lacks() {
    let (_scratch, install) = install();
    let (usage, failed) = (ExitCode::from(2), ExitCode::from(1));

    assert_eq!(dlc_list(&install, &[]), ExitCode::SUCCESS);
    assert_eq!(
        dlc_list(&install, &["--level", "alpha", "--mode", "story", "--json"]),
        ExitCode::SUCCESS
    );
    assert_eq!(
        dlc_list(&install, &["--level=beta", "--mode=multiplayer"]),
        ExitCode::SUCCESS
    );
    assert_eq!(dlc_list(&install, &["--level", "gamma"]), failed);
    assert_eq!(dlc_list(&install, &["--level=gamma"]), failed);
    assert_eq!(dlc_list(&install, &["--mode", "online"]), usage);
    assert_eq!(dlc_list(&install, &["--mode=online"]), usage);
    assert_eq!(dlc_list(&install, &["--dlc=0", "--level", "alpha"]), usage);
    assert_eq!(dlc_list(&install, &["--level"]), usage);
}
