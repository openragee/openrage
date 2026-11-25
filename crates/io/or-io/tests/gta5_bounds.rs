//! A `.ybn` laid out by hand at the declared offsets, with invented content:
//! a geometry bound holding one polygon and one box primitive, read typed and
//! owned through both tags, saved whole to a fixed point, and refused where a
//! polygon's tag or a vertex index would be misread; its octant map and its
//! second-surface displacements kept across a save; a vertex attribute run of
//! any width but one word refused. Needs no game file.

use std::mem::offset_of;

use or_io::container::resource::{Compression, ResourceEnvelope, Rsc7};
use or_io::decl::{Aligned, File, Raw, Resource, View};
use or_io::desc::Image;
use or_io::gta5::bounds::*;
use or_io::gta5::formats;
use or_io::Generation;

const BASE: u64 = 0x5000_0000;

struct Writer(Vec<u8>);

impl Writer {
    fn put(&mut self, at: usize, data: &[u8]) -> &mut Self {
        if self.0.len() < at + data.len() {
            self.0.resize(at + data.len(), 0);
        }
        self.0[at..at + data.len()].copy_from_slice(data);
        self
    }
    fn u16(&mut self, at: usize, v: u16) -> &mut Self {
        self.put(at, &v.to_le_bytes())
    }
    fn u32(&mut self, at: usize, v: u32) -> &mut Self {
        self.put(at, &v.to_le_bytes())
    }
    fn f32(&mut self, at: usize, v: f32) -> &mut Self {
        self.put(at, &v.to_le_bytes())
    }
    fn ptr(&mut self, at: usize, target: usize) -> &mut Self {
        self.put(at, &(BASE + target as u64).to_le_bytes())
    }
}

/// A geometry bound at 0 with four vertices, a triangle naming `third`, and a box.
fn authored(third: u16, triangle_tag: u32) -> (Aligned, usize) {
    let mut w = Writer(vec![0; 0x1c0]);
    w.put(0x10, &[4]).f32(0x14, 2.0);
    w.f32(0x90, 1.0 / 32.0)
        .f32(0x94, 1.0 / 32.0)
        .f32(0x98, 1.0 / 32.0);
    w.ptr(0x88, 0x140)
        .ptr(0xb0, 0x160)
        .u32(0xd0, 4)
        .u32(0xd4, 2);
    w.ptr(0xf0, 0x180).put(0x120, &[1]);
    w.ptr(0x118, 0x188);
    // A triangle: its area's low three bits are its tag.
    let area = (1.5f32.to_bits() & !0b111) | triangle_tag;
    w.u32(0x140, area)
        .u16(0x144, 0)
        .u16(0x146, 1)
        .u16(0x148, third)
        .put(0x14a, &[0xff; 6]);
    // A box primitive: tag 3 in its first word, four corners.
    w.u32(0x150, 3)
        .u16(0x154, 0)
        .u16(0x156, 1)
        .u16(0x158, 2)
        .u16(0x15a, 3);
    for (i, v) in [[0i16, 0, 0], [32, 0, 0], [0, 32, 0], [0, 0, 32]]
        .iter()
        .enumerate()
    {
        for (j, c) in v.iter().enumerate() {
            w.put(0x160 + 6 * i + 2 * j, &c.to_le_bytes());
        }
    }
    w.u32(0x180, 0x0000_0017);
    let size = w.0.len();
    (Aligned::from_bytes(&w.0), size)
}

const OCTANTS: [&[u32]; OCTANT_COUNT] = [&[0], &[], &[1, 2], &[3], &[], &[], &[2], &[0, 1, 3]];
const DISPLACEMENTS: [f32; 4] = [0.5, -0.25, 1.0, 2.0];

const WORD: usize = size_of::<u32>();
const COUNTS_BYTES: usize = OCTANT_COUNT * WORD;
const TABLE_BYTES: usize = size_of::<Raw<OctantVertsTable>>();
const SLOT: usize = TABLE_BYTES / OCTANT_COUNT;
const COUNTS_AT: usize = offset_of!(Raw<PhBoundGeometry>, octant_vert_counts);
const TABLE_AT: usize = offset_of!(Raw<PhBoundGeometry>, octant_verts);
const DISPLACEMENTS_AT: usize =
    offset_of!(Raw<PhBoundGeometry>, second_surface_vertex_displacements);
const DISPLACEMENT_COUNT_AT: usize = offset_of!(
    Raw<PhBoundGeometry>,
    num_second_surface_vertex_displacements
);

/// [`authored`], with an octant map laid out as shipped files lay it out
/// (counts, then the table just past them, then the runs) and a second-surface run.
fn with_octants_and_displacements() -> (Aligned, usize) {
    let (bytes, size) = authored(2, 0);
    let mut w = Writer(bytes.bytes()[..size].to_vec());
    let (counts, table) = (size, size + COUNTS_BYTES);
    let mut run = table + TABLE_BYTES;
    w.ptr(COUNTS_AT, counts).ptr(TABLE_AT, table);
    for (i, octant) in OCTANTS.iter().enumerate() {
        w.u32(counts + WORD * i, octant.len() as u32);
        if !octant.is_empty() {
            w.ptr(table + SLOT * i, run);
        }
        for v in *octant {
            w.u32(run, *v);
            run += WORD;
        }
    }
    let displacements = run.next_multiple_of(16);
    w.ptr(DISPLACEMENTS_AT, displacements)
        .u32(DISPLACEMENT_COUNT_AT, DISPLACEMENTS.len() as u32);
    for (i, d) in DISPLACEMENTS.iter().enumerate() {
        w.f32(displacements + size_of::<f32>() * i, *d);
    }
    let size = w.0.len().next_multiple_of(16);
    w.0.resize(size, 0);
    (Aligned::from_bytes(&w.0), size)
}

fn u32_at(img: Image<'_>, at: usize) -> u32 {
    u32::from_le_bytes(img.slice_at(at, WORD).unwrap().try_into().unwrap())
}

/// The octant runs and second-surface displacements a geometry at 0 states.
fn octants_and_displacements(img: Image<'_>) -> (Vec<Vec<u32>>, Vec<f32>) {
    let resolve = |at: usize| img.resolve(img.pointer_at(at).unwrap()).ok();
    let mut octants = Vec::new();
    if let (Some(counts), Some(table)) = (resolve(COUNTS_AT), resolve(TABLE_AT)) {
        for i in 0..OCTANT_COUNT {
            let n = u32_at(img, counts + WORD * i) as usize;
            let run = resolve(table + SLOT * i);
            octants.push(
                (0..n)
                    .map(|j| u32_at(img, run.expect("a live run") + WORD * j))
                    .collect(),
            );
        }
    }
    let displacements = resolve(DISPLACEMENTS_AT).map_or_else(Vec::new, |at| {
        (0..u32_at(img, DISPLACEMENT_COUNT_AT) as usize)
            .map(|i| f32::from_bits(u32_at(img, at + size_of::<f32>() * i)))
            .collect()
    });
    (octants, displacements)
}

#[test]
fn the_octant_map_and_the_second_surface_survive_a_save() {
    let ybn = formats::format("ybn").unwrap();
    let (bytes, size) = with_octants_and_displacements();
    let shipped = octants_and_displacements(Image::new(bytes.bytes(), size));
    assert_eq!(shipped.0, OCTANTS.map(<[u32]>::to_vec));
    assert_eq!(shipped.1, DISPLACEMENTS);
    let saved = ybn
        .save(
            Generation::Gen8,
            &ybn.read(Generation::Gen8, Image::new(bytes.bytes(), size))
                .unwrap(),
        )
        .unwrap();
    let o = Rsc7::open(&saved).unwrap();
    let again = Aligned::from_bytes(&o.data);
    let img = Image::new(again.bytes(), o.virtual_size as usize);
    assert_eq!(octants_and_displacements(img), shipped);
}

fn payload(file: &[u8]) -> Vec<u8> {
    let o = Rsc7::open(file).unwrap();
    let mut out = o.seal.header_bytes();
    out.extend_from_slice(&o.data);
    out
}

#[test]
fn an_authored_ybn_reads_typed_through_both_tags() {
    let (bytes, size) = authored(2, 0);
    let img = Image::new(bytes.bytes(), size);
    let root = View::<Raw<PhBound>>::at_offset(img, 0).unwrap();
    assert_eq!({ root.r#type }.get(), BoundType::Geometry);
    let g = View::variant::<Raw<PhBoundGeometry>>(&root)
        .unwrap()
        .unwrap();
    assert!(View::variant::<Raw<PhBoundBVH>>(&root).unwrap().is_none());
    let polygons = g.polygons().unwrap();
    let polygon = polygons.view(0).unwrap();
    assert_eq!({ polygon.area }.to_bits() & PRIMITIVE_TYPE_MASK, 0);
    assert_eq!(polygon.vertex_indices, [0, 1, 2]);
    let b = View::variant::<Raw<PhPrimBox>>(&polygons.view(1).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(b.vertex_indices, [0, 1, 2, 3]);
    assert_eq!(g.compressed_vertices().unwrap()[1].x, 32);
    assert_eq!(g.material_ids().unwrap()[0].id, 0x17);
}

#[test]
fn an_authored_ybn_saves_whole_and_reaches_a_fixed_point() {
    let ybn = formats::format("ybn").unwrap();
    let (bytes, size) = authored(2, 0);
    let first = ybn
        .save(
            Generation::Gen8,
            &ybn.read(Generation::Gen8, Image::new(bytes.bytes(), size))
                .unwrap(),
        )
        .unwrap();
    let o = Rsc7::open(&first).unwrap();
    assert_eq!(ybn.target_of_version(o.version).unwrap(), Generation::Gen8);
    let again = Aligned::from_bytes(&o.data);
    let img = Image::new(again.bytes(), o.virtual_size as usize);
    let g =
        View::variant::<Raw<PhBoundGeometry>>(&View::<Raw<PhBound>>::at_offset(img, 0).unwrap())
            .unwrap()
            .unwrap();
    assert_eq!(g.polygons().unwrap().len(), 2);
    let second = ybn
        .save(Generation::Gen8, &ybn.read(Generation::Gen8, img).unwrap())
        .unwrap();
    assert_eq!(payload(&second), payload(&first));
}

#[test]
fn a_polygon_naming_a_vertex_past_the_end_is_refused() {
    let ybn = formats::format("ybn").unwrap();
    let (bytes, size) = authored(9, 0);
    let tree = ybn
        .read(Generation::Gen8, Image::new(bytes.bytes(), size))
        .unwrap();
    let why = ybn.save(Generation::Gen8, &tree).unwrap_err().to_string();
    assert!(why.contains("names vertex 9 of 4"), "{why}");
}

#[test]
fn a_polygon_whose_tag_names_no_primitive_is_refused() {
    let ybn = formats::format("ybn").unwrap();
    let (bytes, size) = authored(2, 7);
    let tree = ybn
        .read(Generation::Gen8, Image::new(bytes.bytes(), size))
        .unwrap();
    let why = ybn.save(Generation::Gen8, &tree).unwrap_err().to_string();
    assert!(why.contains("names no primitive"), "{why}");
}

/// `authored` saved as a whole `.ybn` file, which an owned value opens.
fn file_of((bytes, size): (Aligned, usize)) -> Vec<u8> {
    let ybn = formats::format("ybn").unwrap();
    ybn.save(
        Generation::Gen8,
        &ybn.read(Generation::Gen8, Image::new(bytes.bytes(), size))
            .unwrap(),
    )
    .unwrap()
}

fn geometry(bound: &mut PhBoundAny) -> &mut PhBoundGeometry {
    bound.as_geometry_mut().expect("a geometry")
}

/// `file` with `padding` bytes that deflate to about their own length after its payload, as a
/// street's collision follows its header.
fn padded(file: &[u8], padding: usize) -> Vec<u8> {
    let opened = Rsc7::open(file).unwrap();
    let mut payload = opened.data;
    let mut state = 0x9E37_79B9u32;
    payload.extend((0..padding).map(|_| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as u8
    }));
    let seal = Rsc7::invent_single_chunk_seal(opened.version, payload.len()).unwrap();
    payload.resize(seal.total_size() as usize, 0);
    Rsc7::reseal(&payload, &seal, Compression::fast()).unwrap()
}

/// ⚠ What a read of a bound's box alone leans on: a file cut off after its first kilobytes peeks
/// the root a whole file does, because `Rsc7::open` keeps what a cut stream inflates to.
#[test]
fn a_bounds_root_peeks_from_the_front_of_a_file_cut_short() {
    let whole = padded(&file_of(authored(2, 0)), 100_000);
    let cut = &whole[..4096];

    let front = PhBoundAny::peek(cut).unwrap();
    let full = PhBoundAny::peek(&whole).unwrap();

    let root = size_of::<Raw<PhBound>>();
    assert!(
        front.image().len() < full.image().len(),
        "only the front of the payload is inflated"
    );
    assert_eq!(front.image().bytes()[..root], full.image().bytes()[..root]);
    let at = front.root().unwrap();
    assert_eq!({ at.r#type }.get(), BoundType::Geometry);
    assert_eq!({ at.radius_around_centroid }, 2.0);
}

/// A polygon's kind is `tag_bits(0, 3)` of its area: read owned, carried
/// through a save, and refused where the bits name no primitive.
#[test]
fn an_owned_polygon_keeps_the_primitive_its_area_bits_name() {
    let mut bound = PhBoundAny::from_bytes(&file_of(authored(2, 0))).unwrap();
    let kinds: Vec<u32> = geometry(&mut bound)
        .polygons
        .iter()
        .map(PhPolygon::primitive)
        .collect();
    assert_eq!(kinds, [0, 3]);
    let saved = bound.to_bytes().unwrap();
    let peek = PhBoundAny::peek(&saved).unwrap();
    let root = peek.root().unwrap();
    let g = View::variant::<Raw<PhBoundGeometry>>(&root)
        .unwrap()
        .unwrap();
    let polygons = g.polygons().unwrap();
    let b = View::variant::<Raw<PhPrimBox>>(&polygons.view(1).unwrap())
        .unwrap()
        .expect("still a box");
    assert_eq!(b.vertex_indices, [0, 1, 2, 3]);

    let area = &mut geometry(&mut bound).polygons[1].area;
    *area = f32::from_bits(area.to_bits() | PRIMITIVE_TYPE_MASK);
    let why = bound.to_bytes().unwrap_err().to_string();
    assert!(why.contains("names no primitive"), "{why}");
}

#[test]
fn the_octant_map_and_the_second_surface_survive_an_owned_save() {
    let mut bound = PhBoundAny::from_bytes(&file_of(with_octants_and_displacements())).unwrap();
    let g = geometry(&mut bound);
    let octants = OCTANTS.map(<[u32]>::to_vec);
    assert_eq!(g.octant_verts.as_ref().map(|m| &m.octants), Some(&octants));
    assert_eq!(g.second_surface_vertex_displacements, DISPLACEMENTS);
    g.octant_verts.as_mut().unwrap().octants[1].push(3);
    g.second_surface_vertex_displacements.push(-1.0);

    let mut again = PhBoundAny::from_bytes(&bound.to_bytes().unwrap()).unwrap();
    assert_eq!(again, bound);
    let g = geometry(&mut again);
    assert_eq!(g.octant_verts.as_ref().unwrap().octants[1], [3]);
    assert_eq!(g.second_surface_vertex_displacements.len(), 5);
}

#[test]
fn an_octant_map_given_to_a_geometry_without_one_is_saved() {
    let mut bound = PhBoundAny::from_bytes(&file_of(authored(2, 0))).unwrap();
    assert!(geometry(&mut bound).octant_verts.is_none());
    let map = OctantMap {
        octants: OCTANTS.map(<[u32]>::to_vec),
    };
    geometry(&mut bound).octant_verts = Some(Box::new(map.clone()));
    let saved = bound.to_bytes().unwrap();
    let o = Rsc7::open(&saved).unwrap();
    let data = Aligned::from_bytes(&o.data);
    let (octants, _) = octants_and_displacements(Image::new(data.bytes(), o.virtual_size as usize));
    assert_eq!(octants, map.octants);
}

const WIDTH_AT: usize = offset_of!(Raw<PhBoundGeometry>, num_per_vertex_attribs);

/// [`authored`] saved with one colour word per vertex, its stated width then set to `per`.
fn with_vertex_attribs(per: u8) -> Vec<u8> {
    let (bytes, size) = authored(2, 0);
    let mut w = Writer(bytes.bytes()[..size].to_vec());
    let attribs = size;
    w.put(WIDTH_AT, &[1])
        .ptr(offset_of!(Raw<PhBoundGeometry>, vertex_attribs), attribs);
    for i in 0..4 {
        w.u32(attribs + WORD * i, 0xff00_0000 | i as u32);
    }
    let size = w.0.len().next_multiple_of(16);
    w.0.resize(size, 0);
    let o = Rsc7::open(&file_of((Aligned::from_bytes(&w.0), size))).unwrap();
    let mut data = o.data.clone();
    data[WIDTH_AT] = per;
    Rsc7::reseal(&data, &o.seal, Compression::fast()).unwrap()
}

/// The width a saved geometry at the root states for its `vertex_attribs`.
fn saved_width(file: &[u8]) -> u8 {
    let peek = PhBoundAny::peek(file).unwrap();
    let root = peek.root().unwrap();
    View::variant::<Raw<PhBoundGeometry>>(&root)
        .unwrap()
        .unwrap()
        .num_per_vertex_attribs
}

/// `vertex_attribs` is read as one word per vertex, so a save refuses a
/// file's wider width beside a live run.
#[test]
fn a_vertex_attribs_run_other_than_one_word_per_vertex_is_refused() {
    let mut bound = PhBoundAny::from_bytes(&with_vertex_attribs(1)).unwrap();
    assert_eq!(geometry(&mut bound).vertex_attribs.len(), 4);
    let saved = bound.to_bytes().unwrap();
    assert_eq!(saved_width(&saved), 1);
    assert_eq!(PhBoundAny::from_bytes(&saved).unwrap(), bound);

    let mut wide = PhBoundAny::from_bytes(&with_vertex_attribs(2)).unwrap();
    let why = wide.to_bytes().unwrap_err().to_string();
    assert!(why.contains("`num_per_vertex_attribs` states 2"), "{why}");

    geometry(&mut wide).vertex_attribs.clear();
    let saved = wide.to_bytes().unwrap();
    assert!(geometry(&mut PhBoundAny::from_bytes(&saved).unwrap())
        .vertex_attribs
        .is_empty());
}

/// A geometry whose file states no width saves one word per vertex once it is given a run.
#[test]
fn a_vertex_attribs_run_given_to_a_geometry_states_its_width() {
    let attribs = geometry(&mut PhBoundAny::from_bytes(&with_vertex_attribs(1)).unwrap())
        .vertex_attribs
        .clone();
    for file in [file_of(authored(2, 0)), with_vertex_attribs(0)] {
        let mut bound = PhBoundAny::from_bytes(&file).unwrap();
        geometry(&mut bound).vertex_attribs = attribs.clone();
        let saved = bound.to_bytes().unwrap();
        assert_eq!(saved_width(&saved), 1);
        let mut again = PhBoundAny::from_bytes(&saved).unwrap();
        assert_eq!(geometry(&mut again).vertex_attribs, attribs);
    }
}

/// A composite's child boxes are a `Vec3V` run, minimum then maximum; the
/// engine declares no box struct for them.
#[test]
fn a_composite_child_box_is_a_pair_of_vec3v() {
    for ext in ["ybn", "ybd"] {
        let pack = formats::format(ext)
            .unwrap()
            .description(Generation::Gen8)
            .unwrap();
        assert!(pack.type_by_name("SpdAABB").is_none(), "{ext}");
        let pair = pack.type_by_name("LocalBoxMinMax").expect(ext);
        let at = |f: &str| pair.field_by_name(f).unwrap().offset;
        assert_eq!(at("min"), std::mem::offset_of!(Raw<LocalBoxMinMax>, min));
        assert_eq!(at("max"), std::mem::offset_of!(Raw<LocalBoxMinMax>, max));
        assert_eq!(pair.size, 2 * size_of::<Raw<or_io::gta5::shared::Vec3V>>());
    }
}

/// A BVH's box, centre and quantization are the engine's sixteen-byte `Vector3`, not `Vec3V`.
#[test]
fn a_bvh_box_and_its_quantization_are_vector3() {
    for ext in ["ybn", "ybd"] {
        let pack = formats::format(ext)
            .unwrap()
            .description(Generation::Gen8)
            .unwrap();
        let bvh = pack.type_by_name("PhOptimizedBvh").expect(ext);
        for f in [
            "aabb_min",
            "aabb_max",
            "aabb_center",
            "quantize",
            "inv_quantize",
        ] {
            let field = bvh.field_by_name(f).expect(f);
            assert!(
                matches!(&field.repr, or_io::desc::FieldRepr::Inline { type_name } if type_name == "Vector3"),
                "{ext}: PhOptimizedBvh.{f} is {:?}",
                field.repr
            );
        }
        let vector = pack.type_by_name("Vector3").expect(ext);
        assert_eq!(vector.size, size_of::<Raw<or_io::gta5::shared::Vector3>>());
        assert_eq!(vector.size, 0x10);
    }
}

/// The `w` word of `phBound`'s min corner is its reference count: a bound
/// built from scratch saves it as 1, and moving the corner keeps it.
#[test]
fn a_bound_keeps_its_reference_count_beside_its_min_corner() {
    let min_of = |file: &[u8]| {
        let peek = PhBoundAny::peek(file).unwrap();
        let root = peek.root().unwrap();
        let min = { root.bounding_box_min_xyz_ref_count_w };
        ([min.x, min.y, min.z], min.w.to_bits())
    };
    let scratch = PhBoundAny::PhBoundSphere(Default::default())
        .to_bytes()
        .unwrap();
    assert_eq!(min_of(&scratch), ([0.0; 3], 1));

    let mut bound = PhBoundAny::from_bytes(&scratch).unwrap();
    let PhBoundAny::PhBoundSphere(sphere) = &mut bound else {
        panic!("a sphere");
    };
    sphere.base.set_bounding_box_min([-1.0, -2.0, -3.0]);
    assert_eq!(sphere.base.bounding_box_min(), [-1.0, -2.0, -3.0]);
    assert_eq!(min_of(&bound.to_bytes().unwrap()), ([-1.0, -2.0, -3.0], 1));
}

/// The length a saved geometry at the root states for its shrunk hull, and whether it holds that hull.
fn hull_stated(file: &[u8]) -> (u16, bool) {
    let o = Rsc7::open(file).unwrap();
    let data = Aligned::from_bytes(&o.data);
    let img = Image::new(data.bytes(), o.virtual_size as usize);
    let root = View::<Raw<PhBound>>::at_offset(img, 0).unwrap();
    let g = View::variant::<Raw<PhBoundGeometry>>(&root)
        .unwrap()
        .unwrap();
    (
        g.num_convex_hull_vertices,
        !{ g.compressed_shrunk_vertices }.is_null(),
    )
}

#[test]
fn a_hull_length_stated_beside_no_hull_is_set_by_name_and_kept_by_a_save() {
    use or_io::gta5::bounds::build::{self, BoundMesh};
    let mesh = BoundMesh::new(
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        vec![[0, 1, 2], [0, 2, 3]],
    );
    let mut built = build::geometry(&mesh).unwrap();
    assert_eq!(built.num_convex_hull_vertices(), None);
    let vertices = built.compressed_vertices.len() as u16;
    built.set_num_convex_hull_vertices(vertices);
    let saved = PhBoundAny::PhBoundGeometry(built).to_bytes().unwrap();
    assert_eq!(hull_stated(&saved), (vertices, false));

    let PhBoundAny::PhBoundGeometry(mut opened) = PhBoundAny::from_bytes(&saved).unwrap() else {
        panic!("not a geometry");
    };
    assert_eq!(opened.num_convex_hull_vertices(), Some(vertices));
    // ⚠ A save keeps the length as stated: one more vertex leaves it behind until it is set.
    let again = opened.compressed_vertices[0].clone();
    opened.compressed_vertices.push(again);
    let stale = PhBoundAny::PhBoundGeometry(opened.clone())
        .to_bytes()
        .unwrap();
    assert_eq!(hull_stated(&stale), (vertices, false));
    opened.set_num_convex_hull_vertices(vertices + 1);
    let followed = PhBoundAny::PhBoundGeometry(opened).to_bytes().unwrap();
    assert_eq!(hull_stated(&followed), (vertices + 1, false));
}
