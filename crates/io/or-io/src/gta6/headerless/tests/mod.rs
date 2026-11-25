use super::bound::{BOUND_CHILD, COMPOSITE, GEOMETRY, PLAIN_GEOMETRY};
use super::bytes::{u32_at, u64_at};
use super::drawable::DRAWABLE_ROOT;
use super::*;

mod bound;
mod cloth;
mod dictionary;
mod drawable;
mod entry_skeleton;
mod extra_skeletons;
mod fragment;
mod head;
mod instances;
mod navmesh;
mod octree;
mod skeleton;
mod streams;
mod surfaces;
mod write;
mod write_bone_names;
mod write_bound_placement;
mod write_dictionary_entries;
mod write_embedded;
mod write_embedded_repack;
mod write_filter_tracks;
mod write_filters;
mod write_geometry;
mod write_geometry_cut;
mod write_geometry_per_geometry;
mod write_material;
mod write_skeleton;

/// A virtual section laid out record by record, each 16-aligned.
#[derive(Default)]
struct Image {
    bytes: Vec<u8>,
}

impl Image {
    fn alloc(&mut self, len: usize) -> usize {
        let at = self.bytes.len().next_multiple_of(16);
        self.bytes.resize(at + len, 0);
        at
    }
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.bytes[at..at + bytes.len()].copy_from_slice(bytes);
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
    fn vec3(&mut self, at: usize, v: [f32; 3]) {
        for (a, x) in v.iter().enumerate() {
            self.put(at + 4 * a, &x.to_le_bytes());
        }
    }
    fn ptr(&mut self, at: usize, target: usize) {
        self.u64(at, 0x5000_0000 + target as u64);
    }
    fn phys(&mut self, at: usize, offset: usize) {
        self.u64(at, 0x6000_0000 + offset as u64);
    }
    /// An array of pointers to `targets`.
    fn pointers(&mut self, targets: &[usize]) -> usize {
        let at = self.alloc(8 * targets.len());
        for (i, t) in targets.iter().enumerate() {
            self.ptr(at + 8 * i, *t);
        }
        at
    }
}

const TRIANGLE_AT: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

/// Where a built bound's parts lie.
struct BoundParts {
    root: usize,
    primitives: usize,
    nodes: usize,
}

/// A composite of one geometry child: three vertices, a triangle and a sphere, and a BVH of one
/// leaf over both.
fn bound_into(img: &mut Image) -> BoundParts {
    let root = img.alloc(0xB0);
    img.bytes[root + 0x10] = COMPOSITE;
    img.bytes[root + 0x11] = 1;
    img.vec3(root + 0x20, [1.0, 1.0, 0.0]);
    img.vec3(root + 0x30, [0.0; 3]);
    img.u16(root + 0x98, 1);
    img.u16(root + 0x9A, 1);
    let child = img.alloc(BOUND_CHILD);
    img.bytes[child + 0x10] = GEOMETRY;
    img.vec3(child + 0x20, [1.0, 1.0, 0.0]);
    img.vec3(child + 0x90, [1.0; 3]);
    img.u32(child + 0xD0, 3);
    img.u32(child + 0xD4, 2);
    let vertices = img.alloc(18);
    for (i, v) in TRIANGLE_AT.iter().enumerate() {
        for (a, x) in v.iter().enumerate() {
            img.u16(vertices + 6 * i + 2 * a, *x as i16 as u16);
        }
    }
    img.ptr(child + 0xB0, vertices);
    let primitives = img.alloc(32);
    for (o, i) in [(4, 0), (6, 1), (8, 2)] {
        img.u16(primitives + o, i);
    }
    for o in [0xA, 0xC, 0xE] {
        img.u16(primitives + o, 0xFFFF);
    }
    img.bytes[primitives + 16 + 3] = 1;
    img.ptr(child + 0x88, primitives);
    let bvh = img.alloc(0x60);
    img.vec3(bvh + 0x10, [1.0, 1.0, 0.0]);
    let nodes = img.alloc(32);
    img.u16(nodes + 16 + 0xE, 2);
    img.ptr(bvh + 0x50, nodes);
    img.u32(bvh + 0x58, 2);
    img.ptr(child + 0x130, bvh);
    let children = img.pointers(&[child]);
    img.ptr(root + 0x70, children);
    let matrices = img.alloc(64);
    for r in 0..4 {
        img.put(matrices + 16 * r + 4 * r, &1.0f32.to_le_bytes());
    }
    img.ptr(root + 0x78, matrices);
    img.ptr(root + 0x80, matrices);
    let flags = img.alloc(16);
    img.u64(flags, 0x13);
    img.ptr(root + 0x88, flags);
    BoundParts {
        root,
        primitives,
        nodes,
    }
}

fn bound() -> (Image, BoundParts) {
    let mut img = Image::default();
    let parts = bound_into(&mut img);
    (img, parts)
}

/// Where a built drawable's parts lie, in the virtual section or the physical.
struct DrawableParts {
    root: usize,
    shader: usize,
    parameter_infos: usize,
    vertex_buffer: usize,
    declaration: usize,
    index_buffer: usize,
    index_data: usize,
    /// The geometry's box, min then max.
    geometry_box: usize,
}

/// One stream of a built vertex block: its declaration channel, type byte and elements.
struct Stream {
    channel: u8,
    code: u8,
    size: u8,
    bytes: Vec<u8>,
}

/// The slot code a channel of `code` and `size` is stated with.
fn slot_code(code: u8, size: u8) -> u32 {
    let id = match code {
        0x29 => 1,
        0x1E => 5,
        0x1C => 6,
        0x18 => 7,
        0x26 => 8,
        0x0C => 0xA,
        0x0A => 0xB,
        0x03 => 0xC,
        0x10 => 2,
        0x06 => 3,
        0x02 => 4,
        _ => 0,
    };
    (u32::from(size / 4) << 5) | id
}

/// The vertex data of `count` vertices holding `streams`, each stated by its slot, behind
/// `quantisation` (a quantised position's 0x20-byte block) where there is one, written at the
/// end of `phys`; the built geometry's vertex buffer and declaration are pointed at it.
fn put_vertex_block(
    img: &mut Image,
    phys: &mut Vec<u8>,
    parts: &DrawableParts,
    count: u32,
    mut streams: Vec<Stream>,
    quantisation: Option<[u8; 0x20]>,
) {
    let mut block = vec![0u8; 0x60];
    block[0x5C..0x60].copy_from_slice(&0x200u32.to_le_bytes());
    if let Some(q) = quantisation {
        block.extend(q);
    }
    let d = parts.declaration;
    img.bytes[d..d + 0x140].fill(0);
    streams.sort_by_key(|s| s.channel);
    let mut size = 0u64;
    for s in &streams {
        block.resize(block.len().next_multiple_of(4), 0);
        let offset = block.len() as u32;
        let slot = super::streams::SLOT_CHANNELS
            .iter()
            .position(|c| *c == Some(s.channel))
            .expect("a slot for the channel");
        let word = ((offset / 2) << 8) | slot_code(s.code, s.size);
        block[4 * slot..4 * slot + 4].copy_from_slice(&word.to_le_bytes());
        block.extend(&s.bytes);
        let k = usize::from(s.channel);
        img.u32(d + 4 * k, offset);
        img.bytes[d + 0xD0 + k] = s.size;
        img.bytes[d + 0x104 + k] = s.code;
        size += u64::from(s.size);
    }
    img.u64(d + 0x138, 1 | (size << 2) | (u64::from(count) << 10));
    block.resize(block.len().next_multiple_of(4), 0);
    phys.resize(phys.len().next_multiple_of(16), 0);
    img.phys(parts.vertex_buffer + 0x18, phys.len());
    img.u32(parts.vertex_buffer + 0x08, (block.len() / 4) as u32);
    phys.extend(block);
}

/// Three lanes as an 8-byte quantised element: lane A in bytes 3, 2, 1, lane B in bytes 5, 4, 0
/// and lane C in bytes 7, 6.
fn pack_lanes([a, b, c]: [u32; 3]) -> u64 {
    let lo = (a << 8) | (b & 0xFF);
    let hi = (b >> 8) | (c << 16);
    u64::from(lo) | (u64::from(hi) << 32)
}

/// [`TRIANGLE_AT`] as three floats a vertex.
fn float_positions() -> Stream {
    Stream {
        channel: 0,
        code: 6,
        size: 12,
        bytes: TRIANGLE_AT
            .iter()
            .flatten()
            .flat_map(|x| x.to_le_bytes())
            .collect(),
    }
}

const TEXTURE_KEY: u64 = 0x1122_3344_5566_7788;
const SHADER_HASH: u32 = 0xABCD_0123;
/// The shader's texture records' hashes: the first slot binds [`TEXTURE_KEY`], the second none.
const TEXTURES: [u32; 2] = [0x1111_0001, 0x1111_0002];
/// Its constant records, not in offset order: hash, buffer, byte offset and 4-byte words.
const CONSTANTS: [(u32, u8, u16, u8); 4] = [
    (0x3333_0001, 1, 4, 2),
    (0x3333_0002, 0, 16, 3),
    (0x3333_0003, 0, 0, 2),
    (0x3333_0004, 0, 8, 1),
];

/// Word `i` of the constant `hash`, which no other constant's word equals.
fn constant_word(hash: u32, i: usize) -> u32 {
    hash ^ ((i as u32 + 1) << 20)
}

/// Each constant buffer's size: the end of its last constant, rounded up to 16 bytes.
fn buffer_sizes() -> [usize; 2] {
    let mut ends = [0usize; 2];
    for (_, b, offset, words) in CONSTANTS {
        let end = usize::from(offset) + 4 * usize::from(words);
        ends[usize::from(b)] = ends[usize::from(b)].max(end);
    }
    ends.map(|e| e.next_multiple_of(16))
}

/// A shader of [`TEXTURES`] and [`CONSTANTS`]: its parameter infos, then its block of two buffer
/// pointers, the two buffers and the texture table. Where the shader and its infos lie.
fn shader_into(img: &mut Image) -> (usize, usize) {
    let entry = img.alloc(0x10);
    img.u64(entry + 8, TEXTURE_KEY);
    let binding = img.alloc(0x50);
    img.ptr(binding + 0x30, entry);
    let sizes = buffer_sizes();
    let block = img.alloc(8 * 2 + sizes.iter().sum::<usize>() + 8 * TEXTURES.len());
    let mut buffer_at = [0; 2];
    let mut next = block + 8 * 2;
    for (b, size) in sizes.iter().enumerate() {
        img.ptr(block + 8 * b, next);
        buffer_at[b] = next;
        next += size;
    }
    let table = next;
    // The second texture slot is empty.
    img.ptr(table, binding);
    let count = TEXTURES.len() + CONSTANTS.len();
    let infos = img.alloc(8 + 8 * count);
    img.put(
        infos,
        &[2, TEXTURES.len() as u8, 0, 0, count as u8, 0, 0, 8],
    );
    for (k, hash) in TEXTURES.iter().enumerate() {
        img.u32(infos + 8 + 8 * k, *hash);
        img.u32(infos + 12 + 8 * k, 4 * k as u32);
    }
    for (k, (hash, b, offset, words)) in CONSTANTS.iter().enumerate() {
        let r = infos + 8 + 8 * (TEXTURES.len() + k);
        img.u32(r, *hash);
        let packed = 3 | u32::from(*b) << 2 | u32::from(*offset) << 8 | u32::from(*words) << 22;
        img.u32(r + 4, packed);
        for i in 0..usize::from(*words) {
            let at = buffer_at[usize::from(*b)] + usize::from(*offset) + 4 * i;
            img.u32(at, constant_word(*hash, i));
        }
    }
    let shader = img.alloc(0x40);
    img.u32(shader, SHADER_HASH);
    img.ptr(shader + 0x08, block);
    img.ptr(shader + 0x10, table);
    img.ptr(shader + 0x20, infos);
    (shader, infos)
}

/// A drawable of one level, one model and one geometry: the triangle [`TRIANGLE_AT`] as three
/// floats a vertex and one meshlet, inside the box `box_max`; its data appended to `phys`.
fn drawable_into(img: &mut Image, phys: &mut Vec<u8>, box_max: [f32; 3]) -> DrawableParts {
    let root = img.alloc(DRAWABLE_ROOT);
    let map = img.alloc(0x10);
    img.bytes[map + 8] = 1;
    img.bytes[map + 9] = 1;
    img.ptr(root + 0x08, map);
    img.vec3(root + 0x40, box_max);
    img.put(root + 0x7C, &9998.0f32.to_le_bytes());
    let name = img.alloc(8);
    img.put(name, b"box#dr\0");
    img.ptr(root + 0xA8, name);

    let (shader, infos) = shader_into(img);
    let shaders = img.pointers(&[shader]);
    let group = img.alloc(0x40);
    img.ptr(group + 0x10, shaders);
    img.u16(group + 0x18, 1);
    img.u16(group + 0x1A, 1);
    img.ptr(root + 0x10, group);

    let declaration = img.alloc(0x140);
    let vb = img.alloc(0x50);
    img.u16(vb + 0x0C, 4);
    img.u16(vb + 0x0E, 9);
    img.u32(vb + 0x10, 0x1601);
    img.ptr(vb + 0x40, declaration);
    let ib = img.alloc(0x50);
    let boxes = img.alloc(32);
    let mut parts = DrawableParts {
        root,
        shader,
        parameter_infos: infos,
        vertex_buffer: vb,
        declaration,
        index_buffer: ib,
        index_data: 0,
        geometry_box: boxes,
    };
    put_vertex_block(img, phys, &parts, 3, vec![float_positions()], None);

    phys.resize(phys.len().next_multiple_of(16), 0);
    let index_data = phys.len();
    // One meshlet of one triangle over a list of three vertices: roundup4(3 + 4 + 1 + 2) words.
    let words: [u32; 12] = [
        0x8000_0001,
        1,
        3,
        0,
        0,
        0,
        0,
        1 << 7 | 2 << 14,
        1 << 16,
        2,
        0,
        0,
    ];
    for w in words {
        phys.extend(w.to_le_bytes());
    }
    parts.index_data = index_data;
    img.u32(ib + 0x08, 12);
    img.u16(ib + 0x0C, 4);
    img.u16(ib + 0x0E, 8);
    img.u32(ib + 0x10, 0x1380);
    img.phys(ib + 0x18, index_data);
    let record = img.alloc(0x40);
    img.ptr(record, ib);
    img.u16(record + 0x08, 1);
    img.u16(record + 0x0A, 1);
    img.u32(record + 0x0C, 3 << 8);

    let geometry = img.alloc(0x50);
    img.ptr(geometry + 0x08, vb);
    img.ptr(geometry + 0x40, record);
    let geometries = img.pointers(&[geometry]);
    img.vec3(boxes + 0x10, box_max);
    let shader_indices = img.alloc(2);
    let model = img.alloc(0x38);
    img.ptr(model + 0x08, geometries);
    img.u16(model + 0x10, 1);
    img.u16(model + 0x36, 1);
    img.ptr(model + 0x18, boxes);
    img.ptr(model + 0x20, shader_indices);
    let models = img.pointers(&[model]);
    let level = img.alloc(0x10);
    img.ptr(level, models);
    img.u16(level + 8, 1);
    img.u16(level + 0xA, 1);
    img.ptr(root + 0x50, level);
    parts
}

fn drawable(box_max: [f32; 3]) -> (Image, Vec<u8>, DrawableParts) {
    let (mut img, mut phys) = (Image::default(), Vec::new());
    let parts = drawable_into(&mut img, &mut phys, box_max);
    (img, phys, parts)
}

#[test]
fn a_signature_with_no_measured_layout_is_refused_by_name() {
    let mut head = vec![0u8; 0x40];
    head[8..16].copy_from_slice(&0x5000_0010u64.to_le_bytes());
    let signature = Rpf9Signature::of(&head, 0x40).expect("a signature");
    assert_eq!(signature.to_string(), "0P000000");
    assert!(matches!(
        read(signature, &head, Some(&[])),
        Err(HeaderlessError::NoLayout(s)) if s == signature
    ));
}
