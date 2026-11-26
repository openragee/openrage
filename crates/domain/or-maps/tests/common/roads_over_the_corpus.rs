//! The road rules and the road operations over the path regions staged in the corpus, all of them as one set.
//!
//! ⚠ It walks a corpus, so it is a module of a binary of the slow tier, and it is skipped by name where no `.ynd` is staged.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use or_io::decl::File;
use or_io::gta5::CPathRegion;
use or_maps::roads::operation::{apply, Operation};
use or_maps::roads::rules::{check, Severity};
use or_maps::roads::{read, sources, Address};
use or_source::asset::Source;
use or_source::paths;
use or_source::textures::Extraction;

/// Every staged region as a source of one folder, or none where no `.ynd` is staged.
fn staged(dir: &Path, what: &str) -> Option<PathBuf> {
    let files =
        or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), "ynd").find_or_skip(what)?;
    let folder = dir.join("roads");
    std::fs::create_dir_all(&folder).unwrap();
    let how = Extraction {
        from_game: true,
        virtual_path: None,
    };
    for path in &files {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        // A staged file is named for where it came from, and a source by one plain word.
        let name: String = (name.chars())
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        paths::extract(&std::fs::read(path).unwrap(), &name, &folder, &how)
            .unwrap_or_else(|why| panic!("{}: {why:?}", path.display()));
    }
    Some(folder)
}

fn cooked(dir: &Path) -> CPathRegion {
    let file = paths::compile(&Source::open(dir).unwrap())
        .unwrap_or_else(|why| panic!("{}: {why:?}", dir.display()))
        .bytes;
    let region = CPathRegion::from_bytes(&file).unwrap();
    assert!(
        region.to_bytes().unwrap() == file,
        "{} does not save to its fixed point",
        dir.display()
    );
    region
}

fn documents(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    (sources(folder).unwrap().into_iter())
        .map(|(_, dir)| {
            let document = dir.join(paths::RULES);
            let bytes = std::fs::read(&document).unwrap();
            (document, bytes)
        })
        .collect()
}

#[test]
fn the_rules_read_every_staged_region_and_no_file_of_the_game_is_an_error() {
    let scratch = tempfile::tempdir().unwrap();
    let Some(folder) = staged(scratch.path(), "the road rules over every staged .ynd") else {
        return;
    };
    let set = read(&folder).unwrap();
    let found = check(&set, None);
    let mut by_code: BTreeMap<&str, usize> = BTreeMap::new();
    for finding in &found.findings {
        *by_code.entry(finding.code).or_default() += 1;
    }
    let (nodes, links) = (set.iter().filter_map(|l| l.stated.as_ref().ok())).fold(
        (0usize, 0usize),
        |(nodes, links), stated| {
            (
                nodes + stated.nodes.len(),
                links + stated.nodes.iter().map(|n| n.links.len()).sum::<usize>(),
            )
        },
    );
    println!(
        "{} regions, {nodes} nodes, {links} links: {by_code:?}",
        set.len()
    );
    let errors: Vec<&str> = (found.findings.iter())
        .filter(|f| f.severity == Severity::Error)
        .map(|f| f.message.as_str())
        .take(5)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    assert!(nodes > 0 && links > 0);
    // What the game's own graph states, whichever of its regions are staged: every link is returned, names a node that is there and
    // joins two nodes of one kind.
    for code in [
        "link-one-way",
        "link-node-missing",
        "node-unreached",
        "link-road-to-footpath",
    ] {
        assert_eq!(by_code.get(code), None, "{code}");
    }
}

/// A link into a region: the node that holds it, the node it names, and where each of the two stands.
type Reached = ((u16, u16), (u16, u16), [i16; 3], [i16; 3]);

/// Every link of the set into `region`.
fn reached(folder: &Path, region: u16) -> Vec<Reached> {
    let regions: BTreeMap<u16, CPathRegion> = (sources(folder).unwrap().into_iter())
        .map(|(_, dir)| cooked(&dir))
        .filter_map(|held| Some((held.nodes.first()?.address.region, held)))
        .collect();
    let stand = |node: &or_io::gta5::CPathNode| [node.coors_x, node.coors_y, node.coors_z()];
    let far = &regions[&region];
    let mut found = Vec::new();
    for (id, held) in &regions {
        let mut links = held.links.iter();
        for (index, node) in held.nodes.iter().enumerate() {
            for link in links.by_ref().take(node.num_links()) {
                if link.other_node.region == region {
                    let other = &far.nodes[usize::from(link.other_node.index)];
                    found.push((
                        (*id, index as u16),
                        (region, link.other_node.index),
                        stand(node),
                        stand(other),
                    ));
                }
            }
        }
    }
    found
}

#[test]
fn a_node_of_a_staged_region_is_taken_out_with_its_neighbours_renumbered_and_put_back() {
    let scratch = tempfile::tempdir().unwrap();
    let Some(folder) = staged(
        scratch.path(),
        "a node of a staged .ynd taken out and put back",
    ) else {
        return;
    };
    let before = documents(&folder);
    // The first region another names past its first node, so that a removal renumbers a neighbour.
    let regions: BTreeMap<u16, CPathRegion> = (sources(&folder).unwrap().into_iter())
        .map(|(_, dir)| cooked(&dir))
        .filter_map(|held| Some((held.nodes.first()?.address.region, held)))
        .collect();
    let named: BTreeSet<(u16, u16)> = (regions.iter())
        .flat_map(|(id, held)| {
            (held.links.iter())
                .filter(move |link| link.other_node.region != *id)
                .map(|link| (link.other_node.region, link.other_node.index))
        })
        .collect();
    let (region, node) = (regions.iter())
        .find_map(|(id, held)| {
            let last_named = (named.range((*id, 0)..=(*id, u16::MAX)).next_back())?.1;
            // A node before one a neighbour names, that no neighbour names itself and the junction table does not name.
            let index = (0..last_named).find(|index| {
                !named.contains(&(*id, *index))
                    && !(held.junction_map.junction_map.data.iter())
                        .any(|pair| pair.address() == (*id, *index))
            })?;
            Some((*id, index))
        })
        .expect("a region of the corpus another region names");
    let was = reached(&folder, region);

    let started = Instant::now();
    let applied = apply(
        &folder,
        &Operation::RemoveNode {
            node: Address::new(region, node),
            expects: None,
        },
    )
    .unwrap_or_else(|why| panic!("{region}:{node}: {why}"));
    println!(
        "removing {region}:{node} of {} regions wrote {} source(s) in {:?}",
        regions.len(),
        applied.edited.len(),
        started.elapsed()
    );
    assert!(
        applied.edited.len() > 1,
        "a neighbour is written in the same step"
    );
    assert_ne!(applied.inverse.name(), "rewrite_regions");
    // Each link into the region that neither left the node nor named it reaches the node it reached, by where the two stand: the
    // indexes are what changed.
    let stands = |links: &[Reached], without: Option<(u16, u16)>| {
        let mut stands: Vec<([i16; 3], [i16; 3])> = (links.iter())
            .filter(|(holder, named, ..)| Some(*holder) != without && Some(*named) != without)
            .map(|(.., from, to)| (*from, *to))
            .collect();
        stands.sort();
        stands
    };
    let expected = stands(&was, Some((region, node)));
    assert!(expected.len() < was.len(), "the node had a link");
    assert!(
        was.iter()
            .any(|(holder, named, ..)| holder.0 != region && named.1 > node),
        "a neighbour named a later node"
    );
    assert_eq!(stands(&reached(&folder, region), None), expected);

    apply(&folder, &applied.inverse).unwrap();
    assert!(
        documents(&folder) == before,
        "every document is as it was after the inverse"
    );
}
