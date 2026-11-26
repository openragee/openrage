//! `water.xml`: the level's water as rectangles at a height, and the rectangles where it is calmed
//! and where it is given waves. A document is read into typed values and kept whole beside them, so
//! a save writes every element and attribute it held and saving that again gives the same bytes.
//!
//! The reader types the root `WaterData`, its lists `WaterQuads`, `CalmingQuads` and `WaveQuads`,
//! their `Item`s, and in an item the `value` of `minX`, `maxX`, `minY` and `maxY` and, by list, of
//! `Type`, `IsInvisible`, `HasLimitedDepth`, `NoStencil`, `z` and `a1` to `a4`; of `fDampening`; of
//! `Amplitude`, `XDirection` and `YDirection`. Whatever else a document states is kept as it came,
//! and [`CWaterData::unnamed`] lists it.
//!
//! ⚠ That list awaits a run over an install: no install's water file has been read by this, and
//! or-title's `the_water_file_of_an_install` is the suite that holds it on one.
//!
//! ⚠ A quad's rectangle is taken with its corners either way round, where the game leaves out a quad
//! whose `min` is not under its `max`; and past every quad [`CWaterData::height_at`] answers
//! `None`, where the game answers the height of its sea outside the level.

use std::collections::BTreeSet;

use document::{Attribute, Body, Document, Element, Node};

mod document;

const ROOT: &str = "WaterData";
const ITEM: &str = "Item";
const VALUE: &str = "value";
const CORNERS: [&str; 4] = ["minX", "maxX", "minY", "maxY"];

/// The `Type` of a water quad the game leaves out.
const DELETED: u8 = 5;
/// How far past its cut, in metres, a quad cut to its southern half still holds a point.
const CUT_SLACK: f32 = 0.001;

/// Whether `leaf` is the name of a water file: the level's `water.xml`, or one a pack adds as `water_*.xml`.
pub fn is_water_file(leaf: &str) -> bool {
    let name = leaf.to_ascii_lowercase();
    name == "water.xml" || (name.starts_with("water_") && name.ends_with(".xml"))
}

/// Why bytes are not a water file, or a value is not one the file can state.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum WaterError {
    #[error("the water file is not UTF-8 text from its byte {0} on")]
    NotText(usize),
    #[error("the water file is not XML this reads, at line {line}: {why}")]
    Malformed { line: usize, why: String },
    #[error("the document's root is <{0}>, not <WaterData>")]
    NotWater(String),
    #[error("item {item} of <{list}>, counted from 0, states no <{corner}>")]
    MissingCorner {
        list: &'static str,
        item: usize,
        corner: &'static str,
    },
    #[error("<{element}> of item {item} of <{list}>, counted from 0, has no `value`")]
    NoValue {
        list: &'static str,
        item: usize,
        element: &'static str,
    },
    #[error(
        "<{element}> of item {item} of <{list}>, counted from 0, holds `{text}`, which is not {wanted}"
    )]
    NotANumber {
        list: &'static str,
        item: usize,
        element: &'static str,
        text: String,
        wanted: &'static str,
    },
    #[error(
        "<{element}> of item {item} of <{list}>, counted from 0, is {value}, which the file cannot state"
    )]
    Unstatable {
        list: &'static str,
        item: usize,
        element: &'static str,
        value: String,
    },
}

/// A rectangle of the level, in metres, its `min` at or under its `max`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rectangle {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

impl Rectangle {
    fn of(min_x: i16, max_x: i16, min_y: i16, max_y: i16) -> Self {
        let (min_x, max_x) = (min_x.min(max_x), min_x.max(max_x));
        let (min_y, max_y) = (min_y.min(max_y), min_y.max(max_y));
        Self {
            min: [f32::from(min_x), f32::from(min_y)],
            max: [f32::from(max_x), f32::from(max_y)],
        }
    }

    /// Whether (`x`, `y`) lies in the rectangle, its edges included.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        (self.min[0]..=self.max[0]).contains(&x) && (self.min[1]..=self.max[1]).contains(&y)
    }
}

/// What a quad's item held beyond its typed values: the item as it came, and what stood before it in its list.
#[derive(Debug, Clone, Default)]
struct Carry {
    lead: Vec<Node>,
    /// `None` for a quad no document held.
    item: Option<Element>,
}

/// One item of `WaterQuads`: a level rectangle of water, in whole metres.
#[derive(Debug, Clone)]
pub struct CWaterQuad {
    pub min_x: i16,
    pub max_x: i16,
    pub min_y: i16,
    pub max_y: i16,
    /// 0 the whole rectangle; 1 to 4 the half of it that keeps its south-west, north-west, north-east or south-east corner; 5 a quad the game leaves out.
    pub r#type: u8,
    pub is_invisible: bool,
    pub has_limited_depth: bool,
    pub no_stencil: bool,
    /// The height of its surface, in metres.
    pub z: f32,
    /// The opacity at a corner, of 255; with `a1` at 0 the game takes all four from its tuning.
    pub a1: u8,
    pub a2: u8,
    pub a3: u8,
    pub a4: u8,
    carry: Carry,
}

impl Default for CWaterQuad {
    fn default() -> Self {
        Self {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            r#type: 0,
            is_invisible: false,
            has_limited_depth: false,
            no_stencil: false,
            z: 0.0,
            a1: 26,
            a2: 26,
            a3: 26,
            a4: 26,
            carry: Carry::default(),
        }
    }
}

/// Equal in what they state by name, whatever else their items held.
impl PartialEq for CWaterQuad {
    fn eq(&self, other: &Self) -> bool {
        self.rectangle_stated() == other.rectangle_stated()
            && self.r#type == other.r#type
            && self.is_invisible == other.is_invisible
            && self.has_limited_depth == other.has_limited_depth
            && self.no_stencil == other.no_stencil
            && self.z == other.z
            && [self.a1, self.a2, self.a3, self.a4] == [other.a1, other.a2, other.a3, other.a4]
    }
}

impl CWaterQuad {
    fn rectangle_stated(&self) -> [i16; 4] {
        [self.min_x, self.max_x, self.min_y, self.max_y]
    }

    /// The quad's rectangle, whichever way round its corners are stated.
    pub fn rectangle(&self) -> Rectangle {
        Rectangle::of(self.min_x, self.max_x, self.min_y, self.max_y)
    }

    /// Whether the quad's water lies over (`x`, `y`): in its rectangle, on the kept side of its cut.
    pub fn covers(&self, x: f32, y: f32) -> bool {
        let rectangle = self.rectangle();
        if self.r#type == DELETED || !rectangle.contains(x, y) {
            return false;
        }
        let [west, south] = rectangle.min;
        let [east, north] = rectangle.max;
        if self.r#type == 0 || east == west {
            return true;
        }
        // The cut is a diagonal: for an odd type it falls from the north-west corner to the
        // south-east, for an even one it rises from the south-west to the north-east. A type over
        // 4 other than 5 is cut as 3 or 4 by the same two tests.
        let rise = (north - south) / (east - west);
        let cut = match self.r#type % 2 == 1 {
            true => north - rise * (x - west),
            false => south + rise * (x - west),
        };
        match (self.r#type > 2) == (self.r#type % 2 == 1) {
            true => y >= cut,
            false => y <= cut + CUT_SLACK,
        }
    }
}

/// One item of `CalmingQuads`: a level rectangle where the waves are damped.
#[derive(Debug, Clone, Default)]
pub struct CCalmingQuad {
    pub min_x: i16,
    pub max_x: i16,
    pub min_y: i16,
    pub max_y: i16,
    /// `None` where the item states none.
    pub dampening: Option<f32>,
    carry: Carry,
}

/// Equal in what they state by name, whatever else their items held.
impl PartialEq for CCalmingQuad {
    fn eq(&self, other: &Self) -> bool {
        [self.min_x, self.max_x, self.min_y, self.max_y]
            == [other.min_x, other.max_x, other.min_y, other.max_y]
            && self.dampening == other.dampening
    }
}

impl CCalmingQuad {
    /// The quad's rectangle, whichever way round its corners are stated.
    pub fn rectangle(&self) -> Rectangle {
        Rectangle::of(self.min_x, self.max_x, self.min_y, self.max_y)
    }
}

/// One item of `WaveQuads`: a level rectangle where waves of a height run one way.
#[derive(Debug, Clone, Default)]
pub struct CWaveQuad {
    pub min_x: i16,
    pub max_x: i16,
    pub min_y: i16,
    pub max_y: i16,
    /// Each `None` where the item states none.
    pub amplitude: Option<f32>,
    pub x_direction: Option<f32>,
    pub y_direction: Option<f32>,
    carry: Carry,
}

/// Equal in what they state by name, whatever else their items held.
impl PartialEq for CWaveQuad {
    fn eq(&self, other: &Self) -> bool {
        [self.min_x, self.max_x, self.min_y, self.max_y]
            == [other.min_x, other.max_x, other.min_y, other.max_y]
            && self.amplitude == other.amplitude
            && self.x_direction == other.x_direction
            && self.y_direction == other.y_direction
    }
}

impl CWaveQuad {
    /// The quad's rectangle, whichever way round its corners are stated.
    pub fn rectangle(&self) -> Rectangle {
        Rectangle::of(self.min_x, self.max_x, self.min_y, self.max_y)
    }
}

/// The white space written before a list, an item and a member the document did not hold; the tags
/// that close an item and a list stand behind the lead of the item and of the list.
#[derive(Debug, Clone)]
struct Layout {
    /// What ends a line, and stands before the tag that closes a root that had no white space there.
    newline: &'static str,
    list: String,
    item: String,
    member: String,
}

impl Layout {
    /// Two spaces a level under the root, on lines ended by `newline`.
    fn spaced(newline: &'static str) -> Self {
        Self {
            newline,
            list: format!("{newline}  "),
            item: format!("{newline}    "),
            member: format!("{newline}      "),
        }
    }

    /// A list led by `lead`, and under it an item and a member each a level deeper by what indents the list.
    fn indented_from(&mut self, lead: &str) {
        let level = lead.rsplit(['\n', '\r']).next().unwrap_or_default();
        self.list = lead.to_string();
        self.item = format!("{lead}{level}");
        self.member = format!("{lead}{level}{level}");
    }

    /// The white space that leads the first item of `quads` a document held, and that item's first element.
    fn take_from<Q: Quad>(&mut self, quads: &[Q]) {
        let Some(carry) = quads.first().map(Quad::carry) else {
            return;
        };
        if let Some(lead) = carry.lead.last().and_then(Node::space) {
            self.item = lead.to_string();
        }
        let nodes: &[Node] = match carry.item.as_ref().and_then(|item| item.body.as_ref()) {
            Some(body) => &body.nodes,
            None => &[],
        };
        let first = nodes
            .iter()
            .position(|node| matches!(node, Node::Element(_)));
        let before = first.and_then(|at| nodes[..at].last());
        if let Some(lead) = before.and_then(Node::space) {
            self.member = lead.to_string();
        }
    }
}

/// The lists of the root, in the order [`DocumentCarry::lists`] holds them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum List {
    Water,
    Calming,
    Wave,
}

impl List {
    const ALL: [Self; 3] = [Self::Water, Self::Calming, Self::Wave];

    fn name(self) -> &'static str {
        match self {
            Self::Water => "WaterQuads",
            Self::Calming => "CalmingQuads",
            Self::Wave => "WaveQuads",
        }
    }

    /// The elements of an item whose `value` is typed, past its corners.
    fn members(self) -> &'static [&'static str] {
        match self {
            Self::Water => &[
                "Type",
                "IsInvisible",
                "HasLimitedDepth",
                "NoStencil",
                "z",
                "a1",
                "a2",
                "a3",
                "a4",
            ],
            Self::Calming => &["fDampening"],
            Self::Wave => &["Amplitude", "XDirection", "YDirection"],
        }
    }

    fn types(self, element: &str) -> bool {
        CORNERS.contains(&element) || self.members().contains(&element)
    }
}

/// An element without what stands between its tags.
#[derive(Debug, Clone)]
struct Shell {
    attributes: Vec<Attribute>,
    tail: String,
    /// The white space in its closing tag, or `None` for an element closed in its own tag.
    close_tail: Option<String>,
}

impl Shell {
    fn open() -> Self {
        Self {
            attributes: Vec::new(),
            tail: String::new(),
            close_tail: Some(String::new()),
        }
    }

    /// The element `name` with this shell around `nodes`.
    fn around(&self, name: &str, nodes: Vec<Node>) -> Element {
        let closed = self.close_tail.is_none() && nodes.is_empty();
        // An element closed in its own tag that now holds something loses the white space before its slash.
        let tail = match closed || self.close_tail.is_some() {
            true => self.tail.clone(),
            false => String::new(),
        };
        Element {
            name: name.to_string(),
            attributes: self.attributes.clone(),
            tail,
            body: (!closed).then(|| Body {
                nodes,
                close_tail: self.close_tail.clone().unwrap_or_default(),
            }),
        }
    }
}

/// A list as the document held it, without its items.
#[derive(Debug, Clone)]
struct ListCarry {
    shell: Shell,
    /// What stood after its last item.
    trail: Vec<Node>,
}

#[derive(Debug, Clone)]
enum RootNode {
    Kept(Node),
    /// Where a typed list stands.
    List(List),
}

/// The document around the typed lists.
#[derive(Debug, Clone)]
struct DocumentCarry {
    prolog: String,
    shell: Shell,
    nodes: Vec<RootNode>,
    /// By [`List`]; `None` for a list the document does not hold.
    lists: [Option<ListCarry>; 3],
    epilog: String,
    layout: Layout,
}

impl Default for DocumentCarry {
    fn default() -> Self {
        Self {
            prolog: "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n".to_string(),
            shell: Shell::open(),
            nodes: Vec::new(),
            lists: [None, None, None],
            epilog: "\n".to_string(),
            layout: Layout::spaced("\n"),
        }
    }
}

/// A water file: its quads of water, and the quads that calm it and give it waves.
#[derive(Debug, Clone, Default)]
pub struct CWaterData {
    pub water_quads: Vec<CWaterQuad>,
    pub calming_quads: Vec<CCalmingQuad>,
    pub wave_quads: Vec<CWaveQuad>,
    carry: DocumentCarry,
}

/// Equal in the quads they hold, whatever else their documents state.
impl PartialEq for CWaterData {
    fn eq(&self, other: &Self) -> bool {
        self.water_quads == other.water_quads
            && self.calming_quads == other.calming_quads
            && self.wave_quads == other.wave_quads
    }
}

impl CWaterData {
    /// The water the XML text `bytes` states.
    pub fn open(bytes: &[u8]) -> Result<Self, WaterError> {
        let text = std::str::from_utf8(bytes).map_err(|e| WaterError::NotText(e.valid_up_to()))?;
        let document = Document::parse(text).map_err(|fault| WaterError::Malformed {
            line: 1 + text[..fault.at].matches('\n').count(),
            why: fault.why,
        })?;
        let Document {
            prolog,
            root,
            epilog,
        } = document;
        if root.name != ROOT {
            return Err(WaterError::NotWater(root.name));
        }
        let mut data = Self {
            carry: DocumentCarry {
                prolog,
                shell: Shell {
                    attributes: root.attributes,
                    tail: root.tail,
                    close_tail: root.body.as_ref().map(|body| body.close_tail.clone()),
                },
                nodes: Vec::new(),
                lists: [None, None, None],
                epilog,
                layout: Layout::spaced(match text.contains("\r\n") {
                    true => "\r\n",
                    false => "\n",
                }),
            },
            ..Self::default()
        };
        for node in root.body.map(|body| body.nodes).unwrap_or_default() {
            // A list stated a second time is kept as it came.
            let list = match &node {
                Node::Element(element) => (List::ALL.into_iter())
                    .find(|list| list.name() == element.name)
                    .filter(|list| data.carry.lists[*list as usize].is_none()),
                Node::Other(_) => None,
            };
            let (Some(list), Node::Element(element)) = (list, &node) else {
                data.carry.nodes.push(RootNode::Kept(node));
                continue;
            };
            let carry = match list {
                List::Water => read_list(element, &mut data.water_quads)?,
                List::Calming => read_list(element, &mut data.calming_quads)?,
                List::Wave => read_list(element, &mut data.wave_quads)?,
            };
            data.carry.lists[list as usize] = Some(carry);
            data.carry.nodes.push(RootNode::List(list));
        }
        let first = (data.carry.nodes.iter()).position(|node| matches!(node, RootNode::List(_)));
        let before = first.and_then(|at| data.carry.nodes[..at].last());
        if let Some(RootNode::Kept(before)) = before {
            if let Some(lead) = before.space() {
                data.carry.layout.indented_from(lead);
            }
        }
        // The last to state one stands: the water quads' items, before the calming and the wave quads'.
        data.carry.layout.take_from(&data.wave_quads);
        data.carry.layout.take_from(&data.calming_quads);
        data.carry.layout.take_from(&data.water_quads);
        Ok(data)
    }

    /// The document with its quads as they now stand: an unchanged value is written as its text came.
    pub fn save(&self) -> Result<Vec<u8>, WaterError> {
        let [water, calming, wave] = &self.carry.lists;
        let layout = &self.carry.layout;
        let mut lists = [
            list_element(&self.water_quads, water.as_ref(), layout)?,
            list_element(&self.calming_quads, calming.as_ref(), layout)?,
            list_element(&self.wave_quads, wave.as_ref(), layout)?,
        ];
        let mut nodes = Vec::new();
        for node in &self.carry.nodes {
            match node {
                RootNode::Kept(node) => nodes.push(node.clone()),
                RootNode::List(list) => {
                    nodes.extend(lists[*list as usize].take().map(Node::Element))
                }
            }
        }
        // A list the document did not hold stands after what it held, before the white space that closes the root.
        let fresh: Vec<Element> = lists.into_iter().flatten().collect();
        if !fresh.is_empty() {
            let closing = (nodes.last().is_some_and(Node::is_space))
                .then(|| nodes.pop())
                .flatten();
            for list in fresh {
                nodes.push(Node::Other(layout.list.clone()));
                nodes.push(Node::Element(list));
            }
            nodes.push(closing.unwrap_or_else(|| Node::Other(layout.newline.to_string())));
        }
        let mut out = self.carry.prolog.clone();
        self.carry.shell.around(ROOT, nodes).write(&mut out);
        out.push_str(&self.carry.epilog);
        Ok(out.into_bytes())
    }

    /// The height of the first water quad that lies over (`x`, `y`), in metres, or `None` where none does.
    pub fn height_at(&self, x: f32, y: f32) -> Option<f32> {
        (self.water_quads.iter())
            .find(|quad| quad.covers(x, y))
            .map(|quad| quad.z)
    }

    /// Every element, attribute and text the document states that the reader does not type, by its path; an element that repeats a typed one is among them.
    pub fn unnamed(&self) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        unnamed_attributes(&mut found, ROOT, &self.carry.shell.attributes, false);
        for node in &self.carry.nodes {
            if let RootNode::Kept(node) = node {
                unnamed_node(&mut found, ROOT, node);
            }
        }
        for (list, carry) in List::ALL.into_iter().zip(&self.carry.lists) {
            let Some(carry) = carry else { continue };
            let path = format!("{ROOT}/{}", list.name());
            unnamed_attributes(&mut found, &path, &carry.shell.attributes, false);
            for node in &carry.trail {
                unnamed_node(&mut found, &path, node);
            }
        }
        unnamed_items(&mut found, &self.water_quads);
        unnamed_items(&mut found, &self.calming_quads);
        unnamed_items(&mut found, &self.wave_quads);
        found
    }
}

fn unnamed_attributes(found: &mut BTreeSet<String>, path: &str, all: &[Attribute], value: bool) {
    for attribute in all {
        if !(value && attribute.name == VALUE) {
            found.insert(format!("{path}@{}", attribute.name));
        }
    }
}

fn unnamed_node(found: &mut BTreeSet<String>, path: &str, node: &Node) {
    match node {
        Node::Element(element) => found.insert(format!("{path}/{}", element.name)),
        // A comment and a processing instruction state nothing of the water.
        Node::Other(text) if text.starts_with("<!--") || text.starts_with("<?") => false,
        Node::Other(_) if node.is_space() => false,
        Node::Other(_) => found.insert(format!("{path}/#text")),
    };
}

fn unnamed_items<Q: Quad>(found: &mut BTreeSet<String>, quads: &[Q]) {
    let list = format!("{ROOT}/{}", Q::LIST.name());
    let path = format!("{list}/{ITEM}");
    for quad in quads {
        let carry = quad.carry();
        for node in &carry.lead {
            unnamed_node(found, &list, node);
        }
        let Some(item) = &carry.item else { continue };
        unnamed_attributes(found, &path, &item.attributes, false);
        let mut read: Vec<&str> = Vec::new();
        for node in item.body.iter().flat_map(|body| &body.nodes) {
            match node {
                Node::Element(member)
                    if Q::LIST.types(&member.name) && !read.contains(&member.name.as_str()) =>
                {
                    read.push(&member.name);
                    let path = format!("{path}/{}", member.name);
                    unnamed_attributes(found, &path, &member.attributes, true);
                    for node in member.body.iter().flat_map(|body| &body.nodes) {
                        unnamed_node(found, &path, node);
                    }
                }
                other => unnamed_node(found, &path, other),
            }
        }
    }
}

/// A kind of value an element states in its `value`.
trait Value: Copy {
    /// What a text that does not read is said not to be.
    const WANTED: &'static str;
    fn stated(text: &str) -> Option<Self>;
    /// The text a value the document did not state is written as; [`Self::stated`] reads it back to the same value.
    fn written(self) -> String;
    fn same(self, other: Self) -> bool;
    fn statable(self) -> bool {
        true
    }
}

impl Value for i16 {
    const WANTED: &'static str = "a whole number from -32768 to 32767";
    fn stated(text: &str) -> Option<Self> {
        text.trim().parse().ok()
    }
    fn written(self) -> String {
        self.to_string()
    }
    fn same(self, other: Self) -> bool {
        self == other
    }
}

impl Value for u8 {
    const WANTED: &'static str = "a whole number from 0 to 255";
    fn stated(text: &str) -> Option<Self> {
        text.trim().parse().ok()
    }
    fn written(self) -> String {
        self.to_string()
    }
    fn same(self, other: Self) -> bool {
        self == other
    }
}

impl Value for bool {
    const WANTED: &'static str = "true or false";
    fn stated(text: &str) -> Option<Self> {
        match text.trim() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }
    fn written(self) -> String {
        self.to_string()
    }
    fn same(self, other: Self) -> bool {
        self == other
    }
}

impl Value for f32 {
    const WANTED: &'static str = "a finite number";
    fn stated(text: &str) -> Option<Self> {
        text.trim()
            .parse()
            .ok()
            .filter(|value: &f32| value.is_finite())
    }
    fn written(self) -> String {
        self.to_string()
    }
    // By bits, so a zero keeps the sign it was stated with.
    fn same(self, other: Self) -> bool {
        self.to_bits() == other.to_bits()
    }
    fn statable(self) -> bool {
        self.is_finite()
    }
}

/// An item being read: which it is, for the error that refuses it.
struct Stated<'a> {
    list: List,
    index: usize,
    item: &'a Element,
}

impl Stated<'_> {
    /// The value the first `element` of the item states, or `None` where the item has no such element.
    fn read<T: Value>(&self, element: &'static str) -> Result<Option<T>, WaterError> {
        let Some(child) = self.item.child(element) else {
            return Ok(None);
        };
        let (list, item) = (self.list.name(), self.index);
        let text = child.attribute(VALUE).ok_or(WaterError::NoValue {
            list,
            item,
            element,
        })?;
        let value = T::stated(text).ok_or_else(|| WaterError::NotANumber {
            list,
            item,
            element,
            text: text.to_string(),
            wanted: T::WANTED,
        })?;
        Ok(Some(value))
    }

    /// The four corners, in the order of [`CORNERS`]; an item that states none of them is at zero.
    fn corners(&self) -> Result<[i16; 4], WaterError> {
        let mut corners = [0; 4];
        if CORNERS
            .iter()
            .all(|corner| self.item.child(corner).is_none())
        {
            return Ok(corners);
        }
        for (corner, element) in corners.iter_mut().zip(CORNERS) {
            *corner = self.read(element)?.ok_or(WaterError::MissingCorner {
                list: self.list.name(),
                item: self.index,
                corner: element,
            })?;
        }
        Ok(corners)
    }
}

/// An item being written from a quad: the item as it came, or an empty one for a quad no document held.
struct Writing<'a> {
    list: List,
    index: usize,
    item: Element,
    fresh: bool,
    layout: &'a Layout,
}

impl Writing<'_> {
    /// State `value` under `element`. An element the item holds keeps its text unless that reads to another value; one it lacks is added unless `absent` is what its absence reads as.
    fn put<T: Value>(
        &mut self,
        element: &'static str,
        value: T,
        absent: Option<T>,
    ) -> Result<(), WaterError> {
        if !value.statable() {
            return Err(WaterError::Unstatable {
                list: self.list.name(),
                item: self.index,
                element,
                value: value.written(),
            });
        }
        let held = (self.item.body.iter_mut())
            .flat_map(|body| &mut body.nodes)
            .find_map(|node| match node {
                Node::Element(child) if child.name == element => Some(child),
                _ => None,
            });
        match held {
            Some(child) => {
                let stated = child.attribute(VALUE).and_then(T::stated);
                if !stated.is_some_and(|stated| stated.same(value)) {
                    child.set(VALUE, value.written());
                }
            }
            // A quad no document held states every member it has.
            None if self.fresh || !absent.is_some_and(|absent| absent.same(value)) => {
                self.add(Element::leaf(element, value.written()))
            }
            None => {}
        }
        Ok(())
    }

    /// State the four corners; an item that states none keeps none while all four are zero.
    fn put_corners(&mut self, corners: [i16; 4]) -> Result<(), WaterError> {
        // One corner written alone would leave an item the reader refuses.
        let absent = (corners == [0; 4]).then_some(0);
        for (element, corner) in CORNERS.into_iter().zip(corners) {
            self.put(element, corner, absent)?;
        }
        Ok(())
    }

    /// State `value` under `element`, or leave the item without any such element for `None`.
    fn put_stated(&mut self, element: &'static str, value: Option<f32>) -> Result<(), WaterError> {
        if let Some(value) = value {
            return self.put(element, value, None);
        }
        let Some(body) = &mut self.item.body else {
            return Ok(());
        };
        // An element stated twice goes whole: the one left would be read as the value.
        let named = |node: &Node| matches!(node, Node::Element(child) if child.name == element);
        while let Some(at) = body.nodes.iter().position(named) {
            body.nodes.remove(at);
            if at > 0 && body.nodes[at - 1].is_space() {
                body.nodes.remove(at - 1);
            }
        }
        Ok(())
    }

    /// Add `child` after the item's last element, behind the white space that leads that one.
    fn add(&mut self, child: Element) {
        let body = self.item.body.get_or_insert_with(Body::default);
        let last = (body.nodes.iter()).rposition(|node| matches!(node, Node::Element(_)));
        let lead = match last {
            Some(last) if last > 0 && body.nodes[last - 1].is_space() => {
                Some(body.nodes[last - 1].clone())
            }
            Some(_) => None,
            None => (self.fresh).then(|| Node::Other(self.layout.member.clone())),
        };
        let at = last.map_or(0, |last| last + 1);
        body.nodes
            .splice(at..at, lead.into_iter().chain([Node::Element(child)]));
    }
}

/// A quad of one of the root's lists.
trait Quad: Sized {
    const LIST: List;
    fn read(stated: &Stated, carry: Carry) -> Result<Self, WaterError>;
    fn write(&self, to: &mut Writing<'_>) -> Result<(), WaterError>;
    fn carry(&self) -> &Carry;
}

impl Quad for CWaterQuad {
    const LIST: List = List::Water;

    fn read(stated: &Stated, carry: Carry) -> Result<Self, WaterError> {
        let [min_x, max_x, min_y, max_y] = stated.corners()?;
        let default = Self::default();
        Ok(Self {
            min_x,
            max_x,
            min_y,
            max_y,
            r#type: stated.read("Type")?.unwrap_or(default.r#type),
            is_invisible: (stated.read("IsInvisible")?).unwrap_or(default.is_invisible),
            has_limited_depth: (stated.read("HasLimitedDepth")?)
                .unwrap_or(default.has_limited_depth),
            no_stencil: stated.read("NoStencil")?.unwrap_or(default.no_stencil),
            z: stated.read("z")?.unwrap_or(default.z),
            a1: stated.read("a1")?.unwrap_or(default.a1),
            a2: stated.read("a2")?.unwrap_or(default.a2),
            a3: stated.read("a3")?.unwrap_or(default.a3),
            a4: stated.read("a4")?.unwrap_or(default.a4),
            carry,
        })
    }

    fn write(&self, to: &mut Writing<'_>) -> Result<(), WaterError> {
        let default = Self::default();
        to.put_corners(self.rectangle_stated())?;
        to.put("Type", self.r#type, Some(default.r#type))?;
        to.put("IsInvisible", self.is_invisible, Some(default.is_invisible))?;
        to.put(
            "HasLimitedDepth",
            self.has_limited_depth,
            Some(default.has_limited_depth),
        )?;
        to.put("z", self.z, Some(default.z))?;
        to.put("a1", self.a1, Some(default.a1))?;
        to.put("a2", self.a2, Some(default.a2))?;
        to.put("a3", self.a3, Some(default.a3))?;
        to.put("a4", self.a4, Some(default.a4))?;
        to.put("NoStencil", self.no_stencil, Some(default.no_stencil))
    }

    fn carry(&self) -> &Carry {
        &self.carry
    }
}

impl Quad for CCalmingQuad {
    const LIST: List = List::Calming;

    fn read(stated: &Stated, carry: Carry) -> Result<Self, WaterError> {
        let [min_x, max_x, min_y, max_y] = stated.corners()?;
        Ok(Self {
            min_x,
            max_x,
            min_y,
            max_y,
            dampening: stated.read("fDampening")?,
            carry,
        })
    }

    fn write(&self, to: &mut Writing<'_>) -> Result<(), WaterError> {
        to.put_corners([self.min_x, self.max_x, self.min_y, self.max_y])?;
        to.put_stated("fDampening", self.dampening)
    }

    fn carry(&self) -> &Carry {
        &self.carry
    }
}

impl Quad for CWaveQuad {
    const LIST: List = List::Wave;

    fn read(stated: &Stated, carry: Carry) -> Result<Self, WaterError> {
        let [min_x, max_x, min_y, max_y] = stated.corners()?;
        Ok(Self {
            min_x,
            max_x,
            min_y,
            max_y,
            amplitude: stated.read("Amplitude")?,
            x_direction: stated.read("XDirection")?,
            y_direction: stated.read("YDirection")?,
            carry,
        })
    }

    fn write(&self, to: &mut Writing<'_>) -> Result<(), WaterError> {
        to.put_corners([self.min_x, self.max_x, self.min_y, self.max_y])?;
        to.put_stated("Amplitude", self.amplitude)?;
        to.put_stated("XDirection", self.x_direction)?;
        to.put_stated("YDirection", self.y_direction)
    }

    fn carry(&self) -> &Carry {
        &self.carry
    }
}

/// Read the items of the list `element` onto `quads`, and give the list without them.
fn read_list<Q: Quad>(element: &Element, quads: &mut Vec<Q>) -> Result<ListCarry, WaterError> {
    let mut lead = Vec::new();
    for node in element.body.iter().flat_map(|body| &body.nodes) {
        match node {
            Node::Element(item) if item.name == ITEM => {
                let stated = Stated {
                    list: Q::LIST,
                    index: quads.len(),
                    item,
                };
                let carry = Carry {
                    lead: std::mem::take(&mut lead),
                    item: Some(item.clone()),
                };
                quads.push(Q::read(&stated, carry)?);
            }
            other => lead.push(other.clone()),
        }
    }
    Ok(ListCarry {
        shell: Shell {
            attributes: element.attributes.clone(),
            tail: element.tail.clone(),
            close_tail: element.body.as_ref().map(|body| body.close_tail.clone()),
        },
        trail: lead,
    })
}

/// The list of `quads` as an element, or `None` for an empty list the document did not hold.
fn list_element<Q: Quad>(
    quads: &[Q],
    carry: Option<&ListCarry>,
    layout: &Layout,
) -> Result<Option<Element>, WaterError> {
    if carry.is_none() && quads.is_empty() {
        return Ok(None);
    }
    let mut nodes = Vec::new();
    for (index, quad) in quads.iter().enumerate() {
        let carried = quad.carry();
        let mut to = Writing {
            list: Q::LIST,
            index,
            item: carried.item.clone().unwrap_or_else(|| Element {
                name: ITEM.to_string(),
                body: Some(Body::default()),
                ..Element::default()
            }),
            fresh: carried.item.is_none(),
            layout,
        };
        quad.write(&mut to)?;
        if to.fresh {
            nodes.push(Node::Other(layout.item.clone()));
            let body = to.item.body.get_or_insert_with(Body::default);
            body.nodes.push(Node::Other(layout.item.clone()));
        } else {
            nodes.extend(carried.lead.iter().cloned());
        }
        nodes.push(Node::Element(to.item));
    }
    let open = Shell::open();
    let shell = carry.map_or(&open, |carry| &carry.shell);
    // A list that had no closing tag has no white space before one, and neither has one that closed
    // right behind its last item.
    let fresh_last = (quads.last()).is_some_and(|quad| quad.carry().item.is_none());
    match carry.filter(|carry| carry.shell.close_tail.is_some()) {
        Some(carry) if carry.trail.is_empty() && fresh_last => {
            nodes.push(Node::Other(layout.list.clone()))
        }
        Some(carry) => nodes.extend(carry.trail.iter().cloned()),
        None if nodes.is_empty() => {}
        None => nodes.push(Node::Other(layout.list.clone())),
    }
    Ok(Some(shell.around(Q::LIST.name(), nodes)))
}

#[cfg(test)]
mod tests;
