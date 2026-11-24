// Every folder here is written by the test: an Xbox package's root holds a
// `MicrosoftGame.Config` and a `common.rpf`, and its executable may be absent.

use std::path::{Path, PathBuf};

use or_title::game::{registry, ContainerFamily};
use or_title::gta5::{Gta5, InstallError};
use or_title::Game;

const CONFIG: &str = "MicrosoftGame.Config";

/// A folder removed when the first of the pair drops, so hold it as long as the second is used.
fn scratch(name: &str) -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::Builder::new()
        .prefix(&format!("or-title-gta-vi-{name}-"))
        .tempdir()
        .expect("scratch");
    let dir = scratch.path().to_path_buf();
    (scratch, dir)
}

fn write(dir: &Path, name: &str, bytes: &[u8]) {
    if let Some(parent) = Path::new(name).parent() {
        std::fs::create_dir_all(dir.join(parent)).expect("parents");
    }
    std::fs::write(dir.join(name), bytes).expect("write");
}

/// A GDK title config as the package spells one: a BOM, then a `<Game>` whose children include `<TitleId>`.
fn config(title_id: &str) -> Vec<u8> {
    let mut bytes = b"\xEF\xBB\xBF".to_vec();
    bytes.extend_from_slice(
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
             <Game configVersion=\"1\">\n  \
             <MSAAppId>0000000000000000</MSAAppId>\n  \
             <TitleId>{title_id}</TitleId>\n  \
             <ExecutableList>\n    \
             <Executable Name=\"game.exe\" Id=\"Game\" TargetDeviceFamily=\"Scarlett\" />\n  \
             </ExecutableList>\n\
             </Game>\n"
        )
        .as_bytes(),
    );
    bytes
}

/// The package root without its executable: the config and a `common.rpf`.
fn gta_vi_package(dir: &Path) {
    write(dir, CONFIG, &config("7952987A"));
    write(dir, "common.rpf", b"");
}

#[test]
fn a_package_without_its_executable_is_gta_vi_by_its_config() {
    let (_scratch, dir) = scratch("config");
    gta_vi_package(&dir);

    assert_eq!(registry::detect(&dir).map(|p| p.id()), Some("gta-vi"));
    assert_eq!(Game::detect_from_root(&dir), Game::GtaVI);
}

#[test]
fn the_title_id_is_compared_without_case() {
    let (_scratch, dir) = scratch("lowercase");
    write(&dir, CONFIG, &config(&"7952987A".to_ascii_lowercase()));

    assert_eq!(Game::detect_from_root(&dir), Game::GtaVI);
}

#[test]
fn the_executable_alone_marks_gta_vi() {
    let (_scratch, dir) = scratch("exe");
    write(&dir, "game_scarlett_master.exe", b"");

    assert_eq!(Game::detect_from_root(&dir), Game::GtaVI);
}

#[test]
fn gta_vi_is_known_and_unsupported_and_says_why() {
    let profile = Game::GtaVI.profile();

    assert_eq!(profile.container(), ContainerFamily::Rpf9);
    assert!(!profile.is_supported());
    assert!(!ContainerFamily::Rpf9.is_implemented());
    assert!(!profile.supports_key_extraction());
    assert!(!registry::supported().any(|p| p.id() == "gta-vi"));
    assert!(Game::all().any(|g| g == Game::GtaVI));

    let description = profile.description();
    for word in ["RPF9", "encrypted", "key", "table", "plain folder"] {
        assert!(description.contains(word), "{description:?} lacks {word:?}");
    }
    assert!(!description.contains("none of its archives open"));
    assert_eq!(Game::GtaVI.generation().edition_label(), "GTA VI");
}

#[test]
fn a_gta_vi_package_is_not_opened_as_gta_v() {
    let (_scratch, dir) = scratch("not-gta5");
    gta_vi_package(&dir);

    match Gta5::at(&dir) {
        Err(InstallError::NotGta5 { title, .. }) => assert_eq!(title, "GTA VI"),
        Err(other) => panic!("refused for the wrong reason: {other}"),
        Ok(_) => panic!("a GTA VI package was taken for a GTA V install"),
    }
}

#[test]
fn a_gta_v_install_is_not_gta_vi() {
    let (_scratch, dir) = scratch("gta5");
    write(&dir, "GTA5.exe", b"");
    write(&dir, "common.rpf", b"");
    assert_eq!(Game::detect_from_root(&dir), Game::GtaVLegacy);

    write(&dir, "GTA5_Enhanced.exe", b"");
    assert_eq!(Game::detect_from_root(&dir), Game::GtaVEnhanced);
}

#[test]
fn an_rdr2_install_is_not_gta_vi() {
    let (_scratch, dir) = scratch("rdr2");
    write(&dir, "RDR2.exe", b"");
    write(&dir, "common_0.rpf", b"");

    assert_eq!(Game::detect_from_root(&dir), Game::Rdr2);
}

#[test]
fn a_config_that_does_not_name_gta_vi_marks_nothing() {
    let other_title = config("12345678");
    let longer_id = config("7952987A0");
    let not_a_game = String::from_utf8(config("7952987A"))
        .expect("utf-8")
        .replace("Game", "Package")
        .into_bytes();
    let nested = b"<Game><Other><TitleId>7952987A</TitleId></Other></Game>".to_vec();
    let malformed = b"<Game><TitleId>7952987A</Title".to_vec();
    let mut oversized = config("7952987A");
    oversized.extend(std::iter::repeat_n(b' ', 64 * 1024));
    // Under the size cap, and deep enough that dropping its tree whole would overflow the stack.
    let deep = format!(
        "<Game><TitleId>7952987A</TitleId>{}{}</Game>",
        "<a>".repeat(9_000),
        "</a>".repeat(9_000)
    )
    .into_bytes();
    assert!(deep.len() < 64 * 1024);

    for (what, bytes) in [
        ("another title", other_title),
        ("a longer id", longer_id),
        ("a root that is not Game", not_a_game),
        ("a TitleId below another element", nested),
        ("malformed XML", malformed),
        ("an oversized file", oversized),
        ("elements nested 9,000 deep", deep),
    ] {
        let (_scratch, dir) = scratch("unrelated");
        write(&dir, CONFIG, &bytes);
        write(&dir, "common.rpf", b"");

        assert!(
            registry::detect(&dir).is_none(),
            "{what}: claimed by {:?}",
            registry::detect(&dir).map(|p| p.id())
        );
    }
}
