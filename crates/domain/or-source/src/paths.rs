//! The source of a `.ynd`: every node, link and junction of a path region as one JSON document, and the file it came from.
//!
//! A node holds its own links and a junction its own height samples, so the two chains a region states
//! (a node's first link, a junction's first sample) follow from the document and are written at the
//! save. A node is named by its address, `region:index`, whose index is its place in `nodes`: the game
//! finds a node by that place in the file of its region, so an address that is not its node's place is
//! refused, and so is a link or a junction reference that names a node of this region past its last.
//!
//! The base is the extracted file, kept as `base.ynd`: a compile opens it and sets what the document
//! states, so what the declaration holds privately (the page map's counts, the runtime pointers of a
//! node) stays the file's, and a document nobody edited compiles to the canonical save of the base.
//!
//! ```text
//! <name>.paths.src/    asset.json  base.ynd  paths.json
//! ```

use std::path::{Component, Path, PathBuf};

use or_io::decl::File;
use or_io::gta5::{
    AtBinaryMapDataPair, CNodeAddress, CPathNode, CPathNodeLink, CPathRegion, CPathVirtualJunction,
    PgBasePageMap,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::asset::{folder_name, sha256_hex, AssetManifest, How, Origin, Source, MANIFEST};
use crate::document::{hash_text, list, member, root, stated as stated_member, word, Fault, Trail};
use crate::json::canonical;
use crate::models::{problem, target_of, Compiled};
use crate::textures::Extraction;
use crate::Problem;

/// The `kind` of a path region's source.
pub const KIND: &str = "paths";
/// The document of the region, which a person edits.
pub const RULES: &str = "paths.json";
/// The region the document was written from.
pub const BASE: &str = "base.ynd";

/// The `compile` block of `asset.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Compile {
    /// The SHA-256 of the file kept as the base.
    base: String,
}

/// Everything a compile reads, so a recook can tell nothing changed before it opens anything.
#[derive(Clone)]
pub struct Inputs {
    manifest: AssetManifest,
    compile: Compile,
    base: Vec<u8>,
    rules: Vec<u8>,
}

/// A node's address as a document names it.
fn address_text(address: &CNodeAddress) -> String {
    format!("{}:{}", address.region, address.index)
}

/// The address `v` names, as (region, index).
fn address_of(v: &Value, at: &Trail) -> Result<(u16, u16), Fault> {
    let named = v
        .as_str()
        .and_then(|text| text.split_once(':'))
        .and_then(|(region, index)| Some((region.parse().ok()?, index.parse().ok()?)));
    named.ok_or_else(|| at.fault("is not a node address, which is written `region:index`"))
}

/// The document of `region`, refused where its chains do not close and a node's links cannot be told.
fn document_of(region: &CPathRegion) -> Result<Value, String> {
    let mut links = region.links.iter();
    let mut nodes = Vec::with_capacity(region.nodes.len());
    for (i, node) in region.nodes.iter().enumerate() {
        let own: Vec<Value> = links
            .by_ref()
            .take(node.num_links())
            .map(|link| {
                json!({
                    "other_node": address_text(&link.other_node),
                    "as_integer1": hash_text(link.as_integer1),
                })
            })
            .collect();
        if own.len() != node.num_links() {
            return Err(format!(
                "node {i} states {} links and the region holds {} after those of the nodes before it",
                node.num_links(),
                own.len()
            ));
        }
        nodes.push(json!({
            "address": address_text(&node.address),
            "street_name_hash": hash_text(node.street_name_hash),
            "distance_to_target": node.distance_to_target,
            "coors_x": node.coors_x,
            "coors_y": node.coors_y,
            "as_integer1": hash_text(node.as_integer1),
            "as_integer2": hash_text(node.as_integer2()),
            "links": own,
        }));
    }
    let left = links.count();
    if left > 0 {
        return Err(format!(
            "the region holds {left} links after those its nodes state"
        ));
    }

    let mut samples = region.height_samples.iter();
    let mut junctions = Vec::with_capacity(region.virtual_junctions.len());
    for (i, junction) in region.virtual_junctions.iter().enumerate() {
        let own: Vec<u8> = samples
            .by_ref()
            .take(junction.num_height_samples())
            .copied()
            .collect();
        if own.len() != junction.num_height_samples() {
            return Err(format!(
                "junction {i} states a patch of {} height samples and the region holds {} after those of the junctions before it",
                junction.num_height_samples(),
                own.len()
            ));
        }
        junctions.push(json!({
            "max_z": junction.max_z,
            "min_x": junction.min_x,
            "min_y": junction.min_y,
            "height_base_world": junction.height_base_world,
            "x_samples": junction.x_samples,
            "y_samples": junction.y_samples,
            "height_samples": own,
        }));
    }
    let left = samples.count();
    if left > 0 {
        return Err(format!(
            "the region holds {left} height samples after those its junctions state"
        ));
    }

    let map = &region.junction_map.junction_map;
    let references: Vec<Value> = map
        .data
        .iter()
        .map(|pair| {
            let (region, index) = pair.address();
            json!({ "key": format!("{region}:{index}"), "data": pair.data })
        })
        .collect();
    Ok(json!({
        "first_node": region.first_node.as_deref().map(|page_map| json!({
            "root_virtual_chunk": page_map.root_virtual_chunk,
            "subtype": page_map.subtype,
        })),
        "nodes": nodes,
        "num_nodes_car_nodes": region.num_nodes_car_nodes,
        "num_nodes_ped_nodes": region.num_nodes_ped_nodes,
        "virtual_junctions": junctions,
        "junction_map": { "sorted": map.sorted, "data": references },
    }))
}

/// One node as a document states it: where it stands, and the node each of its links names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatedNode {
    pub coors_x: i16,
    pub coors_y: i16,
    /// The height `as_integer1` holds.
    pub coors_z: i16,
    /// The (region, index) each link names, in the order the node holds them.
    pub links: Vec<(u16, u16)>,
}

/// What a document states of its region, read with none of the checks of a compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stated {
    /// The region the first node's address states; none for a region with no node.
    pub region: Option<u16>,
    /// The nodes by their place, which is the index the game finds each by.
    pub nodes: Vec<StatedNode>,
    pub num_nodes_car_nodes: i32,
    pub num_nodes_ped_nodes: i32,
    /// How many junctions the document states.
    pub virtual_junctions: usize,
}

/// What the document `rules` states of its nodes and links, for a rule to judge.
///
/// ⚠ Nothing a compile refuses is refused here: a link to a node its region does not hold and a node of more links than its word
/// counts are read as they are stated, since those are what a rule reports.
pub fn stated(rules: &[u8]) -> Result<Stated, Problem> {
    let read = || -> Result<Stated, Fault> {
        let doc = root(rules)?;
        let at = &mut Trail::default();
        let mut found = Stated {
            region: None,
            nodes: Vec::new(),
            num_nodes_car_nodes: member(&doc, "num_nodes_car_nodes", at)?,
            num_nodes_ped_nodes: member(&doc, "num_nodes_ped_nodes", at)?,
            virtual_junctions: list(&doc, "virtual_junctions", at)?.len(),
        };
        let nodes = list(&doc, "nodes", at)?;
        at.field("nodes", |at| -> Result<(), Fault> {
            for (i, v) in nodes.iter().enumerate() {
                at.index(i, |at| -> Result<(), Fault> {
                    let o = at.object(v)?;
                    if i == 0 {
                        let (region, _) = at.field("address", |at| {
                            address_of(stated_member(o, "address", at)?, at)
                        })?;
                        found.region = Some(region);
                    }
                    let height = CPathNode {
                        as_integer1: word(o, "as_integer1", at)?,
                        ..Default::default()
                    };
                    let links = list(o, "links", at)?;
                    let links = at.field("links", |at| {
                        (links.iter().enumerate())
                            .map(|(k, v)| {
                                at.index(k, |at| {
                                    let o = at.object(v)?;
                                    at.field("other_node", |at| {
                                        address_of(stated_member(o, "other_node", at)?, at)
                                    })
                                })
                            })
                            .collect::<Result<Vec<_>, Fault>>()
                    })?;
                    found.nodes.push(StatedNode {
                        coors_x: member(o, "coors_x", at)?,
                        coors_y: member(o, "coors_y", at)?,
                        coors_z: height.coors_z(),
                        links,
                    });
                    Ok(())
                })?;
            }
            Ok(())
        })?;
        Ok(found)
    };
    read().map_err(|fault| problem(fault.to_string(), RULES))
}

/// The region the document `doc` states, laid over `base` so that what no document states stays the base's.
fn region_of(doc: &Map<String, Value>, base: &CPathRegion) -> Result<CPathRegion, Fault> {
    let at = &mut Trail::default();
    at.only(
        doc,
        &[
            "first_node",
            "nodes",
            "num_nodes_car_nodes",
            "num_nodes_ped_nodes",
            "virtual_junctions",
            "junction_map",
        ],
    )?;
    let mut region = base.clone();

    region.first_node = match stated_member(doc, "first_node", at)? {
        Value::Null => None,
        v => Some(
            at.field("first_node", |at| -> Result<Box<PgBasePageMap>, Fault> {
                let o = at.object(v)?;
                at.only(o, &["root_virtual_chunk", "subtype"])?;
                let mut page_map: Box<PgBasePageMap> = base.first_node.clone().unwrap_or_default();
                page_map.root_virtual_chunk = member(o, "root_virtual_chunk", at)?;
                page_map.subtype = member(o, "subtype", at)?;
                Ok(page_map)
            })?,
        ),
    };
    region.num_nodes_car_nodes = member(doc, "num_nodes_car_nodes", at)?;
    region.num_nodes_ped_nodes = member(doc, "num_nodes_ped_nodes", at)?;

    let nodes = list(doc, "nodes", at)?;
    region.nodes = Vec::with_capacity(nodes.len());
    region.links = Vec::new();
    // What each link and reference names, with its place, held until the nodes are counted.
    let mut named: Vec<((u16, u16), Fault)> = Vec::new();
    // The region the first node states, which is the region of every node of the file.
    let mut own: Option<u16> = None;
    at.field("nodes", |at| -> Result<(), Fault> {
        for (i, v) in nodes.iter().enumerate() {
            at.index(i, |at| -> Result<(), Fault> {
                let o = at.object(v)?;
                at.only(
                    o,
                    &[
                        "address",
                        "street_name_hash",
                        "distance_to_target",
                        "coors_x",
                        "coors_y",
                        "as_integer1",
                        "as_integer2",
                        "links",
                    ],
                )?;
                let mut node: CPathNode = base.nodes.get(i).cloned().unwrap_or_default();
                let (area, index) = at.field("address", |at| -> Result<(u16, u16), Fault> {
                    let (area, index) = address_of(stated_member(o, "address", at)?, at)?;
                    // ⚠ A node is found by its place: `nodes[index]` of the file of its region.
                    if usize::from(index) != i {
                        return Err(at.fault(format!(
                            "is {area}:{index} and the node is at place {i} of `nodes`: the game finds a node by its place in its region, so the index of an address is that place"
                        )));
                    }
                    if let Some(own) = own.filter(|own| *own != area) {
                        return Err(at.fault(format!(
                            "is {area}:{index} and nodes[0] is of region {own}: a file holds the nodes of one region"
                        )));
                    }
                    Ok((area, index))
                })?;
                own = Some(area);
                node.address.region = area;
                node.address.index = index;
                node.street_name_hash = word(o, "street_name_hash", at)?;
                node.distance_to_target = member(o, "distance_to_target", at)?;
                node.coors_x = member(o, "coors_x", at)?;
                node.coors_y = member(o, "coors_y", at)?;
                node.as_integer1 = word(o, "as_integer1", at)?;
                let flags = word(o, "as_integer2", at)?;
                node.set_as_integer2(flags);
                if node.as_integer2() != flags {
                    return Err(at.field("as_integer2", |at| {
                        at.fault(format!(
                            "is {flags:#010x}, which states the bits that count the links of the node: the count follows from `links`, so the word is stated as {:#010x}",
                            node.as_integer2()
                        ))
                    }));
                }
                let links = list(o, "links", at)?;
                node.set_num_links(links.len()).map_err(|_| {
                    at.fault(format!(
                        "holds {} links, and a node counts its links in five bits, so at most 31",
                        links.len()
                    ))
                })?;
                at.field("links", |at| -> Result<(), Fault> {
                    for (k, v) in links.iter().enumerate() {
                        at.index(k, |at| -> Result<(), Fault> {
                            let o = at.object(v)?;
                            at.only(o, &["other_node", "as_integer1"])?;
                            let mut link: CPathNodeLink = base
                                .links
                                .get(region.links.len())
                                .cloned()
                                .unwrap_or_default();
                            let (area, index) =
                                at.field("other_node", |at| -> Result<(u16, u16), Fault> {
                                    let to = address_of(stated_member(o, "other_node", at)?, at)?;
                                    named.push((to, at.fault("")));
                                    Ok(to)
                                })?;
                            link.other_node.region = area;
                            link.other_node.index = index;
                            link.as_integer1 = word(o, "as_integer1", at)?;
                            region.links.push(link);
                            Ok(())
                        })?;
                    }
                    Ok(())
                })?;
                region.nodes.push(node);
                Ok(())
            })?;
        }
        Ok(())
    })?;

    let junctions = list(doc, "virtual_junctions", at)?;
    region.virtual_junctions = Vec::with_capacity(junctions.len());
    region.height_samples = Vec::new();
    at.field("virtual_junctions", |at| -> Result<(), Fault> {
        for (i, v) in junctions.iter().enumerate() {
            at.index(i, |at| -> Result<(), Fault> {
                let o = at.object(v)?;
                at.only(
                    o,
                    &[
                        "max_z",
                        "min_x",
                        "min_y",
                        "height_base_world",
                        "x_samples",
                        "y_samples",
                        "height_samples",
                    ],
                )?;
                let mut junction: CPathVirtualJunction =
                    base.virtual_junctions.get(i).cloned().unwrap_or_default();
                junction.max_z = member(o, "max_z", at)?;
                junction.min_x = member(o, "min_x", at)?;
                junction.min_y = member(o, "min_y", at)?;
                junction.height_base_world = member(o, "height_base_world", at)?;
                junction.x_samples = member(o, "x_samples", at)?;
                junction.y_samples = member(o, "y_samples", at)?;
                let samples: Vec<u8> = member(o, "height_samples", at)?;
                if samples.len() != junction.num_height_samples() {
                    return Err(at.fault(format!(
                        "holds {} height samples, and its patch is x_samples by y_samples, which is {}",
                        samples.len(),
                        junction.num_height_samples()
                    )));
                }
                region.height_samples.extend(samples);
                region.virtual_junctions.push(junction);
                Ok(())
            })?;
        }
        Ok(())
    })?;

    at.field("junction_map", |at| -> Result<(), Fault> {
        let o = at.object(stated_member(doc, "junction_map", at)?)?;
        at.only(o, &["sorted", "data"])?;
        let map = &mut region.junction_map.junction_map;
        map.sorted = member(o, "sorted", at)?;
        let references = list(o, "data", at)?;
        let was = std::mem::take(&mut map.data);
        at.field("data", |at| -> Result<(), Fault> {
            for (i, v) in references.iter().enumerate() {
                at.index(i, |at| -> Result<(), Fault> {
                    let o = at.object(v)?;
                    at.only(o, &["key", "data"])?;
                    let mut pair: AtBinaryMapDataPair = was.get(i).cloned().unwrap_or_default();
                    let (area, index) = at.field("key", |at| -> Result<(u16, u16), Fault> {
                        let to = address_of(stated_member(o, "key", at)?, at)?;
                        named.push((to, at.fault("")));
                        Ok(to)
                    })?;
                    pair.key = u32::from(area) | u32::from(index) << 16;
                    pair.data = member(o, "data", at)?;
                    map.data.push(pair);
                    Ok(())
                })?;
            }
            Ok(())
        })
    })?;

    // ⚠ Only a node of the region this file holds can be missed here: a link across a boundary names a node of another file.
    for ((area, index), place) in named {
        if own == Some(area) && usize::from(index) >= region.nodes.len() {
            return Err(Fault {
                why: format!("names node {area}:{index}, which is not in this region"),
                ..place
            });
        }
    }
    Ok(region)
}

/// Writes `ynd` into `parent` as `<name>.paths.src` and returns the folder; nothing is left behind on a refusal.
///
/// ⚠ A region whose source does not compile to the canonical save of `ynd` is refused, so no source is made that would ship as changed.
pub fn extract(
    ynd: &[u8],
    name: &str,
    parent: &Path,
    how: &Extraction<'_>,
) -> Result<PathBuf, Vec<Problem>> {
    let target = target_of(ynd, "ynd")?;
    let region = CPathRegion::from_bytes(ynd)
        .map_err(|e| vec![Problem::new(format!("cannot read the .ynd: {e}"))])?;
    let document = document_of(&region).map_err(|why| vec![Problem::new(why)])?;
    let canonical_save = region.to_bytes().map_err(|e| {
        vec![Problem::new(format!(
            "the .ynd does not save, so the game would misread what a source of it compiles to: {e}"
        ))]
    })?;
    let origin = Origin {
        how: How::Extracted,
        from_game: how.from_game,
        virtual_path: how.virtual_path.map(str::to_string),
        game_build: None,
        sha256: Some(sha256_hex(ynd)),
        cooked_sha256: None,
        extra: Default::default(),
    };
    let mut manifest =
        AssetManifest::new(KIND, name, target.slug(), origin).map_err(|p| vec![p])?;
    let compile = Compile {
        base: sha256_hex(ynd),
    };
    // Built from a struct of one string, which always serialises.
    manifest.extra.insert(
        "compile".to_string(),
        serde_json::to_value(&compile).unwrap_or_default(),
    );
    let inputs = Inputs {
        manifest,
        compile,
        base: ynd.to_vec(),
        // A value of numbers and strings always serialises.
        rules: canonical(&document).unwrap_or_default().into_bytes(),
    };
    let dir = parent.join(folder_name(name, KIND));
    if dir.exists() {
        return Err(vec![Problem::new(format!(
            "{} already exists",
            dir.display()
        ))]);
    }
    // ⚠ A source the compile would refuse is not written, nor one that compiles to another file than the base saves as.
    if inputs.compile()?.bytes != canonical_save {
        return Err(vec![Problem::new(
            "its source does not compile back to the canonical save of the file",
        )]);
    }
    let written = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join(MANIFEST), inputs.manifest.to_text()))
        .and_then(|()| std::fs::write(dir.join(BASE), &inputs.base))
        .and_then(|()| std::fs::write(dir.join(RULES), &inputs.rules));
    match written {
        Ok(()) => Ok(dir),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(vec![Problem::new(format!(
                "cannot write {}: {e}",
                dir.display()
            ))])
        }
    }
}

impl Inputs {
    pub fn read(source: &Source) -> Result<Self, Vec<Problem>> {
        let manifest = source.manifest.clone();
        if manifest.kind != KIND {
            return Err(vec![problem(
                format!("{:?} is not a paths source", manifest.kind),
                MANIFEST,
            )]);
        }
        let compile = manifest
            .extra
            .get("compile")
            .ok_or_else(|| {
                "a paths source states what it compiles, and there is no `compile` block"
                    .to_string()
            })
            .and_then(|stated| {
                serde_json::from_value::<Compile>(stated.clone())
                    .map_err(|e| format!("compile: {e}"))
            })
            .map_err(|why| vec![problem(why, MANIFEST)])?;
        let base = std::fs::read(source.dir.join(BASE))
            .map_err(|e| vec![problem(format!("cannot read: {e}"), BASE)])?;
        let rules = std::fs::read(source.dir.join(RULES))
            .map_err(|e| vec![problem(format!("cannot read: {e}"), RULES)])?;
        Ok(Self {
            manifest,
            compile,
            base,
            rules,
        })
    }

    /// A digest of every input.
    pub fn digest(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        let mut put = |bytes: &[u8]| {
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        };
        put(self.manifest.digest_text().as_bytes());
        put(&self.base);
        put(&self.rules);
        hash.finalize().into()
    }

    /// The `.ynd` the inputs compile to, read back through the reader before it is returned.
    pub fn compile(&self) -> Result<Compiled, Vec<Problem>> {
        let held = sha256_hex(&self.base);
        if held != self.compile.base {
            return Err(vec![problem(
                format!(
                    "the base holds {held} and compile.base says {}; the base is not the file the document was written from",
                    self.compile.base
                ),
                BASE,
            )]);
        }
        let base = CPathRegion::from_bytes(&self.base)
            .map_err(|e| vec![problem(format!("the base does not read: {e}"), BASE)])?;
        let region = root(&self.rules)
            .and_then(|doc| region_of(&doc, &base))
            .map_err(|fault| vec![problem(fault.to_string(), RULES)])?;
        let bytes = region.to_bytes().map_err(|e| {
            vec![problem(
                format!("the region would not save, and the game would misread it: {e}"),
                RULES,
            )]
        })?;
        // ⚠ A file the reader refuses would be shipped as a file the game misreads.
        CPathRegion::from_bytes(&bytes).map_err(|e| {
            vec![problem(
                format!("the compiled .ynd does not read back: {e}"),
                RULES,
            )]
        })?;
        Ok(Compiled {
            file_name: output_name(&self.manifest),
            bytes,
            report: Vec::new(),
        })
    }

    /// The document the inputs hold.
    pub fn rules(&self) -> &[u8] {
        &self.rules
    }

    /// Hold `rules` as the document, which nothing has checked.
    pub fn set_rules(&mut self, rules: Vec<u8>) {
        self.rules = rules;
    }

    /// The document that makes the inputs cook to `region`, with the region it cooks to; nothing is written.
    ///
    /// ⚠ Refused unless the region it cooks to states that document again: what a document cannot state of `region` would be lost.
    pub fn rules_of(&self, region: &CPathRegion) -> Result<(Vec<u8>, CPathRegion), Vec<Problem>> {
        let text = exported(region).map_err(|why| vec![problem(why, RULES)])?;
        let candidate = Self {
            rules: text.clone().into_bytes(),
            ..self.clone()
        };
        let cooked = CPathRegion::from_bytes(&candidate.compile()?.bytes).map_err(|e| {
            vec![problem(
                format!("the compiled .ynd does not read back: {e}"),
                RULES,
            )]
        })?;
        if exported(&cooked).ok().as_ref() != Some(&text) {
            return Err(vec![problem(
                "the edit states what a document cannot: the region it cooks to does not state it again",
                RULES,
            )]);
        }
        Ok((text.into_bytes(), cooked))
    }

    /// The name of the file the inputs compile to.
    pub fn output_name(&self) -> String {
        output_name(&self.manifest)
    }
}

/// The canonical text of the document `region` is written as, refused where its chains do not close.
pub fn exported(region: &CPathRegion) -> Result<String, String> {
    canonical(&document_of(region)?).map_err(|e| e.to_string())
}

/// Write `rules` as the document of the paths source in `dir`, to a part file first, so a failure leaves the document that was there.
pub fn write_rules(dir: &Path, rules: &[u8]) -> Result<(), Vec<Problem>> {
    let file = dir.join(RULES);
    let held = dir.join(format!("{RULES}.part"));
    std::fs::write(&held, rules)
        .and_then(|()| std::fs::rename(&held, &file))
        .map_err(|e| {
            let _ = std::fs::remove_file(&held);
            vec![Problem::new(format!(
                "cannot write {}: {e}",
                file.display()
            ))]
        })
}

/// What the folder `source` names compiles to.
pub fn compile(source: &Source) -> Result<Compiled, Vec<Problem>> {
    Inputs::read(source)?.compile()
}

/// The name of the file a paths source compiles to.
pub fn output_name(manifest: &AssetManifest) -> String {
    format!("{}.ynd", manifest.name)
}

/// Whether a path inside a paths source, relative to its folder, is a file the compile reads.
pub fn is_input(inside: &Path) -> bool {
    let parts: Vec<&str> = inside
        .components()
        .filter_map(|c| match c {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    match parts.as_slice() {
        [] => true,
        [file] => *file == MANIFEST || *file == BASE || *file == RULES,
        _ => false,
    }
}
