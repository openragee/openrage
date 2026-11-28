//! Every world file a retail install ships, re-encoded untouched through
//! [`or_world::write::encode`], comes back as the bytes it arrived as.
//!
//! Point `OPENRAGE_GAME_ROOT` at one or more install roots, `;`-separated.
//!
//! ⚠ A synthesised layout reproduces no shipped document, so this is what
//! holds the synthesis fallback to documents that outgrew their own layout.
//! RBM goes back under a fresh DEFLATE that Rockstar's compressor does not
//! reproduce, so its gate is the RSC7 header plus the decompressed payload;
//! PSO and RBF are compared whole.

mod support;

use std::path::Path;
use std::sync::Arc;

use or_io::container::resource::{ResourceEnvelope, Rsc7};
use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::{BinaryFormat, Diagnostics};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_title::install::crawl::{self, ControlFlow};

const WHAT: &str = "every_untouched_world_file_reencodes";

/// A fold: one document's bytes and tree are live at a time.
#[derive(Default)]
struct Census {
    installs: Vec<String>,
    seen: usize,
    rbm: usize,
    pso: usize,
    rbf: usize,
    identical: usize,
    differing: Vec<String>,
    refused: Vec<String>,
}

impl Census {
    fn report(&self) -> String {
        format!(
            "installs: {}\nworld files seen {} ({} RBM, {} PSO, {} RBF); identical {}, \
             differing {}, refused {}",
            self.installs.join(", "),
            self.seen,
            self.rbm,
            self.pso,
            self.rbf,
            self.identical,
            self.differing.len(),
            self.refused.len(),
        )
    }

    fn require_exercised(&self, what: &str, got: usize, floor: usize) {
        assert!(
            got >= floor,
            "{what}: {got} is under the floor of {floor}, so this check passed \
             without exercising anything.\n{}",
            self.report()
        );
    }

    fn fold(&mut self, leaf: &str, bytes: &[u8]) {
        let mut diags = Diagnostics::lenient();
        let (root, layout) = match parse_meta_binary_full(support::schema(), bytes, &mut diags) {
            Ok(pair) => pair,
            Err(e) => {
                self.refused.push(format!("{leaf}: read: {e}"));
                return;
            }
        };
        let written = match or_world::write::encode(&root, &layout) {
            Ok(b) => b,
            Err(e) => {
                self.refused.push(format!("{leaf}: encode: {e}"));
                return;
            }
        };
        let same = match layout.format {
            BinaryFormat::Rbm => {
                self.rbm += 1;
                written.get(..16) == bytes.get(..16)
                    && match (Rsc7::open(bytes), Rsc7::open(&written)) {
                        (Ok(a), Ok(b)) => a.data == b.data,
                        _ => false,
                    }
            }
            BinaryFormat::Pso => {
                self.pso += 1;
                written == bytes
            }
            BinaryFormat::Rbf => {
                self.rbf += 1;
                written == bytes
            }
        };
        if same {
            self.identical += 1;
        } else if self.differing.len() < 20 {
            self.differing.push(leaf.to_string());
        } else {
            self.differing.push(String::new());
        }
    }
}

fn crypto_for(root: &Path) -> Option<Arc<dyn RpfCryptoProvider>> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    Some(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))))
}

#[test]
fn every_untouched_world_file_reencodes() {
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(
            WHAT,
            format_args!("no install root; set {GAME_ROOT_ENV} to a ;-separated list"),
        );
        return;
    }
    let mut census = Census::default();
    for root in &roots {
        let Some(provider) = crypto_for(root) else {
            continue;
        };
        let before = census.seen;
        let mut warnings = Vec::new();
        let _ = crawl::crawl(root, provider, &mut warnings, &mut |found| {
            let leaf = found.leaf.to_ascii_lowercase();
            if !(leaf.ends_with(".ymap") || leaf.ends_with(".ytyp")) {
                return ControlFlow::Continue(());
            }
            census.seen += 1;
            match found.read() {
                Ok(bytes) => census.fold(found.leaf, &bytes),
                Err(e) => census.refused.push(format!("{}: extract: {e}", found.leaf)),
            }
            ControlFlow::Continue(())
        });
        census.installs.push(format!(
            "{} ({} world files)",
            root.display(),
            census.seen - before
        ));
    }
    if census.installs.is_empty() {
        skip_install(
            WHAT,
            "no install root yielded an executable to take keys from",
        );
        return;
    }
    eprintln!("{}", census.report());

    census.require_exercised("re-encoding a world file", census.seen, 10_000);
    assert!(
        census.refused.is_empty(),
        "{}\n  {}",
        census.report(),
        census
            .refused
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert_eq!(
        census.identical,
        census.seen,
        "an untouched world file changed on re-encode:\n  {}\n{}",
        census
            .differing
            .iter()
            .filter(|d| !d.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  "),
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
        std::panic::catch_unwind(|| hollow.require_exercised("control", hollow.seen, 10_000))
            .is_err();
    assert!(rejected, "an empty census passed its own floor");
}
