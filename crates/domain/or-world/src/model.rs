//! What a title's world is to the shared code: the rows of a linked index, what a map body states
//! beside its nodes, the instances a selection lays out, and a model and a texture as plain data.
//!
//! ⚠ A read model: nothing here is written back. An edit changes a title's own records on the
//! title's own file, and what is here is read again from them.

use std::any::Any;
use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::sync::Arc;

use glam::Vec3;

use crate::streaming::{ListDraw, MapBox, NodeDraw, NodeId};

/// A map by its place among the rows of a linked [`Index`]; made by a link, never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MapId(pub u32);

/// The bit of a forest key that says it names an interior's nodes, not a map's.
const INTERIOR: u64 = 1 << 63;
/// The most nodes a map holds for one of them to be an interior's instance a forest can key.
pub const MOST_KEYED: u32 = 1 << 31;

impl MapId {
    /// The key its nodes stand under in the forest, which a child's parent link names it by.
    pub fn key(self) -> u64 {
        u64::from(self.0)
    }

    /// The map a forest key names, where it names a map and not an interior.
    pub fn of_key(key: u64) -> Option<Self> {
        u32::try_from(key).ok().map(Self)
    }
}

/// The forest key of the nodes of the interior that node `node` of `map` places.
// ⚠ A node at or past `MOST_KEYED` shares the key of the node that many before it: the driver
// refuses such an instance before it asks for this.
pub fn interior_key(map: MapId, node: u32) -> u64 {
    INTERIOR | u64::from(map.0) << 31 | u64::from(node % MOST_KEYED)
}

/// Whether a forest key names an interior's nodes.
pub fn is_interior_key(key: u64) -> bool {
    key & INTERIOR != 0
}

/// A title's encoding of what draws an archetype.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetKey(pub u64);

/// One texture asked for: a name inside a scope the title reads in one piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextureAsk {
    /// What holds it, as the title encodes that: a dictionary, a pack, or the model itself.
    pub scope: AssetKey,
    pub name: u64,
    /// The longest side wanted, in pixels: a level above it need not be read.
    pub pixels: u32,
}

/// A switch a person or a script turns while a world is open, by the title's number of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SwitchId(pub u32);

/// Every hour of the day, as the bits of [`Gate::When`].
pub const EVERY_HOUR: u32 = (1 << 24) - 1;

/// What lets a map stream where its box is wanted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Gate {
    /// Nothing holds it back.
    #[default]
    Always,
    /// Only a request by name does, or its switch where it has one.
    Held { switch: Option<SwitchId> },
    /// The clock and the weather: during each hour whose bit is set, under one of the weathers named or under any where none is.
    When { hours: u32, weathers: Vec<u64> },
}

/// The moment a world is looked at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moment {
    /// The hour of the day, from 0 and below 24.
    pub hour: f32,
    /// The weather, by the title's name of it.
    pub weather: u64,
}

impl Moment {
    /// Noon under the weather `weather`.
    pub fn noon(weather: u64) -> Self {
        Self {
            hour: 12.0,
            weather,
        }
    }

    /// The hour of the day it falls in, 0 to 23.
    pub fn hour_of_day(&self) -> u32 {
        (self.hour.rem_euclid(24.0) as u32).min(23)
    }
}

/// What is switched on and asked for by name while a world is open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Switched {
    pub on: BTreeSet<SwitchId>,
    /// The maps a script asked for by name.
    pub requested: BTreeSet<MapId>,
}

impl Gate {
    /// Whether it lets the map `map` in at `at` under `switched`.
    pub fn admits(&self, map: MapId, at: &Moment, switched: &Switched) -> bool {
        if switched.requested.contains(&map) {
            return true;
        }
        match self {
            Gate::Always => true,
            Gate::Held { switch } => switch.is_some_and(|switch| switched.on.contains(&switch)),
            Gate::When { hours, weathers } => {
                hours & (1 << at.hour_of_day()) != 0
                    && (weathers.is_empty() || weathers.contains(&at.weather))
            }
        }
    }
}

/// One map of a linked index.
#[derive(Debug, Clone, PartialEq)]
pub struct MapRow {
    /// The title's own name of it, which no other row of the index bears.
    pub name: u64,
    /// What a person reads it as.
    pub label: String,
    /// The boxes it streams by; `None` where it states none and streams by its parents' children alone.
    pub boxes: Option<MapBox>,
    /// The maps wanted with it, whatever switches them.
    pub parents: Vec<MapId>,
    pub gate: Gate,
    /// The archive that holds it, by its place among the install's archives.
    pub archive: u32,
}

/// Why rows are no index.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IndexFault {
    #[error("two maps bear the name {0:#x}")]
    NameTwice(u64),
    #[error("the map {map:#x} names a parent past the last row")]
    ParentPastTheLast { map: u64 },
    #[error("the map {0:#x} names itself as a parent")]
    OwnParent(u64),
    #[error("an index holds more maps than a map id counts")]
    TooManyMaps,
}

/// Every map of an opened world, linked: a parent is a row, and a name finds its row.
#[derive(Debug, Clone, Default)]
pub struct Index {
    maps: Vec<MapRow>,
    by_name: HashMap<u64, MapId>,
    /// What the title's own link made beside the rows, which its readers find a row's file by.
    title: Option<Arc<dyn Any + Send + Sync>>,
}

impl Index {
    /// The index of `maps`, refused where a name is borne twice or a parent is no row.
    pub fn new(maps: Vec<MapRow>) -> Result<Self, IndexFault> {
        let count = u32::try_from(maps.len()).map_err(|_| IndexFault::TooManyMaps)?;
        let mut by_name = HashMap::with_capacity(maps.len());
        for (at, row) in (0..count).zip(&maps) {
            if by_name.insert(row.name, MapId(at)).is_some() {
                return Err(IndexFault::NameTwice(row.name));
            }
            for parent in &row.parents {
                if parent.0 >= count {
                    return Err(IndexFault::ParentPastTheLast { map: row.name });
                }
                if parent.0 == at {
                    return Err(IndexFault::OwnParent(row.name));
                }
            }
        }
        Ok(Self {
            maps,
            by_name,
            title: None,
        })
    }

    /// The index, carrying what the title's link made beside the rows.
    pub fn with_title(mut self, title: Arc<dyn Any + Send + Sync>) -> Self {
        self.title = Some(title);
        self
    }

    /// What the title's link made beside the rows, where it is of the kind `T`.
    pub fn title<T: Any>(&self) -> Option<&T> {
        self.title.as_ref()?.downcast_ref()
    }

    pub fn maps(&self) -> &[MapRow] {
        &self.maps
    }

    /// The row `id` names; `None` for an id of another index.
    pub fn map(&self, id: MapId) -> Option<&MapRow> {
        self.maps.get(id.0 as usize)
    }

    /// The map a title names `name`.
    pub fn named(&self, name: u64) -> Option<MapId> {
        self.by_name.get(&name).copied()
    }
}

/// What a light is shaped as.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LightKind {
    Point,
    /// A cone about its direction: the half angles it is whole within and gone past, in radians.
    Spot {
        inner: f32,
        outer: f32,
    },
    /// A segment of this length along its direction, in metres.
    Capsule {
        length: f32,
    },
}

/// A light a map body states.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    pub position: Vec3,
    /// A unit vector; unread for a point.
    pub direction: Vec3,
    /// Linear red, green and blue, before its intensity.
    pub colour: [f32; 3],
    pub intensity: f32,
    /// How far it lights, in metres.
    pub range: f32,
    /// How far from the eye it is whole, and how far gone, in metres.
    pub fade: (f32, f32),
    /// The hours it is lit in, a bit an hour.
    pub hours: u32,
}

/// A box that hides what stands behind it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OccluderBox {
    /// From the unit cube about the origin to the world, as three rows.
    pub rows: [[f32; 4]; 3],
}

/// A mesh that hides what stands behind it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OccluderMesh {
    pub vertices: Vec<Vec3>,
    /// Three a triangle.
    pub indices: Vec<u32>,
}

/// The occluders a map body states.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Occluders {
    pub boxes: Vec<OccluderBox>,
    pub meshes: Vec<OccluderMesh>,
}

/// An interior a map body places: its nodes are a body of their own, brought in within its instance's distance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteriorPlaced {
    /// Its instance, by its place among the body's nodes.
    pub node: u32,
    /// The title's name of what declares it.
    pub interior: u64,
}

/// What draws a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Draws {
    /// A model, waited for until it is ready.
    Model(AssetKey),
    /// It draws with nothing to wait for: a box where nothing says what draws it, or a list its body lays out.
    StandIn,
    /// It has no model and never will: it draws nothing and holds nothing back.
    Nothing,
}

/// The nodes of one body a selection draws.
#[derive(Debug, Clone, Copy)]
pub struct Drawn<'a> {
    /// The forest key the body stands under.
    pub key: u64,
    /// Its entities that draw, by their place.
    pub nodes: &'a [NodeDraw],
    /// Its lists that draw, by their place.
    pub lists: &'a [ListDraw],
    /// The instances left out of the lists: what the range of a list indexes.
    pub skipped: &'a [u32],
}

/// The instances of a selection as columns, one entry an instance, and the models they draw with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Instances {
    /// How many words of its own a title gives each instance.
    width: usize,
    /// Each model drawn with, once, in the order first drawn with.
    pub models: Vec<AssetKey>,
    places: HashMap<AssetKey, u32>,
    pub node: Vec<NodeId>,
    /// Which of [`Self::models`] draws it.
    pub model: Vec<u32>,
    /// From the model's own space to the world, as three rows.
    pub rows: Vec<[[f32; 4]; 3]>,
    /// Its sphere in the world: the centre, then the radius.
    pub sphere: Vec<[f32; 4]>,
    /// How opaque, to 1 either side of nought: below nought it fades out for its children.
    pub fade: Vec<f32>,
    /// The title's own words, a block of [`Self::width`] an instance.
    pub words: Vec<u32>,
}

/// One instance, as a body lays it out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instance<'a> {
    pub node: NodeId,
    pub model: AssetKey,
    pub rows: [[f32; 4]; 3],
    pub sphere: [f32; 4],
    pub fade: f32,
    pub words: &'a [u32],
}

impl Instances {
    /// No instance yet, each to carry `width` words of the title's own.
    pub fn of_width(width: usize) -> Self {
        Self {
            width,
            ..Self::default()
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn len(&self) -> usize {
        self.node.len()
    }

    pub fn is_empty(&self) -> bool {
        self.node.is_empty()
    }

    /// Add one instance.
    // ⚠ Its words are taken as given: a block of another width shifts every block after it, which
    // the conformance kit refuses by rule.
    pub fn push(&mut self, instance: Instance<'_>) {
        let next = self.models.len() as u32;
        let model = *self.places.entry(instance.model).or_insert_with(|| {
            self.models.push(instance.model);
            next
        });
        self.node.push(instance.node);
        self.model.push(model);
        self.rows.push(instance.rows);
        self.sphere.push(instance.sphere);
        self.fade.push(instance.fade);
        self.words.extend_from_slice(instance.words);
    }

    /// The model that draws instance `at`.
    pub fn model_of(&self, at: usize) -> Option<AssetKey> {
        self.models.get(*self.model.get(at)? as usize).copied()
    }

    /// Where instance `at` stands: the translation of its rows.
    pub fn position(&self, at: usize) -> Option<Vec3> {
        let rows = self.rows.get(at)?;
        Some(Vec3::new(rows[0][3], rows[1][3], rows[2][3]))
    }
}

/// An instance's fade as its column holds it: below nought where it gives way to its children.
pub fn signed_fade(fade: f32, yields: bool) -> f32 {
    if yields {
        -fade
    } else {
        fade
    }
}

/// The rows of a node that stands at `at`, unturned and unscaled.
pub fn rows_at(at: Vec3) -> [[f32; 4]; 3] {
    [
        [1.0, 0.0, 0.0, at.x],
        [0.0, 1.0, 0.0, at.y],
        [0.0, 0.0, 1.0, at.z],
    ]
}

/// What a vertex element means to a shader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Semantic {
    Position,
    Normal,
    Tangent,
    Colour(u8),
    TexCoord(u8),
    BlendWeights,
    BlendIndices,
}

/// How a vertex element is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElementFormat {
    F32x2,
    F32x3,
    F32x4,
    U8x4Norm,
    U8x4,
    F16x2,
    F16x4,
}

impl ElementFormat {
    /// Its bytes.
    pub fn size(self) -> u32 {
        match self {
            Self::F32x2 => 8,
            Self::F32x3 => 12,
            Self::F32x4 => 16,
            Self::U8x4Norm | Self::U8x4 | Self::F16x2 => 4,
            Self::F16x4 => 8,
        }
    }
}

/// One element of a vertex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Element {
    pub semantic: Semantic,
    pub format: ElementFormat,
    /// Bytes from the start of the vertex.
    pub offset: u32,
}

/// How the game's own vertex bytes are laid out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct VertexLayout {
    /// Bytes a vertex.
    pub stride: u32,
    pub elements: Vec<Element>,
}

/// How a surface's alpha is used.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Alpha {
    #[default]
    Opaque,
    /// Drawn where its alpha is above the threshold.
    Cutout(f32),
    Blend,
}

/// What the plain pass draws a mesh with where no program of the game does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surface {
    pub colour: Option<TextureAsk>,
    pub normal: Option<TextureAsk>,
    pub alpha: Alpha,
    /// Linear red, green, blue and alpha its colour is multiplied by.
    pub tint: [f32; 4],
}

impl Default for Surface {
    fn default() -> Self {
        Self {
            colour: None,
            normal: None,
            alpha: Alpha::Opaque,
            tint: [1.0; 4],
        }
    }
}

/// One mesh of a model: the game's vertex bytes with their layout, and its surface.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub layout: VertexLayout,
    pub vertices: Vec<u8>,
    /// Three a triangle.
    pub indices: Vec<u32>,
    pub surface: Surface,
}

impl Mesh {
    /// How many whole vertices its bytes hold.
    pub fn vertex_count(&self) -> usize {
        match self.layout.stride {
            0 => 0,
            stride => self.vertices.len() / stride as usize,
        }
    }
}

/// One level of detail of a model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lod {
    /// How far it draws, in metres.
    pub distance: f32,
    pub meshes: Vec<Mesh>,
}

/// A model as plain data, the most detailed level first.
#[derive(Clone, Default)]
pub struct Model {
    pub lods: Vec<Lod>,
    /// The title's own record of its materials, which only the title's module of the renderer reads.
    pub title: Option<Arc<dyn Any + Send + Sync>>,
}

/// How a texture's levels are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    Rgba8,
    /// One byte a pixel: its coverage, with no colour.
    A8,
    Bc1,
    /// A colour as BC1 holds it, and four bits of coverage a pixel.
    Bc2,
    Bc3,
    Bc4,
    Bc5,
    Bc7,
}

impl TextureFormat {
    /// The bytes of a level `width` by `height` pixels; the most a count holds where they are more.
    pub fn level_bytes(self, width: u32, height: u32) -> usize {
        let (wide, high, each) = match self {
            Self::Rgba8 => (width, height, 4),
            Self::A8 => (width, height, 1),
            Self::Bc1 | Self::Bc4 => (width.div_ceil(4), height.div_ceil(4), 8),
            Self::Bc2 | Self::Bc3 | Self::Bc5 | Self::Bc7 => {
                (width.div_ceil(4), height.div_ceil(4), 16)
            }
        };
        (u128::from(wide) * u128::from(high) * each)
            .try_into()
            .unwrap_or(usize::MAX)
    }
}

/// A texture as plain data, the largest level first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    pub srgb: bool,
    pub levels: Vec<Vec<u8>>,
}

/// What the moment alone decides of a frame, as the host and the plain pass need it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameFacts {
    /// A unit vector towards the sun.
    pub sun: Vec3,
    /// Linear red, green and blue.
    pub key: [f32; 3],
    pub ambient: [f32; 3],
    pub exposure: f32,
    pub clear: [f32; 3],
}

/// A corner of a level rectangle, by the compass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Corner {
    SouthWest,
    NorthWest,
    NorthEast,
    SouthEast,
}

/// A level sheet of water: a rectangle at a height, or the half of one that keeps a corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterQuad {
    /// Its west and south edges, in metres.
    pub min: [f32; 2],
    /// Its east and north edges.
    pub max: [f32; 2],
    /// The height its surface stands at.
    pub height: f32,
    /// The corner the half that is water keeps; `None` where the whole rectangle is.
    pub half: Option<Corner>,
}

impl WaterQuad {
    /// Its surface as triangles, each wound counter-clockwise as seen from above.
    pub fn triangles(&self) -> Vec<[Vec3; 3]> {
        let at = |x: f32, y: f32| Vec3::new(x, y, self.height);
        let (sw, se) = (at(self.min[0], self.min[1]), at(self.max[0], self.min[1]));
        let (ne, nw) = (at(self.max[0], self.max[1]), at(self.min[0], self.max[1]));
        match self.half {
            None => vec![[sw, se, ne], [sw, ne, nw]],
            Some(Corner::SouthWest) => vec![[sw, se, nw]],
            Some(Corner::NorthWest) => vec![[nw, sw, ne]],
            Some(Corner::NorthEast) => vec![[ne, nw, se]],
            Some(Corner::SouthEast) => vec![[se, ne, sw]],
        }
    }
}

/// The name a value of a title's day reaches its programs by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeedName<'a> {
    /// The name a program's reflection gives the constant.
    Name(&'a str),
    /// A binding and the hash of a name, for programs that carry no names.
    Hash { binding: u32, name: u64 },
}

/// What takes the values of a title's day, each by name.
pub trait Feed {
    fn set(&mut self, name: FeedName<'_>, value: &[f32]);
}

/// One thing a map places, as a search and a list of placements read it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacementRow {
    /// Its place among the map's nodes.
    pub node: u32,
    /// The title's name of its archetype.
    pub archetype: u64,
    pub position: Vec3,
}

/// The skipped instances of the list `list` of `drawn`.
pub fn skipped_of<'a>(drawn: &Drawn<'a>, list: &ListDraw) -> &'a [u32] {
    let Range { start, end } = list.skipped.clone();
    drawn.skipped.get(start..end).unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: u64, parents: &[u32]) -> MapRow {
        MapRow {
            name,
            label: format!("{name}"),
            boxes: None,
            parents: parents.iter().copied().map(MapId).collect(),
            gate: Gate::Always,
            archive: 0,
        }
    }

    #[test]
    fn rows_that_are_no_index_are_refused_with_why() {
        let index = Index::new(vec![row(7, &[1]), row(9, &[])]).expect("two rows that link");
        assert_eq!((index.named(9), index.named(8)), (Some(MapId(1)), None));
        assert_eq!(
            Index::new(vec![row(7, &[]), row(7, &[])]).err(),
            Some(IndexFault::NameTwice(7))
        );
        assert_eq!(
            Index::new(vec![row(7, &[2]), row(9, &[])]).err(),
            Some(IndexFault::ParentPastTheLast { map: 7 })
        );
        assert_eq!(
            Index::new(vec![row(7, &[]), row(9, &[1])]).err(),
            Some(IndexFault::OwnParent(9))
        );
    }

    #[test]
    fn a_gate_by_the_weather_lets_a_map_in_under_the_weathers_it_names_alone() {
        let (map, none) = (MapId(0), Switched::default());
        let at = |hour: f32, weather: u64| Moment { hour, weather };
        let rain = Gate::When {
            hours: EVERY_HOUR,
            weathers: vec![3, 5],
        };
        assert!(rain.admits(map, &at(12.0, 3), &none) && rain.admits(map, &at(12.0, 5), &none));
        assert!(!rain.admits(map, &at(12.0, 4), &none));
        let evening = Gate::When {
            hours: 1 << 23,
            weathers: Vec::new(),
        };
        // Under any weather where it names none, and an hour past the day's end is the day's next.
        assert!(
            evening.admits(map, &at(23.5, 4), &none) && evening.admits(map, &at(47.9, 9), &none)
        );
        assert!(
            !evening.admits(map, &at(22.99, 4), &none) && !evening.admits(map, &at(24.0, 4), &none)
        );
        let mut asked = Switched::default();
        asked.requested.insert(map);
        assert!(
            rain.admits(map, &at(12.0, 4), &asked),
            "a request by name passes any gate"
        );
    }

    #[test]
    fn an_interior_key_is_no_map_key_and_no_two_interiors_share_one() {
        let keys = [
            interior_key(MapId(0), 0),
            interior_key(MapId(0), 1),
            interior_key(MapId(1), 0),
            interior_key(MapId(1 << 31), 0),
            interior_key(MapId(u32::MAX), MOST_KEYED - 1),
            interior_key(MapId(u32::MAX - 1), MOST_KEYED - 1),
            interior_key(MapId(u32::MAX), MOST_KEYED - 2),
        ];
        for (at, key) in keys.iter().enumerate() {
            assert!(is_interior_key(*key) && MapId::of_key(*key).is_none());
            assert!(!keys[at + 1..].contains(key), "{key:#x} is borne twice");
        }
        let map = MapId(u32::MAX);
        assert_eq!(MapId::of_key(map.key()), Some(map));
        assert!(!is_interior_key(map.key()));
    }

    #[test]
    fn a_level_no_memory_holds_is_of_more_bytes_than_any_level_read() {
        assert_eq!(TextureFormat::Rgba8.level_bytes(4, 2), 32);
        assert_eq!(TextureFormat::Bc1.level_bytes(5, 4), 16);
        assert_eq!(TextureFormat::Bc7.level_bytes(1, 1), 16);
        // A width and a height a file states and holds no pixels of must not wrap to a size it does hold.
        for format in [TextureFormat::Rgba8, TextureFormat::Bc7] {
            assert_eq!(format.level_bytes(u32::MAX, u32::MAX), usize::MAX);
        }
    }

    #[test]
    fn instances_list_each_model_once_and_each_instance_names_its_own() {
        let mut instances = Instances::of_width(1);
        let at = |x: f32| Vec3::new(x, 2.0, 3.0);
        for (node, model, x) in [(0, 7, 1.0), (1, 9, 4.0), (2, 7, 5.0)] {
            instances.push(Instance {
                node: (0, node),
                model: AssetKey(model),
                rows: rows_at(at(x)),
                sphere: [x, 2.0, 3.0, 1.0],
                fade: signed_fade(0.5, node == 1),
                words: &[node],
            });
        }
        assert_eq!(instances.models, [AssetKey(7), AssetKey(9)]);
        assert_eq!(instances.model, [0, 1, 0]);
        let models: Vec<_> = (0..4).map(|at| instances.model_of(at)).collect();
        assert_eq!(
            models,
            [
                Some(AssetKey(7)),
                Some(AssetKey(9)),
                Some(AssetKey(7)),
                None
            ]
        );
        assert_eq!(instances.position(2), Some(at(5.0)));
        assert_eq!(instances.fade, [0.5, -0.5, 0.5]);
        assert_eq!((instances.len(), &instances.words[..]), (3, &[0, 1, 2][..]));
    }

    #[test]
    fn a_list_is_handed_the_instances_left_out_of_it_and_no_other_lists() {
        let list = |skipped| ListDraw {
            node: (0, 0),
            fade: 1.0,
            yields: false,
            skipped,
        };
        let drawn = Drawn {
            key: 0,
            nodes: &[],
            lists: &[],
            skipped: &[4, 8, 15, 16],
        };
        assert_eq!(skipped_of(&drawn, &list(1..3)), [8, 15]);
        assert_eq!(skipped_of(&drawn, &list(3..3)), [0u32; 0]);
        // A range past what the selection left out names nothing, and is no panic.
        assert_eq!(skipped_of(&drawn, &list(3..9)), [0u32; 0]);
    }
}
