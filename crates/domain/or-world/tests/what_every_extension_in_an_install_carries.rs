//! Every extension a retail install ships, held to its kind's declaration.
//!
//! Point `OPENRAGE_GAME_ROOT` at one or more install roots, `;`-separated.
//!
//! `or_world::extension` declares 22 kinds and a field list for each; this
//! holds them to every extension on every entity, archetype, interior entity
//! and type file, in every copy the crawl reaches. A kind nobody ships is
//! reported as such rather than counted as checked.

mod support;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::Diagnostics;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_title::install::crawl::{self, ControlFlow};
use or_world::extension::{Extension, ExtensionKind};
use or_world::map_data::MapData;
use or_world::map_types::MapTypes;

const WHAT: &str = "what_every_extension_in_an_install_carries";

/// How many disagreeing instances are kept as examples, per kind.
const EXAMPLES: usize = 5;

#[derive(Default)]
struct PerKind {
    instances: usize,
    exact: usize,
    on_entities: usize,
    on_archetypes: usize,
    on_type_files: usize,
    examples: Vec<String>,
}

/// A fold rather than a collection: a world file's tree is dropped before
/// the next one is parsed.
#[derive(Default)]
struct Census {
    installs: Vec<String>,
    seen: usize,
    read: usize,
    refused: Vec<String>,
    entities: usize,
    archetypes: usize,
    kinds: BTreeMap<ExtensionKind, PerKind>,
}

#[derive(Clone, Copy)]
enum Host {
    Entity,
    Archetype,
    TypeFile,
}

impl Census {
    fn fold_extension(&mut self, ext: Extension<'_>, host: Host, file: &str) {
        let slot = self.kinds.entry(ext.kind()).or_default();
        slot.instances += 1;
        match host {
            Host::Entity => slot.on_entities += 1,
            Host::Archetype => slot.on_archetypes += 1,
            Host::TypeFile => slot.on_type_files += 1,
        }
        let c = ext.conformance();
        if c.is_exact() {
            slot.exact += 1;
        } else if slot.examples.len() < EXAMPLES {
            slot.examples.push(format!("{file}: {c:?}"));
        }
    }

    fn fold_root(&mut self, root: &or_io::meta::MetaInstance, file: &str) {
        if let Some(map) = MapData::new(root) {
            for entity in map.entities() {
                self.entities += 1;
                for ext in entity.extensions() {
                    self.fold_extension(ext, Host::Entity, file);
                }
            }
        } else if let Some(types) = MapTypes::new(root) {
            for ext in types.extensions() {
                self.fold_extension(ext, Host::TypeFile, file);
            }
            for arch in types.archetypes() {
                self.archetypes += 1;
                for ext in arch.extensions() {
                    self.fold_extension(ext, Host::Archetype, file);
                }
                let Some(interior) = arch.interior() else {
                    continue;
                };
                let set_entities = interior.entity_sets().flat_map(|s| s.entities());
                for entity in interior.entities().chain(set_entities) {
                    self.entities += 1;
                    for ext in entity.extensions() {
                        self.fold_extension(ext, Host::Entity, file);
                    }
                }
            }
        }
    }

    fn instances(&self) -> usize {
        self.kinds.values().map(|k| k.instances).sum()
    }

    fn exact(&self) -> usize {
        self.kinds.values().map(|k| k.exact).sum()
    }

    fn other(&self) -> usize {
        self.kinds
            .iter()
            .filter(|(k, _)| matches!(k, ExtensionKind::Other(_)))
            .map(|(_, v)| v.instances)
            .sum()
    }

    fn report(&self) -> String {
        let mut out = format!(
            "installs: {}\n\
             world files seen {}, read {}, refused {}\n\
             {} entities and {} archetypes carry {} extensions; {} conform exactly, {} of an unnamed type\n\
             {:<22} {:>9} {:>9} {:>9} {:>9} {:>9}\n",
            self.installs.join(", "),
            self.seen,
            self.read,
            self.refused.len(),
            self.entities,
            self.archetypes,
            self.instances(),
            self.exact(),
            self.other(),
            "kind",
            "instances",
            "exact",
            "entity",
            "archetype",
            "type file",
        );
        for kind in ExtensionKind::KNOWN {
            let k = self.kinds.get(kind);
            out.push_str(&format!(
                "{:<22} {:>9} {:>9} {:>9} {:>9} {:>9}\n",
                format!("{kind:?}"),
                k.map_or(0, |k| k.instances),
                k.map_or(0, |k| k.exact),
                k.map_or(0, |k| k.on_entities),
                k.map_or(0, |k| k.on_archetypes),
                k.map_or(0, |k| k.on_type_files),
            ));
        }
        for (kind, k) in &self.kinds {
            if let ExtensionKind::Other(hash) = kind {
                out.push_str(&format!("unnamed {hash:#010X}: {}\n", k.instances));
            }
            for example in &k.examples {
                out.push_str(&format!("  {kind:?} {example}\n"));
            }
        }
        out
    }

    fn require_exercised(&self, what: &str, got: usize, floor: usize) {
        assert!(
            got >= floor,
            "{what}: {got} is under the floor of {floor}, so this check passed \
             without exercising anything.\n{}",
            self.report()
        );
    }
}

fn crypto_for(root: &Path) -> Option<Arc<dyn RpfCryptoProvider>> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    Some(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))))
}

fn census() -> Option<Census> {
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(
            WHAT,
            format_args!(
                "no install root; set {GAME_ROOT_ENV} to a ;-separated list of install \
                 directories. World files live inside encrypted archives."
            ),
        );
        return None;
    }

    let library = support::schema();
    let mut census = Census::default();
    for root in &roots {
        let Some(provider) = crypto_for(root) else {
            continue;
        };
        let (read_before, extensions_before) = (census.read, census.instances());
        census.installs.push(root.display().to_string());
        let mut warnings = Vec::new();
        let _ = crawl::crawl(root, provider, &mut warnings, &mut |found| {
            let leaf = found.leaf.to_ascii_lowercase();
            if !(leaf.ends_with(".ymap") || leaf.ends_with(".ytyp")) {
                return ControlFlow::Continue(());
            }
            census.seen += 1;
            let bytes = match found.read() {
                Ok(bytes) => bytes,
                Err(e) => {
                    census.refused.push(format!("{}: {e}", found.leaf));
                    return ControlFlow::Continue(());
                }
            };
            let mut diags = Diagnostics::lenient();
            match parse_meta_binary_full(library, &bytes, &mut diags) {
                Ok((root, _)) => {
                    census.read += 1;
                    census.fold_root(&root, found.leaf);
                }
                Err(e) => census.refused.push(format!("{}: {e}", found.leaf)),
            }
            ControlFlow::Continue(())
        });
        let here = format!(
            " ({} world files, {} extensions)",
            census.read - read_before,
            census.instances() - extensions_before
        );
        if let Some(label) = census.installs.last_mut() {
            label.push_str(&here);
        }
    }

    if census.installs.is_empty() {
        skip_install(
            WHAT,
            "no install root yielded an executable to take keys from",
        );
        return None;
    }
    Some(census)
}

/// Every shipped extension is of a named kind and carries exactly the fields
/// that kind declares, in the shapes it declares.
#[test]
fn what_every_extension_in_an_install_carries() {
    let Some(census) = census() else {
        return;
    };
    eprintln!("{}", census.report());

    assert!(
        census.refused.is_empty(),
        "{} of {} world files refused:\n  {}\n{}",
        census.refused.len(),
        census.seen,
        census
            .refused
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  "),
        census.report()
    );

    // One retail install of either edition crawls over 12,000 world files
    // carrying tens of thousands of extensions; the floors sit well under
    // one edition so a single install still runs and an empty crawl cannot.
    census.require_exercised("reading a world file", census.read, 10_000);
    census.require_exercised("reading an extension", census.instances(), 10_000);
    // Fifteen of the twenty-two kinds ship in either edition; the other
    // seven are held to the schema alone, and the report names them.
    let shipped = ExtensionKind::KNOWN
        .iter()
        .filter(|k| census.kinds.get(k).is_some_and(|v| v.instances > 0))
        .count();
    census.require_exercised("kinds a shipped file exercises", shipped, 12);

    assert_eq!(
        census.other(),
        0,
        "an extension of a type no kind names shipped\n{}",
        census.report()
    );
    assert_eq!(
        census.exact(),
        census.instances(),
        "a shipped extension disagreed with its kind's declaration\n{}",
        census.report()
    );
}

#[test]
#[cfg_attr(
    all(windows, debug_assertions),
    ignore = "Cranelift cannot unwind a panic on Windows: run it with `cargo test --release`"
)]
fn the_floor_rejects_a_census_that_read_nothing() {
    let hollow = Census::default();
    let rejected =
        std::panic::catch_unwind(|| hollow.require_exercised("control", hollow.read, 10_000))
            .is_err();
    assert!(rejected, "an empty census passed its own floor");
}
