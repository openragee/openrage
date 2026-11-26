use std::mem::offset_of;

use super::*;

const VIRTUAL: u64 = 0x5000_0000;
const FRAMES: u16 = 70;
const FRAMES_PER_CHUNK: u16 = 255;
const DURATION: f32 = 1.0;
const DICTIONARY: &str = "dict_a";
const ANIMATION: &str = "walk";
const SCALE: f32 = 0.25;
const OFFSET: f32 = -1.0;
const ROOT_WORD_60: u64 = 0x0102_0304_0506_0708;
const ROOT_WORD_98: u64 = 0x1112_1314_1516_1718;
const SIGNATURE: u32 = 0x2468_ACE1;
/// The second entry of a group clip's warp points.
const WARP: [[f32; 2]; 3] = [[0.0, 0.0], [1.4, 1.375], [1.5, 1.5]];

fn hash(name: &str) -> u32 {
    name_hash32(name.as_bytes())
}

/// A virtual section laid out object by object, each on 16 bytes.
#[derive(Default)]
struct Bytes(Vec<u8>);

impl Bytes {
    fn alloc(&mut self, len: usize) -> usize {
        let at = self.0.len().next_multiple_of(16);
        self.0.resize(at + len, 0);
        at
    }
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.0[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn u16(&mut self, at: usize, v: u16) {
        self.put(at, &v.to_le_bytes());
    }
    fn u32(&mut self, at: usize, v: u32) {
        self.put(at, &v.to_le_bytes());
    }
    fn u64(&mut self, at: usize, v: u64) {
        self.put(at, &v.to_le_bytes());
    }
    fn f32(&mut self, at: usize, v: f32) {
        self.put(at, &v.to_le_bytes());
    }
    fn ptr(&mut self, at: usize, to: usize) {
        self.u64(at, VIRTUAL + to as u64);
    }
    fn text(&mut self, s: &str) -> usize {
        let at = self.alloc(s.len() + 1);
        self.put(at, s.as_bytes());
        at
    }
    /// The pointer form of a 16-byte name.
    fn name(&mut self, at: usize, s: &str) {
        let text = self.text(s);
        self.ptr(at, text);
        self.u16(at + 8, s.len() as u16);
        self.u16(at + 10, s.len() as u16 + 1);
        self.u32(at + 12, 1);
    }
    /// The inline form of a 16-byte name.
    fn inline_name(&mut self, at: usize, s: &str) {
        self.put(at, s.as_bytes());
        self.0[at + 15] = 0x80 | s.len() as u8;
    }
    fn array(&mut self, at: usize, first: usize, count: u16) {
        self.ptr(at, first);
        self.u16(at + 8, count);
        self.u16(at + 10, count);
    }
    /// An array of `values`, each `stride` bytes, the array's head at `at`.
    fn list(&mut self, at: usize, stride: usize, values: &[&[u8]]) -> usize {
        let first = self.alloc(stride * values.len().max(1));
        for (i, v) in values.iter().enumerate() {
            self.put(first + i * stride, v);
        }
        self.array(at, first, values.len() as u16);
        first
    }
    fn map(&mut self, at: usize, slots: u32, entries: &[(u64, usize)], next_at: usize) {
        let buckets = self.alloc(slots as usize * 8);
        for &(key, entry) in entries {
            let head = buckets + (key % u64::from(slots)) as usize * 8;
            let next: [u8; 8] = self.0[head..head + 8].try_into().expect("eight bytes");
            self.put(entry + next_at, &next);
            self.ptr(head, entry);
        }
        self.ptr(at, buckets);
        self.u32(at + 8, slots);
        self.u32(at + 0x10, entries.len() as u32);
        self.0[at + 0x17] = 1;
    }
    /// A relative array at `at` of `count` elements from `first`.
    fn rel(&mut self, at: usize, first: usize, count: u32) {
        self.u32(at, count);
        self.u32(at + 4, count);
        self.u64(at + 8, (first as i64 - (at as i64 + 8)) as u64);
    }
    fn empty_rel(&mut self, at: usize) {
        self.u64(at + 8, 1);
    }
}

/// Zigzag codes of `values`' second differences, per segment of 64 frames, in eight interleaved lanes.
fn delta_channel(values: &[i64]) -> Vec<u32> {
    let frames = values.len();
    let steps = frames - 1;
    let segments = frames.div_ceil(64);
    let groups = steps.div_ceil(8);
    let mut codes = vec![0u64; groups * 8];
    for segment in 0..segments {
        let mut velocity = 0;
        for step in 64 * segment..(64 * segment + 64).min(steps) {
            let v = values[step + 1] - values[step];
            let a = v - velocity;
            codes[step] = ((a << 1) ^ (a >> 63)) as u64;
            velocity = v;
        }
    }
    let bit_len = |v: u64| 64 - v.leading_zeros();
    let widths: Vec<u32> = codes
        .chunks(8)
        .map(|g| bit_len(*g.iter().max().expect("eight")))
        .collect();
    let addresses: Vec<u64> = (0..segments)
        .map(|s| widths.iter().take(8 * s).map(|w| u64::from(*w)).sum())
        .collect();
    let initials: Vec<u64> = (0..segments).map(|s| values[64 * s] as u64).collect();
    let a = bit_len(*addresses.last().expect("a segment"));
    let i = bit_len(*initials.iter().max().expect("a segment"));
    let sum: u32 = widths.iter().sum();
    let lane_words = sum.div_ceil(32) as usize;
    let width_end =
        128 + segments * (a + i) as usize + widths.iter().map(|w| *w as usize + 1).sum::<usize>();
    let head_words = width_end.div_ceil(32);
    let stride = head_words + 8 * lane_words;
    let mut words = vec![0u32; stride];
    let put = |words: &mut Vec<u32>, at: usize, n: u32, v: u64| {
        for b in 0..n as usize {
            if v >> b & 1 == 1 {
                words[(at + b) / 32] |= 1 << ((at + b) % 32);
            }
        }
    };
    words[0] = stride as u32;
    words[1] = a | i << 8 | (lane_words as u32) << 16;
    words[2] = SCALE.to_bits();
    words[3] = OFFSET.to_bits();
    let mut at = 128;
    for address in &addresses {
        put(&mut words, at, a, *address);
        at += a as usize;
    }
    for initial in &initials {
        put(&mut words, at, i, *initial);
        at += i as usize;
    }
    for width in &widths {
        at += *width as usize;
        put(&mut words, at, 1, 1);
        at += 1;
    }
    let mut lanes = vec![vec![0u32; lane_words]; 8];
    let mut bit = 0;
    for (g, width) in widths.iter().enumerate() {
        for (j, lane) in lanes.iter_mut().enumerate() {
            put(lane, bit, *width, codes[8 * g + j]);
        }
        bit += *width as usize;
    }
    for (j, lane) in lanes.iter().enumerate() {
        for (r, word) in lane.iter().enumerate() {
            words[head_words + 8 * r + j] = *word;
        }
    }
    words
}

/// A quantised value for every frame: never negative, not smooth enough to fit a line.
fn quantised(frames: usize, phase: i64) -> Vec<i64> {
    (0..frames as i64)
        .map(|f| 40 + (f * 7 + phase) % 23 + f * f / 9)
        .collect()
}

fn floats(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}

/// What a built resource holds: its root's word at 0x6C, its clip's type, its DOFs' types and
/// tracks, its block's directory sections and constants, and the frames of its one block.
struct Spec {
    root: u32,
    clip: u32,
    frames: u16,
    dofs: Vec<(u32, &'static str)>,
    sections: Vec<(usize, Vec<u16>)>,
    constants: Vec<u32>,
}

impl Default for Spec {
    /// A float, a three-float vector and a float whose channel is in section 10: a constant in
    /// section 2, a constant vector in section 1, and the delta code of [`quantised`] in section 10.
    fn default() -> Self {
        let mut constants = floats(&[1.0, 2.0, 3.0, 0.5]);
        constants.extend(delta_channel(&quantised(usize::from(FRAMES), 0)));
        Self {
            root: 2,
            clip: 1,
            frames: FRAMES,
            dofs: vec![
                (4, "genericControl"),
                (2, "boneTranslation"),
                (4, "facialTranslation"),
            ],
            sections: vec![(1, vec![1 << 2]), (2, vec![0]), (10, vec![2 << 2])],
            constants,
        }
    }
}

/// Where a built resource's objects are.
struct Built {
    bytes: Vec<u8>,
    set: usize,
    animation: usize,
    clip: usize,
    block: usize,
    /// The clip's tags object's first tag.
    tag: usize,
}

fn payload(spec: &Spec) -> (Vec<u8>, usize) {
    let mut p: Vec<u8> = spec
        .constants
        .iter()
        .flat_map(|c| c.to_le_bytes())
        .collect();
    let constant_size = p.len();
    let mut runs = vec![Vec::new(); SECTIONS];
    for (section, run) in &spec.sections {
        runs[*section] = run.clone();
    }
    let pad = (spec.dofs.len() * 4) as u16;
    let mut words: Vec<u16> = runs.iter().map(|r| r.len() as u16).collect();
    for run in &runs {
        words.extend(run);
        words.resize(words.len() + (4 - run.len() % 4) % 4, pad);
    }
    p.extend(words.iter().flat_map(|w| w.to_le_bytes()));
    (p, constant_size)
}

/// A record at `at` of `kind`, index `index`, named `name`, whose object is `back`.
fn record(b: &mut Bytes, at: usize, back: usize, kind: u16, index: u16, name: &str) {
    b.empty_rel(at + 0x10);
    b.empty_rel(at + 0x20);
    b.ptr(at + 0x30, back);
    b.u16(at + 0x38, kind);
    b.u16(at + 0x3A, index);
    b.u32(at + 0x3C, 0x1234_5678);
    b.u32(at + 0x40, hash(name));
}

/// The properties object and tags object of the owner at `owner`, from `properties_at`: a
/// `guid` property with a GUID, and a `foot` tag with a `right` and a `heel` bool and a string.
/// Returns the tags object and its tag.
fn owner_metadata(b: &mut Bytes, owner: usize, properties_at: usize) -> (usize, usize) {
    let tags_at = properties_at + 0x28;
    b.u64(properties_at + 0x10, 1);
    let property = b.alloc(0x48);
    b.rel(properties_at + 0x18, property, 1);
    record(b, property, properties_at, 1, 0xFFFF, "guid");
    let guid = b.alloc(0x20);
    b.u32(guid, hash("guid"));
    b.u16(guid + 4, 14);
    b.u16(guid + 6, 0xFFFF);
    b.put(guid + 0x10, &[0xA5; 16]);
    b.rel(property + 0x20, guid, 1);

    b.ptr(tags_at + 0x10, owner);
    let tag = b.alloc(0x50);
    b.rel(tags_at + 0x18, tag, 1);
    record(b, tag, tags_at, 2, 0, "foot");
    b.f32(tag + 0x48, 0.25);
    b.f32(tag + 0x4C, 0.5);
    let bools = b.alloc(24);
    for (i, (name, value)) in [("right", 1), ("heel", 0)].into_iter().enumerate() {
        let a = bools + 12 * i;
        b.u32(a, hash(name));
        b.u16(a + 4, 3);
        b.u16(a + 6, 0xFFFF);
        b.u32(a + 8, value);
    }
    b.rel(tag + 0x10, bools, 2);
    let string = b.alloc(0x20);
    let text = b.text("step");
    b.u32(string, hash("name"));
    b.u16(string + 4, 4);
    b.u16(string + 6, 0xFFFF);
    b.u64(string + 0x18, (text as i64 - (string as i64 + 0x18)) as u64);
    b.rel(tag + 0x20, string, 1);
    (tags_at, tag)
}

fn build(spec: &Spec) -> Built {
    let mut b = Bytes::default();
    let root = b.alloc(if spec.root == 2 { 0xB0 } else { 0x70 });
    b.u32(root + 0x6C, spec.root);
    let set = if spec.root == 2 {
        b.name(root + 0x38, DICTIONARY);
        b.u32(root + 0x68, hash(DICTIONARY));
        b.alloc(0x80)
    } else {
        root
    };
    let set_name = format!("{DICTIONARY}-0");
    b.inline_name(set + 0x38, &set_name);
    b.u32(set + 0x68, hash(&set_name));

    b.u64(root + 0x60, ROOT_WORD_60);
    if spec.root == 2 {
        b.u64(root + 0x98, ROOT_WORD_98);
    }
    let animation = b.alloc(0x70);
    b.u32(animation + 0x10, 3);
    b.ptr(animation + 0x18, set);
    b.u16(animation + 0x20, 0x140);
    b.u16(animation + 0x22, 2);
    b.u32(animation + 0x2C, SIGNATURE);
    b.u16(animation + 0x24, spec.frames);
    b.u16(animation + 0x26, FRAMES_PER_CHUNK);
    b.f32(animation + 0x28, DURATION);
    let name = b.text(&format!("pack:/{ANIMATION}"));
    b.ptr(animation + 0x30, name);
    let dofs = b.alloc(12 * spec.dofs.len());
    for (i, (kind, track)) in spec.dofs.iter().enumerate() {
        let at = dofs + 12 * i;
        b.u32(at, hash("SKEL_ROOT").wrapping_add(i as u32));
        b.u32(at + 4, hash(track));
        b.u32(at + 8, *kind);
    }
    b.array(animation + 0x60, dofs, spec.dofs.len() as u16);

    let (data, constant_size) = payload(spec);
    let block = b.alloc(0x20 + data.len());
    let field = |name: &str| match name {
        "data_size" => offset_of!(Raw<CrBlockStream>, data_size),
        "constant_size" => offset_of!(Raw<CrBlockStream>, constant_size),
        "channel_offset" => offset_of!(Raw<CrBlockStream>, channel_offset),
        "slop_size" => offset_of!(Raw<CrBlockStream>, slop_size),
        "num_frames" => offset_of!(Raw<CrBlockStream>, num_frames),
        _ => offset_of!(Raw<CrBlockStream>, segment_size),
    };
    b.u32(block + field("data_size"), data.len() as u32);
    b.u32(block + field("constant_size"), constant_size as u32);
    b.u32(block + field("channel_offset"), data.len() as u32 + 0x20);
    b.u16(block + field("slop_size"), 2);
    b.u16(block + field("num_frames"), spec.frames);
    b.0[block + field("segment_size")] = 11;
    b.put(block + 0x20, &data);
    let list = b.alloc(8);
    b.ptr(list, block);
    b.array(animation + 0x50, list, 1);
    b.u32(animation + 0x48, data.len() as u32 + 0x20);

    let entry = b.alloc(0x20);
    b.u32(entry, hash(ANIMATION));
    b.ptr(entry + 8, animation);
    b.map(set + 0x48, 3, &[(u64::from(hash(ANIMATION)), entry)], 0x10);
    if spec.root != 2 {
        return Built {
            bytes: b.0,
            set,
            animation,
            clip: 0,
            block,
            tag: 0,
        };
    }

    let list = b.alloc(16);
    b.ptr(list, set);
    b.u32(list + 8, hash(&set_name));
    b.u32(list + 12, 1);
    b.array(root + 0x70, list, 1);
    let key_entry = b.alloc(0x10);
    b.u32(key_entry, hash(ANIMATION));
    b.map(
        root + 0x80,
        11,
        &[(u64::from(hash(ANIMATION)), key_entry)],
        8,
    );

    let key = u64::from(hash(DICTIONARY)) << 32 | u64::from(hash("clip_a"));
    let clip = b.alloc(if spec.clip == 2 { 0x80 } else { 0xA0 });
    b.u64(clip + 0x10, 1);
    b.ptr(clip + 0x18, root);
    b.u32(clip + 0x20, spec.clip);
    b.name(clip + 0x28, "pack:/clip_a.clip");
    b.u64(clip + 0x58, key);
    if spec.clip == 2 {
        let group = b.alloc(0x18);
        let entries = b.alloc(2 * 0x38);
        b.array(clip + 0x68, group, 1);
        b.f32(clip + 0x78, 2.0);
        b.array(group, entries, 2);
        b.f32(group + 0x10, 1.5);
        for (i, (start, end, rate)) in [(0.0, 1.0, 1.0), (0.25, 0.75, 2.0)].into_iter().enumerate()
        {
            let e = entries + i * 0x38;
            b.ptr(e, animation);
            b.u32(e + 8, hash(ANIMATION));
            b.f32(e + 0x28, start);
            b.f32(e + 0x2C, end);
            b.f32(e + 0x30, rate);
        }
        let points = b.alloc(3 * 16);
        for (i, [x, y]) in WARP.into_iter().enumerate() {
            b.f32(points + 16 * i, x);
            b.f32(points + 16 * i + 4, y);
        }
        b.array(entries + 0x38 + 0x10, points, 3);
    } else {
        b.ptr(clip + 0x68, animation);
        b.u32(clip + 0x70, hash(ANIMATION));
        b.f32(clip + 0x94, DURATION);
        b.f32(clip + 0x98, 1.0);
    }
    let entry = b.alloc(0x20);
    b.u64(entry, key);
    b.ptr(entry + 8, clip);
    b.map(root + 0x48, 11, &[(key, entry)], 0x10);

    let properties = b.alloc(0x58);
    b.ptr(clip + 0x48, properties);
    let (tags, tag) = owner_metadata(&mut b, clip, properties);
    b.ptr(clip + 0x40, tags);
    let end = b.alloc(0);
    b.ptr(root + 0xA0, properties);
    b.u32(root + 0xA8, (end - properties) as u32);
    Built {
        bytes: b.0,
        set,
        animation,
        clip,
        block,
        tag,
    }
}

fn the_animation(resource: &AnimResource) -> &Animation {
    &resource.sets[0].animations[0]
}

fn pointer_at(bytes: &[u8], at: usize) -> usize {
    (u64::from_le_bytes(bytes[at..at + 8].try_into().expect("eight bytes")) - VIRTUAL) as usize
}

#[test]
fn a_clip_dictionary_states_its_clip_its_set_and_the_animation_they_play() {
    let read = read(&build(&Spec::default()).bytes).expect("reads");
    assert_eq!(read.root, RootKind::ClipDictionary);
    assert_eq!(read.root_word, 2);
    assert_eq!(read.name, DICTIONARY);
    assert_eq!(read.key, hash(DICTIONARY));
    assert_eq!(read.sets.len(), 1);
    assert_eq!(read.sets[0].name, format!("{DICTIONARY}-0"));
    assert_eq!(read.sets[0].key, hash(&read.sets[0].name));
    assert_eq!(read.animation_keys, [hash(ANIMATION)]);
    assert!(read.external_sets.is_empty());

    let animation = the_animation(&read);
    assert_eq!(animation.name, format!("pack:/{ANIMATION}"));
    assert_eq!(animation.key, hash(ANIMATION));
    assert_eq!((animation.frames, animation.duration), (FRAMES, DURATION));
    assert_eq!(animation.rate(), Some(f32::from(FRAMES - 1) / DURATION));
    let tracks: Vec<_> = animation.dofs.iter().map(Dof::track_name).collect();
    assert_eq!(
        tracks,
        [
            Some("genericControl"),
            Some("boneTranslation"),
            Some("facialTranslation")
        ]
    );
    assert_eq!(animation.dofs[1].kind, 2);
    assert_eq!(animation.blocks[0].sections[DELTA_SECTION], 1);
    let built = build(&Spec::default());
    let at = built.block + offset_of!(Raw<CrBlockStream>, data_size);
    let data_size = u32::from_le_bytes(built.bytes[at..at + 4].try_into().expect("four bytes"));
    assert_eq!(animation.max_block_size, data_size + 0x20);
    assert_eq!(
        (
            animation.references,
            animation.flags,
            animation.project_flags,
            animation.signature
        ),
        (3, 0x140, 2, SIGNATURE)
    );
    assert_eq!(
        (read.word_at_60, read.word_at_98),
        (ROOT_WORD_60, Some(ROOT_WORD_98))
    );

    assert_eq!(read.clips.len(), 1);
    let clip = &read.clips[0];
    assert_eq!(clip.name, "pack:/clip_a.clip");
    assert_eq!(clip.key >> 32, u64::from(hash(DICTIONARY)));
    assert_eq!(
        clip.play,
        ClipPlay::One {
            animation: AnimAt {
                set: 0,
                animation: 0
            },
            key: hash(ANIMATION),
            start: 0.0,
            end: DURATION,
            rate: 1.0
        }
    );
}

#[test]
fn a_clip_states_its_properties_and_its_tags_with_their_typed_attributes() {
    let read = read(&build(&Spec::default()).bytes).expect("reads");
    let clip = &read.clips[0];
    assert_eq!(clip.properties.len(), 1);
    let property = &clip.properties[0];
    assert_eq!(known_name(property.name), Some("guid"));
    assert_eq!(property.phases, None);
    assert_eq!(
        property.attributes[0].value,
        AttributeValue::Guid([0xA5; 16])
    );

    assert_eq!(clip.tags.len(), 1);
    let tag = &clip.tags[0];
    assert_eq!(known_name(tag.name), Some("foot"));
    assert_eq!(tag.phases, Some([0.25, 0.5]));
    let attributes: Vec<_> = tag
        .attributes
        .iter()
        .map(|a| (known_name(a.name), a.value.clone()))
        .collect();
    assert_eq!(
        attributes,
        [
            (Some("right"), AttributeValue::Bool(true)),
            (Some("heel"), AttributeValue::Bool(false)),
            (Some("name"), AttributeValue::String("step".into())),
        ]
    );
    assert_eq!(known_name(hash("not a known word")), None);
}

#[test]
fn a_tag_whose_phase_is_not_finite_is_refused_by_name() {
    let built = build(&Spec::default());
    let mut b = Bytes(built.bytes);
    b.f32(built.tag + 0x4C, f32::INFINITY);
    assert!(matches!(
        read(&b.0).expect_err("refused"),
        AnimError::Metadata { why, .. } if why.contains("phases")
    ));
}

#[test]
fn an_attribute_of_a_type_no_record_holds_is_refused_by_name() {
    let built = build(&Spec::default());
    let mut b = Bytes(built.bytes);
    // The offset field of the tag's relative array of 12-byte attributes, at 0x10.
    let field = built.tag + 0x18;
    let offset = i64::from_le_bytes(b.0[field..field + 8].try_into().expect("eight bytes"));
    let attribute = (field as i64 + offset) as usize;
    b.u16(attribute + 4, 16);
    assert!(matches!(
        read(&b.0).expect_err("refused"),
        AnimError::Metadata { why, .. } if why.contains("type 16")
    ));
}

#[test]
fn a_section_10_channel_decodes_to_its_values_at_every_frame() {
    let read = read(&build(&Spec::default()).bytes).expect("reads");
    let animation = the_animation(&read);
    assert!(animation.in_delta_section(2));
    assert!(!animation.in_delta_section(1));
    let rate = animation.rate().expect("a rate");
    for (frame, q) in quantised(usize::from(FRAMES), 0).into_iter().enumerate() {
        let value = animation.sample(2, frame as f32 / rate).expect("decodes");
        let expected = SCALE * q as f32 + OFFSET;
        assert!(
            (value.as_slice()[0] - expected).abs() < 1e-3,
            "frame {frame}: {:?} against {expected}",
            value.as_slice()
        );
    }
    assert_eq!(
        animation.sample(0, 0.5).expect("a constant").as_slice(),
        [0.5]
    );
    assert_eq!(
        animation
            .sample(1, 0.0)
            .expect("a constant vector")
            .as_slice(),
        [1.0, 2.0, 3.0]
    );
    assert!(matches!(
        animation.sample(3, 0.0),
        Err(AnimError::NoDof {
            dof: 3,
            count: 3,
            ..
        })
    ));
}

#[test]
fn sampling_each_frame_of_a_block_at_once_gives_what_each_sample_gives() {
    let small: Vec<Vec<i64>> = (0..3).map(|p| quantised(usize::from(FRAMES), p)).collect();
    let mut constants = Vec::new();
    for values in &small {
        constants.extend(delta_channel(values));
    }
    let spec = Spec {
        dofs: vec![(0, "boneRotation")],
        sections: vec![(7, vec![3]), (10, vec![0, 1, 2])],
        constants,
        ..Spec::default()
    };
    let read = read(&build(&spec).bytes).expect("reads");
    let animation = the_animation(&read);
    let rate = animation.rate().expect("a rate");
    // Whole frames, a time between two and one past the end, out of order.
    let mut times: Vec<f32> = (0..FRAMES).map(|f| f32::from(f) / rate).collect();
    times.extend([0.5 / rate, animation.duration * 2.0, 0.0]);
    let each = animation
        .sample_each(0, times.iter().copied())
        .expect("decodes");
    assert_eq!(each.len(), times.len());
    for (time, value) in times.iter().zip(&each) {
        assert_eq!(
            *value,
            animation.sample(0, *time).expect("decodes"),
            "at {time}"
        );
    }
    assert!(matches!(
        animation.sample_each(1, [0.0]),
        Err(AnimError::NoDof { dof: 1, .. })
    ));
}

#[test]
fn a_mover_track_is_named_and_a_track_word_no_name_hashes_to_is_carried_and_labelled_by_its_word() {
    let spec = Spec {
        dofs: vec![
            (4, "notATrackName"),
            (2, "moverAbsoluteTranslation"),
            (4, "facialTranslation"),
        ],
        ..Spec::default()
    };
    let read = read(&build(&spec).bytes).expect("reads");
    let animation = the_animation(&read);
    let unknown = animation.dofs[0];
    assert_eq!(unknown.track, hash("notATrackName"));
    assert_eq!(unknown.track_name(), None);
    assert_eq!(
        unknown.track_label(),
        format!("hash_{:08X}", hash("notATrackName"))
    );
    assert_eq!(
        animation.sample(0, 0.5).expect("a constant").as_slice(),
        [0.5]
    );
    let mover = animation.dofs[1];
    assert_eq!(mover.track_name(), Some("moverAbsoluteTranslation"));
    assert_eq!(mover.track_label(), "moverAbsoluteTranslation");
    assert_eq!(
        animation.sample(1, 0.0).expect("a vector").as_slice(),
        [1.0, 2.0, 3.0]
    );
}

#[test]
fn a_last_segment_of_one_frame_is_its_initial_alone() {
    let frames = 129u16;
    let mut constants = floats(&[1.0, 2.0, 3.0, 0.5]);
    constants.extend(delta_channel(&quantised(usize::from(frames), 5)));
    let spec = Spec {
        frames,
        constants,
        ..Spec::default()
    };
    let read = read(&build(&spec).bytes).expect("reads");
    let animation = the_animation(&read);
    let last = animation.sample(2, animation.duration).expect("decodes");
    let q = quantised(usize::from(frames), 5)[usize::from(frames) - 1];
    assert!((last.as_slice()[0] - (SCALE * q as f32 + OFFSET)).abs() < 1e-3);
}

#[test]
fn section_10_components_are_placed_before_a_quaternion_is_reconstructed() {
    let small: Vec<Vec<i64>> = (0..3).map(|p| quantised(usize::from(FRAMES), p)).collect();
    let mut constants = Vec::new();
    for values in &small {
        constants.extend(delta_channel(values));
    }
    let spec = Spec {
        dofs: vec![(0, "boneRotation")],
        sections: vec![(7, vec![3]), (10, vec![0, 1, 2])],
        constants,
        ..Spec::default()
    };
    let read = read(&build(&spec).bytes).expect("reads");
    let animation = the_animation(&read);
    let value = animation.sample(0, 0.0).expect("decodes");
    let xyz: Vec<f32> = small.iter().map(|v| SCALE * v[0] as f32 + OFFSET).collect();
    let squared = 1.0 - xyz.iter().map(|x| x * x).sum::<f32>();
    let w = if squared > f32::MIN_POSITIVE {
        squared.sqrt()
    } else {
        0.0
    };
    assert_eq!(value.as_slice(), [xyz[0], xyz[1], xyz[2], w]);
}

#[test]
fn a_section_10_segment_that_does_not_end_on_the_next_initial_is_refused() {
    let mut spec = Spec::default();
    let mut channel = delta_channel(&quantised(usize::from(FRAMES), 0));
    // The second initial's lowest bit: the address table holds two fields of `A` bits first.
    let a = channel[1] & 0xFF;
    let bit = 128 + 2 * a as usize + (channel[1] >> 8 & 0xFF) as usize;
    channel[bit / 32] ^= 1 << (bit % 32);
    spec.constants.truncate(4);
    spec.constants.extend(channel);
    assert!(matches!(
        read(&build(&spec).bytes).expect_err("refused"),
        AnimError::Delta { dof: 2, why, .. } if why.contains("segment 0")
    ));
}

#[test]
fn a_block_whose_channels_do_not_consume_its_constants_is_refused() {
    let mut spec = Spec::default();
    spec.constants.push(0);
    assert!(matches!(
        read(&build(&spec).bytes).expect_err("refused"),
        AnimError::Directory { why, .. } if why.contains("consume")
    ));
}

#[test]
fn a_clip_of_groups_states_each_entry_and_its_window() {
    let read = read(
        &build(&Spec {
            clip: 2,
            ..Spec::default()
        })
        .bytes,
    )
    .expect("reads");
    let ClipPlay::Groups { total, groups } = &read.clips[0].play else {
        panic!("{:?}", read.clips[0].play);
    };
    assert_eq!(*total, 2.0);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].duration, 1.5);
    let windows: Vec<_> = groups[0]
        .entries
        .iter()
        .map(|e| (e.animation, e.key, e.start, e.end, e.rate))
        .collect();
    let at = Some(AnimAt {
        set: 0,
        animation: 0,
    });
    let key = hash(ANIMATION);
    assert_eq!(
        windows,
        [(at, key, 0.0, 1.0, 1.0), (at, key, 0.25, 0.75, 2.0)]
    );
    let warps: Vec<&[[f32; 2]]> = groups[0]
        .entries
        .iter()
        .map(|e| &e.warp_points[..])
        .collect();
    assert_eq!(warps, [&[][..], &WARP[..]]);
}

#[test]
fn a_clip_whose_entry_points_at_no_animation_of_the_sets_is_unresolved() {
    let built = build(&Spec {
        clip: 2,
        ..Spec::default()
    });
    let mut bytes = built.bytes;
    let group = pointer_at(&bytes, built.clip + 0x68);
    let entry = pointer_at(&bytes, group);
    bytes[entry..entry + 8].copy_from_slice(&(VIRTUAL + built.set as u64).to_le_bytes());
    let read = read(&bytes).expect("reads");
    assert_eq!(read.clips[0].play, ClipPlay::Unresolved);
}

/// [`build`]'s dictionary with a second set-list entry `{0, key, word}`, and the clip's first entry
/// made null.
fn with_external_set(key: u32, word: u32) -> Vec<u8> {
    let built = build(&Spec {
        clip: 2,
        ..Spec::default()
    });
    let mut b = Bytes(built.bytes);
    let first = pointer_at(&b.0, 0x70);
    let set_entry: [u8; 16] = b.0[first..first + 16].try_into().expect("sixteen bytes");
    let mut external = [0u8; 16];
    external[8..12].copy_from_slice(&key.to_le_bytes());
    external[12..].copy_from_slice(&word.to_le_bytes());
    b.list(0x70, 16, &[&set_entry, &external]);
    let group = pointer_at(&b.0, built.clip + 0x68);
    let entry = pointer_at(&b.0, group);
    b.u64(entry, 0);
    b.0
}

#[test]
fn a_set_stored_in_another_body_is_named_by_key_and_its_clip_entries_by_their_animation_key() {
    let read = read(&with_external_set(hash("dict_a-1"), 0)).expect("reads");
    assert_eq!(read.external_sets, [hash("dict_a-1")]);
    let ClipPlay::Groups { groups, .. } = &read.clips[0].play else {
        panic!("{:?}", read.clips[0].play);
    };
    assert_eq!(groups[0].entries[0].animation, None);
    assert_eq!(groups[0].entries[0].key, hash(ANIMATION));
    assert!(groups[0].entries[1].animation.is_some());
}

#[test]
fn an_external_set_entry_with_a_word_where_0_was_measured_is_refused() {
    assert!(matches!(
        read(&with_external_set(hash("dict_a-1"), 1)).expect_err("refused"),
        AnimError::SetList { index: 1, .. }
    ));
}

#[test]
fn a_null_clip_entry_in_a_dictionary_with_no_external_set_is_refused() {
    let built = build(&Spec {
        clip: 2,
        ..Spec::default()
    });
    let mut bytes = built.bytes;
    let group = pointer_at(&bytes, built.clip + 0x68);
    let entry = pointer_at(&bytes, group);
    bytes[entry..entry + 8].fill(0);
    assert!(matches!(
        read(&bytes).expect_err("refused"),
        AnimError::External { .. }
    ));
}

#[test]
fn a_dictionary_that_stores_none_of_its_sets_is_refused() {
    let mut b = Bytes(build(&Spec::default()).bytes);
    let mut external = [0u8; 16];
    external[8..12].copy_from_slice(&hash("dict_a-0").to_le_bytes());
    b.list(0x70, 16, &[&external]);
    assert!(matches!(read(&b.0).expect_err("refused"), AnimError::NoSet));
}

#[test]
fn a_dictionary_of_root_word_3_is_a_clip_dictionary() {
    let mut bytes = build(&Spec::default()).bytes;
    bytes[0x6C] = 3;
    let read = read(&bytes).expect("reads");
    assert_eq!((read.root, read.root_word), (RootKind::ClipDictionary, 3));
}

#[test]
fn an_animation_set_standing_alone_is_a_root_with_no_clips() {
    let read = read(
        &build(&Spec {
            root: 0,
            ..Spec::default()
        })
        .bytes,
    )
    .expect("reads");
    assert_eq!(read.root, RootKind::AnimationSet);
    assert_eq!(read.name, format!("{DICTIONARY}-0"));
    assert_eq!(read.key, hash(&read.name));
    assert!(read.clips.is_empty());
    assert_eq!(the_animation(&read).name, format!("pack:/{ANIMATION}"));
    assert_eq!((read.word_at_60, read.word_at_98), (ROOT_WORD_60, None));
}

fn refused(bytes: &[u8]) -> AnimError {
    read(bytes).expect_err("refused")
}

#[test]
fn a_section_cut_short_is_refused() {
    let bytes = build(&Spec::default()).bytes;
    assert!(matches!(
        refused(&bytes[..0x80]),
        AnimError::PastEnd { what: "root", .. }
    ));
}

#[test]
fn a_root_with_a_byte_where_every_root_holds_zero_is_refused() {
    for at in [0x03, 0x20] {
        let mut bytes = build(&Spec::default()).bytes;
        bytes[at] = 1;
        assert!(matches!(refused(&bytes), AnimError::RootNotZero));
    }
}

#[test]
fn a_root_of_another_kind_is_refused() {
    let mut bytes = build(&Spec::default()).bytes;
    bytes[0x6C] = 1;
    assert!(matches!(refused(&bytes), AnimError::RootKind(1)));
}

#[test]
fn a_root_whose_key_is_not_its_name_s_hash_is_refused() {
    let mut bytes = build(&Spec::default()).bytes;
    bytes[0x68] ^= 1;
    assert!(matches!(
        refused(&bytes),
        AnimError::Key { what: "root", .. }
    ));
}

#[test]
fn a_clip_whose_key_is_not_its_names_hashes_is_refused() {
    let built = build(&Spec::default());
    let mut bytes = built.bytes;
    bytes[built.clip + 0x58] ^= 1;
    assert!(matches!(refused(&bytes), AnimError::ClipKey { .. }));
}

#[test]
fn a_pointer_into_the_physical_section_is_refused() {
    let built = build(&Spec::default());
    let mut bytes = built.bytes;
    let list = pointer_at(&bytes, 0x70);
    let physical = 0x6000_0000 + built.set as u64;
    bytes[list..list + 8].copy_from_slice(&physical.to_le_bytes());
    assert!(matches!(
        refused(&bytes),
        AnimError::Pointer {
            what: "animation set",
            ..
        }
    ));
}

#[test]
fn a_set_list_that_names_one_set_many_times_is_refused() {
    const NAMED: u16 = 1024;
    let built = build(&Spec::default());
    let mut b = Bytes(built.bytes);
    let list = b.alloc(16 * usize::from(NAMED));
    for i in 0..usize::from(NAMED) {
        b.ptr(list + 16 * i, built.set);
        b.u32(list + 16 * i + 8, hash(&format!("{DICTIONARY}-0")));
        b.u32(list + 16 * i + 12, 1);
    }
    b.array(0x70, list, NAMED);
    assert!(matches!(refused(&b.0), AnimError::Aliased { .. }));
}

#[test]
fn a_map_entry_in_a_bucket_its_key_does_not_name_is_refused() {
    let mut bytes = build(&Spec::default()).bytes;
    let buckets = pointer_at(&bytes, 0x48);
    let entry = (0..11)
        .find_map(|b| {
            let p = u64::from_le_bytes(bytes[buckets + 8 * b..][..8].try_into().unwrap());
            (p != 0).then(|| (p - VIRTUAL) as usize)
        })
        .expect("an entry");
    bytes[entry] = bytes[entry].wrapping_add(1);
    assert!(matches!(
        refused(&bytes),
        AnimError::Map { what: "clip", .. }
    ));
}

#[test]
fn an_animation_key_the_key_set_does_not_hold_is_refused() {
    let mut bytes = build(&Spec::default()).bytes;
    bytes[0x80 + 0x10] = 0;
    let buckets = pointer_at(&bytes, 0x80);
    bytes[buckets..buckets + 8 * 11].fill(0);
    assert!(matches!(
        refused(&bytes),
        AnimError::Map {
            what: "animation key",
            ..
        }
    ));
}

#[test]
fn a_clip_of_an_unknown_type_is_refused() {
    let built = build(&Spec::default());
    let mut bytes = built.bytes;
    bytes[built.clip + 0x20] = 3;
    assert!(matches!(
        refused(&bytes),
        AnimError::ClipType { value: 3, .. }
    ));
}

#[test]
fn an_animation_its_set_does_not_own_is_refused() {
    let built = build(&Spec::default());
    let mut bytes = built.bytes;
    let at = built.animation + 0x18;
    bytes[at..at + 8].copy_from_slice(&VIRTUAL.to_le_bytes());
    assert!(matches!(refused(&bytes), AnimError::Owner { .. }));
}

#[test]
fn a_lone_set_root_whose_map_objects_it_does_not_own_is_refused_as_no_animation_set() {
    let built = build(&Spec {
        root: 0,
        ..Spec::default()
    });
    let mut bytes = built.bytes;
    let at = built.animation + 0x18;
    let elsewhere = VIRTUAL + built.animation as u64;
    bytes[at..at + 8].copy_from_slice(&elsewhere.to_le_bytes());
    let word = u32::from_le_bytes(bytes[0x70..0x74].try_into().expect("four bytes"));
    let error = refused(&bytes);
    assert!(
        matches!(error, AnimError::NotAnimationSet { word: w } if w == word),
        "{error}"
    );
    assert!(error.to_string().contains("no animation set"), "{error}");
}

#[test]
fn a_block_count_its_frames_do_not_give_is_refused() {
    let built = build(&Spec::default());
    let mut bytes = built.bytes;
    bytes[built.animation + 0x24] = 200;
    assert!(matches!(refused(&bytes), AnimError::Blocks { .. }));
}

#[test]
fn a_dof_whose_directory_entries_do_not_fit_its_type_is_refused() {
    let mut spec = Spec::default();
    spec.dofs[0].0 = 3;
    let error = refused(&build(&spec).bytes);
    assert!(
        matches!(&error, AnimError::Directory { block: 0, why, .. } if why.contains("DOF 0 has type 3")),
        "{error}"
    );
}

#[test]
fn a_channel_in_section_6_or_9_is_refused_by_name() {
    for section in UNKNOWN_SECTIONS {
        let mut spec = Spec::default();
        spec.dofs[1].0 = 0;
        spec.sections
            .push((section, vec![(1 << 2) | 1, (1 << 2) | 2, (1 << 2) | 3]));
        assert!(matches!(
            refused(&build(&spec).bytes),
            AnimError::Section { section: s, count: 3, .. } if s == section
        ));
    }
}

#[test]
fn gta_v_s_nine_count_directory_reads_these_blocks_wrong_without_an_error() {
    let built = build(&Spec::default());
    let aligned = Aligned::from_bytes(&built.bytes);
    let stream = Section {
        image: Image::new(aligned.bytes(), built.bytes.len()),
        left: Cell::new(built.bytes.len()),
    }
    .block_stream(built.block)
    .expect("GTA V's header reads");
    let walk = ChannelWalk::new(&stream).expect("the 11 counts read as 9");
    let channels = walk.channels().expect("walks");
    let rewritten = read(&built.bytes).expect("reads");
    let block = &the_animation(&rewritten).blocks[0].stream;
    let right = ChannelWalk::new(block)
        .expect("9 counts")
        .channels()
        .expect("walks");
    assert_ne!(channels, right);
}

/// [`build`]'s dictionary stored as an RSC8 stores it: an end region at root 0x08 of the record
/// `{0, 1, 0}`, `names`, a word and the set list, then a pointer and a zero to the section's end.
fn stored(names: &[&str]) -> Vec<u8> {
    stored_around(names, &[])
}

/// [`stored`] with `after` between the set list and the pointer that ends the region.
fn stored_around(names: &[&str], after: &[&str]) -> Vec<u8> {
    let mut b = Bytes(build(&Spec::default()).bytes);
    let set_entry: [u8; 16] = {
        let at = pointer_at(&b.0, 0x70);
        b.0[at..at + 16].try_into().expect("sixteen bytes")
    };
    let start = b.alloc(0x18);
    b.u64(start + 8, 1);
    b.ptr(0x08, start);
    for name in names {
        let at = b.0.len().next_multiple_of(8);
        b.0.resize(at, 0);
        b.0.extend(name.as_bytes());
        b.0.push(0);
    }
    let word = b.alloc(16);
    b.u32(word, 0x0BAD_F00D);
    let list = b.alloc(16);
    b.put(list, &set_entry);
    b.array(0x70, list, 1);
    for name in after {
        b.0.extend(name.as_bytes());
        b.0.push(0);
    }
    let last = b.alloc(16);
    b.ptr(last, 0xB0);
    b.0
}

#[test]
fn a_stored_resource_whose_name_follows_its_set_list_states_it() {
    let read = read_stored(&stored_around(&[], &["pack:/made_up"])).expect("reads");
    assert_eq!(read.end_region.expect("a region").names, ["pack:/made_up"]);
    let mut bytes = stored_around(&[], &["pack:/made_up"]);
    let start = pointer_at(&bytes, 0x70) + 16;
    bytes[start + 1] = 0x80;
    assert!(matches!(read_stored(&bytes), Err(AnimError::EndRegion(_))));
}

#[test]
fn a_stored_resource_states_its_end_region_s_names_and_word() {
    let read = read_stored(&stored(&["first_name", "second"])).expect("reads");
    let region = read.end_region.expect("an end region");
    assert_eq!(region.names, ["first_name", "second"]);
    assert_eq!(region.word, 0x0BAD_F00D);
    assert_eq!(read.block_map, Some([1, 0]));
    assert!(read_stored(&stored(&[]))
        .expect("reads")
        .end_region
        .expect("a region")
        .names
        .is_empty());
}

#[test]
fn a_stored_resource_whose_end_region_does_not_open_with_its_record_is_refused() {
    let mut bytes = stored(&[]);
    let start = pointer_at(&bytes, 0x08);
    bytes[start + 8] = 2;
    assert!(matches!(
        read_stored(&bytes).expect_err("refused"),
        AnimError::EndRegion(_)
    ));
}

/// Where a built motion dictionary's objects are.
struct Motions {
    bytes: Vec<u8>,
    /// The kind-1 motion and its first time warp.
    first: usize,
    warp: usize,
}

const MOTIONS: &str = "motions_a";
const CLIP_DICTIONARY: &str = "dict_b";

/// A motion dictionary drawing on two clips of [`CLIP_DICTIONARY`]: a kind-1 motion `idle` with
/// phase curves and a range, and a kind-2 motion `walk` with one group, a database and a typed
/// parameter object.
fn build_motions() -> Motions {
    let mut b = Bytes::default();
    let root = b.alloc(0xB0);
    b.name(root + 0x38, MOTIONS);
    b.u32(root + 0x68, hash(MOTIONS));
    let mut entry = [0u8; 16];
    entry[8..12].copy_from_slice(&hash(CLIP_DICTIONARY).to_le_bytes());
    b.list(root + 0x70, 16, &[&entry]);
    let index = b.alloc(0x10);
    b.u32(index, hash(CLIP_DICTIONARY));
    b.map(
        root + 0x80,
        11,
        &[(u64::from(hash(CLIP_DICTIONARY)), index)],
        8,
    );
    let region = b.0.len().next_multiple_of(16);

    let clip = |leaf: &str| {
        let mut e = [0u8; 16];
        e[8..12].copy_from_slice(&hash(leaf).to_le_bytes());
        e[12..].copy_from_slice(&hash(CLIP_DICTIONARY).to_le_bytes());
        e
    };
    let typed = |b: &mut Bytes, kind: u32| {
        let at = b.alloc(0x40);
        b.u32(at + 8, kind);
        at
    };
    let points = |b: &mut Bytes, at: usize, pts: &[[f32; 2]]| {
        let bytes: Vec<Vec<u8>> = pts
            .iter()
            .map(|p| p.iter().flat_map(|v| v.to_le_bytes()).collect())
            .collect();
        let refs: Vec<&[u8]> = bytes.iter().map(Vec::as_slice).collect();
        b.list(at, 8, &refs);
    };
    let mut objects = Vec::new();
    let mut first = 0;
    let mut warp = 0;
    for (kind, name) in [(1u32, "idle"), (2, "walk")] {
        let motion = b.alloc(0x150);
        objects.push((u64::from(hash(name)), motion));
        b.u64(motion + 0x10, 1);
        b.ptr(motion + 0x18, root);
        b.u32(motion + 0x28, hash(name));
        b.u32(motion + 0x2C, hash(MOTIONS));
        b.name(motion + 0x30, &format!("pack:/{name}.motion"));
        b.u32(motion + 0x58, kind);
        let (c0, c1) = (clip("idle_a"), clip("idle_b"));
        b.list(motion + 0x60, 16, &[&c0, &c1]);
        let warps = b.alloc(2 * 0x28);
        b.array(motion + 0x70, warps, 2);
        for i in 0..2 {
            let w = warps + 0x28 * i;
            b.f32(w, 2.0);
            b.f32(w + 4, 2.0);
            points(&mut b, w + 8, &[[0.0, 0.0], [2.0, 2.0]]);
        }
        let weight: Vec<u8> =
            [1u32.to_le_bytes(), 1f32.to_le_bytes(), 0.5f32.to_le_bytes()].concat();
        let list = b.alloc(16);
        b.list(list, 12, &[&weight]);
        b.array(motion + 0x90, list, 1);
        let properties = b.alloc(0x58);
        b.ptr(motion + 0x20, properties);
        b.empty_rel(properties + 0x18);
        if kind == 2 {
            interface(&mut b, properties);
        }
        if kind == 1 {
            first = motion;
            warp = warps;
            let tags = properties + 0x28;
            b.ptr(tags + 0x10, motion);
            b.empty_rel(tags + 0x18);
            b.ptr(motion + 0x120, tags);
            let curves = typed(&mut b, 2);
            let curve = b.alloc(2 * 0x20);
            b.array(curves + 0x10, curve, 2);
            for i in 0..2 {
                let c = curve + 0x20 * i;
                points(&mut b, c, &[[4.0, 0.0], [5.0, 1.0], [1.0, 2.0]]);
                b.f32(c + 0x14, 5.0);
                b.f32(c + 0x18, 2.0);
                b.u32(c + 0x1C, 5);
            }
            b.ptr(motion + 0x130, curves);
            let range = typed(&mut b, 3);
            b.f32(range + 0x30, -1.5);
            b.f32(range + 0x34, 1.5);
            b.ptr(motion + 0x138, range);
        } else {
            let tags = b.alloc(0x30);
            b.ptr(tags + 0x10, motion);
            b.empty_rel(tags + 0x18);
            let group = b.alloc(0x38);
            b.array(motion + 0x120, group, 1);
            b.ptr(group, tags);
            let constants = typed(&mut b, 1);
            let values: Vec<u8> = [1.0f32, 2.0].iter().flat_map(|v| v.to_le_bytes()).collect();
            b.list(constants + 0x10, 4, &[&values[..4], &values[4..]]);
            b.ptr(group + 0x10, constants);
            let database = database(&mut b);
            b.ptr(group + 0x18, database);
            b.u32(group + 0x24, 2);
            b.u32(group + 0x2C, 1);
            let lists = typed(&mut b, 4);
            let pointer = b.alloc(8);
            b.ptr(pointer, lists);
            b.array(motion + 0x130, pointer, 1);
        }
    }
    let entries: Vec<(u64, usize)> = objects
        .iter()
        .map(|&(key, motion)| {
            let e = b.alloc(0x20);
            b.u64(e, key);
            b.ptr(e + 8, motion);
            (key, e)
        })
        .collect();
    b.map(root + 0x48, 11, &entries, 0x10);
    let end = b.alloc(0);
    b.ptr(root + 0xA0, region);
    b.u32(root + 0xA8, (end - region) as u32);
    Motions {
        bytes: b.0,
        first,
        warp,
    }
}

#[test]
fn a_motion_dictionary_states_its_motions_the_clips_they_draw_on_and_their_curves() {
    let read = read(&build_motions().bytes).expect("reads");
    assert_eq!(read.root, RootKind::MotionDictionary);
    assert_eq!(read.dictionaries, [hash(CLIP_DICTIONARY)]);
    assert!(read.sets.is_empty() && read.clips.is_empty());
    let mut motions = read.motions.clone();
    motions.sort_by_key(|m| m.kind);
    let [idle, walk] = &motions[..] else {
        panic!("{motions:?}");
    };
    assert_eq!(
        (idle.name.as_str(), idle.key),
        ("pack:/idle.motion", hash("idle"))
    );
    let clip = |leaf: &str| u64::from(hash(CLIP_DICTIONARY)) << 32 | u64::from(hash(leaf));
    assert_eq!(idle.clips, [clip("idle_a"), clip("idle_b")]);
    assert_eq!(idle.time_warps[0].points, [[0.0, 0.0], [2.0, 2.0]]);
    assert_eq!(
        idle.weight_lists,
        [vec![Weight {
            clip: 1,
            values: [1.0, 0.5]
        }]]
    );
    assert_eq!(idle.groups.len(), 1);
    let Typed::PhaseCurves(curves) = &idle.groups[0].curve else {
        panic!("{:?}", idle.groups[0].curve);
    };
    assert_eq!((curves.len(), curves[0].x1, curves[0].cycles), (2, 5.0, 5));
    let Typed::Range { lo, hi, .. } = idle.groups[0].range else {
        panic!("{:?}", idle.groups[0].range);
    };
    assert_eq!((lo, hi), (-1.5, 1.5));

    assert_eq!(walk.kind, 2);
    let group = &walk.groups[0];
    assert_eq!(
        (group.first_clip, group.clips, group.weight_lists),
        (0, 2, 1)
    );
    assert_eq!(group.curve, Typed::Constants(vec![1.0, 2.0]));
    assert_eq!(group.range, Typed::Rows(the_database()));
    let kinds: Vec<u32> = walk.parameter_objects.iter().map(|o| o.kind).collect();
    assert_eq!(kinds, [4]);
}

/// The motion named `name` in a built motion dictionary's root map.
fn motion_at(bytes: &[u8], name: &str) -> usize {
    let key = u64::from(hash(name));
    let buckets = pointer_at(bytes, 0x48);
    let mut entry = pointer_at(bytes, buckets + 8 * (key % 11) as usize);
    while u64::from_le_bytes(bytes[entry..entry + 8].try_into().expect("eight bytes")) != key {
        entry = pointer_at(bytes, entry + 0x10);
    }
    pointer_at(bytes, entry + 8)
}

#[test]
fn a_motion_s_parameter_items_and_its_parameter_objects_are_read_as_stated() {
    let mut b = Bytes(build_motions().bytes);
    let walk = motion_at(&b.0, "walk");
    // One parameter, so each of the two clips holds one item of points and four floats.
    let parameter: Vec<u8> = [hash("speed").to_le_bytes(), [2, 0, 3, 0]].concat();
    b.list(walk + 0xA0, 8, &[&parameter]);
    b.list(walk + 0xB0, 4, &[&20u32.to_le_bytes()]);
    b.list(walk + 0xC0, 4, &[&(-1f32).to_le_bytes()]);
    b.list(walk + 0xD0, 4, &[&1f32.to_le_bytes()]);
    let item = |b: &mut Bytes, clip: usize| {
        let at = b.alloc(0x20);
        let points: Vec<Vec<u8>> = [[0.0f32, 0.5], [clip as f32, 1.0]]
            .iter()
            .map(|p| p.iter().flat_map(|v| v.to_le_bytes()).collect())
            .collect();
        let refs: Vec<&[u8]> = points.iter().map(Vec::as_slice).collect();
        b.list(at, 8, &refs);
        for i in 0..4 {
            b.f32(at + 0x10 + 4 * i, (10 * clip + i) as f32);
        }
        at
    };
    let heads = b.alloc(32);
    for clip in 0..2 {
        let at = item(&mut b, clip);
        b.array(heads + 16 * clip, at, 1);
    }
    b.array(walk + 0x80, heads, 2);
    // The type-4 object holds a list of two words and a hash, and an array at 0x10 read raw.
    let lists = pointer_at(&b.0, pointer_at(&b.0, walk + 0x130));
    let words = b.alloc(8);
    b.u32(words, 5);
    b.u32(words + 4, 6);
    let list = b.alloc(16);
    b.array(list, words, 2);
    b.array(lists + 0x20, list, 1);
    b.list(lists + 0x30, 4, &[&0xAAu32.to_le_bytes()]);
    b.array(lists + 0x10, words, 3);
    // A type-2 object beside it, whose heads alone are read.
    let other = b.alloc(0x40);
    b.u32(other + 8, 2);
    b.array(other + 0x20, words, 1);
    let pointers = b.alloc(16);
    b.ptr(pointers, lists);
    b.ptr(pointers + 8, other);
    b.array(walk + 0x130, pointers, 2);

    let read = read(&b.0).expect("reads");
    let walk = (read.motions.iter())
        .find(|m| m.kind == 2)
        .expect("the kind-2 motion");
    let expected: Vec<Vec<ParameterItem>> = (0..2)
        .map(|clip| {
            vec![ParameterItem {
                points: vec![[0.0, 0.5], [clip as f32, 1.0]],
                floats: [0, 1, 2, 3].map(|i| (10 * clip + i) as f32),
            }]
        })
        .collect();
    assert_eq!(walk.parameter_items, expected);
    let head = |at: usize, count: u16| ArrayHead {
        pointer: VIRTUAL + at as u64,
        count,
        capacity: count,
        word: 0,
    };
    let null = ArrayHead {
        pointer: 0,
        count: 0,
        capacity: 0,
        word: 0,
    };
    assert_eq!(
        walk.parameter_objects,
        [
            ParameterObject {
                kind: 4,
                heads: [head(words, 3), head(list, 1)],
                lists: Some((vec![vec![5, 6]], vec![0xAA])),
            },
            ParameterObject {
                kind: 2,
                heads: [null, head(words, 1)],
                lists: None,
            },
        ]
    );
}

#[test]
fn a_motion_whose_time_warp_falls_is_refused_by_name() {
    let built = build_motions();
    let mut b = Bytes(built.bytes);
    let points = pointer_at(&b.0, built.warp + 8);
    b.f32(points + 12, -1.0);
    assert!(matches!(
        read(&b.0).expect_err("refused"),
        AnimError::Motion { motion, .. } if motion == "pack:/idle.motion"
    ));
}

#[test]
fn a_motion_of_a_kind_no_motion_was_seen_to_have_is_refused_by_name() {
    let built = build_motions();
    let mut b = Bytes(built.bytes);
    b.u32(built.first + 0x58, 3);
    assert!(matches!(
        read(&b.0).expect_err("refused"),
        AnimError::Motion { why, .. } if why.contains("kind word")
    ));
}

/// A column naming the one parameter [`interface`] lists, a float.
const DATABASE_KEY: (&str, [u16; 2]) = ("speed", [0, 0]);

/// An `interface` property of the properties object at `properties_at`, whose `parameters`
/// object holds the float [`DATABASE_KEY`] names.
fn interface(b: &mut Bytes, properties_at: usize) {
    let property = b.alloc(0x48);
    b.rel(properties_at + 0x18, property, 1);
    record(b, property, properties_at, 1, 0xFFFF, "interface");
    let parameters = b.alloc(0x20);
    b.u32(parameters, hash("parameters"));
    b.u16(parameters + 4, 13);
    b.u16(parameters + 6, 0xFFFF);
    b.rel(property + 0x20, parameters, 1);
    let object = b.alloc(0x40);
    b.u64(
        parameters + 0x18,
        (object as i64 - (parameters as i64 + 0x18)) as u64,
    );
    // A table-address-like first word and an address-like word at 0x30, as every nested object holds.
    b.u64(object, 0x1_4000_0000);
    b.empty_rel(object + 0x20);
    b.u64(object + 0x30, VIRTUAL);
    b.u16(object + 0x38, 3);
    b.u16(object + 0x3A, 0xFFFF);
    let speed = b.alloc(12);
    b.u32(speed, hash(DATABASE_KEY.0));
    b.u16(speed + 4, 1);
    b.u16(speed + 6, 0xFFFF);
    b.f32(speed + 8, 1.0);
    b.rel(object + 0x10, speed, 1);
}
const DATABASE_CELLS: [[i32; 2]; 3] = [[-2, 1], [0, 3], [-1, 1]];

/// Half `h` of row `r` of a built database's table: different in every place.
fn half(r: usize, h: usize) -> u16 {
    0x3C00 + 16 * r as u16 + h as u16
}

/// The type-6 database [`database`] builds.
fn the_database() -> Database {
    Database {
        keys: vec![(hash(DATABASE_KEY.0), DATABASE_KEY.1)],
        tables: vec![(0..2)
            .map(|r| std::array::from_fn(|h| half(r, h)))
            .collect()],
        grids: vec![Grid {
            words: [5, 2],
            cells: DATABASE_CELLS,
            floats: std::array::from_fn(|i| 0.5 * i as f32),
        }],
        lists: 1,
        bounds: std::array::from_fn(|i| i as f32 - 4.0),
        hash: hash("database"),
    }
}

/// A type-6 database at a new place: one key, a table of two rows, one grid, its weight-list
/// count, its bounds and its hash.
fn database(b: &mut Bytes) -> usize {
    let want = the_database();
    let at = b.alloc(0x88);
    b.u32(at + 8, 6);
    let mut key = hash(DATABASE_KEY.0).to_le_bytes().to_vec();
    key.extend(DATABASE_KEY.1.iter().flat_map(|w| w.to_le_bytes()));
    b.list(at + 0x10, 8, &[&key]);
    let rows: Vec<Vec<u8>> = (want.tables[0].iter())
        .map(|row| row.iter().flat_map(|h| h.to_le_bytes()).collect())
        .collect();
    let first = b.list(at + 0x20, 16, &[&[0u8; 16]]);
    b.list(first, 0x20, &[&rows[0], &rows[1]]);
    let grid = &want.grids[0];
    let mut bytes: Vec<u8> = grid.words.iter().flat_map(|w| w.to_le_bytes()).collect();
    bytes.extend(
        grid.cells
            .as_flattened()
            .iter()
            .flat_map(|c| c.to_le_bytes()),
    );
    bytes.extend(grid.floats.iter().flat_map(|f| f.to_le_bytes()));
    b.list(at + 0x30, 0x50, &[&bytes]);
    b.u32(at + 0x50, want.lists);
    for (i, bound) in want.bounds.iter().enumerate() {
        b.f32(at + 0x54 + 4 * i, *bound);
    }
    b.u32(at + 0x80, want.hash);
    at
}
#[path = "anim/write_tests.rs"]
mod write;
#[path = "anim/write_derived_and_grown_tests.rs"]
mod write_derived_and_grown;
#[path = "anim/write_edit_tests.rs"]
mod write_edit;
#[path = "anim/write_in_extent_tests.rs"]
mod write_in_extent;
#[path = "anim/write_mapped_tests.rs"]
mod write_mapped;
#[path = "anim/write_relayout_tests.rs"]
mod write_relayout;

#[test]
fn one_sampler_gives_every_dof_at_every_frame_what_a_sample_alone_gives() {
    let read = read(&build(&Spec::default()).bytes).expect("reads");
    let animation = the_animation(&read);
    let rate = animation.rate().expect("a rate");
    let mut sampler = animation.sampler();
    let values = quantised(usize::from(FRAMES), 0);
    // Back and forth over the frames, so a frame's kept words are asked for again after another's.
    for frame in (0..usize::from(FRAMES)).chain((0..usize::from(FRAMES)).rev()) {
        let time = frame as f32 / rate;
        let mut at = |dof| sampler.sample(dof, time).expect("decodes");
        let expected = SCALE * values[frame] as f32 + OFFSET;
        assert!(
            (at(2).as_slice()[0] - expected).abs() < 1e-3,
            "frame {frame}"
        );
        assert_eq!(at(0).as_slice(), [0.5], "frame {frame}");
        assert_eq!(at(1).as_slice(), [1.0, 2.0, 3.0], "frame {frame}");
        assert_eq!(
            at(2).as_slice(),
            animation.sample(2, time).expect("decodes").as_slice()
        );
    }
    assert!(matches!(
        sampler.sample(3, 0.0),
        Err(AnimError::NoDof { dof: 3, .. })
    ));
}
