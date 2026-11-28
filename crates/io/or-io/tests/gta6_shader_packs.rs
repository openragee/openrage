//! The shader packs of GTA VI's builds, side by side: each build's `shaders.rpf` holds `SGD5`
//! effect containers. The Xbox Series build's open, and a program's stream inflates to a `DXBC`
//! container. The PlayStation 5 build ships the same `SGD5` magic but a binding layout not measured
//! on it (its first layout head states a count that is not its record count, 88 for 28 records), so
//! [`Sgd5File::open_for`] refuses a PlayStation 5 container by that build's name (R72i, the platforms
//! plan's decision 3). The read reaching the layout still names where it stops, kept here beside the
//! by-name refusal as the record of the structural difference.
//!
//! Install-gated: it skips cleanly without the builds.
//!   `OPENRAGE_GTA6_BUILDS`  the roots of an Xbox Series build and a PlayStation 5 build, `;` between

#![cfg(all(feature = "gta6", feature = "bundled-keys"))]

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::container::rpf::gta6_extract::snapshot_for_exe;
use or_io::container::rpf::{Gta6Context, GtaViKeys, NameResolver, RpfArchive, RpfEntry};
use or_io::gta5::shaders::sgd5::Sgd5File;
use or_io::Platform;
use or_test_support::skip_install;

const BUILDS_ENV: &str = "OPENRAGE_GTA6_BUILDS";
const EXECUTABLES: [&str; 2] = ["game_scarlett_master.exe", "eboot.bin"];
/// The programs of one container whose stream is inflated to name what it holds.
const SAMPLE: usize = 64;

/// What one build's shader archive came to.
#[derive(Debug, Default, PartialEq, Eq)]
struct Packs {
    containers: usize,
    opened: usize,
    programs: usize,
    effects: usize,
    /// The first four bytes a sampled program's stream inflates to, with how many begin so.
    heads: BTreeMap<String, usize>,
    refused: Vec<String>,
}

fn packs_of(root: &Path, platform: Platform) -> Packs {
    let exe = (EXECUTABLES.iter().map(|name| root.join(name)))
        .find(|path| path.is_file())
        .expect("the build's executable");
    let (plain, _) = snapshot_for_exe(None, &exe).expect("the executable keys its snapshot");
    let context = Arc::new(Gta6Context {
        keys: Arc::new(GtaViKeys::parse(&plain).expect("the snapshot parses")),
        names: Arc::new(NameResolver::new()),
    });
    let archive = RpfArchive::open_with(root.join("shaders.rpf"), context).expect("shaders.rpf");
    let mut packs = Packs::default();
    for index in 0..archive.entries.len() as u32 {
        if !matches!(archive.entries[index as usize], RpfEntry::Binary(_)) {
            continue;
        }
        let bytes = match archive.read_entry(index) {
            Ok(bytes) => bytes,
            Err(e) => {
                packs
                    .refused
                    .push(format!("entry {index} does not read: {e}"));
                continue;
            }
        };
        if !bytes.starts_with(b"SGD5") {
            continue;
        }
        packs.containers += 1;
        let file = match Sgd5File::open_for(&bytes, platform) {
            Ok(file) => file,
            Err(e) => {
                // The platform-named refusal (decision 3), and — for the record — where a read on
                // the Xbox layout stops, so the structural difference stays visible.
                let probe = (Sgd5File::open(&bytes).err())
                    .map_or_else(String::new, |e| format!(" [Xbox-layout read: {e}]"));
                packs.refused.push(format!("entry {index}: {e}{probe}"));
                continue;
            }
        };
        packs.opened += 1;
        packs.programs += file.programs.len();
        packs.effects += file.effects.len();
        let step = (file.programs.len() / SAMPLE).max(1);
        for program in file.programs.iter().step_by(step) {
            let mut head = [0u8; 4];
            let stream = bytes.get(program.stream.clone()).unwrap_or_default();
            let read = flate2::read::DeflateDecoder::new(stream).read_exact(&mut head);
            let named = match read {
                Ok(()) => String::from_utf8_lossy(&head).into_owned(),
                Err(_) => "does not inflate".to_owned(),
            };
            *packs.heads.entry(named).or_default() += 1;
        }
    }
    packs
}

#[test]
fn a_playstation_5_shader_pack_is_the_xbox_series_s_container_with_another_binding_layout() {
    let roots: Vec<PathBuf> = std::env::var_os(BUILDS_ENV)
        .map(|v| std::env::split_paths(&v).filter(|p| p.is_dir()).collect())
        .unwrap_or_default();
    let (Some(xbox), Some(ps5)) = (
        roots.iter().find(|r| r.join(EXECUTABLES[0]).is_file()),
        roots.iter().find(|r| r.join(EXECUTABLES[1]).is_file()),
    ) else {
        skip_install(
            "the shader packs of GTA VI's builds",
            format_args!("set {BUILDS_ENV} to an Xbox Series build and a PlayStation 5 build"),
        );
        return;
    };
    let (xbox, ps5) = (
        packs_of(xbox, Platform::XboxSeries),
        packs_of(ps5, Platform::Ps5),
    );
    for (label, packs) in [("Xbox Series", &xbox), ("PlayStation 5", &ps5)] {
        println!(
            "{label}: {} SGD5 container(s), {} opened, {} programs, {} effects, streams begin {:?}",
            packs.containers, packs.opened, packs.programs, packs.effects, packs.heads
        );
        for line in &packs.refused {
            println!("  {line}");
        }
    }
    assert!(
        xbox.containers > 0 && ps5.containers > 0,
        "each build holds a container"
    );
    assert_eq!(
        xbox.opened, xbox.containers,
        "every Xbox Series container opens"
    );
    assert_eq!(xbox.heads.keys().collect::<Vec<_>>(), ["DXBC"]);
    // A PlayStation 5 container is refused by that build's name (decision 3), and the Xbox-layout
    // probe still names where the read stops: its first layout states, where the Xbox Series states
    // its record count, a larger number (88 for 28 records).
    assert_eq!((ps5.opened, ps5.refused.len()), (0, ps5.containers));
    for refused in &ps5.refused {
        assert!(
            refused.contains(Platform::Ps5.label()) && refused.contains("is not measured"),
            "a PlayStation 5 container is refused by name: {refused}"
        );
        assert!(
            refused.contains("does not hold to the measured layout")
                && refused.contains("a layout of"),
            "the Xbox-layout read still names where it stops: {refused}"
        );
    }
}
