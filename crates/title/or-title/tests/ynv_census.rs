//! The navmesh identities, over every `.ynv` a real install ships, read
//! through the owned `CNavMesh`, and the running totals it derives through a
//! peek of what the file states.
//!
//! Point `OPENRAGE_GAME_ROOT` at one or more install roots, `;`-separated.
//! Nothing here runs without one, and the skip is recorded rather than
//! silent.
//!
//! ⚠ The stride of each of the four split arrays is derived from the file's
//! own partition — every sub-array but the last is saturated at
//! `16384 / stride` — which is evidence only if it holds on every cell in the
//! game. `or-io-verify`'s `navmesh.rs` runs over the staged corpus sample,
//! because 4,404 cells per edition is 616 MiB of shared corpus; a crawl sees
//! every copy in every archive, more files than a mounted VFS resolves.
//!
//! ⚠ **Every assertion here is over an exercised count, and the counts are
//! asserted too.** A suite that only checked "no cell disagreed" would pass
//! having read nothing.

use std::path::Path;
use std::sync::Arc;

use or_io::container::resource::YNV_FILE_VERSION;
use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::{ResourceEnvelope, RpfCryptoProvider, Rsc7};
use or_io::decl::{Element, File, Owned, Param, Plain, Raw, Resource, View, ViewError};
use or_io::gta5::navmesh::{
    items, AiSplitArray, CNavMesh, CNavMeshCompressedVertex, TAdjPoly, TNavMeshPoly, CHUNK_BYTES,
};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_title::install::crawl::{self, ControlFlow};

const WHAT: &str = "what_every_navmesh_in_an_install_states";

/// One fold over every `.ynv` in every install offered.
///
/// A fold rather than a collection: the two editions hold roughly 21,000
/// copies between them, and an unbounded install sweep has run a machine out
/// of memory.
#[derive(Default)]
struct Census {
    installs: Vec<String>,
    /// Files whose extension matched, whether or not they read.
    seen: usize,
    /// Files that opened owned.
    read: usize,
    refused: Vec<String>,
    versions: std::collections::BTreeMap<u32, usize>,

    vertices: usize,
    indices: usize,
    adjacency: usize,
    polys: usize,
    chunks: usize,

    /// Identity 1: a list's per-sub-array counts sum to the item count it states.
    counts_sum: usize,
    /// Identity 2: its start indices are the running total.
    starts_chain: usize,
    /// Identity 3: every sub-array but the last is saturated at the capacity
    /// its stride implies. This is the one that names the stride.
    partition_matches: usize,
    /// Lists looked at, the denominator for the three above.
    lists: usize,

    /// Identity 4: a polygon's index run is the running total before it.
    runs_tile: usize,
    runs_close: usize,
    adjacency_twins: usize,
    indices_in_range: usize,
}

/// A split array's item count and start indices, as the file states them.
fn stated<T: Owned>(
    list: Result<Option<View<'_, Raw<AiSplitArray<T>>>>, ViewError>,
) -> (Option<u32>, Vec<u32>)
where
    Raw<T>: Plain + Element + Param,
{
    let Ok(Some(list)) = list else {
        return (None, Vec::new());
    };
    let starts = list
        .start_indices()
        .map_or(Vec::new(), |s| s.iter().copied().collect());
    (Some(list.num_elements), starts)
}

impl Census {
    /// One split array against the item count and the starts the file states for it.
    fn fold_list<T: Owned>(
        &mut self,
        list: Option<&AiSplitArray<T>>,
        (stated, starts): (Option<u32>, Vec<u32>),
    ) where
        Raw<T>: Plain + Element + Param,
    {
        let Some(list) = list else {
            return;
        };
        let capacity = CHUNK_BYTES / std::mem::size_of::<Raw<T>>();
        let counts: Vec<usize> = list.sub_arrays.iter().map(|s| s.elements.len()).collect();
        self.lists += 1;
        self.chunks += counts.len();
        self.counts_sum += usize::from(stated == Some(list.len() as u32));
        let mut running = 0usize;
        let mut chain = starts.len() == counts.len();
        for (start, n) in starts.iter().zip(&counts) {
            chain &= *start as usize == running;
            running += n;
        }
        self.starts_chain += usize::from(chain);
        let last = counts.len().saturating_sub(1);
        self.partition_matches += usize::from(counts.iter().enumerate().all(|(i, n)| {
            if i < last {
                *n == capacity
            } else {
                (1..=capacity).contains(n)
            }
        }));
    }

    fn fold(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mesh = CNavMesh::from_bytes(bytes).map_err(|e| e.to_string())?;
        let peek = CNavMesh::peek(bytes).map_err(|e| e.to_string())?;
        let root = peek.root().map_err(|e| e.to_string())?;
        self.read += 1;
        self.fold_list::<CNavMeshCompressedVertex>(
            mesh.compressed_vertex_array.as_deref(),
            stated::<CNavMeshCompressedVertex>(root.compressed_vertex_array()),
        );
        self.fold_list::<u16>(
            mesh.vertex_index_array.as_deref(),
            stated::<u16>(root.vertex_index_array()),
        );
        self.fold_list::<TAdjPoly>(
            mesh.adjacent_polys_array.as_deref(),
            stated::<TAdjPoly>(root.adjacent_polys_array()),
        );
        self.fold_list::<TNavMeshPoly>(
            mesh.polys_array.as_deref(),
            stated::<TNavMeshPoly>(root.polys_array()),
        );
        // The first corner each polygon states, which the owned value derives at save.
        let first_corners: Vec<usize> = root
            .polys_array()
            .ok()
            .flatten()
            .and_then(|l| items::<TNavMeshPoly>(&l, "polygon").ok())
            .map_or(Vec::new(), |ps| {
                ps.iter()
                    .map(|p| (p.as_integer2 & 0xffff) as usize)
                    .collect()
            });

        let vertices = mesh
            .compressed_vertex_array
            .as_deref()
            .map_or(0, |l| l.len());
        let indices: Vec<u16> = mesh
            .vertex_index_array
            .as_deref()
            .map_or(Vec::new(), |l| l.iter().copied().collect());
        let edges = mesh.adjacent_polys_array.as_deref().map_or(0, |l| l.len());
        let polys: Vec<&TNavMeshPoly> = mesh
            .polys_array
            .as_deref()
            .map_or(Vec::new(), |l| l.iter().collect());
        self.vertices += vertices;
        self.indices += indices.len();
        self.adjacency += edges;
        self.polys += polys.len();

        let mut running = 0usize;
        let mut tile = true;
        for (i, p) in polys.iter().enumerate() {
            tile &= first_corners.get(i) == Some(&running);
            running += p.num_vertices();
        }
        self.runs_tile += usize::from(tile);
        self.runs_close += usize::from(running == indices.len());
        self.adjacency_twins += usize::from(edges == indices.len());
        self.indices_in_range += usize::from(indices.iter().all(|i| usize::from(*i) < vertices));
        Ok(())
    }

    fn report(&self) -> String {
        format!(
            "installs: {}\n\
             .ynv seen {}, read {}, refused {}\n\
             versions: {:?}\n\
             {} vertices, {} indices, {} adjacency, {} polygons over {} chunks\n\
             identity 1 (counts sum): {}/{} lists\n\
             identity 2 (starts chain): {}/{} lists\n\
             identity 3 (partition names the stride, {CHUNK_BYTES} bytes a chunk): {}/{} lists\n\
             identity 4 (index runs tile): {}/{} files, closing on the count {}/{}\n\
             adjacency is the index run's twin: {}/{}; every index names a vertex: {}/{}",
            self.installs.join(", "),
            self.seen,
            self.read,
            self.refused.len(),
            self.versions,
            self.vertices,
            self.indices,
            self.adjacency,
            self.polys,
            self.chunks,
            self.counts_sum,
            self.lists,
            self.starts_chain,
            self.lists,
            self.partition_matches,
            self.lists,
            self.runs_tile,
            self.read,
            self.runs_close,
            self.read,
            self.adjacency_twins,
            self.read,
            self.indices_in_range,
            self.read,
        )
    }

    /// ⚠ The one guard every check below ends with. Each identity is a
    /// count of agreements, and zero files agreeing vacuously is zero.
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
                 directories. A .ynv lives inside an encrypted archive, so there is no \
                 loose-file fallback."
            ),
        );
        return None;
    }

    let mut census = Census::default();
    for root in &roots {
        let Some(provider) = crypto_for(root) else {
            continue;
        };
        census.installs.push(root.display().to_string());
        let mut warnings = Vec::new();
        let _ = crawl::crawl(root, provider, &mut warnings, &mut |found| {
            if !found.leaf.to_ascii_lowercase().ends_with(".ynv") {
                return ControlFlow::Continue(());
            }
            census.seen += 1;
            let Ok(bytes) = found.read() else {
                census
                    .refused
                    .push(format!("{}: could not extract", found.leaf));
                return ControlFlow::Continue(());
            };
            if let Ok(seal) = Rsc7::peek(&bytes) {
                *census.versions.entry(seal.version()).or_default() += 1;
            }
            if let Err(e) = census.fold(&bytes) {
                census.refused.push(format!("{}: {e}", found.leaf));
            }
            ControlFlow::Continue(())
        });
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

/// Every `.ynv` an install ships opens, and the four identities hold on all
/// of them.
#[test]
fn what_every_navmesh_in_an_install_states() {
    let Some(census) = census() else {
        return;
    };
    eprintln!("{}", census.report());

    // ⚠ A refusal is a failure rather than a statistic. A corpus that is
    // present and unreadable is not an absent corpus.
    assert!(
        census.refused.is_empty(),
        "{} of {} .ynv refused:\n  {}\n{}",
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
    assert_eq!(census.read, census.seen);

    // A retail install of either edition crawls 10,567. The floor is well
    // under one edition, so a single install still runs and zero cannot
    // pass.
    census.require_exercised("opening a navmesh", census.read, 8_000);
    census.require_exercised("walking a chunked list", census.lists, 32_000);

    assert_eq!(
        census.versions.keys().copied().collect::<Vec<_>>(),
        vec![YNV_FILE_VERSION],
        "every .ynv is at one version, and a second would be a second layout"
    );

    assert_eq!(
        (
            census.counts_sum,
            census.starts_chain,
            census.partition_matches
        ),
        (census.lists, census.lists, census.lists),
        "a chunked list disagreed with itself. The third of those is the one \
         that names the stride: a list states its partition and never its \
         stride, so a chunk that is not saturated at 16384/stride is the file \
         refusing the reader.\n{}",
        census.report()
    );

    assert_eq!(
        (
            census.runs_tile,
            census.runs_close,
            census.adjacency_twins,
            census.indices_in_range
        ),
        (census.read, census.read, census.read, census.read),
        "a polygon's index run, the adjacency twin or an index bound failed \
         on a shipped cell.\n{}",
        census.report()
    );
}

/// The control: every count above is a sum, so the floors are the gate, and
/// this shows one rejecting.
#[test]
#[cfg_attr(
    all(windows, debug_assertions),
    ignore = "Cranelift cannot unwind a panic on Windows: run it with `cargo test --release`"
)]
fn the_floor_rejects_a_census_that_read_nothing() {
    let hollow = Census::default();
    let rejected =
        std::panic::catch_unwind(|| hollow.require_exercised("control", hollow.read, 8_000))
            .is_err();
    assert!(
        rejected,
        "a census that opened no file passed its own floor, so every count in \
         this suite is reporting green about a crawl that never happened"
    );
}

/// The sub-array capacity the whole format rests on, at the engine's block size for each element.
#[test]
fn a_chunk_holds_what_sixteen_kibibytes_divided_by_the_stride_holds() {
    use std::mem::size_of;
    for (stride, want) in [
        (size_of::<Raw<CNavMeshCompressedVertex>>(), 2730usize),
        (size_of::<u16>(), 8192),
        (size_of::<Raw<TAdjPoly>>(), 2048),
        (size_of::<Raw<TNavMeshPoly>>(), 341),
    ] {
        assert_eq!(CHUNK_BYTES / stride, want);
    }
}
