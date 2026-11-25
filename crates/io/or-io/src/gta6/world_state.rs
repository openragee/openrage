//! GTA VI's world-state selectors: the bodies of a game-shape archive whose root holds placed
//! nodes, each node's selectors choosing a configuration that switches script-managed map groups
//! and game-shape layers on and off, and the rules that pick a configuration.
//!
//! A selector takes the configuration of the highest request on it, the first made of equal ones,
//! and its default with none; a rule set requests the configuration of its first rule that holds.
//! Scripts and scenes request too, which the data does not state: [`Assumed`] carries such
//! requests, and the flags set, where a caller supplies them.

use std::collections::{HashMap, HashSet};

use crate::hash::literal_string_hash as lit;
use crate::meta::binary::hash_index::default_hash_index;
use crate::meta::binary::pso::table_form;
use crate::meta::binary::rbm::schema::RbmModel;
use crate::meta::{MetaInstance, Value};

/// The root class of a world-state body.
pub const WORLD_STATE: u32 = 0xD52B_2D2F;
/// The root class of a game-shape archive's layer manifest.
pub const LAYER_MANIFEST: u32 = 0xB00F_FAFC;
/// A world-state body's layer: the hash of the layer's name.
const BODY_LAYER: u32 = 0x1957_9573;
const LAYERS: u32 = lit(b"Layers");
/// A layer's name: the hash a body states as its layer.
const LAYER_NAME: u32 = lit(b"Name");
/// Set on a layer the level loads; clear on one a script or a system loads when it asks.
const LAYER_LOADED: u32 = 0xB012_DEBA;
const NODES: u32 = lit(b"Nodes");
const COMPONENTS: u32 = lit(b"Components");
const SELECTOR_COMPONENT: u32 = 0x13D6_866D;
const SELECTORS: u32 = lit(b"Selectors");
const DEFAULT_CONFIG: u32 = lit(b"DefaultConfig");
const CONFIGS: u32 = lit(b"Configs");
const CHANGES: u32 = lit(b"Changes");
/// The on, off and state-2 lists of the two change classes: words, and pairs of words.
const WORD_LISTS: [u32; 3] = [0x2699_5CF4, 0xC8BD_754A, 0x07DD_7D6D];
const PAIR_LISTS: [u32; 3] = [0x8105_B9F3, 0x67F6_A736, 0x666D_D869];
/// The group word of a pair of words.
const PAIR_WORD: u32 = 0xE255_4B92;
/// The change that loads game-shape layers: its `Enabled` list, each an archive and a layer name.
const LAYER_CHANGE: u32 = 0x84DD_2A2B;
const ENABLED: u32 = lit(b"Enabled");
const LAYER_ARCHIVE: u32 = 0x1227_E5BA;
const LAYER_OF_CHANGE: u32 = 0xF549_71C9;
const RULE_COMPONENT: u32 = 0x8740_CA71;
const RULE_SETS: u32 = 0xA646_5A62;
const RULE_SELECTOR: u32 = 0xAABB_E900;
const RULES: u32 = lit(b"Rules");
const RULE_CONFIG: u32 = lit(b"Config");
const CONDITION: u32 = lit(b"Condition");
const CONDITIONS: u32 = lit(b"Conditions");
const TIME_OF_DAY: u32 = lit(b"CAIConditionTimeOfDay");
const TIME_WITH_MINUTES: u32 = lit(b"CAIConditionTimeOfDayWithMinutes");
const NOT: u32 = lit(b"CAIConditionNot");
const AND: u32 = lit(b"CAIConditionAnd");
const OR: u32 = lit(b"CAIConditionOr");
const TRUE: u32 = lit(b"CAIConditionTrue");
const RAINING: u32 = lit(b"CAIConditionRaining");
const RAIN_AMOUNT: u32 = lit(b"RainAmount");
const PED_WANTED: u32 = lit(b"CAIConditionPedWanted");
const PED_WANTED_LEVEL: u32 = lit(b"CAIConditionPedWantedLevel");
const LOWER_BOUND: u32 = lit(b"LowerBound");
const UPPER_BOUND: u32 = lit(b"UpperBound");
const ITEM_UNLOCKED: u32 = lit(b"CAIConditionIsItemUnlocked");
const HAS_ITEM: u32 = lit(b"CAIConditionHasInventoryItem");
const MIN_ITEM_COUNT: u32 = lit(b"MinItemCount");
const RANDOM: u32 = lit(b"CAIConditionRandom");
const CHANCES: u32 = lit(b"Chances");
/// A selector in a configuration, for a span of real seconds and one of game hours since it took it.
const IN_CONFIG: u32 = 0x507E_EB10;
const IN_CONFIG_SELECTOR: u32 = 0xAABB_E900;
const MIN_SECONDS: u32 = lit(b"Min");
const MIN_GAME_HOURS: u32 = 0x11D8_B2D9;
/// The priority a rule set requests its configuration with.
const REQUEST_PRIORITY: u32 = lit(b"RequestPriority");
/// A flag of the game's flag set is set: scripts, a save and volumes near the player set them.
const FLAG_SET: u32 = 0x666A_479D;
const FLAG: u32 = 0x9645_61C1;
/// Whether anyone owns a property: none does in a new game.
const PROPERTY_OWNED: u32 = 0x0EB4_8027;
const ANY_OWNER: u32 = lit(b"Any");
/// A node's location name: the hash of its label and its kind.
const LOCATION_NAME: u32 = 0xB9A1_0BAF;
const LOCATION_LABEL: u32 = lit(b"LocName");
const LOCATION_KIND: u32 = lit(b"Type");
/// A node's shape: an outline on the ground.
const SHAPE: u32 = 0xAB0B_13D9;
const VERTS: u32 = lit(b"Verts");
const NODE_NAME: u32 = lit(b"Name");
const START_HOUR: u32 = lit(b"startHour");
const START_MINUTE: u32 = lit(b"startMinute");
const END_HOUR: u32 = lit(b"endHour");
const END_MINUTE: u32 = lit(b"endMinute");

/// Why a body is not a world-state body.
#[derive(Debug, thiserror::Error)]
pub enum WorldStateError {
    #[error(transparent)]
    Meta(#[from] crate::meta::MetaError),
    #[error("the root structure {0:#010X} is not a world-state body (hash_D52B2D2F)")]
    NotWorldState(u32),
    #[error("the root structure {0:#010X} is not a layer manifest (hash_B00FFAFC)")]
    NotLayerManifest(u32),
    #[error("the world-state body is broken: {0}")]
    Broken(&'static str),
}

/// The selectors of one world-state body, every node's in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldState {
    /// The hash of the name of the layer that holds the body.
    pub layer: u32,
    pub selectors: Vec<Selector>,
    /// The places its nodes name with an outline: regions, islands and districts.
    pub places: Vec<Place>,
}

/// A place a node names: the hash of its name and of its label (0 for none), its kind, and its outline on the ground.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Place {
    pub name: u32,
    pub label: u32,
    pub kind: i32,
    pub outline: Vec<[f32; 2]>,
}

impl Place {
    /// Whether `point` lies inside the outline.
    pub fn holds(&self, point: [f32; 2]) -> bool {
        outline_holds(&self.outline, point)
    }

    /// The outline's area in square metres.
    pub fn area(&self) -> f32 {
        outline_area(&self.outline)
    }
}

/// Whether `point` lies inside `outline`, by the edges a line to the east of it crosses.
pub fn outline_holds(outline: &[[f32; 2]], [x, y]: [f32; 2]) -> bool {
    let mut inside = false;
    let n = outline.len();
    for k in 0..n {
        let ([ax, ay], [bx, by]) = (outline[k], outline[(k + 1) % n]);
        if (ay > y) != (by > y) && x < ax + (y - ay) * (bx - ax) / (by - ay) {
            inside = !inside;
        }
    }
    inside
}

/// The area `outline` encloses, in square metres.
pub fn outline_area(outline: &[[f32; 2]]) -> f32 {
    let n = outline.len();
    let twice: f32 = (0..n)
        .map(|k| {
            let ([ax, ay], [bx, by]) = (outline[k], outline[(k + 1) % n]);
            ax * by - bx * ay
        })
        .sum();
    twice.abs() / 2.0
}

/// A selector: the configurations it chooses among, its default, and the rules that pick one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Selector {
    pub key: u32,
    /// The key of the configuration it takes when no rule holds; 0 for none.
    pub default: u32,
    pub configs: Vec<Config>,
    /// The rule sets of its node that request a configuration of it, in the order the body states them.
    pub rule_sets: Vec<RuleSet>,
}

/// A rule set: its rules in the order the body states them, and the priority it requests with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleSet {
    pub priority: i32,
    pub rules: Vec<Rule>,
}

/// A request the data does not state, made of the selector of key `selector` at the hours of the mask `hours` (bit `h` for the hour `h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub selector: u32,
    pub config: u32,
    pub priority: i32,
    pub hours: u32,
}

/// What is taken of a game beyond the data: requests scripts make, and the flags set; `None` where the flags are not known.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assumed {
    pub requests: Vec<Request>,
    pub flags: Option<HashSet<u32>>,
}

/// A configuration: the group words its changes switch on, off and to state 2, and the layers it loads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub key: u32,
    pub on: Vec<u64>,
    pub off: Vec<u64>,
    pub state2: Vec<u64>,
    /// The game-shape layers it requests: the hash of the archive's name and of the layer's.
    pub layers: Vec<(u32, u32)>,
}

/// A rule: the configuration it picks while its condition holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub config: u32,
    pub condition: Condition,
}

/// A condition of a rule.
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    /// From the start hour to the end hour, both inside; past midnight where the start is not earlier.
    TimeOfDay {
        start: i32,
        end: i32,
    },
    /// The same to the minute: hour and minute of the start, then of the end.
    TimeWithMinutes {
        start: (i32, i32),
        end: (i32, i32),
    },
    Not(Box<Condition>),
    And(Vec<Condition>),
    Or(Vec<Condition>),
    True,
    Raining(f32),
    /// A condition on the player that a new game settles: no wanted level, nothing unlocked or held.
    NewGame(bool),
    /// True with the chance it states, each time it is asked.
    Random(f32),
    /// The selector of key `selector` is in the configuration `config`; `from_start` where it holds
    /// from the moment it took it, with no least span of time.
    InConfig {
        selector: u32,
        config: u32,
        from_start: bool,
    },
    /// The flag of this hash is in the game's flag set.
    Flag(u32),
    /// A class this does not evaluate, by the name the value tree gives it.
    Other(String),
}

impl Condition {
    /// Whether it holds at the whole hour `hour` of clear weather, in a new game where no selector has a configuration; `None` where nothing here says.
    pub fn holds(&self, hour: u32) -> Option<bool> {
        self.holds_in(hour, &|_, _| false)
    }

    /// The same where `in_config(selector, config)` says whether a selector is in a configuration.
    pub fn holds_in(&self, hour: u32, in_config: &dyn Fn(u32, u32) -> bool) -> Option<bool> {
        self.holds_with(hour, in_config, None)
    }

    /// The same where `flags`, where known, is the game's flag set.
    pub fn holds_with(
        &self,
        hour: u32,
        in_config: &dyn Fn(u32, u32) -> bool,
        flags: Option<&HashSet<u32>>,
    ) -> Option<bool> {
        let hour_i = hour as i32;
        let inner = |c: &Condition| c.holds_with(hour, in_config, flags);
        Some(match self {
            Self::TimeOfDay { start, end } => match start < end {
                true => *start <= hour_i && hour_i <= *end,
                false => hour_i <= *end || *start <= hour_i,
            },
            // The half hour stands for the whole one; which way round goes by the hours alone.
            // ⚠ The hours and minutes are a body's own numbers, which no clock bounds.
            Self::TimeWithMinutes { start, end } => {
                let minutes = |(h, m): (i32, i32)| i64::from(h) * 60 + i64::from(m);
                let now = i64::from(hour) * 60 + 30;
                let (from, to) = (minutes(*start), minutes(*end));
                match start.0 < end.0 {
                    true => from <= now && now <= to,
                    false => now <= to || from <= now,
                }
            }
            Self::Not(not) => !inner(not)?,
            Self::And(all) => {
                let held: Vec<_> = all.iter().map(inner).collect();
                if held.contains(&Some(false)) {
                    false
                } else {
                    held.iter().all(Option::is_some).then_some(true)?
                }
            }
            Self::Or(any) => {
                let held: Vec<_> = any.iter().map(inner).collect();
                if held.contains(&Some(true)) {
                    true
                } else {
                    held.iter().all(Option::is_some).then_some(false)?
                }
            }
            Self::True => true,
            Self::Raining(_) => false,
            Self::NewGame(held) => *held,
            Self::Random(chance) if *chance <= 0.0 => false,
            Self::Random(chance) if *chance >= 1.0 => true,
            Self::Random(_) => return None,
            Self::InConfig {
                selector,
                config,
                from_start,
            } => *from_start && in_config(*selector, *config),
            Self::Flag(flag) => flags?.contains(flag),
            Self::Other(_) => return None,
        })
    }
}

impl Selector {
    /// The configuration taken at the whole hour `hour` in a new game, by the data alone; `None` for a key no configuration has, or no default.
    pub fn pick(&self, hour: u32) -> Option<&Config> {
        self.pick_in(hour, &|_, _| false)
    }

    /// The same where `in_config` says which configuration other selectors are in.
    pub fn pick_in(&self, hour: u32, in_config: &dyn Fn(u32, u32) -> bool) -> Option<&Config> {
        self.pick_with(hour, in_config, &Assumed::default())
    }

    /// The same with what `assumed` takes of the game: the highest request wins, the first of
    /// equal ones, where a rule set requests the configuration of its first rule that holds; with
    /// none, or a request of configuration 0, the default.
    pub fn pick_with(
        &self,
        hour: u32,
        in_config: &dyn Fn(u32, u32) -> bool,
        assumed: &Assumed,
    ) -> Option<&Config> {
        let flags = assumed.flags.as_ref();
        let by_rules = (self.rule_sets.iter()).filter_map(|set| {
            let rule = (set.rules.iter())
                .find(|r| r.condition.holds_with(hour, in_config, flags) == Some(true))?;
            Some((set.priority, rule.config))
        });
        let by_requests = (assumed.requests.iter())
            .filter(|r| r.selector == self.key && r.hours & 1 << hour.min(31) != 0)
            .map(|r| (r.priority, r.config));
        let mut best: Option<(i32, u32)> = None;
        for (priority, config) in by_rules.chain(by_requests) {
            if best.is_none_or(|(top, _)| priority > top) {
                best = Some((priority, config));
            }
        }
        let key = match best {
            Some((_, config)) if config != 0 => config,
            _ => self.default,
        };
        (key != 0)
            .then(|| self.configs.iter().find(|c| c.key == key))
            .flatten()
    }
}

/// The configuration each of `selectors` takes at the whole hour `hour`, rules on another selector's configuration answered by the others' picks until they agree.
pub fn picks<'a>(selectors: &[&'a Selector], hour: u32) -> Vec<Option<&'a Config>> {
    picks_with(selectors, hour, &Assumed::default())
}

/// The same with what `assumed` takes of the game.
pub fn picks_with<'a>(
    selectors: &[&'a Selector],
    hour: u32,
    assumed: &Assumed,
) -> Vec<Option<&'a Config>> {
    let mut taken: HashMap<u32, u32> = HashMap::new();
    let mut picked = Vec::new();
    // Each round settles one more link of a chain of selectors; a cycle that never agrees stops.
    for _ in 0..8 {
        let in_config = |selector, config| taken.get(&selector) == Some(&config);
        picked = selectors
            .iter()
            .map(|s| s.pick_with(hour, &in_config, assumed))
            .collect();
        let next: HashMap<u32, u32> = (selectors.iter().zip(&picked))
            .filter_map(|(s, c)| Some((s.key, c.as_ref()?.key)))
            .collect();
        if next == taken {
            break;
        }
        taken = next;
    }
    picked
}

/// The group words on at the whole hour `hour` under `selectors`: those a picked configuration turns on or to state 2 and none turns off.
pub fn words_on<'a>(selectors: impl IntoIterator<Item = &'a Selector>, hour: u32) -> HashSet<u64> {
    let all: Vec<&Selector> = selectors.into_iter().collect();
    words_of(picks(&all, hour).into_iter().flatten())
}

/// The group words `configs` leave on: those one turns on or to state 2 and none turns off.
pub fn words_of<'a>(configs: impl IntoIterator<Item = &'a Config>) -> HashSet<u64> {
    let (mut on, mut off) = (HashSet::new(), HashSet::new());
    for config in configs {
        on.extend(config.on.iter().chain(&config.state2).copied());
        off.extend(config.off.iter().copied());
    }
    on.retain(|word| !off.contains(word));
    on
}

impl WorldState {
    /// The selectors of the table-form document `bytes`.
    pub fn open(bytes: &[u8]) -> Result<Self, WorldStateError> {
        Self::read(&table_form::model_of(bytes)?)
    }

    /// The selectors of a table-form document's type table and blocks.
    pub fn read(model: &RbmModel) -> Result<Self, WorldStateError> {
        let root = (model.block(model.root_block_id as i32)).map_or(0, |b| b.data_type);
        if root != WORLD_STATE {
            return Err(WorldStateError::NotWorldState(root));
        }
        let tree = table_form::decode(model)?;
        let Some(Value::Array(nodes)) = tree.get(&name(NODES)) else {
            return Err(WorldStateError::Broken("its nodes are not an array"));
        };
        let (mut selectors, mut places) = (Vec::new(), Vec::new());
        for node in nodes.iter().filter_map(object) {
            let components: Vec<&MetaInstance> = match node.get(&name(COMPONENTS)) {
                Some(Value::Array(items)) => items.iter().filter_map(object).collect(),
                _ => {
                    return Err(WorldStateError::Broken(
                        "a node's components are not an array",
                    ))
                }
            };
            places.extend(place_of(node, &components));
            let (rule_class, selector_class) = (name(RULE_COMPONENT), name(SELECTOR_COMPONENT));
            // A rule set names the key of its selector, in the same node; each requests on its own.
            let mut rule_sets: Vec<(u32, RuleSet)> = Vec::new();
            for component in components.iter().filter(|c| c.type_name == rule_class) {
                for (_, set) in pairs(component, RULE_SETS) {
                    let Some(key) = set.get(&name(RULE_SELECTOR)).and_then(key32) else {
                        continue;
                    };
                    let rules = rules_of(set);
                    let priority = number(set, REQUEST_PRIORITY);
                    rule_sets.push((key, RuleSet { priority, rules }));
                }
            }
            for component in components.iter().filter(|c| c.type_name == selector_class) {
                for (key, selector) in pairs(component, SELECTORS) {
                    let configs = (pairs(selector, CONFIGS).into_iter())
                        .map(|(key, config)| config_of(key, config))
                        .collect();
                    selectors.push(Selector {
                        key,
                        default: selector
                            .get(&name(DEFAULT_CONFIG))
                            .and_then(key32)
                            .unwrap_or(0),
                        configs,
                        rule_sets: (rule_sets.iter())
                            .filter(|(k, _)| *k == key)
                            .map(|(_, set)| set.clone())
                            .collect(),
                    });
                }
            }
        }
        Ok(Self {
            layer: tree.get(&name(BODY_LAYER)).and_then(key32).unwrap_or(0),
            selectors,
            places,
        })
    }
}

/// The place `node` names, where it has a location name and an outline of three points or more.
fn place_of(node: &MetaInstance, components: &[&MetaInstance]) -> Option<Place> {
    let location = (components.iter()).find(|c| c.type_name == name(LOCATION_NAME))?;
    let shape = node.get(&name(SHAPE)).and_then(object)?;
    let outline: Vec<[f32; 2]> = (array(shape, VERTS).iter())
        .filter_map(|v| match v {
            Value::Vec2([x, y]) | Value::Vec3([x, y, _]) | Value::Vec4([x, y, _, _]) => {
                Some([*x, *y])
            }
            _ => None,
        })
        .filter(|[x, y]| x.is_finite() && y.is_finite())
        .collect();
    (outline.len() >= 3).then(|| Place {
        name: node.get(&name(NODE_NAME)).and_then(key32).unwrap_or(0),
        label: location
            .get(&name(LOCATION_LABEL))
            .and_then(key32)
            .unwrap_or(0),
        kind: number(location, LOCATION_KIND),
        outline,
    })
}

/// Each layer of the layer manifest `bytes`: the hash of its name, and whether the level loads it.
pub fn layers_loaded(bytes: &[u8]) -> Result<Vec<(u32, bool)>, WorldStateError> {
    let model = table_form::model_of(bytes)?;
    let root = (model.block(model.root_block_id as i32)).map_or(0, |b| b.data_type);
    if root != LAYER_MANIFEST {
        return Err(WorldStateError::NotLayerManifest(root));
    }
    let tree = table_form::decode(&model)?;
    let Some(Value::Array(layers)) = tree.get(&name(LAYERS)) else {
        return Err(WorldStateError::Broken("its layers are not an array"));
    };
    Ok((layers.iter().filter_map(object))
        .filter_map(|layer| {
            let loaded = match layer.get(&name(LAYER_LOADED)) {
                Some(Value::Bool(b)) => *b,
                Some(Value::UInt(u)) => *u != 0,
                Some(Value::Int(i)) => *i != 0,
                _ => false,
            };
            Some((layer.get(&name(LAYER_NAME)).and_then(key32)?, loaded))
        })
        .collect())
}

/// What the value tree names a member or class of hash `hash` by.
fn name(hash: u32) -> String {
    default_hash_index().name_or_literal(hash)
}

/// The structure a value holds in place or points at.
fn object(value: &Value) -> Option<&MetaInstance> {
    match value {
        Value::Struct(s) | Value::Pointer(Some(s)) => Some(s),
        _ => None,
    }
}

/// A 32-bit hash as a value states it.
fn key32(value: &Value) -> Option<u32> {
    match value {
        Value::Hash(h) => Some(h.hash),
        Value::UInt(u) => u32::try_from(*u).ok(),
        _ => None,
    }
}

/// A 64-bit group word as a value states it: the number, or the text a name gives it.
fn word64(value: &Value) -> Option<u64> {
    match value {
        Value::UInt(u) => Some(*u),
        Value::String(text) => Some(crate::gta6::hash::name_hash64(text.as_bytes())),
        _ => object(value).and_then(|pair| word64(pair.get(&name(PAIR_WORD))?)),
    }
}

fn float(of: &MetaInstance, member: u32) -> f32 {
    match of.get(&name(member)) {
        Some(Value::Float(v)) => *v as f32,
        _ => 0.0,
    }
}

fn number(of: &MetaInstance, member: u32) -> i32 {
    match of.get(&name(member)) {
        Some(Value::Int(v)) => *v as i32,
        Some(Value::UInt(v)) => *v as i32,
        Some(Value::Bool(v)) => i32::from(*v),
        _ => 0,
    }
}

/// The pairs of the map `member` of `owner` whose key is a hash and whose value is a structure.
fn pairs(owner: &MetaInstance, member: u32) -> Vec<(u32, &MetaInstance)> {
    let Some(Value::Map(pairs)) = owner.get(&name(member)) else {
        return Vec::new();
    };
    (pairs.iter())
        .filter_map(|(key, value)| Some((key32(key)?, object(value)?)))
        .collect()
}

fn array(owner: &MetaInstance, member: u32) -> &[Value] {
    match owner.get(&name(member)) {
        Some(Value::Array(items)) => items,
        _ => &[],
    }
}

fn config_of(key: u32, config: &MetaInstance) -> Config {
    let mut out = Config {
        key,
        ..Config::default()
    };
    for change in array(config, CHANGES).iter().filter_map(object) {
        if change.type_name == name(LAYER_CHANGE) {
            out.layers.extend(
                (array(change, ENABLED).iter().filter_map(object)).filter_map(|layer| {
                    Some((
                        layer.get(&name(LAYER_ARCHIVE)).and_then(key32)?,
                        layer.get(&name(LAYER_OF_CHANGE)).and_then(key32)?,
                    ))
                }),
            );
            continue;
        }
        for lists in [WORD_LISTS, PAIR_LISTS] {
            let [on, off, state2] = lists.map(|list| array(change, list).iter().filter_map(word64));
            out.on.extend(on);
            out.off.extend(off);
            out.state2.extend(state2);
        }
    }
    out
}

fn rules_of(set: &MetaInstance) -> Vec<Rule> {
    (array(set, RULES).iter().filter_map(object))
        .map(|rule| Rule {
            config: rule.get(&name(RULE_CONFIG)).and_then(key32).unwrap_or(0),
            // A rule that names no condition always holds.
            condition: match rule.get(&name(CONDITION)) {
                Some(Value::Pointer(None)) | None => Condition::True,
                Some(value) => object(value).map_or_else(unread, condition_of),
            },
        })
        .collect()
}

fn condition_of(c: &MetaInstance) -> Condition {
    let class = [
        TIME_OF_DAY,
        TIME_WITH_MINUTES,
        NOT,
        AND,
        OR,
        TRUE,
        RAINING,
        PED_WANTED,
        PED_WANTED_LEVEL,
        ITEM_UNLOCKED,
        HAS_ITEM,
        RANDOM,
        IN_CONFIG,
        PROPERTY_OWNED,
        FLAG_SET,
    ]
    .into_iter()
    .find(|class| c.type_name == name(*class));
    let all = || {
        array(c, CONDITIONS)
            .iter()
            .filter_map(object)
            .map(condition_of)
            .collect()
    };
    match class {
        Some(TIME_OF_DAY) => Condition::TimeOfDay {
            start: number(c, START_HOUR),
            end: number(c, END_HOUR),
        },
        Some(TIME_WITH_MINUTES) => Condition::TimeWithMinutes {
            start: (number(c, START_HOUR), number(c, START_MINUTE)),
            end: (number(c, END_HOUR), number(c, END_MINUTE)),
        },
        Some(NOT) => Condition::Not(Box::new(
            (c.get(&name(CONDITION)).and_then(object)).map_or_else(unread, condition_of),
        )),
        Some(AND) => Condition::And(all()),
        Some(OR) => Condition::Or(all()),
        Some(TRUE) => Condition::True,
        Some(RAINING) => Condition::Raining(match c.get(&name(RAIN_AMOUNT)) {
            Some(Value::Float(v)) => *v as f32,
            _ => 0.0,
        }),
        // A new game: wanted level nought, nothing unlocked, no item held.
        Some(PED_WANTED | ITEM_UNLOCKED) => Condition::NewGame(false),
        Some(PED_WANTED_LEVEL) => {
            Condition::NewGame(number(c, LOWER_BOUND) <= 0 && 0 <= number(c, UPPER_BOUND))
        }
        Some(HAS_ITEM) => Condition::NewGame(number(c, MIN_ITEM_COUNT) <= 0),
        Some(RANDOM) => Condition::Random(float(c, CHANCES)),
        // A least span of -1 is none; at the start of a game none has passed.
        Some(IN_CONFIG) => Condition::InConfig {
            selector: c
                .get(&name(IN_CONFIG_SELECTOR))
                .and_then(key32)
                .unwrap_or(0),
            config: c.get(&name(RULE_CONFIG)).and_then(key32).unwrap_or(0),
            from_start: float(c, MIN_SECONDS) <= 0.0 && float(c, MIN_GAME_HOURS) <= 0.0,
        },
        Some(PROPERTY_OWNED) if number(c, ANY_OWNER) != 0 => Condition::NewGame(false),
        Some(FLAG_SET) => match c.get(&name(FLAG)).and_then(key32) {
            Some(flag) => Condition::Flag(flag),
            None => Condition::Other(c.type_name.clone()),
        },
        _ => Condition::Other(c.type_name.clone()),
    }
}

/// A condition whose value is no structure: one nothing here evaluates.
fn unread() -> Condition {
    Condition::Other(String::new())
}

#[cfg(test)]
#[path = "world_state_tests.rs"]
mod tests;
