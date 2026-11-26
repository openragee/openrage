//! The vocabulary of road edits: each change to the path regions of a set, as a named operation that is data.
//!
//! An [`Operation`] names a node by its address and a link by its node and its place among that node's links. [`apply`] does one to the
//! sources of a folder and returns the operation that takes it back, and [`apply_all`] does a list of them, all or none. A list is the
//! file [`Operations`] reads and writes, with a format name and a schema. The operations that carry a record are what [`apply`] gives as
//! inverses, and are operations like the others: a list may hold them.
//!
//! An operation edits the regions the sources cook to and writes each back as its document, so a region is accepted only once that
//! document compiles, reads back through `CPathRegion` and states the same document again.
//!
//! ⚠ **An inverse puts the documents back byte for byte, and is checked before it is given.** It is given only once the inverse, read
//! back from its JSON and applied to the edited regions, exports each document as it was. Otherwise the inverse is
//! [`Operation::RewriteRegions`] with the documents as they were: a document a person typed, which is not the export of its region,
//! is put back that way.
//!
//! ⚠ **An inverse states what it applies to.** An operation may state `expects`, the digest of the documents the set is to hold
//! ([`digest_of`]), and is refused when the set holds others ([`Error::Stale`]), as an operation of [`crate::operation`] is for its
//! source. Every inverse states the digest of the documents its operation left. The digest is of every document of the set and not
//! of those the inverse writes alone, because a link names a node of another region by its index: a region renumbered since is
//! one the inverse may write no document of and still name the wrong node of.
//!
//! ⚠ **What a moved or a removed node leaves of the junction table.** A node the junction table names is not moved, since the height
//! samples of its junction are of the ground where it stands and nothing here samples the ground again. Taking such a node out takes
//! its references out with it, and a junction no reference names any more with its height samples, and the inverse carries them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use or_io::decl::File;
use or_io::gta5::{
    AtBinaryMapDataPair, CNodeAddress, CPathNode, CPathNodeLink, CPathRegion, CPathVirtualJunction,
};
use or_source::paths::{self, Inputs};
use serde::{Deserialize, Serialize};

use super::{sources, Address, Unread, Word};
use crate::edit::Edited;

/// The file a list of road operations states it is.
pub const FORMAT: &str = "openrage-road-operations";
/// The one schema of a list this build reads and writes.
pub const SCHEMA: u32 = 1;

/// Why a list of operations was not read, or an operation not applied.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} is not a list of road operations: {1}")]
    Json(String, serde_json::Error),
    #[error("{0} is not a list of road operations: its format is `{1}`, not `{FORMAT}`")]
    Format(String, String),
    #[error(
        "{0} is a list of road operations of schema {1}, and this OpenRage reads schema {SCHEMA}"
    )]
    Schema(String, u64),
    #[error("{0}")]
    Unread(#[from] Unread),
    /// A paths source of the folder that does not open, by its name and the reason.
    #[error("`{0}`: {1}")]
    Source(String, String),
    #[error("{name} of {at}: {why}")]
    Refused {
        /// The name of the operation, as its JSON states it.
        name: &'static str,
        /// The node or the sources it names.
        at: String,
        why: String,
    },
    /// The set does not hold the documents an operation states, said as [`crate::operation::Error::Stale`] says it of a source.
    #[error("{}", stale(.name, .at, .expects, .found))]
    Stale {
        name: &'static str,
        /// The folder that holds the set.
        at: String,
        expects: String,
        /// The digest of the documents the set holds.
        found: String,
    },
    #[error("operation {number} of {of}, {refused}")]
    InList {
        /// Counted from one.
        number: usize,
        of: usize,
        refused: Box<Error>,
    },
    #[error("{0}")]
    Write(String),
}

/// A refusal for a set that is not as an operation expects it, in the words a map operation is refused with.
fn stale(name: &'static str, at: &str, expects: &str, found: &str) -> crate::operation::Error {
    crate::operation::Error::Stale {
        name,
        at: at.to_string(),
        expects: expects.to_string(),
        found: found.to_string(),
    }
}

/// What a node states of itself, as a document states it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeRecord {
    pub street_name_hash: Word,
    pub distance_to_target: i16,
    pub coors_x: i16,
    pub coors_y: i16,
    pub as_integer1: Word,
    /// The flags, without the bits that count the links.
    pub as_integer2: Word,
}

/// One link out of a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkRecord {
    pub other_node: Address,
    pub as_integer1: Word,
}

/// A link of another node of the region that names an inserted node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedBy {
    /// The node that holds the link, by its address before the insertion.
    pub node: Address,
    /// The place the link takes among the links of that node.
    pub link: usize,
    pub as_integer1: Word,
}

/// A reference of the junction table that names an inserted node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairRecord {
    /// The place the reference takes in the table.
    pub place: usize,
    /// The junction it names, by its index once the junctions of the operation are in.
    pub data: i32,
}

/// A junction with the height samples of its patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JunctionRecord {
    /// The index the junction takes among the junctions of the region.
    pub index: usize,
    pub max_z: u16,
    pub min_x: i16,
    pub min_y: i16,
    pub height_base_world: u16,
    pub x_samples: u8,
    pub y_samples: u8,
    pub height_samples: Vec<u8>,
}

/// The document of one source of the set, as text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    /// The folder of the source, by its name.
    pub source: String,
    pub document: String,
}

/// One edit of the path regions of a set, by the name its JSON states as `op`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    /// State values of a node, each member left out staying as it is; `coors_z` is the height `as_integer1` holds in its high sixteen bits.
    SetNode {
        node: Address,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        street_name_hash: Option<Word>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        distance_to_target: Option<i16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        coors_x: Option<i16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        coors_y: Option<i16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        coors_z: Option<i16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        as_integer1: Option<Word>,
        /// The flags, without the bits that count the links.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        as_integer2: Option<Word>,
    },
    /// State the word of a link, whole: the declaration names nothing inside it.
    SetLink {
        node: Address,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        link: usize,
        as_integer1: Word,
    },
    /// Add a link from `node` to `other_node` at the place `link`, last when none, and with `back_as_integer1` the link back, which the region of `other_node` states.
    AddLink {
        node: Address,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        other_node: Address,
        as_integer1: Word,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        back_as_integer1: Option<Word>,
        /// The place of the link back among the links of `other_node`, last when none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        back_link: Option<usize>,
    },
    /// Take a link out of `node`, with the link back: the one at `back_link`, else the first the far node states, and none with `one_way`.
    RemoveLink {
        node: Address,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        link: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        back_link: Option<usize>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        one_way: bool,
    },
    /// Add a node to a region, after its vehicle nodes or with `ped` after its pedestrian ones, as a copy of `linked_to` at a place of its own, linked to it both ways.
    AddNode {
        region: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        ped: bool,
        coors_x: i16,
        coors_y: i16,
        coors_z: i16,
        /// A node of the same region.
        linked_to: Address,
        /// The word of the link from the new node to `linked_to`.
        as_integer1: Word,
        /// The word of the link from `linked_to` to the new node.
        back_as_integer1: Word,
    },
    /// Take a node out with its links and the links of its region that name it, and renumber what names a later node of its region; refused while another region of the set names it.
    RemoveNode {
        node: Address,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
    },
    /// Put a node into a region at an address, with its links, the links that name it and its junction: what takes a removal back.
    InsertNode {
        node: Address,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        /// Whether it stands among the pedestrian nodes, which an address at the end of the vehicle nodes does not say.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        ped: bool,
        record: NodeRecord,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        links: Vec<LinkRecord>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        named_by: Vec<NamedBy>,
        /// The junctions that come back with the node, by rising index.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        virtual_junctions: Vec<JunctionRecord>,
        /// The references of the junction table that name the node, by rising place.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        junction_map: Vec<PairRecord>,
    },
    /// Disable a node: always refused, since the declaration of a node names no state that says so.
    DisableNode {
        node: Address,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
    },
    /// Hold each `document` as the document of its source, as it is given: what takes back an edit of a document that was not the export of its region.
    RewriteRegions {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        documents: Vec<Document>,
    },
}

impl Operation {
    /// The name the JSON form states as `op`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::SetNode { .. } => "set_node",
            Self::SetLink { .. } => "set_link",
            Self::AddLink { .. } => "add_link",
            Self::RemoveLink { .. } => "remove_link",
            Self::AddNode { .. } => "add_node",
            Self::RemoveNode { .. } => "remove_node",
            Self::InsertNode { .. } => "insert_node",
            Self::DisableNode { .. } => "disable_node",
            Self::RewriteRegions { .. } => "rewrite_regions",
        }
    }

    /// What it edits, for a sentence: a node, a region, or the sources it rewrites.
    pub fn target(&self) -> String {
        match self {
            Self::SetNode { node, .. }
            | Self::SetLink { node, .. }
            | Self::AddLink { node, .. }
            | Self::RemoveLink { node, .. }
            | Self::RemoveNode { node, .. }
            | Self::InsertNode { node, .. }
            | Self::DisableNode { node, .. } => format!("node {node}"),
            Self::AddNode { region, .. } => format!("region {region}"),
            Self::RewriteRegions { documents, .. } => (documents.iter())
                .map(|d| format!("`{}`", d.source))
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    /// The digest the documents of the set are to have when it is applied, when it states one: every inverse does.
    pub fn expects(&self) -> Option<&str> {
        match self {
            Self::SetNode { expects, .. }
            | Self::SetLink { expects, .. }
            | Self::AddLink { expects, .. }
            | Self::RemoveLink { expects, .. }
            | Self::AddNode { expects, .. }
            | Self::RemoveNode { expects, .. }
            | Self::InsertNode { expects, .. }
            | Self::DisableNode { expects, .. }
            | Self::RewriteRegions { expects, .. } => expects.as_deref(),
        }
    }

    /// State the digest the documents of the set are to have, or that any will do.
    pub fn set_expects(&mut self, digest: Option<String>) {
        match self {
            Self::SetNode { expects, .. }
            | Self::SetLink { expects, .. }
            | Self::AddLink { expects, .. }
            | Self::RemoveLink { expects, .. }
            | Self::AddNode { expects, .. }
            | Self::RemoveNode { expects, .. }
            | Self::InsertNode { expects, .. }
            | Self::DisableNode { expects, .. }
            | Self::RewriteRegions { expects, .. } => *expects = digest,
        }
    }

    /// The operation, refused unless the documents of the set have the digest `digest`.
    pub fn expecting(mut self, digest: impl Into<String>) -> Self {
        self.set_expects(Some(digest.into()));
        self
    }
}

/// The digest of the documents of a set, each with the name of its source: what an operation that states `expects` is held to.
pub fn digest_of<'a>(documents: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> String {
    let each = documents
        .into_iter()
        .map(|(source, document)| (source, crate::operation::digest_of(document)));
    digest_of_each(each)
}

/// The digest of a set from the digest of each of its documents, in the order of the names of their sources.
fn digest_of_each<'a>(digests: impl IntoIterator<Item = (&'a str, String)>) -> String {
    let by_name: BTreeMap<&str, String> = digests.into_iter().collect();
    let mut stated = String::new();
    for (source, digest) in by_name {
        // A digest is of one length and NUL is in no name, so no two sets state one text.
        stated.push_str(&digest);
        stated.push_str(source);
        stated.push('\0');
    }
    crate::operation::digest_of(stated.as_bytes())
}

/// A list of operations as a file: what `openrage-cli maps roads --apply` takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operations {
    pub format: String,
    pub schema: u32,
    pub operations: Vec<Operation>,
}

impl Operations {
    pub fn new(operations: Vec<Operation>) -> Self {
        Self {
            format: FORMAT.to_string(),
            schema: SCHEMA,
            operations,
        }
    }

    /// The list as JSON with a two-space indent and LF.
    pub fn to_json(&self) -> Result<String, Error> {
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Json("the list".to_string(), e))?;
        text.push('\n');
        Ok(text)
    }

    /// The list `bytes` hold; `shown` names them in an error. A format that is not [`FORMAT`] and a schema that is not [`SCHEMA`] are refused before an operation is read.
    pub fn from_json(bytes: &[u8], shown: &str) -> Result<Self, Error> {
        let json = |e| Error::Json(shown.to_string(), e);
        let top: serde_json::Value = serde_json::from_slice(bytes).map_err(json)?;
        let format = top.get("format").and_then(|f| f.as_str()).unwrap_or("");
        if format != FORMAT {
            return Err(Error::Format(shown.to_string(), format.to_string()));
        }
        let schema = top.get("schema").and_then(serde_json::Value::as_u64);
        if schema != Some(u64::from(SCHEMA)) {
            return Err(Error::Schema(shown.to_string(), schema.unwrap_or(0)));
        }
        serde_json::from_value(top).map_err(json)
    }
}

/// What an operation did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// The operation that puts every source back byte for byte, which states the digest of the documents this one left.
    pub inverse: Operation,
    /// The document of each source it changed, as it was and as the operation left it, in the order of the sources' names.
    pub edited: Vec<Edited>,
    /// The address of the node it added, when it added one.
    pub added: Option<Address>,
    /// What it left for a person to see to, a sentence each: a link stated one way, a region the set does not hold.
    pub notes: Vec<String>,
}

/// The regions of a set while an operation changes them: a region is copied the first time it is edited.
struct Regions<'a> {
    /// Each region with the source that states it.
    base: BTreeMap<u16, (&'a str, &'a CPathRegion)>,
    changed: BTreeMap<u16, CPathRegion>,
}

impl Regions<'_> {
    fn ids(&self) -> Vec<u16> {
        self.base.keys().copied().collect()
    }

    fn get(&self, region: u16) -> Option<&CPathRegion> {
        (self.changed.get(&region)).or_else(|| self.base.get(&region).map(|(_, held)| *held))
    }

    fn held(&self, region: u16) -> Result<&CPathRegion, String> {
        self.get(region)
            .ok_or_else(|| format!("region {region} is not in the set"))
    }

    fn edit(&mut self, region: u16) -> Result<&mut CPathRegion, String> {
        if !self.changed.contains_key(&region) {
            let held = self.held(region)?.clone();
            self.changed.insert(region, held);
        }
        self.changed
            .get_mut(&region)
            .ok_or_else(|| format!("region {region} is not in the set"))
    }

    fn source(&self, region: u16) -> Result<&str, String> {
        (self.base.get(&region).map(|(source, _)| *source))
            .ok_or_else(|| format!("region {region} is not in the set"))
    }

    fn node(&self, at: Address) -> Result<&CPathNode, String> {
        node_at(self.held(at.region)?, at)
    }
}

fn node_at(region: &CPathRegion, at: Address) -> Result<&CPathNode, String> {
    let held = region.nodes.len();
    (region.nodes.get(usize::from(at.index))).ok_or_else(|| {
        format!(
            "node {at} is past the {held} node(s) region {} holds",
            at.region
        )
    })
}

/// Where the links of the node at `index` start among the links of `region`: after those of every node before it.
fn first_link(region: &CPathRegion, index: usize) -> usize {
    (region.nodes.iter().take(index))
        .map(CPathNode::num_links)
        .sum()
}

fn links_of(region: &CPathRegion, index: usize) -> Result<&[CPathNodeLink], String> {
    let start = first_link(region, index);
    let count = region.nodes.get(index).map_or(0, CPathNode::num_links);
    (region.links.get(start..start + count))
        .ok_or_else(|| "the region holds fewer links than its nodes count".to_string())
}

fn names(address: &CNodeAddress, node: Address) -> bool {
    address.region == node.region && address.index == node.index
}

fn link_to(node: Address, as_integer1: u32) -> CPathNodeLink {
    CPathNodeLink {
        other_node: CNodeAddress {
            region: node.region,
            index: node.index,
            ..Default::default()
        },
        as_integer1,
        ..Default::default()
    }
}

/// The key the junction table finds `node` by.
fn key_of(node: Address) -> u32 {
    u32::from(node.region) | u32::from(node.index) << 16
}

fn names_pair(pair: &AtBinaryMapDataPair, node: Address) -> bool {
    pair.address() == (node.region, node.index)
}

/// Put `link` among the links of the node at `index`, at `place`.
fn put_link(
    region: &mut CPathRegion,
    index: usize,
    place: usize,
    link: CPathNodeLink,
) -> Result<(), String> {
    let start = first_link(region, index);
    let id = region.nodes.first().map_or(0, |n| n.address.region);
    let node = (region.nodes.get_mut(index))
        .ok_or_else(|| format!("node {id}:{index} is not in its region"))?;
    let count = node.num_links();
    if place > count {
        return Err(format!(
            "link {place} is past the end of the {count} link(s) node {id}:{index} holds"
        ));
    }
    node.set_num_links(count + 1).map_err(|e| {
        format!("node {id}:{index} holds {count} link(s), and its word counts no more: {e}")
    })?;
    if start + place > region.links.len() {
        return Err("the region holds fewer links than its nodes count".to_string());
    }
    region.links.insert(start + place, link);
    Ok(())
}

/// Take the link at `place` out of the links of the node at `index`.
fn take_link(
    region: &mut CPathRegion,
    index: usize,
    place: usize,
) -> Result<CPathNodeLink, String> {
    let start = first_link(region, index);
    let id = region.nodes.first().map_or(0, |n| n.address.region);
    let node = (region.nodes.get_mut(index))
        .ok_or_else(|| format!("node {id}:{index} is not in its region"))?;
    let count = node.num_links();
    if place >= count {
        return Err(format!(
            "link {place} is past the {count} link(s) node {id}:{index} holds"
        ));
    }
    if start + place >= region.links.len() {
        return Err("the region holds fewer links than its nodes count".to_string());
    }
    node.set_num_links(count - 1).map_err(|e| e.to_string())?;
    Ok(region.links.remove(start + place))
}

/// Where the height samples of the junction at `index` start among those of `region`: after the patch of every junction before it.
fn first_sample(region: &CPathRegion, index: usize) -> usize {
    (region.virtual_junctions.iter().take(index))
        .map(CPathVirtualJunction::num_height_samples)
        .sum()
}

/// Take the junction at `index` out of `region`, with the height samples of its patch.
fn take_junction(region: &mut CPathRegion, index: usize) -> Result<JunctionRecord, String> {
    let start = first_sample(region, index);
    let held = (region.virtual_junctions.get(index))
        .ok_or_else(|| format!("junction {index} is not in its region"))?;
    let end = start + held.num_height_samples();
    if end > region.height_samples.len() {
        return Err("the region holds fewer height samples than its junctions count".to_string());
    }
    let record = JunctionRecord {
        index,
        max_z: held.max_z,
        min_x: held.min_x,
        min_y: held.min_y,
        height_base_world: held.height_base_world,
        x_samples: held.x_samples,
        y_samples: held.y_samples,
        height_samples: region.height_samples.drain(start..end).collect(),
    };
    region.virtual_junctions.remove(index);
    Ok(record)
}

/// Put the junction `record` states into `region` at its index, with the height samples of its patch.
fn put_junction(region: &mut CPathRegion, record: &JunctionRecord) -> Result<(), String> {
    let held = region.virtual_junctions.len();
    if record.index > held {
        return Err(format!(
            "junction {} is past the end of the {held} the region holds",
            record.index
        ));
    }
    let junction = CPathVirtualJunction {
        max_z: record.max_z,
        min_x: record.min_x,
        min_y: record.min_y,
        height_base_world: record.height_base_world,
        x_samples: record.x_samples,
        y_samples: record.y_samples,
        ..Default::default()
    };
    if record.height_samples.len() != junction.num_height_samples() {
        return Err(format!(
            "junction {} holds {} height samples, and its patch is x_samples by y_samples, which is {}",
            record.index,
            record.height_samples.len(),
            junction.num_height_samples()
        ));
    }
    let start = first_sample(region, record.index);
    if start > region.height_samples.len() {
        return Err("the region holds fewer height samples than its junctions count".to_string());
    }
    (region.height_samples).splice(start..start, record.height_samples.iter().copied());
    region.virtual_junctions.insert(record.index, junction);
    Ok(())
}

/// The height a node's first word holds, as the declaration reads it.
fn height_of(as_integer1: u32) -> i16 {
    CPathNode {
        as_integer1,
        ..Default::default()
    }
    .coors_z()
}

/// `as_integer1` with `height` in its high sixteen bits, refused unless the declaration reads that height back.
fn with_height(as_integer1: u32, height: i16) -> Result<u32, String> {
    let word = (as_integer1 & 0xFFFF) | u32::from(height as u16) << 16;
    (height_of(word) == height).then_some(word).ok_or_else(|| {
        "the declaration of a node does not read its height from the high sixteen bits of `as_integer1`"
            .to_string()
    })
}

/// The flags `stated` gives a node's second word, refused when they state the bits that count its links.
fn flags_of(node: &CPathNode, stated: Word) -> Result<u32, String> {
    let mut probe = node.clone();
    probe.set_as_integer2(stated.0);
    if probe.as_integer2() != stated.0 {
        return Err(format!(
            "`as_integer2` is {:#010x}, which states the bits that count the links of the node: the count follows from its links, so the word is stated as {:#010x}",
            stated.0,
            probe.as_integer2()
        ));
    }
    Ok(stated.0)
}

fn record_of(node: &CPathNode) -> NodeRecord {
    NodeRecord {
        street_name_hash: Word(node.street_name_hash),
        distance_to_target: node.distance_to_target,
        coors_x: node.coors_x,
        coors_y: node.coors_y,
        as_integer1: Word(node.as_integer1),
        as_integer2: Word(node.as_integer2()),
    }
}

/// The regions of the set whose junction table names `node`.
fn junction_tables_naming(regions: &Regions<'_>, node: Address) -> Vec<u16> {
    (regions.ids().into_iter())
        .filter(|id| {
            regions.get(*id).is_some_and(|held| {
                let table = &held.junction_map.junction_map.data;
                table.iter().any(|pair| names_pair(pair, node))
            })
        })
        .collect()
}

/// Every link and junction reference of another region of the set that names `node`, a phrase each.
fn named_elsewhere(regions: &Regions<'_>, node: Address) -> Result<Vec<String>, String> {
    let mut found = Vec::new();
    for id in regions.ids().into_iter().filter(|id| *id != node.region) {
        let held = regions.held(id)?;
        let source = regions.source(id)?;
        for index in 0..held.nodes.len() {
            for (place, link) in links_of(held, index)?.iter().enumerate() {
                if names(&link.other_node, node) {
                    found.push(format!("link {place} of node {id}:{index} (`{source}`)"));
                }
            }
        }
        let table = &held.junction_map.junction_map.data;
        if table.iter().any(|pair| names_pair(pair, node)) {
            found.push(format!("the junction table of region {id} (`{source}`)"));
        }
    }
    Ok(found)
}

/// Refuse to take `node` out while it links to a region the set does not hold.
///
/// ⚠ A link back from that region would name a node that is gone, and could not be taken out, since its file is not at hand.
fn every_far_end_is_held(regions: &Regions<'_>, node: Address) -> Result<(), String> {
    let held = regions.held(node.region)?;
    let mut absent: BTreeSet<u16> = BTreeSet::new();
    let mut first: Option<String> = None;
    for link in links_of(held, usize::from(node.index))? {
        let other = link.other_node.region;
        if regions.get(other).is_none() {
            absent.insert(other);
            first.get_or_insert_with(|| format!("{other}:{}", link.other_node.index));
        }
    }
    let Some(first) = first else {
        return Ok(());
    };
    let listed: Vec<String> = absent.iter().map(ToString::to_string).collect();
    Err(format!(
        "it links to {first}, of a region the set does not hold: the link back to it could not be taken out and would name a node that is gone, so add region(s) {} to the set first",
        listed.join(", ")
    ))
}

/// Refuse to renumber the nodes of `region` at or past `from` while one of them links to a region the set does not hold.
///
/// ⚠ A link back from that region would name a renumbered node and could not be rewritten, since its file is not at hand.
fn every_neighbour_is_held(regions: &Regions<'_>, region: u16, from: usize) -> Result<(), String> {
    let held = regions.held(region)?;
    let mut absent: BTreeSet<u16> = BTreeSet::new();
    let mut first: Option<String> = None;
    for index in from..held.nodes.len() {
        for link in links_of(held, index)? {
            let other = link.other_node.region;
            if regions.get(other).is_none() {
                absent.insert(other);
                first.get_or_insert_with(|| {
                    format!(
                        "node {region}:{index} links to {other}:{}",
                        link.other_node.index
                    )
                });
            }
        }
    }
    let Some(first) = first else {
        return Ok(());
    };
    let listed: Vec<String> = absent.iter().map(ToString::to_string).collect();
    Err(format!(
        "it renumbers the nodes of region {region} from {region}:{from}, and {first}: a link back from a region the set does not hold would name the wrong node and could not be rewritten, so add region(s) {} to the set first",
        listed.join(", ")
    ))
}

/// Move by `by` the index every link and junction reference of the set states for a node of `region` at or past `from`.
fn renumber(regions: &mut Regions<'_>, region: u16, from: u16, by: i32) -> Result<(), String> {
    let moves = |r: u16, index: u16| r == region && index >= from;
    let moved = |index: u16| {
        u16::try_from(i32::from(index) + by)
            .map_err(|_| "a node past what an address can name cannot be renumbered".to_string())
    };
    for id in regions.ids() {
        let held = regions.held(id)?;
        let table = &held.junction_map.junction_map.data;
        let any = (held.links.iter()).any(|l| moves(l.other_node.region, l.other_node.index))
            || table.iter().any(|pair| {
                let (r, index) = pair.address();
                moves(r, index)
            });
        if !any {
            continue;
        }
        let held = regions.edit(id)?;
        for link in &mut held.links {
            if moves(link.other_node.region, link.other_node.index) {
                link.other_node.index = moved(link.other_node.index)?;
            }
        }
        for pair in &mut held.junction_map.junction_map.data {
            let (r, index) = pair.address();
            if moves(r, index) {
                pair.key = key_of(Address::new(r, moved(index)?));
            }
        }
    }
    Ok(())
}

/// State each node's address as its place in `region`, which is how the game finds it.
fn readdress(region: &mut CPathRegion, id: u16) -> Result<(), String> {
    for (place, node) in region.nodes.iter_mut().enumerate() {
        node.address.region = id;
        node.address.index = u16::try_from(place)
            .map_err(|_| "a region holds no more nodes than an address can name".to_string())?;
    }
    Ok(())
}

/// What a change gives back: the operation that takes it back, the node it added, and what it leaves to see to.
struct Changed {
    inverse: Operation,
    added: Option<Address>,
    notes: Vec<String>,
}

impl Changed {
    fn taken_back_by(inverse: Operation) -> Self {
        Self {
            inverse,
            added: None,
            notes: Vec::new(),
        }
    }
}

/// The members a `set_node` may state.
struct NodeValues {
    street_name_hash: Option<Word>,
    distance_to_target: Option<i16>,
    coors_x: Option<i16>,
    coors_y: Option<i16>,
    coors_z: Option<i16>,
    as_integer1: Option<Word>,
    as_integer2: Option<Word>,
}

fn set_node(
    regions: &mut Regions<'_>,
    node: Address,
    stated: &NodeValues,
) -> Result<Changed, String> {
    let held = regions.node(node)?.clone();
    let nothing = stated.street_name_hash.is_none()
        && stated.distance_to_target.is_none()
        && stated.coors_x.is_none()
        && stated.coors_y.is_none()
        && stated.coors_z.is_none()
        && stated.as_integer1.is_none()
        && stated.as_integer2.is_none();
    if nothing {
        return Err("it states no value to set".to_string());
    }
    let mut word = stated.as_integer1.map_or(held.as_integer1, |word| word.0);
    if let Some(height) = stated.coors_z {
        if stated.as_integer1.is_some() && height_of(word) != height {
            return Err(format!(
                "`as_integer1` holds the height {} in its high sixteen bits and `coors_z` states {height}",
                height_of(word)
            ));
        }
        word = with_height(word, height)?;
    }
    let flags = match stated.as_integer2 {
        Some(flags) => Some(flags_of(&held, flags)?),
        None => None,
    };
    let (x, y) = (
        stated.coors_x.unwrap_or(held.coors_x),
        stated.coors_y.unwrap_or(held.coors_y),
    );
    let moves = (x, y, height_of(word)) != (held.coors_x, held.coors_y, held.coors_z());
    if moves {
        if let Some(table) = junction_tables_naming(regions, node).first() {
            return Err(format!(
                "the junction table of region {table} names the node, and the height samples of its junction are of the ground where it stands: nothing here samples the ground again, so the node is not moved"
            ));
        }
    }
    let edited = &mut regions.edit(node.region)?.nodes[usize::from(node.index)];
    if let Some(hash) = stated.street_name_hash {
        edited.street_name_hash = hash.0;
    }
    if let Some(distance) = stated.distance_to_target {
        edited.distance_to_target = distance;
    }
    edited.coors_x = x;
    edited.coors_y = y;
    edited.as_integer1 = word;
    if let Some(flags) = flags {
        edited.set_as_integer2(flags);
    }
    Ok(Changed::taken_back_by(Operation::SetNode {
        node,
        expects: None,
        street_name_hash: (stated.street_name_hash).map(|_| Word(held.street_name_hash)),
        distance_to_target: stated.distance_to_target.map(|_| held.distance_to_target),
        coors_x: stated.coors_x.map(|_| held.coors_x),
        coors_y: stated.coors_y.map(|_| held.coors_y),
        coors_z: stated.coors_z.map(|_| held.coors_z()),
        as_integer1: stated.as_integer1.map(|_| Word(held.as_integer1)),
        as_integer2: stated.as_integer2.map(|_| Word(held.as_integer2())),
    }))
}

fn set_link(
    regions: &mut Regions<'_>,
    node: Address,
    link: usize,
    as_integer1: Word,
) -> Result<Changed, String> {
    let held = regions.held(node.region)?;
    node_at(held, node)?;
    let index = usize::from(node.index);
    let count = links_of(held, index)?.len();
    if link >= count {
        return Err(format!(
            "link {link} is past the {count} link(s) node {node} holds"
        ));
    }
    let at = first_link(held, index) + link;
    let edited = &mut regions.edit(node.region)?.links[at];
    let was = edited.as_integer1;
    edited.as_integer1 = as_integer1.0;
    Ok(Changed::taken_back_by(Operation::SetLink {
        node,
        expects: None,
        link,
        as_integer1: Word(was),
    }))
}

/// A link to add, and the link back.
struct NewLink {
    node: Address,
    other_node: Address,
    as_integer1: Word,
    link: Option<usize>,
    back_as_integer1: Option<Word>,
    back_link: Option<usize>,
}

fn add_link(regions: &mut Regions<'_>, new: &NewLink) -> Result<Changed, String> {
    let (node, other) = (new.node, new.other_node);
    regions.node(node)?;
    if node == other {
        return Err("a node is not linked to itself".to_string());
    }
    if new.back_link.is_some() && new.back_as_integer1.is_none() {
        return Err(
            "`back_link` is the place of the link back, and no `back_as_integer1` states one"
                .to_string(),
        );
    }
    let mut notes = Vec::new();
    match regions.get(other.region) {
        Some(far) => {
            node_at(far, other)?;
            let returns = (links_of(far, usize::from(other.index))?.iter())
                .any(|link| names(&link.other_node, node));
            if new.back_as_integer1.is_none() && !returns {
                notes.push(format!(
                    "the link is stated one way: node {other} states no link back to {node}"
                ));
            }
        }
        None if new.back_as_integer1.is_some() => {
            return Err(format!(
                "region {} is not in the set, so the link back from {other} cannot be stated in it",
                other.region
            ));
        }
        None => notes.push(format!(
            "region {} is not in the set, so nothing says node {other} is there or links back",
            other.region
        )),
    }
    let count = links_of(regions.held(node.region)?, usize::from(node.index))?.len();
    let place = new.link.unwrap_or(count);
    put_link(
        regions.edit(node.region)?,
        usize::from(node.index),
        place,
        link_to(other, new.as_integer1.0),
    )?;
    let back = match new.back_as_integer1 {
        Some(word) => {
            let count = links_of(regions.held(other.region)?, usize::from(other.index))?.len();
            let place = new.back_link.unwrap_or(count);
            put_link(
                regions.edit(other.region)?,
                usize::from(other.index),
                place,
                link_to(node, word.0),
            )?;
            Some(place)
        }
        None => None,
    };
    Ok(Changed {
        inverse: Operation::RemoveLink {
            node,
            expects: None,
            link: place,
            back_link: back,
            one_way: back.is_none(),
        },
        added: None,
        notes,
    })
}

fn remove_link(
    regions: &mut Regions<'_>,
    node: Address,
    link: usize,
    back_link: Option<usize>,
    one_way: bool,
) -> Result<Changed, String> {
    if one_way && back_link.is_some() {
        return Err(
            "`one_way` leaves the link back, and `back_link` names one to take out".to_string(),
        );
    }
    let held = regions.held(node.region)?;
    node_at(held, node)?;
    let own = links_of(held, usize::from(node.index))?;
    let removed = own.get(link).cloned().ok_or_else(|| {
        format!(
            "link {link} is past the {} link(s) node {node} holds",
            own.len()
        )
    })?;
    let other = Address::new(removed.other_node.region, removed.other_node.index);
    let mut notes = Vec::new();
    // The links the far node states, when its region is in the set and holds it.
    let far = match regions.get(other.region) {
        Some(far) if usize::from(other.index) < far.nodes.len() => {
            Some(links_of(far, usize::from(other.index))?)
        }
        _ => None,
    };
    let back = match (back_link, far) {
        _ if one_way || other == node => None,
        (Some(place), Some(far)) if far.get(place).is_some_and(|l| names(&l.other_node, node)) => {
            Some(place)
        }
        (Some(place), _) => {
            return Err(format!(
                "link {place} of node {other} is not a link back to {node}"
            ));
        }
        (None, Some(far)) => far.iter().position(|l| names(&l.other_node, node)),
        (None, None) => {
            if regions.get(other.region).is_none() {
                notes.push(format!(
                    "region {} is not in the set, so a link back from {other} is left if it states one",
                    other.region
                ));
            }
            None
        }
    };
    let back_word = match back {
        Some(place) => Some(Word(
            take_link(regions.edit(other.region)?, usize::from(other.index), place)?.as_integer1,
        )),
        None => None,
    };
    take_link(regions.edit(node.region)?, usize::from(node.index), link)?;
    Ok(Changed {
        inverse: Operation::AddLink {
            node,
            expects: None,
            other_node: other,
            as_integer1: Word(removed.as_integer1),
            link: Some(link),
            back_as_integer1: back_word,
            back_link: back,
        },
        added: None,
        notes,
    })
}

/// A node to put into a region, with what comes with it.
struct NewNode<'a> {
    node: Address,
    ped: bool,
    record: &'a NodeRecord,
    links: &'a [LinkRecord],
    named_by: &'a [NamedBy],
    virtual_junctions: &'a [JunctionRecord],
    junction_map: &'a [PairRecord],
}

fn insert_node(regions: &mut Regions<'_>, new: &NewNode<'_>) -> Result<Changed, String> {
    let node = new.node;
    let held = regions.held(node.region)?;
    let (place, len) = (usize::from(node.index), held.nodes.len());
    let car = usize::try_from(held.num_nodes_car_nodes)
        .map_err(|_| "the region states a count of vehicle nodes below zero".to_string())?;
    let fits = if new.ped {
        car <= place && place <= len
    } else {
        place <= car
    };
    if !fits {
        return Err(format!(
            "node {node} is not a place for a {} node: the region holds {len} node(s), the first {car} of them vehicle nodes, and the vehicle nodes come first",
            if new.ped { "pedestrian" } else { "vehicle" }
        ));
    }
    if u16::try_from(len).is_err() {
        return Err("the region holds as many nodes as an address can name".to_string());
    }
    if let Some(named) = (new.named_by.iter())
        .find(|named| named.node.region != node.region || usize::from(named.node.index) >= len)
    {
        return Err(format!(
            "node {} is not a node of region {} as it stands, and only a link of the same region comes with a node: a link of another region is added with add_link",
            named.node, node.region
        ));
    }
    if place < len {
        every_neighbour_is_held(regions, node.region, place)?;
    }
    renumber(regions, node.region, node.index, 1)?;

    let region = regions.edit(node.region)?;
    let mut inserted = CPathNode {
        street_name_hash: new.record.street_name_hash.0,
        distance_to_target: new.record.distance_to_target,
        coors_x: new.record.coors_x,
        coors_y: new.record.coors_y,
        as_integer1: new.record.as_integer1.0,
        ..Default::default()
    };
    let flags = flags_of(&inserted, new.record.as_integer2)?;
    inserted.set_as_integer2(flags);
    inserted.set_num_links(new.links.len()).map_err(|e| {
        format!(
            "the node holds {} link(s), and its word counts no more: {e}",
            new.links.len()
        )
    })?;
    let start = first_link(region, place);
    if start > region.links.len() {
        return Err("the region holds fewer links than its nodes count".to_string());
    }
    region.nodes.insert(place, inserted);
    region.links.splice(
        start..start,
        (new.links.iter()).map(|link| link_to(link.other_node, link.as_integer1.0)),
    );
    if new.ped {
        region.num_nodes_ped_nodes += 1;
    } else {
        region.num_nodes_car_nodes += 1;
    }
    readdress(region, node.region)?;
    for named in new.named_by {
        let before = usize::from(named.node.index);
        let index = if before >= place { before + 1 } else { before };
        put_link(
            region,
            index,
            named.link,
            link_to(node, named.as_integer1.0),
        )?;
    }

    let table =
        |region: &mut CPathRegion| std::mem::take(&mut region.junction_map.junction_map.data);
    for junction in new.virtual_junctions {
        put_junction(region, junction)?;
        let mut pairs = table(region);
        for pair in &mut pairs {
            if usize::try_from(pair.data).is_ok_and(|data| data >= junction.index) {
                pair.data += 1;
            }
        }
        region.junction_map.junction_map.data = pairs;
    }
    let mut pairs = table(region);
    for pair in new.junction_map {
        if pair.place > pairs.len() {
            return Err(format!(
                "reference {} is past the end of the {} the junction table holds",
                pair.place,
                pairs.len()
            ));
        }
        pairs.insert(
            pair.place,
            AtBinaryMapDataPair {
                key: key_of(node),
                data: pair.data,
                ..Default::default()
            },
        );
    }
    // ⚠ The game finds a node's junction by a binary search of the table, and trusts the index it finds.
    if pairs
        .windows(2)
        .any(|two| two[0].address() > two[1].address())
    {
        return Err(
            "the junction table would not be in the order of its addresses, and a lookup would miss a node"
                .to_string(),
        );
    }
    let junctions = region.virtual_junctions.len();
    if let Some(pair) = (new.junction_map.iter())
        .find(|pair| !usize::try_from(pair.data).is_ok_and(|data| data < junctions))
    {
        return Err(format!(
            "a reference names junction {} of the {junctions} the region holds",
            pair.data
        ));
    }
    region.junction_map.junction_map.data = pairs;
    Ok(Changed {
        inverse: Operation::RemoveNode {
            node,
            expects: None,
        },
        added: Some(node),
        notes: Vec::new(),
    })
}

fn remove_node(regions: &mut Regions<'_>, node: Address) -> Result<Changed, String> {
    let held = regions.held(node.region)?;
    let removed = node_at(held, node)?.clone();
    let index = usize::from(node.index);
    let named = named_elsewhere(regions, node)?;
    if !named.is_empty() {
        return Err(format!(
            "another region of the set names the node, by {}: a node another region names is not taken out, since what names it would name a node that is gone",
            named.join(", ")
        ));
    }
    every_far_end_is_held(regions, node)?;
    every_neighbour_is_held(regions, node.region, index + 1)?;
    let car = usize::try_from(held.num_nodes_car_nodes)
        .map_err(|_| "the region states a count of vehicle nodes below zero".to_string())?;
    let ped = index >= car;
    let links: Vec<LinkRecord> = (links_of(held, index)?.iter())
        .map(|link| LinkRecord {
            other_node: Address::new(link.other_node.region, link.other_node.index),
            as_integer1: Word(link.as_integer1),
        })
        .collect();
    // The links of the other nodes of the region that name it, by the node as it stands and the place of the link.
    let mut naming: Vec<(usize, usize, u32)> = Vec::new();
    for other in (0..held.nodes.len()).filter(|other| *other != index) {
        for (place, link) in links_of(held, other)?.iter().enumerate() {
            if names(&link.other_node, node) {
                naming.push((other, place, link.as_integer1));
            }
        }
    }
    let after = |other: usize| if other > index { other - 1 } else { other };
    let named_by = (naming.iter())
        .map(|(other, place, word)| {
            Ok(NamedBy {
                node: Address::new(
                    node.region,
                    u16::try_from(after(*other)).map_err(|_| {
                        "a region holds no more nodes than an address can name".to_string()
                    })?,
                ),
                link: *place,
                as_integer1: Word(*word),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let region = regions.edit(node.region)?;
    let mut junction_map = Vec::new();
    let mut kept = Vec::new();
    let table = std::mem::take(&mut region.junction_map.junction_map.data);
    for (place, pair) in table.into_iter().enumerate() {
        if names_pair(&pair, node) {
            junction_map.push(PairRecord {
                place,
                data: pair.data,
            });
        } else {
            kept.push(pair);
        }
    }
    // A junction only the references of this node named goes with them.
    let junctions = region.virtual_junctions.len();
    let orphaned: BTreeSet<usize> = (junction_map.iter())
        .filter_map(|pair| usize::try_from(pair.data).ok())
        .filter(|data| {
            *data < junctions
                && !kept
                    .iter()
                    .any(|pair| usize::try_from(pair.data) == Ok(*data))
        })
        .collect();
    let mut virtual_junctions = Vec::new();
    for junction in orphaned.iter().rev() {
        virtual_junctions.push(take_junction(region, *junction)?);
    }
    virtual_junctions.reverse();
    for pair in &mut kept {
        let below = (orphaned.iter())
            .filter(|gone| usize::try_from(pair.data).is_ok_and(|data| **gone < data))
            .count();
        pair.data -= i32::try_from(below).unwrap_or(0);
    }
    region.junction_map.junction_map.data = kept;

    // From the last, so that the place of each link before it stays true.
    for (other, place, _) in naming.iter().rev() {
        take_link(region, *other, *place)?;
    }
    let start = first_link(region, index);
    let end = start + removed.num_links();
    if end > region.links.len() {
        return Err("the region holds fewer links than its nodes count".to_string());
    }
    region.links.drain(start..end);
    region.nodes.remove(index);
    if ped {
        region.num_nodes_ped_nodes -= 1;
    } else {
        region.num_nodes_car_nodes -= 1;
    }
    readdress(region, node.region)?;
    if let Some(later) = node.index.checked_add(1) {
        renumber(regions, node.region, later, -1)?;
    }
    Ok(Changed::taken_back_by(Operation::InsertNode {
        node,
        expects: None,
        ped,
        record: record_of(&removed),
        links,
        named_by,
        virtual_junctions,
        junction_map,
    }))
}

/// A node to add as a copy of the one it is linked to.
struct AddedNode {
    region: u16,
    ped: bool,
    coors: [i16; 3],
    linked_to: Address,
    as_integer1: Word,
    back_as_integer1: Word,
}

fn add_node(regions: &mut Regions<'_>, new: &AddedNode) -> Result<Changed, String> {
    let held = regions.held(new.region)?;
    if new.linked_to.region != new.region {
        return Err(format!(
            "node {} is of another region: a node is added linked to a node of its own region, and a link into another region is added after it with add_link",
            new.linked_to
        ));
    }
    let like = node_at(held, new.linked_to)?;
    let car = usize::try_from(held.num_nodes_car_nodes)
        .map_err(|_| "the region states a count of vehicle nodes below zero".to_string())?;
    let place = if new.ped { held.nodes.len() } else { car };
    let index = u16::try_from(place)
        .map_err(|_| "the region holds as many nodes as an address can name".to_string())?;
    let [coors_x, coors_y, coors_z] = new.coors;
    let record = NodeRecord {
        street_name_hash: Word(like.street_name_hash),
        distance_to_target: like.distance_to_target,
        coors_x,
        coors_y,
        as_integer1: Word(with_height(like.as_integer1, coors_z)?),
        as_integer2: Word(like.as_integer2()),
    };
    let back_place = links_of(held, usize::from(new.linked_to.index))?.len();
    // The node it is linked to, as it is numbered once the new one stands before it.
    let linked_after = if usize::from(new.linked_to.index) >= place {
        (new.linked_to.index.checked_add(1))
            .map(|index| Address::new(new.region, index))
            .ok_or_else(|| "the region holds as many nodes as an address can name".to_string())?
    } else {
        new.linked_to
    };
    insert_node(
        regions,
        &NewNode {
            node: Address::new(new.region, index),
            ped: new.ped,
            record: &record,
            links: &[LinkRecord {
                other_node: linked_after,
                as_integer1: new.as_integer1,
            }],
            named_by: &[NamedBy {
                node: new.linked_to,
                link: back_place,
                as_integer1: new.back_as_integer1,
            }],
            virtual_junctions: &[],
            junction_map: &[],
        },
    )
}

/// Do `operation` to the regions of a set.
fn change(regions: &mut Regions<'_>, operation: &Operation) -> Result<Changed, String> {
    match operation {
        Operation::SetNode {
            node,
            expects: _,
            street_name_hash,
            distance_to_target,
            coors_x,
            coors_y,
            coors_z,
            as_integer1,
            as_integer2,
        } => set_node(
            regions,
            *node,
            &NodeValues {
                street_name_hash: *street_name_hash,
                distance_to_target: *distance_to_target,
                coors_x: *coors_x,
                coors_y: *coors_y,
                coors_z: *coors_z,
                as_integer1: *as_integer1,
                as_integer2: *as_integer2,
            },
        ),
        Operation::SetLink {
            node,
            expects: _,
            link,
            as_integer1,
        } => set_link(regions, *node, *link, *as_integer1),
        Operation::AddLink {
            node,
            expects: _,
            other_node,
            as_integer1,
            link,
            back_as_integer1,
            back_link,
        } => add_link(
            regions,
            &NewLink {
                node: *node,
                other_node: *other_node,
                as_integer1: *as_integer1,
                link: *link,
                back_as_integer1: *back_as_integer1,
                back_link: *back_link,
            },
        ),
        Operation::RemoveLink {
            node,
            expects: _,
            link,
            back_link,
            one_way,
        } => remove_link(regions, *node, *link, *back_link, *one_way),
        Operation::AddNode {
            region,
            expects: _,
            ped,
            coors_x,
            coors_y,
            coors_z,
            linked_to,
            as_integer1,
            back_as_integer1,
        } => add_node(
            regions,
            &AddedNode {
                region: *region,
                ped: *ped,
                coors: [*coors_x, *coors_y, *coors_z],
                linked_to: *linked_to,
                as_integer1: *as_integer1,
                back_as_integer1: *back_as_integer1,
            },
        ),
        Operation::RemoveNode { node, .. } => remove_node(regions, *node),
        Operation::InsertNode {
            node,
            expects: _,
            ped,
            record,
            links,
            named_by,
            virtual_junctions,
            junction_map,
        } => insert_node(
            regions,
            &NewNode {
                node: *node,
                ped: *ped,
                record,
                links,
                named_by,
                virtual_junctions,
                junction_map,
            },
        ),
        Operation::DisableNode { node, .. } => {
            regions.node(*node)?;
            Err(
                "the file states no such thing: the declaration of a path node names two words of flags, a height and a count of links, and no state that disables a node, so none is invented here; a node no other region names is taken out with remove_node"
                    .to_string(),
            )
        }
        Operation::RewriteRegions { .. } => {
            Err("rewrite_regions holds documents, and edits no region".to_string())
        }
    }
}

fn problems(found: Vec<or_source::Problem>) -> String {
    found
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// A source the work holds: its inputs with the document the operations so far leave, and the document on disk.
struct Held {
    dir: PathBuf,
    inputs: Inputs,
    /// The digest of the document held, which the digest of the set is made of.
    digest: String,
    on_disk: Vec<u8>,
    /// The region the document held cooks to, or why it does not cook.
    cooked: Result<CPathRegion, String>,
}

fn cooked(inputs: &Inputs) -> Result<CPathRegion, String> {
    let bytes = inputs.compile().map_err(problems)?.bytes;
    CPathRegion::from_bytes(&bytes).map_err(|e| e.to_string())
}

/// What an operation leaves each source it changes holding: its document, and the region the document cooks to.
type Left = BTreeMap<String, (Vec<u8>, CPathRegion)>;

/// The paths sources of a folder while operations are applied: nothing is written until [`Work::commit`].
struct Work {
    /// The folder that holds the set, as a refusal names it.
    folder: String,
    held: BTreeMap<String, Held>,
}

impl Work {
    fn open(folder: &Path) -> Result<Self, Error> {
        let mut held = BTreeMap::new();
        for (name, dir) in sources(folder)? {
            let refused = |why: String| Error::Source(name.clone(), why);
            let opened =
                or_source::asset::Source::open(&dir).map_err(|p| refused(p.to_string()))?;
            let inputs = Inputs::read(&opened).map_err(|p| refused(problems(p)))?;
            let on_disk = inputs.rules().to_vec();
            let cooked = cooked(&inputs);
            let source = Held {
                dir,
                digest: crate::operation::digest_of(&on_disk),
                inputs,
                on_disk,
                cooked,
            };
            held.insert(name, source);
        }
        Ok(Self {
            folder: folder.display().to_string(),
            held,
        })
    }

    /// The digest of the documents the work holds.
    fn digest(&self) -> String {
        let each = self.held.iter();
        digest_of_each(each.map(|(source, held)| (source.as_str(), held.digest.clone())))
    }

    /// Each region of the set with the source that states it, refused while a source does not cook or two state one region.
    fn regions(&self) -> Result<BTreeMap<u16, (&str, &CPathRegion)>, String> {
        let mut found: BTreeMap<u16, (&str, &CPathRegion)> = BTreeMap::new();
        for (source, held) in &self.held {
            let region = held.cooked.as_ref().map_err(|why| {
                format!("`{source}` does not cook, so the regions of the set cannot be told: {why}")
            })?;
            // A region with no node states no address, and nothing can name a node of it.
            let Some(first) = region.nodes.first() else {
                continue;
            };
            let id = first.address.region;
            if let Some((other, _)) = found.insert(id, (source.as_str(), region)) {
                return Err(format!(
                    "`{other}` and `{source}` both state the nodes of region {id}, and the game finds a node in one file"
                ));
            }
        }
        Ok(found)
    }

    /// Do `operation` to the regions the work holds, and say what each source it changes is left holding.
    fn through(&self, operation: &Operation) -> Result<(Left, Changed), String> {
        let mut regions = Regions {
            base: self.regions()?,
            changed: BTreeMap::new(),
        };
        let done = change(&mut regions, operation)?;
        let mut left = Left::new();
        for (id, region) in &regions.changed {
            let source = regions.source(*id)?;
            let (_, before) =
                (regions.base.get(id)).ok_or_else(|| format!("region {id} is not in the set"))?;
            let held = (self.held.get(source))
                .ok_or_else(|| format!("`{source}` is not a source of the set"))?;
            let (text, cooked) = (held.inputs.rules_of(region))
                .map_err(|found| format!("`{source}`: {}", problems(found)))?;
            // ⚠ A region the operation left as it was keeps its document, which a person may have typed.
            if paths::exported(before).is_ok_and(|was| was.as_bytes() == text.as_slice()) {
                continue;
            }
            left.insert(source.to_string(), (text, cooked));
        }
        Ok((left, done))
    }

    /// `inverse` as its JSON reads back, once doing that to the regions `left` puts back every document as the work holds it.
    fn proven(&self, left: &Left, inverse: &Operation) -> Option<Operation> {
        let read_back: Operation =
            serde_json::from_str(&serde_json::to_string(inverse).ok()?).ok()?;
        let mut base = self.regions().ok()?;
        for (source, (_, cooked)) in left {
            base.insert(
                cooked.nodes.first()?.address.region,
                (source.as_str(), cooked),
            );
        }
        let mut regions = Regions {
            base,
            changed: BTreeMap::new(),
        };
        change(&mut regions, &read_back).ok()?;
        let mut owed: BTreeSet<&str> = left.keys().map(String::as_str).collect();
        for (id, region) in &regions.changed {
            let source = regions.source(*id).ok()?;
            let held = self.held.get(source)?;
            let (text, _) = held.inputs.rules_of(region).ok()?;
            if text != held.inputs.rules() {
                return None;
            }
            owed.remove(source);
        }
        owed.is_empty().then_some(read_back)
    }

    /// Hold each document of `documents` as the document of its source, once it compiles.
    fn rewritten(&self, documents: &[Document]) -> Result<(Left, Changed), String> {
        if documents.is_empty() {
            return Err("it names no source to rewrite".to_string());
        }
        let mut left = Left::new();
        let mut was = Vec::with_capacity(documents.len());
        for stated in documents {
            let source = &stated.source;
            let held = (self.held.get(source))
                .ok_or_else(|| format!("no paths source of the folder is called `{source}`"))?;
            if left.contains_key(source) {
                return Err(format!("`{source}` is named twice"));
            }
            // ⚠ A document that does not compile would be a source nothing cooks.
            let mut candidate = held.inputs.clone();
            candidate.set_rules(stated.document.clone().into_bytes());
            let region = cooked(&candidate).map_err(|why| format!("`{source}`: {why}"))?;
            was.push(Document {
                source: source.clone(),
                document: text_of(source, &held.inputs)?,
            });
            left.insert(source.clone(), (candidate.rules().to_vec(), region));
        }
        let inverse = Operation::RewriteRegions {
            expects: None,
            documents: was,
        };
        Ok((left, Changed::taken_back_by(inverse)))
    }

    /// Do `operation` to the documents the work holds, once they have the digest the operation states.
    fn apply(&mut self, operation: &Operation) -> Result<Applied, Error> {
        if let Some(expects) = operation.expects() {
            let found = self.digest();
            if found != expects {
                return Err(Error::Stale {
                    name: operation.name(),
                    at: self.folder.clone(),
                    expects: expects.to_string(),
                    found,
                });
            }
        }
        (self.change(operation)).map_err(|why| refused(operation, why))
    }

    /// Do `operation` to the documents the work holds.
    fn change(&mut self, operation: &Operation) -> Result<Applied, String> {
        let (left, done) = match operation {
            Operation::RewriteRegions { documents, .. } => self.rewritten(documents)?,
            _ => {
                let (left, mut done) = self.through(operation)?;
                done.inverse = match self.proven(&left, &done.inverse) {
                    Some(inverse) => inverse,
                    // An operation that changed no document is taken back by one that changes none.
                    None if left.is_empty() => done.inverse,
                    None => Operation::RewriteRegions {
                        expects: None,
                        documents: (left.keys())
                            .map(|source| {
                                let held = (self.held.get(source)).ok_or_else(|| {
                                    format!("`{source}` is not a source of the set")
                                })?;
                                Ok(Document {
                                    source: source.clone(),
                                    document: text_of(source, &held.inputs)?,
                                })
                            })
                            .collect::<Result<_, String>>()?,
                    },
                };
                (left, done)
            }
        };
        let mut edited = Vec::with_capacity(left.len());
        for (source, (text, region)) in left {
            let held = (self.held.get_mut(&source))
                .ok_or_else(|| format!("`{source}` is not a source of the set"))?;
            edited.push(Edited {
                file: held.dir.join(paths::RULES),
                before: held.inputs.rules().to_vec(),
                after: text.clone(),
            });
            held.digest = crate::operation::digest_of(&text);
            held.inputs.set_rules(text);
            held.cooked = Ok(region);
        }
        Ok(Applied {
            inverse: done.inverse.expecting(self.digest()),
            edited,
            added: done.added,
            notes: done.notes,
        })
    }

    /// Write the document of every source the operations changed; when one cannot be written, the ones written before it are put back.
    fn commit(self) -> Result<(), Error> {
        let mut written: Vec<&Held> = Vec::new();
        for held in self.held.values() {
            if held.inputs.rules() == held.on_disk.as_slice() {
                continue;
            }
            if let Err(why) = paths::write_rules(&held.dir, held.inputs.rules()) {
                let mut said = problems(why);
                for earlier in written {
                    if let Err(left) = paths::write_rules(&earlier.dir, &earlier.on_disk) {
                        said.push_str(&format!(
                            "; and {} could not be put back as it was: {}",
                            earlier.dir.display(),
                            problems(left)
                        ));
                    }
                }
                return Err(Error::Write(said));
            }
            written.push(held);
        }
        Ok(())
    }
}

/// The document `inputs` hold, which an inverse carries as text.
fn text_of(source: &str, inputs: &Inputs) -> Result<String, String> {
    String::from_utf8(inputs.rules().to_vec()).map_err(|_| {
        format!("the document of `{source}` is not UTF-8, so the operation could not be taken back")
    })
}

fn refused(operation: &Operation, why: String) -> Error {
    Error::Refused {
        name: operation.name(),
        at: operation.target(),
        why,
    }
}

/// Apply `operation` to the path regions the sources of `folder` state, and say what takes it back; a refusal leaves every source as it was.
pub fn apply(folder: &Path, operation: &Operation) -> Result<Applied, Error> {
    let mut work = Work::open(folder)?;
    let applied = work.apply(operation)?;
    work.commit()?;
    Ok(applied)
}

/// Apply `operations` in order to the path regions the sources of `folder` state, all of them or none, and say what each did; with `dry_run` nothing is written.
pub fn apply_all(
    folder: &Path,
    operations: &[Operation],
    dry_run: bool,
) -> Result<Vec<Applied>, Error> {
    let mut work = Work::open(folder)?;
    let mut applied = Vec::with_capacity(operations.len());
    for (index, operation) in operations.iter().enumerate() {
        let done = work.apply(operation).map_err(|refused| Error::InList {
            number: index + 1,
            of: operations.len(),
            refused: Box::new(refused),
        })?;
        applied.push(done);
    }
    if !dry_run {
        work.commit()?;
    }
    Ok(applied)
}

/// The hand-built regions the suites of `tests/` share.
#[cfg(test)]
#[path = "../../tests/common/roads.rs"]
mod built;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inverse_that_leaves_a_source_as_the_operation_left_it_is_not_given() {
        let scratch = tempfile::tempdir().unwrap();
        let folder = scratch.path().join("roads");
        let seven = [
            built::node([0, 0, 0], &[(7, 1, 1)]),
            built::node([10, 0, 0], &[(7, 0, 2)]),
        ];
        built::extract(&folder, "nodes7", &built::region(7, 2, &seven, &[]));
        let eight = [built::node([100, 0, 0], &[])];
        built::extract(&folder, "nodes8", &built::region(8, 1, &eight, &[]));
        let work = Work::open(&folder).unwrap();
        let linked = Operation::AddLink {
            node: Address::new(7, 0),
            expects: None,
            other_node: Address::new(8, 0),
            as_integer1: Word(3),
            link: None,
            back_as_integer1: Some(Word(4)),
            back_link: None,
        };
        let (left, done) = work.through(&linked).unwrap();
        assert_eq!(left.len(), 2, "the link is stated in both regions");
        assert_eq!(
            work.proven(&left, &done.inverse),
            Some(done.inverse.clone())
        );
        // The link taken out of its own region alone: that document is as it was, and the other still holds the link back.
        let half = Operation::RemoveLink {
            node: Address::new(7, 0),
            expects: None,
            link: 1,
            back_link: None,
            one_way: true,
        };
        assert_eq!(work.proven(&left, &half), None);
    }

    #[test]
    fn a_node_that_links_out_of_the_set_is_refused_for_its_link_back_and_a_later_one_for_its_number(
    ) {
        let scratch = tempfile::tempdir().unwrap();
        let folder = scratch.path().join("roads");
        let seven = [
            built::node([0, 0, 0], &[(7, 1, 1)]),
            built::node([10, 0, 0], &[(7, 0, 2), (7, 2, 3)]),
            built::node([20, 0, 0], &[(7, 1, 4), (216, 0, 5)]),
        ];
        built::extract(&folder, "nodes7", &built::region(7, 3, &seven, &[]));
        let refused = |index| {
            let node = Address::new(7, index);
            let removed = Operation::RemoveNode {
                node,
                expects: None,
            };
            let why = apply(&folder, &removed).unwrap_err();
            why.to_string()
        };
        // Nothing follows the last node, so nothing is renumbered: what is in the way is the link back to the node itself.
        let last = refused(2);
        assert!(
            last.contains("it links to 216:0, of a region the set does not hold")
                && last.contains("would name a node that is gone")
                && last.contains("add region(s) 216"),
            "{last}"
        );
        assert!(!last.contains("renumbers"), "{last}");
        let earlier = refused(0);
        assert!(
            earlier.contains("it renumbers the nodes of region 7 from 7:1")
                && earlier.contains("node 7:2 links to 216:0"),
            "{earlier}"
        );
    }
}
