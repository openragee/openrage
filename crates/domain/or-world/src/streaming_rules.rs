//! Streaming, level-of-detail, layer, selector, clock and portal rules of the world, as pure functions.
//! The per-level fade ranges, the category and lodtype scales, the fade base and the parent's shared fade are not known from the build, so every caller supplies them.

use glam::Vec3;

/// Distance added to a map file's sphere radius, metres; every map tier has the same, read from this build.
pub const MAP_TIER_DISTANCE: f32 = 50.0;

/// Smallest half extent, metres, below which a map file has no sphere; read from this build.
pub const MIN_SPHERE_HALF_EXTENT: f32 = 1.0;

/// Move of the focus in one update past which the game calls a jump handler, metres; read from this build, what the handler does is not known.
pub const FOCUS_JUMP_DISTANCE: f32 = 300.0;

/// Fade range of a level-5 entity with a short record id, metres; read from this build.
pub const ORPHAN_HD_FADE_RANGE: f32 = 5.0;

/// Lod level an entity of level 0 with no parent takes; read from this build (the names HD and orphan HD are GTA V's, assumed).
pub const ORPHAN_HD_LEVEL: u8 = 5;

/// Record ids below this take the level-5 fade range; read from this build.
pub const ORPHAN_HD_SHORT_ID_LIMIT: u16 = 0x15;

/// Entity type byte whose lodtype scale is raised to at least 1; read from this build, that it is an interior is inferred.
pub const INTERIOR_ENTITY_TYPE: u8 = 0xC;

/// Bias added before truncating a fade alpha, about 0.0255, unitless; read from this build.
pub const FADE_ALPHA_BIAS: f32 = f32::from_bits(0x3CD0_E560);

/// Distance offset in the request priority, metres; read from this build.
pub const PRIORITY_DISTANCE_EPSILON: f32 = 1e-6;

/// Largest distance term of a request priority, unitless; read from this build.
pub const PRIORITY_DISTANCE_TERM_MAX: f32 = 0.95;

/// Per-level bias added to the distance term when the call's argument bit 0 is set (when it is set is not known); read from this build.
pub const PRIORITY_LEVEL_BIAS: [f32; 8] = [0.0, 0.08, 0.16, 0.24, 0.32, 0.02, 0.4, 0.0];

/// Band of a record in the main phase mask with argument bit 1 or a passing visibility test (that the test is on-screen or occlusion is inferred); read from this build.
pub const BAND_VISIBLE: f32 = 7.0;

/// Band of a record in the main phase mask whose visibility test fails; read from this build.
pub const BAND_IN_VIEW_HIDDEN: f32 = 6.0;

/// Band of a record whose draw word has 0x100 and 0x04, lod level not 0 or 5; read from this build.
pub const BAND_LOW_DETAIL: f32 = 5.0;

/// Highest of the three interior bands, chosen by the interior and room match; read from this build.
pub const BAND_INTERIOR_HIGH: f32 = 4.0;

/// Middle interior band; read from this build.
pub const BAND_INTERIOR_MIDDLE: f32 = 3.0;

/// Lowest interior band; read from this build.
pub const BAND_INTERIOR_LOW: f32 = 2.0;

/// Band of every other request; read from this build.
pub const BAND_OTHER: f32 = 1.0;

/// Draw-word bits (0x100 and 0x04) of a band-5 record; read from this build.
pub const DRAW_LOW_DETAIL_BITS: u64 = 0x104;

/// Draw-word bit of an interior record; read from this build.
pub const DRAW_INTERIOR_BIT: u64 = 0x1;

/// Entries the model request queue holds, past which entries are dropped; read from this build.
pub const REQUEST_QUEUE_CAPACITY: usize = 12288;

/// Instances per instance-culling group, each group one batch id; read from this build.
pub const INSTANCE_GROUP_SIZE: u32 = 64;

/// Least change of the LOD scale that sets its changed bit; read from this build.
pub const LOD_SCALE_CHANGED_EPSILON: f32 = 0.001;

/// Default start of the far stand-in fade, metres if its input is the camera distance (inferred); read from this build.
pub const STAND_IN_FADE_START: f32 = 300.0;

/// Default end of the far stand-in fade, metres if its input is the camera distance (inferred); read from this build.
pub const STAND_IN_FADE_END: f32 = 2000.0;

/// Default start of the second stand-in range, metres; read from this build.
pub const STAND_IN_SECOND_START: f32 = 190.0;

/// Default end of the second stand-in range, metres; read from this build.
pub const STAND_IN_SECOND_END: f32 = 2000.0;

/// Smallest span of the second stand-in range, metres; read from this build.
pub const STAND_IN_SECOND_MIN_SPAN: f32 = 1e-4;

/// Tunable hash of the far stand-in fade start; read from this build.
pub const STAND_IN_FADE_START_HASH: u32 = 0x7C6A_4FB5;

/// Tunable hash of the far stand-in fade end; read from this build.
pub const STAND_IN_FADE_END_HASH: u32 = 0x10EF_3F4D;

/// Tunable hash of the second stand-in range start; read from this build.
pub const STAND_IN_SECOND_START_HASH: u32 = 0x828F_F079;

/// Tunable hash of the second stand-in range end; read from this build.
pub const STAND_IN_SECOND_END_HASH: u32 = 0x94B8_99F5;

/// Imposter quality offset Q[q] by quality setting 0..=3; read from this build.
pub const IMPOSTER_Q: [f32; 4] = [0.0, 0.0, -1.0, -1.0];

/// Default imposter quality thresholds (the first three lanes of the tunable vec4); read from this build.
pub const IMPOSTER_THRESHOLDS: [f32; 3] = [1.0, 1.5, 2.0];

/// Requester under which registration turns on a layer whose descriptor byte +0x0C is 1; read from this build.
pub const REGISTRATION_REQUESTER: u32 = 0x796A_071D;

/// Constructor default of a layer action's random minimum; read from this build.
pub const LAYER_ACTION_MIN_DEFAULT: u32 = 1;

/// Constructor default of a layer action's random maximum; read from this build.
pub const LAYER_ACTION_MAX_DEFAULT: u32 = 100;

/// Longest random list the layer action applies; read from this build.
pub const RANDOM_LIST_MAX: u32 = 31;

/// Class A's fixed valid selector states (the keys of its map +0x68 are valid too); read from this build.
pub const CLASS_A_FIXED_STATES: [u32; 5] = [
    0xD5D2_F552,
    0x005F_46C7,
    0x69D3_EE70,
    0x9C78_3318,
    0xB2AD_28E9,
];

/// Class A's default when its gate passes ("on" is inferred); read from this build.
pub const CLASS_A_GATE_DEFAULT: u32 = 0x9C78_3318;

/// Class A's default when its gate fails ("off" is inferred); read from this build.
pub const CLASS_A_FALLBACK_DEFAULT: u32 = 0x005F_46C7;

/// Class D's default selector state; read from this build.
pub const CLASS_D_DEFAULT: u32 = 0xE4DF_46D5;

/// Class D's valid selector states; read from this build.
pub const CLASS_D_STATES: [u32; 2] = [0xE4DF_46D5, 0x69E5_6252];

/// Portal flag bit of a link portal, through which a walk continues into the other instance's room-0 portal; read from this build.
pub const PORTAL_LINK: u32 = 0x02;

/// Portal flag bit of a mirror, which every walk that tests it skips; read from this build.
pub const PORTAL_MIRROR: u32 = 0x04;

/// Portal flag bit that lets a portal close; read from this build.
pub const PORTAL_ALLOW_CLOSING: u32 = 0x40;

/// Defaults of the portal definition's floats +0x1C, +0x20, +0x24, +0x28, +0x2C (meanings not known); read from this build.
pub const PORTAL_FLOAT_DEFAULTS: [f32; 5] = [1.0, 1.0, 1.0, 1.0, 0.75];

/// Default of a room's exteriorVisibiltyDepth (+0x48); read from this build, the name inferred.
pub const ROOM_EXTERIOR_VISIBILITY_DEPTH_DEFAULT: i32 = -1;

/// Half size of the box about the camera that an interior's bounds must overlap for the portal factor, metres; read from this build.
pub const PORTAL_FACTOR_CAMERA_BOX: f32 = 4.0;

/// Distance to a room-0 portal under which the camera portal factor drops, metres; read from this build.
pub const PORTAL_FACTOR_REACH: f32 = 25.0;

/// Factor per metre of portal distance (1 / 25); read from this build.
pub const PORTAL_FACTOR_SCALE: f32 = 0.04;

/// Camera portal factor under which the interior ambient is chosen; read from this build.
pub const INTERIOR_AMBIENT_FACTOR: f32 = 0.05;

/// Door open ratio below which a door counts as shut, 0.005, unitless; read from this build.
pub const DOOR_OPEN_RATIO: f32 = f32::from_bits(0x3BA3_D70A);

/// Door flags (dword +0x34) mask of the keep-open test; read from this build.
pub const DOOR_KEEP_OPEN_MASK: u32 = 0x28_0000;

/// Value the keep-open mask must equal to keep the portal open; read from this build.
pub const DOOR_KEEP_OPEN_VALUE: u32 = 0x8_0000;

/// Door flag bit 22 (dword +0x34; byte +0x36 bit 0x40 is the same bit), which keeps the portal open; read from this build.
pub const DOOR_HOLDS_OPEN_BIT: u32 = 1 << 22;

/// Archetype flag bit (+0x54) of a door that keeps its portal open; read from this build.
pub const DOOR_ARCHETYPE_KEEP_OPEN_BIT: u32 = 0x1;

/// A map file's streaming sphere, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamingSphere {
    /// Centre of the file's box, metres.
    pub centre: Vec3,
    /// Smallest half extent of the box plus the tier distance, metres.
    pub radius: f32,
}

impl StreamingSphere {
    /// Whether a point, metres, is inside or on the sphere.
    pub fn contains(&self, point: Vec3) -> bool {
        (point - self.centre).length_squared() <= self.radius * self.radius
    }
}

/// The streaming sphere of a map file's box B, metres, or none when a half extent is under 1 m; which of the file's two boxes is B is not known.
pub fn map_streaming_sphere(box_min: Vec3, box_max: Vec3) -> Option<StreamingSphere> {
    let half: Vec3 = 0.5 * (box_max - box_min);
    // A NaN or infinite lane is a malformed box; f32::min would drop a NaN lane, so the whole vector is tested.
    if !half.is_finite() {
        return None;
    }
    let smallest: f32 = half.x.min(half.y).min(half.z);
    if smallest < MIN_SPHERE_HALF_EXTENT {
        return None;
    }
    Some(StreamingSphere {
        centre: 0.5 * (box_min + box_max),
        radius: smallest + MAP_TIER_DISTANCE,
    })
}

/// Whether a focus move in one update is past the game's jump distance; treating it as a jump in the viewer is inferred.
pub fn is_focus_jump(old: Vec3, new: Vec3) -> bool {
    (new - old).length_squared() > FOCUS_JUMP_DISTANCE * FOCUS_JUMP_DISTANCE
}

/// A map file's u16 flag word after its group enters `state` 0 to 3, of which only 0 is drawn; other states leave it.
pub fn group_state_flag_word(word: u16, state: u8) -> u16 {
    // That states 1 and 2 are always in the search result, kept resident but not shown, is inferred.
    match state {
        0 => word & !0x80,
        1 | 2 => word | 0x180,
        3 => (word & !0x180) | 0x80,
        _ => word,
    }
}

/// Whether every map search skips a file by its flag word, `(word & 0x180) == 0x80`.
pub fn search_skips(word: u16) -> bool {
    (word & 0x180) == 0x80
}

/// The lod level an entity takes: an entity of level 0 with no parent becomes level 5.
pub fn resolved_lod_level(def_level: u8, has_parent: bool) -> u8 {
    if def_level == 0 && !has_parent {
        ORPHAN_HD_LEVEL
    } else {
        def_level
    }
}

/// The definition's lodDist, metres, or the archetype's (+0x3C) when it is 0 or less.
pub fn resolved_lod_distance(def_lod_distance: f32, archetype_lod_distance: f32) -> f32 {
    if def_lod_distance <= 0.0 {
        archetype_lod_distance
    } else {
        def_lod_distance
    }
}

/// The fields of a parent entity that attaching a child changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodParent {
    /// childLodDist, metres; the game stores it truncated to u16, which is the caller's.
    pub child_lod_distance: f32,
    /// Whether the definition states a childLodDist (> 0), entity +0x95 bit 0x10.
    pub states_child_lod: bool,
    /// Stated children, raised to the attached count.
    pub stated_children: u16,
    /// Attached child entities.
    pub attached_children: u16,
}

/// Attaches a child of resolved lodDist `child_lod_distance`, metres, and returns the child's lodDist after the parent's rule.
pub fn attach_child(parent: &mut LodParent, child_lod_distance: f32) -> f32 {
    let resolved: f32 = if parent.states_child_lod {
        parent.child_lod_distance
    } else {
        parent.child_lod_distance = parent.child_lod_distance.max(child_lod_distance);
        child_lod_distance
    };
    parent.attached_children = parent.attached_children.saturating_add(1);
    parent.stated_children = parent.stated_children.max(parent.attached_children);
    resolved
}

/// The fade scale of an entity: category scale times lodtype scale, the latter at least 1 for type 0xC.
pub fn fade_scale(category_scale: f32, lodtype_scale: f32, entity_type: u8) -> f32 {
    let lodtype: f32 = if entity_type == INTERIOR_ENTITY_TYPE {
        lodtype_scale.max(1.0)
    } else {
        lodtype_scale
    };
    category_scale * lodtype
}

/// The distance, metres, at which an entity starts to fade out.
pub fn fade_start(lod_distance: f32, fade_base: f32, scale: f32) -> f32 {
    if scale < 1.0 {
        fade_base + scale * (lod_distance - fade_base)
    } else {
        scale * lod_distance
    }
}

/// The fade range, metres: 5 for a level-5 record with an id under 0x15, else the caller's per-level range.
pub fn fade_range(lod_level: u8, record_id: u16, level_range: f32) -> f32 {
    if lod_level == ORPHAN_HD_LEVEL && record_id < ORPHAN_HD_SHORT_ID_LIMIT {
        ORPHAN_HD_FADE_RANGE
    } else {
        level_range
    }
}

/// The distance alpha, 0..=255: 255 up to the fade start, 0 past start plus range; a range of 0 or less gives 0, as the game's unguarded division does (inferred).
pub fn distance_alpha(distance: f32, start: f32, range: f32) -> u8 {
    let end: f32 = start + range;
    // max then min, not f32::clamp, which panics when the range is negative. Both drop a NaN distance, which so takes the start and gives 255.
    let clamped: f32 = distance.max(start).min(end);
    let value: f32 = (end - clamped) * 255.0 / range + FADE_ALPHA_BIAS;
    // The game truncates toward zero; `as` maps the NaN of a zero range to 0.
    value as u8
}

/// A parent's alpha after the cross-fade step, and whether the cross-fade is in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrossFade {
    /// The parent's alpha, 0..=255.
    pub alpha: u8,
    /// True while the shared fade is neither 0 nor 255.
    pub in_progress: bool,
}

/// A parent's alpha: 255 minus the shared fade when every stated child is attached, its record alpha is 255 and the entity's flags permit it, else its own.
pub fn parent_cross_fade(
    record_alpha: u8,
    shared_fade: u8,
    stated_children: u16,
    attached_children: u16,
    flags_permit: bool,
) -> CrossFade {
    // That attached counts child entities made, not child models loaded, is inferred; force is never set in the viewer.
    // `flags_permit` is `(+0x94 & 0x23) == 0x20 && (+0x95 & 0x20) == 0 && draw word & 0x8 == 0`.
    let all_attached: bool = attached_children == stated_children;
    if stated_children != 0 && all_attached && flags_permit && record_alpha == 255 {
        CrossFade {
            alpha: 255 - shared_fade,
            in_progress: shared_fade != 0 && shared_fade != 255,
        }
    } else {
        CrossFade {
            alpha: record_alpha,
            in_progress: false,
        }
    }
}

/// A child's alpha: its parent's shared fade when the parent has one, the record alpha is not 0 and the child has no interior index.
pub fn child_alpha(
    record_alpha: u8,
    parent_shared_fade: Option<u8>,
    has_interior_index: bool,
) -> u8 {
    match parent_shared_fade {
        Some(fade) if fade != 0 && record_alpha != 0 && !has_interior_index => fade,
        _ => record_alpha,
    }
}

/// Whether a child pins its parent's alpha at 255; that `model_not_ready` (child +0x95 bit 0x08) means the child's model is not ready is inferred.
pub fn child_pins_parent(
    model_not_ready: bool,
    child_alpha: u8,
    record_alpha: u8,
    phase: u8,
    child_flag_bit0: bool,
    level_in_mask: bool,
) -> bool {
    // Force is never set in the viewer; with it no child pins.
    model_not_ready
        && child_alpha != 0
        && phase != 6
        && (((record_alpha != 255 || child_alpha == 255) && !child_flag_bit0)
            || (record_alpha == 255 && child_alpha < 255 && (!child_flag_bit0 || level_in_mask)))
}

/// A model request's band, in the table's order (that the code tests in that order is inferred); the scope-zoom case of band 7 is the caller's.
pub fn request_band(
    in_main_phase: bool,
    passes_visibility: bool,
    draw_word: u64,
    lod_level: u8,
    interior_band: f32,
) -> f32 {
    // `passes_visibility` includes argument bit 1; `interior_band` is the caller's 4, 3 or 2, whose mapping to the room match is not known.
    if in_main_phase {
        if passes_visibility {
            BAND_VISIBLE
        } else {
            BAND_IN_VIEW_HIDDEN
        }
    } else if (draw_word & DRAW_LOW_DETAIL_BITS) == DRAW_LOW_DETAIL_BITS
        && lod_level != 0
        && lod_level != ORPHAN_HD_LEVEL
    {
        BAND_LOW_DETAIL
    } else if (draw_word & DRAW_INTERIOR_BIT) != 0 {
        interior_band
    } else {
        BAND_OTHER
    }
}

/// A model request's queued priority: band plus min(1 / (distance + 1e-6), 0.95), plus the level's bias when given, floored at 0.
pub fn request_priority(band: f32, distance: f32, level_bias: Option<u8>) -> f32 {
    // A NaN distance takes the capped 0.95 term, since f32::min drops NaN.
    let near: f32 = (1.0 / (distance + PRIORITY_DISTANCE_EPSILON)).min(PRIORITY_DISTANCE_TERM_MAX);
    let bias: f32 = level_bias.map_or(0.0, |level: u8| PRIORITY_LEVEL_BIAS[usize::from(level & 7)]);
    floor_at_zero(band + (near + bias))
}

fn floor_at_zero(value: f32) -> f32 {
    // Written as `> 0` so that NaN and -0.0 both become +0.0, which the bit-pattern sort needs.
    if value > 0.0 {
        value
    } else {
        0.0
    }
}

/// One queued model request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelRequest {
    /// The caller's entity key, the tie-break of the sort.
    pub entity: u64,
    /// The queued priority from `request_priority`; a negative or NaN one sorts as 0.
    pub priority: f32,
}

/// Queue A's requests in issue order, highest priority first, after dropping what is past its capacity.
pub fn order_requests(queued: &[ModelRequest]) -> Vec<ModelRequest> {
    // Capacity applies in append order, before the sort. That the issue cap of 20000 never binds for queue A alone is inferred.
    let mut kept: Vec<ModelRequest> = queued
        .iter()
        .take(REQUEST_QUEUE_CAPACITY)
        .copied()
        .collect();
    // Bit-pattern order equals float order only for non-negative priorities, so the queue's max(0, v) is applied here.
    kept.sort_by_key(|request: &ModelRequest| {
        (floor_at_zero(request.priority).to_bits(), request.entity)
    });
    // The queue is walked from its end.
    kept.reverse();
    kept
}

/// Instance-culling groups of `count` instances, ceil(count / 64); the first-instance offset advances by groups * 64.
pub fn instance_groups(count: u32) -> u32 {
    count.div_ceil(INSTANCE_GROUP_SIZE)
}

/// The scope's LOD scale and second scale for a zoom, or none below zoom 2 or without the scope.
pub fn scope_lod_scales(zoom: f32, scope: bool) -> Option<(f32, f32)> {
    if scope && zoom >= 2.0 {
        if zoom >= 4.0 {
            Some((2.5, 2.0))
        } else {
            Some((1.8, 1.6))
        }
    } else {
        None
    }
}

/// The blended LOD scale at time `t` of a blend lasting `duration`; the game's duration is not read, so the caller's is a stand-in.
pub fn blend_lod_scale(prev: f32, target: f32, t: f32, duration: f32) -> f32 {
    let f: f32 = if duration > t {
        saturate(t / duration)
    } else {
        1.0
    };
    prev + (target - prev) * f
}

/// The far stand-in fade, 0..1: saturate(a * distance + b) with a = 1 / (end - start), b = -start * a; that `distance` is the camera distance, and which way the fade blends, are inferred.
pub fn stand_in_fade(distance: f32, start: f32, end: f32) -> f32 {
    // The division is unguarded, as the game's is: equal bounds give NaN and so 0.
    let a: f32 = 1.0 / (end - start);
    let b: f32 = -start * a;
    saturate(a * distance + b)
}

/// The second stand-in range's ramp, 0..1, from start to end metres, its span at least 1e-4.
pub fn stand_in_second_fade(distance: f32, start: f32, end: f32) -> f32 {
    // The values are read from this build; that the shader ramps over them this way is inferred.
    let inverse_span: f32 = 1.0 / (end - start).max(STAND_IN_SECOND_MIN_SPAN);
    saturate((distance - start) * inverse_span)
}

#[allow(clippy::manual_clamp)]
fn saturate(value: f32) -> f32 {
    // max then min maps NaN to 0, as saturate does; f32::clamp keeps NaN.
    value.max(0.0).min(1.0)
}

/// A stand-in's imposter level: 3, 2 or 1 by `int_k / 256 + Q[quality]` against the thresholds, else `base`, then at least `item_min`.
pub fn imposter_level(
    int_k: u32,
    quality: usize,
    thresholds: [f32; 3],
    base: u8,
    item_min: u8,
) -> u8 {
    // Quality settings run 0..=3; a larger one takes the last entry.
    let v: f32 = int_k as f32 / 256.0 + IMPOSTER_Q[quality.min(3)];
    let level: u8 = if v < thresholds[0] {
        3
    } else if v < thresholds[1] {
        2
    } else if v < thresholds[2] {
        1
    } else {
        base
    };
    level.max(item_min)
}

/// Hour condition (struct 0xCFB331ED): whether hour lies in A..=B, wrapping past midnight when A >= B.
pub fn hour_in_range(hour: i32, start: i32, end: i32) -> bool {
    if start < end {
        start <= hour && hour <= end
    } else {
        hour <= end || hour >= start
    }
}

/// Hour and minute condition (struct 0xB889B2B5): whether the minute of day lies in start..=end, the wrap decided on hours alone; that the second and fourth members (+0x14, +0x1C) are minutes is inferred.
pub fn hour_minute_in_range(
    hour: i32,
    minute: i32,
    start_hour: i32,
    start_minute: i32,
    end_hour: i32,
    end_minute: i32,
) -> bool {
    let now: i32 = 60 * hour + minute;
    let start: i32 = 60 * start_hour + start_minute;
    let end: i32 = 60 * end_hour + end_minute;
    if start_hour < end_hour {
        start <= now && now <= end
    } else {
        now <= end || now >= start
    }
}

/// Day phase condition (struct 0x79437E80): whether the sun's day phase, 0..1, lies in min..max, wrapping when max <= min.
pub fn day_phase_in_range(phase: f64, min: f32, max: f32) -> bool {
    // The phase arrives as f64 like every other module's; the bounds are f32 members, so the compare is in f32.
    let phase: f32 = phase as f32;
    if max > min {
        min <= phase && phase < max
    } else {
        phase < max || phase >= min
    }
}

/// Hour condition of the other family (struct 0x9E68A6C0): whether hour lies in A..B, end excluded, wrapping when A >= B.
pub fn hour_in_half_open_range(hour: u32, start: u32, end: u32) -> bool {
    if start < end {
        start <= hour && hour < end
    } else {
        hour < end || hour >= start
    }
}

/// The record a layer lookup finds in `(archive, layer)` hashes: with archive 0 the last whose layer matches, else the first matching both.
pub fn find_layer(records: &[(u32, u32)], archive: u32, layer: u32) -> Option<usize> {
    if archive != 0 {
        records
            .iter()
            .position(|record: &(u32, u32)| *record == (archive, layer))
    } else {
        records
            .iter()
            .rposition(|record: &(u32, u32)| record.1 == layer)
    }
}

/// A layer's requester set; the layer is on exactly while the set is not empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerRequesters {
    /// Requester hashes, each at most once.
    pub requesters: Vec<u32>,
}

impl LayerRequesters {
    /// Whether the layer is on.
    pub fn is_on(&self) -> bool {
        !self.requesters.is_empty()
    }

    /// Adds a requester; returns true when this turned the layer on.
    pub fn request(&mut self, requester: u32) -> bool {
        if self.requesters.contains(&requester) {
            return false;
        }
        self.requesters.push(requester);
        self.requesters.len() == 1
    }

    /// Removes a requester; returns true when this turned the layer off.
    pub fn release(&mut self, requester: u32) -> bool {
        let before: usize = self.requesters.len();
        self.requesters.retain(|held: &u32| *held != requester);
        before != 0 && self.requesters.is_empty()
    }

    /// Drops every requester (op 3, force off); returns true when this turned the layer off.
    pub fn force_off(&mut self) -> bool {
        let was_on: bool = self.is_on();
        self.requesters.clear();
        was_on
    }
}

/// The range of k a random list of `n` entries draws from, min(min, n)..=min(max, n).
pub fn random_k_range(min: u32, max: u32, n: u32) -> std::ops::RangeInclusive<u32> {
    min.min(n)..=max.min(n)
}

/// Whether a random list of `n` entries is applied at all; whether k == 0 or k == n still applies to a longer one is not known.
pub fn random_list_applies(n: u32) -> bool {
    n <= RANDOM_LIST_MAX
}

/// One request for a selector state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectorRequest {
    /// Requester hash.
    pub requester: u32,
    /// Requested state hash; 0 asks for the class default.
    pub state: u32,
    /// Priority; a negative one never wins.
    pub priority: i32,
}

/// Adds a request in place of the same requester's entry, and returns whether it queues a re-evaluation (a set force byte also does, which is the caller's).
pub fn add_selector_request(requests: &mut Vec<SelectorRequest>, request: SelectorRequest) -> bool {
    // That "every existing one" includes the entry being overwritten is inferred.
    let queues: bool = requests
        .iter()
        .all(|held: &SelectorRequest| request.priority >= held.priority);
    for held in requests.iter_mut() {
        if held.requester == request.requester {
            held.state = request.state;
            held.priority = request.priority;
            return queues;
        }
    }
    requests.push(request);
    queues
}

/// Erases every request `accept` takes, and returns whether any was erased, which queues a re-evaluation.
pub fn remove_selector_requests(
    requests: &mut Vec<SelectorRequest>,
    accept: impl Fn(&SelectorRequest) -> bool,
) -> bool {
    let before: usize = requests.len();
    requests.retain(|held: &SelectorRequest| !accept(held));
    requests.len() != before
}

/// Class A's default state: 0x9C783318 when its gate passes (gate id 0 always does), else 0x005F46C7.
pub fn class_a_default(gate_passes: bool) -> u32 {
    if gate_passes {
        CLASS_A_GATE_DEFAULT
    } else {
        CLASS_A_FALLBACK_DEFAULT
    }
}

/// The state the requests pick (highest priority, earliest on a tie, else the class default), or none when the class's state set lacks it.
pub fn pick_selector_state(
    requests: &[SelectorRequest],
    default_state: u32,
    valid_states: &[u32],
) -> Option<u32> {
    let mut best: i32 = -1;
    let mut state: u32 = 0;
    for request in requests {
        if request.priority > best {
            best = request.priority;
            state = request.state;
        }
    }
    if state == 0 {
        state = default_state;
    }
    // None drops the request: the selector keeps its state.
    valid_states.contains(&state).then_some(state)
}

/// Whether a portal leads outside: it touches room 0 and is not a link portal.
pub fn portal_leads_outside(room_a: u32, room_b: u32, flags: u32) -> bool {
    (room_a == 0 || room_b == 0) && (flags & PORTAL_LINK) == 0
}

/// A portal object's opacity, 0..=1, from the definition's percentage; that the value at 100 or more is 1.0 is inferred.
pub fn portal_opacity(def_opacity: u32) -> f32 {
    if def_opacity >= 100 {
        1.0
    } else {
        def_opacity as f32 * 0.01
    }
}

/// The camera portal factor: 0 inside an interior, else the least of 1 and d * 0.04 over the portal distances under 25 m.
pub fn camera_portal_factor(
    inside_interior: bool,
    portal_distances: impl IntoIterator<Item = f32>,
) -> f32 {
    // The caller passes room-0 portal quads of live instances with proxy state >= 4 whose bounds overlap the camera's +-4 m box.
    if inside_interior {
        return 0.0;
    }
    portal_distances
        .into_iter()
        .filter(|distance: &f32| *distance < PORTAL_FACTOR_REACH)
        .fold(1.0, |factor: f32, distance: f32| {
            factor.min(distance * PORTAL_FACTOR_SCALE)
        })
}

/// Whether the camera portal factor chooses the interior ambient.
pub fn uses_interior_ambient(factor: f32) -> bool {
    factor < INTERIOR_AMBIENT_FACTOR
}

/// One door on a portal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortalDoor {
    /// Open ratio; its magnitude is compared against 0.005.
    pub open_ratio: f32,
    /// True when a keep-open condition holds for this door.
    pub keeps_open: bool,
}

/// Whether a door's own flags (+0x34) or its archetype's flags (+0x54) keep its portal open.
pub fn door_keeps_portal_open(door_flags: u32, archetype_flags: u32) -> bool {
    (door_flags & DOOR_KEEP_OPEN_MASK) == DOOR_KEEP_OPEN_VALUE
        || (door_flags & DOOR_HOLDS_OPEN_BIT) != 0
        || (archetype_flags & DOOR_ARCHETYPE_KEEP_OPEN_BIT) != 0
}

/// Whether a portal is shut: allow-closing set, the proxy's state byte non-zero, a door placed, every door shut with none keeping it open; a link portal's other side is the caller's.
pub fn portal_is_shut(portal_flags: u32, proxy_state_set: bool, doors: &[PortalDoor]) -> bool {
    // `proxy_state_set` is the proxy's state byte +0xC1 being non-zero, not the populated test (>= 4).
    if !proxy_state_set || (portal_flags & PORTAL_ALLOW_CLOSING) == 0 || doors.is_empty() {
        return false;
    }
    doors
        .iter()
        .all(|door: &PortalDoor| door.open_ratio.abs() < DOOR_OPEN_RATIO && !door.keeps_open)
}

#[cfg(test)]
#[path = "streaming_rules_tests.rs"]
mod tests;
