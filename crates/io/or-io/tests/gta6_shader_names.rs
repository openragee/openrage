//! The names an Xbox Series build's `SGD5` containers state for their resources: every record that
//! points at a name is a sampler whose name hashes to the hash at its +0x10, and every sampler
//! points at one.
//!
//! Install-gated: it skips cleanly without the build.
//!   `OPENRAGE_GTA6_BUILD`  the root of an Xbox Series build, holding `shaders.rpf`

#![cfg(all(feature = "gta6", feature = "bundled-keys"))]

use std::path::PathBuf;
use std::sync::Arc;

use or_io::container::rpf::gta6_extract::snapshot_for_exe;
use or_io::container::rpf::{Gta6Context, GtaViKeys, NameResolver, RpfArchive, RpfEntry};
use or_io::gta5::shaders::sgd5::{Sgd5File, Sgd5Table};
use or_io::gta6::hash::name_hash32;
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const SAMPLER: u8 = 0x23;
/// Measured on the Xbox Series build 1.0.2.1: two containers, 932 effects, and this many sampler
/// records in their tables, each naming itself.
const NAMED: usize = 46_742;

#[test]
fn every_sampler_of_a_build_s_shader_containers_names_itself_by_a_name_that_hashes_to_its_record() {
    let Some(root) = std::env::var_os(BUILD_ENV)
        .map(PathBuf::from)
        .filter(|p| p.join("shaders.rpf").is_file())
    else {
        skip_install(
            "the names of GTA VI's shader resources",
            format_args!("set {BUILD_ENV} to an Xbox Series build"),
        );
        return;
    };
    let (plain, _) = snapshot_for_exe(None, &root.join("game_scarlett_master.exe"))
        .expect("the executable keys its snapshot");
    let context = Arc::new(Gta6Context {
        keys: Arc::new(GtaViKeys::parse(&plain).expect("the snapshot parses")),
        names: Arc::new(NameResolver::new()),
    });
    let archive = RpfArchive::open_with(root.join("shaders.rpf"), context).expect("shaders.rpf");
    let (mut containers, mut named, mut wrong) = (0, 0, Vec::new());
    for index in 0..archive.entries.len() as u32 {
        if !matches!(archive.entries[index as usize], RpfEntry::Binary(_)) {
            continue;
        }
        let bytes = archive
            .read_entry(index)
            .expect("an entry of shaders.rpf reads");
        if !bytes.starts_with(b"SGD5") {
            continue;
        }
        containers += 1;
        let file = Sgd5File::open(&bytes).expect("an Xbox Series container opens");
        let tables = [&file.globals]
            .into_iter()
            .chain(file.programs.iter().map(|p| &p.variables))
            .chain(file.effects.iter().flat_map(|e| [&e.parameters, &e.second]));
        for table in tables {
            named += check(table, &mut wrong);
        }
    }
    for line in wrong.iter().take(20) {
        println!("  {line}");
    }
    assert!(wrong.is_empty(), "{} records break the rule", wrong.len());
    assert_eq!((containers, named), (2, NAMED));
}

/// How many records of `table` name themselves; each that breaks the rule goes to `wrong`.
fn check(table: &Sgd5Table, wrong: &mut Vec<String>) -> usize {
    assert_eq!(table.resource_names.len(), table.resources.len());
    let mut named = 0;
    for (r, name) in table.resources.iter().zip(&table.resource_names) {
        match name {
            Some(name) if r.code == SAMPLER && name_hash32(name.as_bytes()) == r.name_hash => {
                named += 1;
            }
            None if r.code != SAMPLER => {}
            Some(_) => wrong.push(format!(
                "resource {:08X} of type {:#04X} names itself by a name of another hash, or is no sampler",
                r.name_hash, r.code
            )),
            None => wrong.push(format!(
                "sampler {:08X} names itself by no string",
                r.name_hash
            )),
        }
    }
    named
}
