//! `.ydr` through its declarations, both targets, over the corpus: typed
//! reads against the tree, the owned form against the tree's save, the
//! recorded testimony re-keyed by offset, the canonical fixed point, the
//! alignment census, the shaders' blobs, the texture bindings and the words
//! the owned form carries.

mod bound_walk;
mod gta6_accel;

use std::collections::BTreeMap;
use std::mem::{offset_of, size_of};

use or_io::decl::{Aligned, Described, Gen8, Gen9, Plain, Raw, Record, View, ViewError};
use or_io::desc::Image;
use or_io::gta5::bounds::PhBound;
use or_io::gta5::drawable::{gen8, gen9, DrawableTarget, GrmGeometry, GtaDrawable, RmcDrawable};
use or_io::gta5::texture_dict::gen9::GrcTextureG9;
use or_io::gta5::{PgDictionary, TextureSlot};
use or_io::Asset;
use or_io::Generation;
use or_io_verify::gates::{
    self, files, name, FieldMoved, Filed, Gates, Opened, Overread, Relocated,
};

const EXT: &str = "ydr";

type E = ViewError;

fn put<R: Record + Described>(out: &mut Filed, v: &View<'_, R>) {
    out.structs.insert(
        (View::offset(v), R::name()),
        Plain::as_bytes(View::get(v)).to_vec(),
    );
}

fn run<R: Record + Described>(out: &mut Filed, run: or_io::decl::Run<'_, R>) {
    for v in run.views() {
        put(out, &v);
    }
}

/// What each target's typed walk reads of the four types Enhanced changed.
trait Walked: DrawableTarget + Sized {
    fn buffers(g: &View<'_, Raw<GrmGeometry<Self>>>, out: &mut Filed) -> Result<(), E>;
    fn shader(s: &View<'_, Raw<Self::InstanceData>>, out: &mut Filed) -> Result<(), E>;
}

impl Walked for Gen8 {
    fn buffers(g: &View<'_, Raw<GrmGeometry<Gen8>>>, out: &mut Filed) -> Result<(), E> {
        if let Some(vb) = g.vertex_buffer()? {
            put(out, &vb);
            if let Some(fvf) = vb.fvf()? {
                put(out, &fvf);
            }
        }
        if let Some(ib) = g.index_buffer()? {
            put(out, &ib);
        }
        Ok(())
    }

    fn shader(s: &View<'_, Raw<gen8::GrcInstanceData>>, out: &mut Filed) -> Result<(), E> {
        put(out, s);
        Ok(())
    }
}

impl Walked for Gen9 {
    fn buffers(g: &View<'_, Raw<GrmGeometry<Gen9>>>, out: &mut Filed) -> Result<(), E> {
        if let Some(vb) = g.vertex_buffer()? {
            put(out, &vb);
            if let Some(srv) = vb.srv()? {
                put(out, &srv);
            }
            if let Some(info) = vb.info()? {
                put(out, &info);
            }
        }
        if let Some(ib) = g.index_buffer()? {
            put(out, &ib);
            if let Some(srv) = ib.srv()? {
                put(out, &srv);
            }
        }
        Ok(())
    }

    fn shader(s: &View<'_, Raw<gen9::GrcInstanceDataG9>>, out: &mut Filed) -> Result<(), E> {
        put(out, s);
        if let Some(infos) = s.param_infos()? {
            put(out, &infos);
            run(out, infos.params()?);
        }
        Ok(())
    }
}

/// Every non-null level of detail a drawable holds, highest first.
fn lods<'a, G: DrawableTarget>(
    image: Image<'a>,
    d: &View<'a, Raw<RmcDrawable<G>>>,
) -> Result<Vec<View<'a, Raw<or_io::gta5::RmcLod<G>>>>, E> {
    d.lod_group
        .lods
        .iter()
        .filter(|slot| !slot.is_null())
        .map(|slot| View::at_address(image, slot.raw()))
        .collect()
}

fn walk<G: Walked>(image: Image<'_>, out: &mut Filed) -> Result<(), E> {
    let root = View::<Raw<GtaDrawable<G>>>::at_offset(image, 0)?;
    put(out, &root);
    run(out, root.lights()?);
    if let Some(b) = root.bound()? {
        let b = View::<Raw<PhBound>>::at_offset(image, View::offset(&b))?;
        bound_walk::walk(image, b, out)?;
    }
    let d = root.base();
    if let Some(group) = d.shader_group()? {
        put(out, &group);
        if let Some(dict) = group.texture_dictionary()? {
            put(out, &dict);
            let entries = dict.entries()?;
            for i in 0..entries.len() {
                if let Some(t) = entries.follow(i)? {
                    put(out, &t);
                }
            }
        }
        let shaders = group.shaders()?;
        for i in 0..shaders.len() {
            if let Some(s) = shaders.follow(i)? {
                G::shader(&s, out)?;
            }
        }
    }
    if let Some(skeleton) = d.skeleton()? {
        put(out, &skeleton);
        run(out, skeleton.bones()?);
        run(out, skeleton.inverse_transforms()?);
        run(out, skeleton.default_transforms()?);
    }
    if let Some(joints) = d.joint_data()? {
        put(out, &joints);
        run(out, joints.rotation_limits()?);
        run(out, joints.translation_limits()?);
        run(out, joints.scale_limits()?);
    }
    for lod in lods(image, &d)? {
        put(out, &lod);
        let models = lod.models()?;
        for m in 0..models.len() {
            let Some(model) = models.follow(m)? else {
                continue;
            };
            put(out, &model);
            run(out, model.aabbs()?);
            let geometries = model.geometries()?;
            for g in 0..geometries.len() {
                if let Some(geometry) = geometries.follow(g)? {
                    put(out, &geometry);
                    G::buffers(&geometry, out)?;
                }
            }
        }
    }
    Ok(())
}

fn typed<G: Walked>(image: Image<'_>) -> Result<Filed, String> {
    let mut out = Filed::default();
    walk::<G>(image, &mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

/// The older record's reader took one box past the end of a single-geometry
/// model's run, out of the geometry allocated after it: the box, and its two
/// corners at the geometry's start and sixteen bytes in.
const OVERREAD: &[Overread] = &[
    Overread {
        record: "corpus-cda05819",
        inside: "ShaderResourceViewG9",
        delta: 0,
        bytes: 0x20,
        because: "the same box past an Enhanced model's run, read from the buffer view after it",
    },
    Overread {
        record: "corpus-cda05819",
        inside: "ShaderResourceViewG9",
        delta: 0,
        bytes: 0x10,
        because: "the near corner of that box",
    },
    Overread {
        record: "corpus-cda05819",
        inside: "ShaderResourceViewG9",
        delta: 0x10,
        bytes: 0x10,
        because: "the far corner of that box",
    },
    Overread {
        record: "corpus-cda05819",
        inside: "GrmGeometry",
        delta: 0,
        bytes: 0x20,
        because: "a box past a one-geometry model's run, read from the geometry after it",
    },
    Overread {
        record: "corpus-cda05819",
        inside: "GrmGeometry",
        delta: 0,
        bytes: 0x10,
        because: "the near corner of that box",
    },
    Overread {
        record: "corpus-cda05819",
        inside: "GrmGeometry",
        delta: 0x10,
        bytes: 0x10,
        because: "the far corner of that box",
    },
];

/// An Enhanced texture's view, as the `.ytd` suite records it.
const MOVED: &[FieldMoved] = &[FieldMoved {
    target: "gen9",
    inside: "GrcTextureG9",
    now: offset_of!(Raw<GrcTextureG9>, view),
    was: 0x50,
    len: size_of::<Raw<or_io::gta5::gen9::ShaderResourceViewG9>>(),
}];

const RELOCATED: &[Relocated] = &[Relocated {
    record: "corpus-cda05819",
    inside: "GrcTextureG9",
    delta: 0x50,
    because: "the view is embedded at 0x58; the recorder took 0x50 from the description of the day",
}];

const LEGACY: Gates = Gates {
    census_types: 20,
    relocated: RELOCATED,
    overread: OVERREAD,
    moved: MOVED,
    ..Gates::new(EXT, Generation::Gen8, ("GrmGeometry", "indices_count"))
};

const ENHANCED: Gates = Gates {
    census_types: 15,
    relocated: RELOCATED,
    overread: OVERREAD,
    moved: MOVED,
    ..Gates::new(EXT, Generation::Gen9, ("GrmGeometry", "indices_count"))
};

or_io_verify::declared_gates! {
    gates: LEGACY,
    typed: typed::<Gen8>,
    reads_typed: every_ydr_reads_typed_as_the_tree_reads,
    owned: GtaDrawable<Gen8> => every_legacy_ydr_opens_owned_and_saves_as_its_tree_saves,
    fixed_point: every_ydr_saves_to_a_fixed_point,
    census: every_ydr_instance_is_aligned_for_its_view,
    testimony: the_ydr_testimony_holds_through_its_declarations,
    one_byte_late: a_declared_field_one_byte_late_is_named_by_the_testimony,
}

or_io_verify::declared_gates! {
    gates: ENHANCED,
    typed: typed::<Gen9>,
    reads_typed: every_enhanced_ydr_reads_typed_as_the_tree_reads,
    owned: GtaDrawable<Gen9> => every_enhanced_ydr_opens_owned_and_saves_as_its_tree_saves,
    fixed_point: every_enhanced_ydr_saves_to_a_fixed_point,
    census: every_enhanced_ydr_instance_is_aligned_for_its_view,
    testimony: the_enhanced_ydr_testimony_holds_through_its_declarations,
    one_byte_late: a_declared_enhanced_field_one_byte_late_is_named_by_the_testimony,
}

/// Every Legacy shader group's shaders, as views.
fn legacy_shaders(image: Image<'_>) -> Result<Vec<View<'_, Raw<gen8::GrcInstanceData>>>, E> {
    let root = View::<Raw<GtaDrawable<Gen8>>>::at_offset(image, 0)?;
    let Some(group) = root.base().shader_group()? else {
        return Ok(Vec::new());
    };
    let run = group.shaders()?;
    (0..run.len())
        .filter_map(|i| run.follow(i).transpose())
        .collect()
}

/// The entry table, then the rows, then the hash table at `spu_size`, all
/// inside `total_size`; and the textures lead.
#[test]
fn every_material_blob_holds_its_entries_and_hashes() {
    let Some(paths) = files(EXT, Some(Generation::Gen8), "the .ydr material blobs") else {
        return;
    };
    let (mut materials, mut fits, mut textures_lead) = (0usize, 0usize, 0usize);
    let mut failures = Vec::new();
    for path in &paths {
        let opened = Opened::read(path);
        for (i, m) in legacy_shaders(opened.image()).unwrap().iter().enumerate() {
            materials += 1;
            let count = usize::from(m.count);
            let entries = m.entries().unwrap();
            let (hashes, size) = (usize::from(m.spu_size), usize::from(m.total_size));
            if count
                * size_of::<Raw<or_io::gta5::drawable_codecs::gen8::layout::GrcInstanceDataEntry>>()
                <= hashes
                && hashes + count * 4 <= size
            {
                fits += 1;
            } else {
                failures.push(format!(
                    "{}: material {i}: {count} entries, hashes at {hashes:#x}, {size:#x} bytes",
                    name(path)
                ));
            }
            let lead = entries.iter().take_while(|e| e.count == 0).count();
            let all = entries.iter().filter(|e| e.count == 0).count();
            textures_lead += usize::from(lead == all && all == usize::from(m.texture_count));
        }
    }
    println!(
        "{materials} materials: {fits} hold their entries and hashes inside their blob; \
         {textures_lead} lead with exactly their texture_count texture entries"
    );
    failures.truncate(20);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(fits, materials);
    assert_eq!(textures_lead, materials);
}

/// Every Enhanced material's slot tables inside its block, one copy per
/// worker thread, and the block's size the one the header states.
#[test]
fn every_enhanced_material_block_holds_its_slot_tables() {
    let Some(paths) = files(
        EXT,
        Some(Generation::Gen9),
        "the Enhanced .ydr material blocks",
    ) else {
        return;
    };
    let (mut materials, mut fits) = (0usize, 0usize);
    let mut failures = Vec::new();
    for path in &paths {
        let opened = Opened::read(path);
        let image = opened.image();
        let root = View::<Raw<GtaDrawable<Gen9>>>::at_offset(image, 0).unwrap();
        let Some(group) = root.base().shader_group().unwrap() else {
            continue;
        };
        let run = group.shaders().unwrap();
        for i in 0..run.len() {
            let Some(m) = run.follow(i).unwrap() else {
                continue;
            };
            materials += 1;
            let infos = m
                .param_infos()
                .unwrap()
                .expect("every material states its infos");
            let copies = usize::from(infos.buffer_copies.max(1));
            let size = usize::from(m.param_data_size);
            let base = image.resolve(m.params.raw()).unwrap();
            let window = |ptr: u64, per_copy: u8| -> Option<(usize, usize)> {
                (ptr != 0).then(|| {
                    let at = image.resolve(ptr).unwrap() - base;
                    (at, at + usize::from(per_copy) * copies * 8)
                })
            };
            let tables = [
                Some((0, usize::from(infos.num_buffers) * copies * 8)),
                window(m.texture_refs.raw(), infos.num_textures),
                window(m.other_refs.raw(), infos.num_unknowns),
            ];
            if tables.iter().flatten().all(|&(_, end)| end <= size) {
                fits += 1;
            } else {
                failures.push(format!(
                    "{}: material {i}: {tables:x?} in {size:#x}",
                    name(path)
                ));
            }
        }
    }
    println!("{materials} Enhanced materials: {fits} hold every slot table inside their block");
    failures.truncate(20);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(fits, materials);
}

/// A geometry's own vertex pointer: its buffer's run, in both targets.
#[test]
fn every_geometry_points_at_its_buffers_vertices() {
    fn count<G: VertexDataAt>(t: Generation) {
        let Some(paths) = files(EXT, Some(t), "geometry vertex pointers") else {
            return;
        };
        let (mut geometries, mut shared) = (0usize, 0usize);
        for path in &paths {
            let opened = Opened::read(path);
            let image = opened.image();
            let root = View::<Raw<GtaDrawable<G>>>::at_offset(image, 0).unwrap();
            let d = root.base();
            for lod in lods(image, &d).unwrap() {
                let models = lod.models().unwrap();
                for model in (0..models.len()).filter_map(|i| models.follow(i).unwrap()) {
                    let run = model.geometries().unwrap();
                    for g in (0..run.len()).filter_map(|i| run.follow(i).unwrap()) {
                        geometries += 1;
                        let buffer = image.pointer_at(
                            View::offset(&g) + offset_of!(Raw<GrmGeometry<G>>, vertex_buffer),
                        );
                        let own = g.vertex_data.raw();
                        let data = buffer.ok().and_then(|b| {
                            let at = image.resolve(b).ok()?;
                            image.pointer_at(at + G::vertex_data_at()).ok()
                        });
                        shared += usize::from(own != 0 && Some(own) == data);
                    }
                }
            }
        }
        println!(
            "{} {geometries} geometries: {shared} at their buffer's run",
            t.edition_label()
        );
        assert_eq!(shared, geometries);
    }
    count::<Gen8>(Generation::Gen8);
    count::<Gen9>(Generation::Gen9);
}

/// Where each target's vertex buffer holds its vertices.
trait VertexDataAt: DrawableTarget + Sized {
    fn vertex_data_at() -> usize;
}

impl VertexDataAt for Gen8 {
    fn vertex_data_at() -> usize {
        offset_of!(Raw<gen8::GrcVertexBuffer>, data)
    }
}

impl VertexDataAt for Gen9 {
    fn vertex_data_at() -> usize {
        offset_of!(Raw<gen9::GrcVertexBufferG9>, data)
    }
}

/// A vertex buffer's `vertex_count()` on the owned drawable is the count the file states, in both targets.
#[test]
fn every_vertex_count_reads_as_its_buffer_states_it() {
    fn count<G: DrawableTarget>(
        t: Generation,
        owned: fn(&G::VertexBuffer) -> usize,
        stated: fn(Image<'_>, u64) -> usize,
    ) where
        GtaDrawable<G>: or_io::decl::File,
    {
        let Some(paths) = files(EXT, Some(t), "vertex counts") else {
            return;
        };
        let mut buffers = 0usize;
        for path in &paths {
            let opened = Opened::read(path);
            let image = opened.image();
            let root = View::<Raw<GtaDrawable<G>>>::at_offset(image, 0).unwrap();
            let mut want = Vec::new();
            for lod in lods(image, &root.base()).unwrap() {
                let models = lod.models().unwrap();
                for model in (0..models.len()).filter_map(|i| models.follow(i).unwrap()) {
                    let run = model.geometries().unwrap();
                    for g in (0..run.len()).filter_map(|i| run.follow(i).unwrap()) {
                        let at = View::offset(&g) + offset_of!(Raw<GrmGeometry<G>>, vertex_buffer);
                        want.push(stated(image, image.pointer_at(at).unwrap()));
                    }
                }
            }
            let d = <GtaDrawable<G> as or_io::decl::File>::open(path)
                .unwrap_or_else(|e| panic!("{path:?}: {e}"));
            let got: Vec<usize> = (d.base.lod_group.lods.iter())
                .flat_map(|lod| &lod.models)
                .flat_map(|m| &m.geometries)
                .map(|g| owned(&g.vertex_buffer))
                .collect();
            assert_eq!(got, want, "{path:?}");
            buffers += got.len();
        }
        println!("{} {buffers} vertex buffers", t.edition_label());
        assert!(buffers > 0);
    }
    count::<Gen8>(
        Generation::Gen8,
        gen8::GrcVertexBuffer::vertex_count,
        |image, at| {
            View::<Raw<gen8::GrcVertexBuffer>>::at_address(image, at)
                .unwrap()
                .vertex_count as usize
        },
    );
    count::<Gen9>(
        Generation::Gen9,
        gen9::GrcVertexBufferG9::vertex_count,
        |image, at| {
            View::<Raw<gen9::GrcVertexBufferG9>>::at_address(image, at)
                .unwrap()
                .vertex_count as usize
        },
    );
}

/// How each shipped material binds its textures, as the owned form holds them.
#[test]
fn every_texture_binding_is_the_dictionarys_or_a_reference() {
    #[derive(Default, Debug)]
    struct Kinds {
        unbound: usize,
        embedded: usize,
        references: usize,
    }
    fn tally<T, R>(kinds: &mut Kinds, slot: &TextureSlot<T, R>) {
        match slot {
            TextureSlot::Unbound => kinds.unbound += 1,
            TextureSlot::Embedded(_) => kinds.embedded += 1,
            TextureSlot::Reference(_) => kinds.references += 1,
        }
    }
    let (mut legacy, mut enhanced) = (Kinds::default(), Kinds::default());
    let mut opened = 0usize;
    for paths in [Generation::Gen8, Generation::Gen9]
        .into_iter()
        .filter_map(|t| files(EXT, Some(t), "the .ydr texture bindings"))
    {
        for path in &paths {
            match or_io::open(path) {
                Ok(Asset::Drawable(d)) => {
                    opened += 1;
                    let group = d.shader_group.as_deref();
                    for s in group.iter().flat_map(|g| &g.shaders) {
                        for e in &s.entries {
                            if let or_io::gta5::gen8::EntryData::Texture(slot) = &e.data {
                                tally(&mut legacy, slot);
                            }
                        }
                    }
                }
                Ok(Asset::DrawableGen9(d)) => {
                    let group = d.shader_group.as_deref();
                    for s in group.iter().flat_map(|g| &g.shaders) {
                        s.params
                            .textures
                            .iter()
                            .for_each(|slot| tally(&mut enhanced, slot));
                    }
                }
                Ok(_) => panic!("{} did not open as a drawable", name(path)),
                Err(e) => panic!("{}: {e}", name(path)),
            }
        }
    }
    println!("Legacy: {legacy:?}; Enhanced: {enhanced:?}");
    assert!(opened == 0 || (legacy.embedded > 0 && legacy.references > 0));
}

/// Each owned `.ydr` names its dictionary's textures by the code the dictionary files them under.
#[test]
fn an_embedded_binding_names_a_code_its_dictionary_holds() {
    let Some(paths) = files(EXT, Some(Generation::Gen8), "the .ydr embedded bindings") else {
        return;
    };
    let mut checked = 0usize;
    for path in &paths {
        let Ok(d) = GtaDrawable::<Gen8>::open(path) else {
            continue;
        };
        let Some(group) = d.shader_group.as_deref() else {
            continue;
        };
        let codes: &[u32] = group
            .texture_dictionary
            .as_deref()
            .map_or(&[], |t: &PgDictionary<_>| &t.codes);
        for s in &group.shaders {
            for e in &s.entries {
                if let or_io::gta5::gen8::EntryData::Texture(TextureSlot::Embedded(k)) = &e.data {
                    let key = k.key.expect("an embedded binding states its code");
                    assert!(codes.contains(&key), "{}: {key:#010x}", name(path));
                    checked += 1;
                }
            }
        }
    }
    println!("{checked} embedded bindings name a code their dictionary holds");
    assert!(checked > 0);
}

/// The tint pointer and the shader group's variables, which nothing shipped sets.
#[test]
fn pointers_no_shipped_drawable_sets() {
    fn count<G: DrawableTarget>(t: Generation) {
        let Some(paths) = files(EXT, Some(t), "the .ydr pointers nothing sets") else {
            return;
        };
        let (mut tints, mut vars) = (0usize, 0usize);
        for path in &paths {
            let opened = Opened::read(path);
            let image = opened.image();
            let root = View::<Raw<GtaDrawable<G>>>::at_offset(image, 0).unwrap();
            tints += usize::from(!root.tint_data.is_null());
            if let Some(group) = root.base().shader_group().unwrap() {
                vars += usize::from(group.shader_group_vars.raw() != 0);
            }
        }
        println!(
            "{} {} drawables: {tints} set a tint pointer; {vars} shader groups set their variables",
            paths.len(),
            t.edition_label()
        );
        assert_eq!((tints, vars), (0, 0));
    }
    count::<Gen8>(Generation::Gen8);
    count::<Gen9>(Generation::Gen9);
}

#[test]
#[ignore = "a timing, meaningful only in a release build; run by name"]
fn what_a_ydr_read_costs() {
    use std::hint::black_box;
    use std::time::{Duration, Instant};
    // Every geometry's counts, typed from the root.
    #[inline(never)]
    fn typed_sum<G: DrawableTarget>(image: Image<'_>) -> usize {
        let root = View::<Raw<GtaDrawable<G>>>::at_offset(image, 0).unwrap();
        let mut total = 0usize;
        for lod in lods(image, &root.base()).unwrap() {
            let models = lod.models().unwrap();
            for model in (0..models.len()).filter_map(|i| models.follow(i).ok().flatten()) {
                let geometries = model.geometries().unwrap();
                for g in (0..geometries.len()).filter_map(|i| geometries.follow(i).ok().flatten()) {
                    total += g.indices_count as usize + usize::from(g.vertices_count);
                }
            }
        }
        total
    }
    let best = |rounds: usize, f: &mut dyn FnMut() -> usize| {
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let start = Instant::now();
            let mut sink = 0usize;
            for _ in 0..rounds {
                sink = sink.wrapping_add(f());
            }
            black_box(sink);
            best = best.min(start.elapsed());
        }
        best.as_secs_f64() * 1e3 / rounds as f64
    };
    for t in [Generation::Gen8, Generation::Gen9] {
        let Some(paths) = files(EXT, Some(t), "the .ydr read cost") else {
            continue;
        };
        let opened: Vec<Opened> = paths.iter().map(|p| Opened::read(p)).collect();
        let sum = if t == Generation::Gen9 {
            typed_sum::<Gen9>
        } else {
            typed_sum::<Gen8>
        };
        let typed_ms = best(2_000, &mut || {
            opened.iter().map(|o| sum(black_box(o.image()))).sum()
        });
        let tree_ms = best(5, &mut || {
            opened
                .iter()
                .map(|o| {
                    usize::from(
                        gates::format(EXT)
                            .read(t, o.image())
                            .unwrap()
                            .record()
                            .is_some(),
                    )
                })
                .sum()
        });
        println!(
            "{} {} files: every geometry's counts typed from the root {typed_ms:.4} ms; the whole \
             tree read {tree_ms:.2} ms ({:.0}x)",
            paths.len(),
            t.edition_label(),
            tree_ms / typed_ms
        );
    }
}

/// Each word the owned records carry with a note, tallied over the corpus: a value the note does not state fails.
#[test]
fn the_carried_drawable_words_hold_what_their_notes_state() {
    #[derive(Default)]
    struct Seen(BTreeMap<&'static str, (usize, usize)>);
    impl Seen {
        fn hold(&mut self, what: &'static str, holds: bool) {
            let e = self.0.entry(what).or_default();
            e.0 += 1;
            e.1 += usize::from(holds);
        }
    }
    fn words<G: DrawableTarget>(t: Generation, seen: &mut Seen) -> bool {
        let Some(paths) = files(EXT, Some(t), "the .ydr carried words") else {
            return false;
        };
        for path in &paths {
            let opened = Opened::read(path);
            let image = opened.image();
            let root = View::<Raw<GtaDrawable<G>>>::at_offset(image, 0).unwrap();
            for l in root.lights().unwrap().views() {
                seen.hold("CLightAttr.vft zero", l.vft == 0);
                seen.hold(
                    "CLightAttr.padding zero",
                    (l.padding1, l.padding2, l.padding3) == (0, 0, 0),
                );
            }
            let d = root.base();
            if let Some(s) = d.skeleton().unwrap() {
                seen.hold("CrSkeletonData.properties null", s.properties.is_null());
                let (count, bones) = (s.bone_tags_count, s.num_bones);
                seen.hold(
                    "CrSkeletonData.bone_tags_count the bone count or one less",
                    count == bones || count + 1 == bones,
                );
            }
            if let Some(g) = d.shader_group().unwrap() {
                seen.hold(
                    "GrmShaderGroup.shader_group_vars empty",
                    g.shader_group_vars.raw() == 0,
                );
            }
            for lod in lods(image, &d).unwrap() {
                let models = lod.models().unwrap();
                for model in (0..models.len()).filter_map(|i| models.follow(i).unwrap()) {
                    let run = model.geometries().unwrap();
                    for g in (0..run.len()).filter_map(|i| run.follow(i).unwrap()) {
                        seen.hold(
                            "GrmGeometry's unfollowed pointers null",
                            [
                                g.vtx_decl,
                                g.vb_slot1,
                                g.vb_slot2,
                                g.vb_slot3,
                                g.ib_slot1,
                                g.ib_slot2,
                                g.ib_slot3,
                                g.vtx_decl_offset,
                                g.offset_buffer,
                            ]
                            .iter()
                            .all(|p| p.is_null()),
                        );
                    }
                }
            }
        }
        true
    }
    let mut seen = Seen::default();
    let found =
        words::<Gen8>(Generation::Gen8, &mut seen) | words::<Gen9>(Generation::Gen9, &mut seen);
    let mut wrong = Vec::new();
    for (what, (n, held)) in &seen.0 {
        println!("{what:<60} {held} of {n}");
        if held != n {
            wrong.push(*what);
        }
    }
    assert!(!found || !seen.0.is_empty(), "nothing was tallied");
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// The Legacy buffers' and shaders' carried words, and the Enhanced ones', each as its note states.
#[test]
fn the_carried_buffer_and_shader_words_hold_what_their_notes_state() {
    let (mut n, mut wrong) = (0usize, Vec::new());
    if let Some(paths) = files(EXT, Some(Generation::Gen8), "the Legacy carried words") {
        for path in &paths {
            let opened = Opened::read(path);
            let image = opened.image();
            let root = View::<Raw<GtaDrawable<Gen8>>>::at_offset(image, 0).unwrap();
            for lod in lods(image, &root.base()).unwrap() {
                let models = lod.models().unwrap();
                for model in (0..models.len()).filter_map(|i| models.follow(i).unwrap()) {
                    let run = model.geometries().unwrap();
                    for g in (0..run.len()).filter_map(|i| run.follow(i).unwrap()) {
                        let vb = g.vertex_buffer().unwrap().unwrap();
                        n += 1;
                        if (vb.reserved0, vb.reserved) != (0, 0) {
                            wrong.push(format!("{}: a vertex buffer's reserved words", name(path)));
                        }
                    }
                }
            }
        }
    }
    if let Some(paths) = files(EXT, Some(Generation::Gen9), "the Enhanced carried words") {
        for path in &paths {
            let opened = Opened::read(path);
            let image = opened.image();
            let root = View::<Raw<GtaDrawable<Gen9>>>::at_offset(image, 0).unwrap();
            let d = root.base();
            if let Some(group) = d.shader_group().unwrap() {
                let run = group.shaders().unwrap();
                for s in (0..run.len()).filter_map(|i| run.follow(i).unwrap()) {
                    n += 1;
                    let infos = s.param_infos().unwrap().unwrap();
                    if (
                        s.after_param_infos,
                        s.second_after_param_infos,
                        s.third_after_param_infos,
                        infos.after_num_params,
                        infos.second_after_num_params,
                    ) != (0, 0, 0, 0, 0)
                        || !s.other_refs.is_null()
                    {
                        wrong.push(format!("{}: a shader's carried words", name(path)));
                    }
                }
            }
            for lod in lods(image, &d).unwrap() {
                let models = lod.models().unwrap();
                for model in (0..models.len()).filter_map(|i| models.follow(i).unwrap()) {
                    let run = model.geometries().unwrap();
                    for g in (0..run.len()).filter_map(|i| run.follow(i).unwrap()) {
                        n += 1;
                        let vb = g.vertex_buffer().unwrap().unwrap();
                        let ib = g.index_buffer().unwrap().unwrap();
                        if (
                            vb.after_stride,
                            vb.after_bind_flags,
                            vb.after_data,
                            vb.second_after_data,
                        ) != (0, 0, 0, 0)
                            || (
                                ib.after_index_size,
                                ib.after_bind_flags,
                                ib.after_indices,
                                ib.second_after_indices,
                                ib.after_srv,
                            ) != (0, 0, 0, 0, 0)
                        {
                            wrong.push(format!("{}: a buffer's carried words", name(path)));
                        }
                    }
                }
            }
        }
    }
    println!("{n} buffers and shaders hold the words their notes state");
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// A material pointed at a code its drawable's dictionary does not hold is refused at save.
#[test]
fn an_embed_outside_the_dictionary_is_refused_at_save() {
    use or_io::decl::{File, Keyed};
    use or_io::gta5::gen8::EntryData;
    let Some(paths) = files(
        EXT,
        Some(Generation::Gen8),
        "an embed outside the dictionary",
    ) else {
        return;
    };
    let mut refused = 0usize;
    for path in &paths {
        let Ok(mut d) = GtaDrawable::<Gen8>::open(path) else {
            continue;
        };
        let Some(group) = d.shader_group.as_deref_mut() else {
            continue;
        };
        let Some(dict) = group.texture_dictionary.as_deref() else {
            continue;
        };
        let stray = (0..=u32::MAX)
            .find(|c| !dict.codes.contains(c))
            .expect("a code the dictionary does not hold");
        let Some(entry) = group
            .shaders
            .iter_mut()
            .flat_map(|s| s.entries.iter_mut())
            .find(|e| matches!(e.data, EntryData::Texture(_)))
        else {
            continue;
        };
        entry.data = EntryData::Texture(TextureSlot::Embedded(Keyed::to(stray)));
        let err = d
            .to_bytes()
            .expect_err("a binding outside the dictionary must not save");
        let why = err.to_string();
        assert!(
            why.contains(&format!("{stray:#010x}")) && why.contains("dictionary"),
            "{}: refused without naming the binding: {why}",
            name(path)
        );
        refused += 1;
    }
    assert!(refused > 0, "no staged drawable carries a dictionary");
}

// What a material may not break, Legacy and Enhanced, and what embedding a texture keeps.

/// Every drawable a Legacy drawable-bearing file holds.
fn legacy_drawables(asset: &Asset) -> Vec<&RmcDrawable<Gen8>> {
    match asset {
        Asset::Drawable(d) => vec![&d.base],
        Asset::DrawableDict(dict) => dict.entries.iter().flatten().map(|d| &d.base).collect(),
        Asset::Fragment(f) => fragment_drawables(&f.base),
        _ => Vec::new(),
    }
}

/// Every drawable an Enhanced drawable-bearing file holds.
fn enhanced_drawables(asset: &Asset) -> Vec<&RmcDrawable<Gen9>> {
    match asset {
        Asset::DrawableGen9(d) => vec![&d.base],
        Asset::DrawableDictGen9(dict) => dict.entries.iter().flatten().map(|d| &d.base).collect(),
        Asset::FragmentGen9(f) => fragment_drawables(&f.base),
        _ => Vec::new(),
    }
}

fn fragment_drawables<G: DrawableTarget>(f: &or_io::gta5::FragType<G>) -> Vec<&RmcDrawable<G>> {
    let mut out: Vec<&RmcDrawable<G>> = Vec::new();
    out.extend(f.common_drawable.as_deref().map(|d| &d.base));
    out.extend(f.extra_drawables.iter().map(|d| &d.base));
    out.extend(f.common_cloth_drawable.as_deref().map(|d| &d.base));
    if let Some(group) = f.physics_lod_group.as_deref() {
        for lod in [&group.high_lod, &group.medium_lod, &group.low_lod]
            .into_iter()
            .flatten()
        {
            for child in &lod.children {
                out.extend(child.undamaged_entity.as_deref().map(|d| &d.base));
                out.extend(child.damaged_entity.as_deref().map(|d| &d.base));
            }
        }
    }
    out
}

type Found = Vec<(String, or_io::gta5::shaders::shader_defs::MaterialRef)>;

/// Every Legacy material in every format that can hold one, as the owned walk reads it, and the files it refused.
///
/// ⚠ `.ypt` is here for its model dictionary, not its particle rules: a rule
/// states no texture count, but every drawable in the dictionary carries a
/// real `grcInstanceData`.
fn legacy_materials(what: &str) -> Option<(Found, Vec<String>)> {
    use or_io::gta5::shaders::shader_defs::materials::materials_in;
    let (mut found, mut refused, mut any) = (Vec::new(), Vec::new(), false);
    for ext in ["ydr", "ydd", "yft", "ypt"] {
        let Some(paths) = files(ext, Some(Generation::Gen8), what) else {
            continue;
        };
        any = true;
        for path in &paths {
            let bytes = std::fs::read(path).expect("a corpus file is readable");
            match materials_in(path, &bytes) {
                Ok(ms) => found.extend(
                    (ms.into_iter())
                        .filter(|m| m.declared_texture_count.is_some())
                        .map(|m| (name(path), m)),
                ),
                Err(e) => refused.push(format!("{}: {e}", name(path))),
            }
        }
    }
    any.then_some((found, refused))
}

/// The claim the editor's insert position rests on: every shipped Legacy
/// material holds its texture slots as a leading run and states a count equal
/// to its length. The owned open refuses a blob whose stated count its
/// entries do not lay out, and moves the textures of a file that states them
/// after a constant ahead of it, as a tool that appends a sampler writes one.
#[test]
fn a_texture_slot_is_a_prefix_of_the_entry_list_and_the_count_says_how_long() {
    let Some((found, refused)) = legacy_materials("a material's textures lead") else {
        return;
    };
    let wrong: Vec<String> = (found.iter())
        .filter(|(_, m)| {
            !m.bindings_are_a_prefix()
                || m.declared_texture_count != Some(m.leading_bindings() as u8)
        })
        .map(|(file, m)| format!("{file}#{}", m.owner))
        .chain(refused)
        .collect();
    println!(
        "{} Legacy materials: texture slots lead in all but {}",
        found.len(),
        wrong.len()
    );
    assert!(!found.is_empty(), "no material was reached");
    assert!(wrong.is_empty(), "{:?}", &wrong[..wrong.len().min(10)]);
}

/// ⚠ A register is the shader's constant register and not the entry's
/// position, which is what the name-hash table is keyed by; were the two
/// equal everywhere, an editor written against either would look right.
#[test]
fn a_registers_value_is_not_its_position_in_the_list() {
    let Some((found, _)) = legacy_materials("a register is not a list position") else {
        return;
    };
    assert!(
        (found.iter()).any(|(_, m)| (m.entries.iter().enumerate())
            .any(|(i, e)| usize::from(e.register) != i)),
        "no material has a register other than its list position, so nothing here tells \
         the two apart"
    );
}

/// Which of a parameter's two spellings a Legacy drawable hashes: the declared name, which `GrcInstanceDataEntry::declared` writes.
#[test]
fn a_drawable_hashes_a_parameters_declared_name_and_not_its_semantic() {
    let Some(defs) = or_io::gta5::shaders::shader_defs::staged() else {
        or_test_support::skip_corpus(
            "which spelling a drawable hashes",
            "no shader definition database staged",
        );
        return;
    };
    let Some((found, _)) = legacy_materials("which spelling a drawable hashes") else {
        return;
    };
    let (mut by_name, mut by_semantic_only, mut examples) = (0usize, 0usize, Vec::new());
    for (_, material) in &found {
        let Some(shader) = defs.shader(material.shader_hash) else {
            continue;
        };
        for entry in &material.entries {
            let named = (shader.parameters.iter()).any(|p| p.name_hash == entry.name_hash);
            let semantic =
                (shader.parameters.iter()).find(|p| p.semantic_hash == Some(entry.name_hash));
            match (named, semantic) {
                (true, _) => by_name += 1,
                (false, Some(p)) => {
                    by_semantic_only += 1;
                    if examples.len() < 10 {
                        examples.push(format!("{} {:?}", shader.name, p.semantic));
                    }
                }
                (false, None) => {}
            }
        }
    }
    println!(
        "{by_name} drawable entries match the declared name, {by_semantic_only} only a semantic"
    );
    assert!(by_name > 0, "nothing resolved, so nothing was measured");
    assert_eq!(by_semantic_only, 0, "{examples:?}");
}

/// A Legacy material whose texture follows a constant is refused at save: the engine reads the textures as a prefix.
#[test]
fn a_texture_after_a_constant_is_refused_at_save() {
    use or_io::decl::File;
    use or_io::gta5::gen8::EntryData;
    let Some(paths) = files(EXT, Some(Generation::Gen8), "a texture after a constant") else {
        return;
    };
    let mut refused = 0usize;
    for path in &paths {
        let mut d = GtaDrawable::<Gen8>::open(path).unwrap();
        let Some(shader) = (d.shader_group.as_deref_mut()).and_then(|g| {
            g.shaders.iter_mut().find(|s| {
                s.entries
                    .iter()
                    .any(|e| matches!(e.data, EntryData::Texture(_)))
                    && s.entries
                        .iter()
                        .any(|e| matches!(e.data, EntryData::Float(_)))
            })
        }) else {
            continue;
        };
        let at = (shader.entries.iter())
            .position(|e| matches!(e.data, EntryData::Texture(_)))
            .unwrap();
        let texture = shader.entries.remove(at);
        shader.entries.push(texture);
        let why = d
            .to_bytes()
            .expect_err("a texture after a constant must not save");
        assert!(
            why.to_string().contains("textures lead"),
            "{}: {why}",
            name(path)
        );
        refused += 1;
    }
    assert!(
        refused > 0,
        "no drawable had a material with both kinds of entry"
    );
}

/// Every edit the material editor offers, made on a real drawable and read back.
///
/// ⚠ A save lays the whole blob out again from the entry list — the count,
/// the texture count, both sizes, the rows and the hash table — so a change
/// to the list changes five other things.
#[test]
fn every_edit_the_editor_offers_survives_a_write_and_a_read() {
    use or_io::decl::{File, Vec4};
    use or_io::gta5::gen8::{EntryData, GrcInstanceDataEntry};
    use or_io::gta5::shaders::shader_defs::{hash_name, ShaderParamDef, ShaderParamKind};
    let Some(paths) = files(
        EXT,
        Some(Generation::Gen8),
        "an edited material round-trips",
    ) else {
        return;
    };
    let def = |name: &str, kind| ShaderParamDef {
        name: name.to_string(),
        name_hash: hash_name(name),
        kind,
        array_len: 1,
        register: 200,
        cbuffer: None,
        drawable_place: None,
        semantic: None,
        semantic_hash: None,
        ui_name: None,
        default: None,
        preset_default: None,
    };
    let constant = def("openrage_roundtrip_constant", ShaderParamKind::Float4);
    let sampler = def("openrage_roundtrip_sampler", ShaderParamKind::Texture);
    let value = Vec4 {
        x: 0.25,
        y: 0.5,
        z: 0.75,
        w: 1.0,
    };
    let mut checked = 0usize;
    for path in &paths {
        let mut d = GtaDrawable::<Gen8>::open(path).unwrap();
        let Some(group) = d.shader_group.as_deref_mut() else {
            continue;
        };
        let bound = group.binding("openrage_roundtrip");
        let Some(shader) = group.shaders.first_mut() else {
            continue;
        };
        let cleared = (shader.entries.iter())
            .find(|e| matches!(e.data, EntryData::Texture(_)))
            .map(|e| e.name_hash.get());
        let mut c = GrcInstanceDataEntry::declared(&constant);
        c.data = EntryData::Float(vec![value]);
        let at_constant = shader.insert_entry(c);
        assert_eq!(
            at_constant + 1,
            shader.entries.len(),
            "a constant joins the tail"
        );
        let mut s = GrcInstanceDataEntry::declared(&sampler);
        s.data = EntryData::Texture(bound.clone());
        let at_sampler = shader.insert_entry(s);
        assert!(
            shader.entries[..at_sampler]
                .iter()
                .all(|e| matches!(e.data, EntryData::Texture(_))),
            "a sampler joins the end of the texture prefix"
        );
        if let Some(h) = cleared {
            shader.entry_mut(h).unwrap().data = EntryData::Texture(TextureSlot::Unbound);
        }
        let before = shader.clone();

        let bytes = d
            .to_bytes()
            .unwrap_or_else(|e| panic!("{}: {e}", name(path)));
        let reread = GtaDrawable::<Gen8>::from_bytes(&bytes).unwrap();
        let after = &reread.shader_group.as_deref().unwrap().shaders[0];
        assert_eq!(after.entries, before.entries, "{}", name(path));
        assert_eq!(
            after.entry(constant.name_hash).map(|e| &e.data),
            Some(&EntryData::Float(vec![value]))
        );
        assert_eq!(
            after.entry(sampler.name_hash).map(|e| &e.data),
            Some(&EntryData::Texture(bound))
        );
        if let Some(h) = cleared {
            assert_eq!(
                after.entry(h).map(|e| &e.data),
                Some(&EntryData::Texture(TextureSlot::Unbound))
            );
        }
        checked += 1;
    }
    println!(
        "{checked} of {} drawables round-tripped an edit",
        paths.len()
    );
    assert!(checked > 0, "no drawable offered a material to edit");
}

/// A material's bindings into its group's own dictionary still land there after a save.
///
/// ⚠ A binding is a pointer at a texture the dictionary owns, so one written
/// before its dictionary resolves nowhere; the names survive such a save, so
/// what is counted is bindings that land inside the group's own dictionary.
#[test]
fn a_material_keeps_its_texture_bindings_across_a_save() {
    use or_io::decl::File;
    fn into_own_dictionary(d: &RmcDrawable<Gen8>) -> usize {
        let Some(group) = d.shader_group.as_deref() else {
            return 0;
        };
        let codes: &[u32] = group
            .texture_dictionary
            .as_deref()
            .map_or(&[], |t| &t.codes);
        (group.shaders.iter())
            .flat_map(|s| s.textures())
            .filter(|slot| {
                matches!(slot, TextureSlot::Embedded(k) if k.key.is_some_and(|c| codes.contains(&c)))
            })
            .count()
    }
    let (mut before_total, mut opened, mut lost) = (0usize, 0usize, Vec::new());
    for ext in ["ydr", "ydd"] {
        let Some(paths) = files(
            ext,
            Some(Generation::Gen8),
            "texture bindings across a save",
        ) else {
            continue;
        };
        for path in &paths {
            let asset = or_io::open(path).unwrap_or_else(|e| panic!("{}: {e}", name(path)));
            let before: Vec<usize> = (legacy_drawables(&asset).into_iter())
                .map(into_own_dictionary)
                .collect();
            let bytes = asset.to_bytes().unwrap();
            // ⚠ A `.ydr` and a `.ydd` share a version, so the header alone names neither.
            let saved = match &asset {
                Asset::DrawableDict(_) => {
                    Asset::DrawableDict(PgDictionary::<GtaDrawable>::from_bytes(&bytes).unwrap())
                }
                _ => Asset::Drawable(GtaDrawable::<Gen8>::from_bytes(&bytes).unwrap()),
            };
            let after: Vec<usize> = (legacy_drawables(&saved).into_iter())
                .map(into_own_dictionary)
                .collect();
            before_total += before.iter().sum::<usize>();
            opened += 1;
            if before != after {
                lost.push(format!("{}: {before:?} then {after:?}", name(path)));
            }
        }
    }
    println!("{before_total} bindings into their group's own dictionary before a save");
    assert!(lost.is_empty(), "{lost:?}");
    assert!(
        opened == 0 || before_total > 0,
        "no material binds a texture its own group carries"
    );
}

/// Unembedding a carried texture rebinds its materials by name, and embedding it again binds them to it again.
#[test]
fn unembedding_and_embedding_again_keeps_every_binding() {
    use or_io::decl::File;
    let Some(paths) = files(
        EXT,
        Some(Generation::Gen8),
        "unembedding and embedding again",
    ) else {
        return;
    };
    let bindings = |d: &GtaDrawable<Gen8>| -> Vec<_> {
        (d.shader_group.as_deref().iter())
            .flat_map(|g| g.shaders.iter().flat_map(|s| s.textures().cloned()))
            .collect()
    };
    let (mut moved, mut rebound) = (0usize, 0usize);
    for path in &paths {
        let original = GtaDrawable::<Gen8>::open(path).unwrap();
        let mut d = original.clone();
        let Some(group) = d.shader_group.as_deref_mut() else {
            continue;
        };
        let Some(carried) = (group.texture_dictionary.as_deref())
            .and_then(|t| t.entries.iter().flatten().next())
            .map(|t| t.name.clone())
        else {
            continue;
        };
        let code = or_io::string_hash_str(&carried);
        let users = group.users(&carried);
        let texture = group.unembed(&carried).expect("carried");
        assert_eq!(
            group.users(&carried),
            users,
            "{}: the references stay",
            name(path)
        );
        assert!(
            (group.shaders.iter())
                .flat_map(|s| s.textures())
                .all(|slot| !matches!(slot, TextureSlot::Embedded(k) if k.key == Some(code))),
            "{}: a binding still names the unembedded code",
            name(path)
        );
        d.to_bytes()
            .unwrap_or_else(|e| panic!("{}: an unembedded drawable saves: {e}", name(path)));
        let group = d.shader_group.as_deref_mut().unwrap();
        assert!(group.embed(texture).is_none());
        assert_eq!(group.users(&carried), users);
        let saved = GtaDrawable::<Gen8>::from_bytes(&d.to_bytes().unwrap()).unwrap();
        assert_eq!(bindings(&saved), bindings(&original), "{}", name(path));
        moved += 1;
        rebound += usize::from(!users.is_empty());
    }
    println!("{moved} textures unembedded and embedded again, {rebound} of them bound");
    assert!(
        rebound > 0,
        "no drawable carries a texture a material binds"
    );
}

/// Every owned Enhanced drawable-bearing file, by name.
fn enhanced_files(what: &str) -> Option<Vec<(String, Asset)>> {
    let mut out = Vec::new();
    for ext in ["ydr", "ydd", "yft"] {
        for path in files(ext, Some(Generation::Gen9), what).unwrap_or_default() {
            let asset = or_io::open(&path).unwrap_or_else(|e| panic!("{}: {e}", name(&path)));
            out.push((name(&path), asset));
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Every shipped Enhanced material saves, so the save-time claim refuses broken edits and not real files.
#[test]
fn every_shipped_enhanced_material_already_satisfies_the_claim() {
    let Some(assets) = enhanced_files("what an Enhanced material may not break") else {
        return;
    };
    let mut materials = 0usize;
    for (file, asset) in &assets {
        materials += (enhanced_drawables(asset).iter())
            .filter_map(|d| d.shader_group.as_deref())
            .map(|g| g.shaders.len())
            .sum::<usize>();
        asset.to_bytes().unwrap_or_else(|e| panic!("{file}: {e}"));
    }
    println!(
        "{materials} Enhanced materials across {} files save",
        assets.len()
    );
    assert!(materials > 0);
}

/// Where each edited material was, and what saving it gave.
type Saved = Vec<(String, Result<Vec<u8>, or_io::Error>)>;

/// Each Enhanced material of every `.ydr`, edited by `edit` where it returns true, and saved.
fn each_enhanced_material(
    what: &str,
    mut edit: impl FnMut(&mut gen9::GrcInstanceDataG9) -> bool,
) -> Option<Saved> {
    use or_io::decl::File;
    let paths = files(EXT, Some(Generation::Gen9), what)?;
    let mut out = Vec::new();
    for path in &paths {
        let original = GtaDrawable::<Gen9>::open(path).unwrap();
        let count = (original.shader_group.as_deref()).map_or(0, |g| g.shaders.len());
        for i in 0..count {
            let mut d = original.clone();
            let shader = &mut d.shader_group.as_deref_mut().unwrap().shaders[i];
            if edit(shader) {
                out.push((format!("{}#{i}", name(path)), d.to_bytes()));
            }
        }
    }
    Some(out)
}

/// ⚠ The Legacy prefix does not transfer: a texture names its slot and a
/// constant its buffer by an index of its own, so the order of the list is
/// free, and every shipped material reordered still saves.
#[test]
fn the_order_of_the_parameter_list_is_not_load_bearing() {
    let Some(saved) = each_enhanced_material("an Enhanced list reordered", |s| {
        let params = &mut s.param_infos.params;
        match params.len().checked_sub(1) {
            Some(last) if last > 0 => {
                params.swap(0, last);
                true
            }
            _ => false,
        }
    }) else {
        return;
    };
    let refused: Vec<String> = (saved.iter())
        .filter_map(|(at, r)| r.as_ref().err().map(|e| format!("{at}: {e}")))
        .collect();
    println!("{} Enhanced materials reordered", saved.len());
    assert!(!saved.is_empty(), "no material had two parameters to swap");
    assert!(refused.is_empty(), "{refused:?}");
}

/// An edited constant reaches the file and moves no other constant.
#[test]
fn an_edited_constant_survives_a_write_and_the_rest_of_the_material_does_not_move() {
    use or_io::decl::File;
    let Some(paths) = files(EXT, Some(Generation::Gen9), "an edited Enhanced constant") else {
        return;
    };
    let mut edited = 0usize;
    for path in &paths {
        let mut d = GtaDrawable::<Gen9>::open(path).unwrap();
        let Some(shader) = (d.shader_group.as_deref_mut()).and_then(|g| g.shaders.first_mut())
        else {
            continue;
        };
        let Some(hash) = (shader.param_infos.params.iter())
            .find(|p| p.constant().is_some_and(|(_, r)| !r.is_empty()))
            .map(|p| p.name.get())
        else {
            continue;
        };
        let bytes = shader.constant_mut(hash).expect("inside its buffer");
        let wanted: Vec<u8> = (0..bytes.len()).map(|i| i as u8 ^ 0x5a).collect();
        bytes.copy_from_slice(&wanted);
        let before = shader.params.buffers.clone();
        let saved = GtaDrawable::<Gen9>::from_bytes(&d.to_bytes().unwrap()).unwrap();
        let mut after = saved.shader_group.as_deref().unwrap().shaders[0].clone();
        assert_eq!(after.constant_mut(hash).map(|b| b.to_vec()), Some(wanted));
        assert_eq!(after.params.buffers, before, "{}", name(path));
        edited += 1;
    }
    assert!(
        edited > 0,
        "no Enhanced drawable offered a constant to edit"
    );
}

/// ⚠ A block's counts are its lists' lengths, so no edit can make them contradict the lists: a slot no parameter names saves, and reopens counted.
#[test]
fn a_materials_counts_are_its_lists_lengths() {
    use or_io::decl::File;
    let mut before = Vec::new();
    let Some(saved) = each_enhanced_material("an Enhanced texture slot added", |s| {
        s.params.textures.push(TextureSlot::Unbound);
        before.push(s.params.textures.len());
        true
    }) else {
        return;
    };
    assert!(!saved.is_empty(), "nothing was presented");
    for ((at, r), wanted) in saved.iter().zip(before) {
        let bytes = r.as_ref().unwrap_or_else(|e| panic!("{at}: {e}"));
        let i: usize = at.rsplit('#').next().unwrap().parse().unwrap();
        let d = GtaDrawable::<Gen9>::from_bytes(bytes).unwrap();
        let shader = &d.shader_group.as_deref().unwrap().shaders[i];
        assert_eq!(shader.params.textures.len(), wanted, "{at}");
    }
}

/// An index past its own run is refused; two parameters naming one slot address a real slot and save.
#[test]
fn an_index_may_collide_and_the_claim_deliberately_stops_at_range() {
    const SLOT: u32 = 0xFF << 2;
    fn textures(s: &gen9::GrcInstanceDataG9) -> Vec<usize> {
        (s.param_infos.params.iter().enumerate())
            .filter(|(_, p)| p.texture().is_some())
            .map(|(i, _)| i)
            .collect()
    }
    let Some(past) = each_enhanced_material("an Enhanced index past its run", |s| {
        let Some(&i) = textures(s).first() else {
            return false;
        };
        let slot = s.params.textures.len() as u32;
        let p = &mut s.param_infos.params[i];
        p.data = (p.data & !SLOT) | (slot << 2);
        true
    }) else {
        return;
    };
    assert!(!past.is_empty(), "no material binds a texture");
    for (at, r) in &past {
        let why = r.as_ref().expect_err("an index past its run must not save");
        assert!(
            why.to_string().contains("binds texture slot"),
            "{at}: {why}"
        );
    }
    let collided = each_enhanced_material("an Enhanced index collided", |s| {
        let t = textures(s);
        if t.len() < 2 {
            return false;
        }
        let first = s.param_infos.params[t[0]].data & SLOT;
        let p = &mut s.param_infos.params[t[1]];
        p.data = (p.data & !SLOT) | first;
        true
    })
    .unwrap_or_default();
    for (at, r) in &collided {
        assert!(
            r.is_ok(),
            "{at}: two parameters naming one slot was refused: {r:?}"
        );
    }
    println!(
        "{} materials past their run refused, {} collided and saved",
        past.len(),
        collided.len()
    );
}

/// ⚠ `preset`, where Legacy states `material_hash`, holds one constant in
/// every shipped Enhanced material, a FourCC rather than a hash; the key is
/// `name`, the `.sps` preset name's hash, which is why the Legacy shader
/// database answers for an Enhanced material.
#[test]
fn the_preset_field_is_a_constant_tag_and_the_name_field_is_the_key() {
    let Some(assets) = enhanced_files("the Enhanced preset field") else {
        return;
    };
    let (mut presets, mut names) = (BTreeMap::new(), BTreeMap::new());
    for (_, asset) in &assets {
        for s in (enhanced_drawables(asset).into_iter())
            .filter_map(|d| d.shader_group.as_deref())
            .flat_map(|g| &g.shaders)
        {
            *presets.entry(s.preset.get()).or_insert(0usize) += 1;
            *names.entry(s.name.get()).or_insert(0usize) += 1;
        }
    }
    println!("{} distinct names, presets {presets:x?}", names.len());
    assert!(names.len() > 1, "every material names one shader");
    assert_eq!(presets.len(), 1, "{presets:x?}");
}

/// ⚠ A save derives each stride, Legacy from the FVF's size and Enhanced from the declaration's, and the geometry's from its buffer's.
#[test]
fn every_shipped_stride_is_what_a_save_derives() {
    let (mut n, mut wrong) = (0usize, Vec::new());
    for target in [Generation::Gen8, Generation::Gen9] {
        let Some(paths) = files(EXT, Some(target), "the shipped strides") else {
            continue;
        };
        for path in &paths {
            let opened = Opened::read(path);
            let image = opened.image();
            let mut each = |derived: Option<usize>, buffer: usize, geometry: u8| {
                n += 1;
                if derived != Some(buffer) || usize::from(geometry) != buffer {
                    wrong.push(format!(
                        "{}: derives {derived:?}, the buffer states {buffer}, the geometry {geometry}",
                        name(path)
                    ));
                }
            };
            if target == Generation::Gen8 {
                let root = View::<Raw<GtaDrawable<Gen8>>>::at_offset(image, 0).unwrap();
                for lod in lods(image, &root.base()).unwrap() {
                    let models = lod.models().unwrap();
                    for model in (0..models.len()).filter_map(|i| models.follow(i).unwrap()) {
                        let run = model.geometries().unwrap();
                        for g in (0..run.len()).filter_map(|i| run.follow(i).unwrap()) {
                            let vb = g.vertex_buffer().unwrap().unwrap();
                            let fvf = vb.fvf().unwrap().map(|f| usize::from(f.fvf_size));
                            each(fvf, usize::from(vb.stride), g.vertex_stride);
                        }
                    }
                }
            } else {
                let root = View::<Raw<GtaDrawable<Gen9>>>::at_offset(image, 0).unwrap();
                for lod in lods(image, &root.base()).unwrap() {
                    let models = lod.models().unwrap();
                    for model in (0..models.len()).filter_map(|i| models.follow(i).unwrap()) {
                        let run = model.geometries().unwrap();
                        for g in (0..run.len()).filter_map(|i| run.follow(i).unwrap()) {
                            let vb = g.vertex_buffer().unwrap().unwrap();
                            let info = vb.info().unwrap().map(|d| ((d.data >> 2) & 0xFF) as usize);
                            each(info, usize::from(vb.stride), g.vertex_stride);
                        }
                    }
                }
            }
        }
    }
    println!(
        "{n} shipped strides, {} not what a save derives",
        wrong.len()
    );
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// Three vertices of Float3 position and one triangle over them: every count agrees.
fn sound() -> GtaDrawable<Gen8> {
    use or_io::codec::mesh::{GrcDataSize, GrcFvfChannel, VertexLayout};
    use or_io::decl::{Hash32, Shared};
    use or_io::gta5::{GrmModel, GrmShaderGroup, RmcLod, SpdAABB};
    let (channel, size) = (GrcFvfChannel::Position, GrcDataSize::Float3);
    let mut fvf = gen8::GrcFvf {
        fvf: 1 << channel as u32,
        channel_sizes: (size as u64) << (channel as u64 * 4),
        channel_count: 1,
        ..Default::default()
    };
    fvf.fvf_size = VertexLayout::of(fvf.fvf, fvf.channel_sizes).packed_stride() as u8;
    let geometry = GrmGeometry::<Gen8> {
        primitive_type: 3,
        vertex_data: Shared::Same,
        vertex_buffer: Box::new(gen8::GrcVertexBuffer {
            data: vec![0; 36],
            lock_ptr: Shared::Same,
            fvf: Some(Box::new(fvf)),
            ..Default::default()
        }),
        index_buffer: Box::new(gen8::GrcIndexBuffer {
            indices: vec![0, 1, 2],
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut base = RmcDrawable::<Gen8> {
        shader_group: Some(Box::new(GrmShaderGroup {
            shaders: vec![Box::new(gen8::GrcInstanceData {
                basis_hash: Hash32::of("default"),
                ..Default::default()
            })],
            ..Default::default()
        })),
        ..Default::default()
    };
    base.lod_group.lods[0] = RmcLod {
        models: vec![Box::new(GrmModel {
            aabbs: vec![SpdAABB::default()],
            shader_indices: vec![0],
            geometries: vec![Box::new(geometry)],
            ..Default::default()
        })],
        ..Default::default()
    }
    .into();
    GtaDrawable {
        base,
        ..Default::default()
    }
}

/// The first geometry of the first level, to perturb.
fn geometry0(d: &mut GtaDrawable<Gen8>) -> &mut GrmGeometry<Gen8> {
    &mut model0(d).geometries[0]
}

fn model0(d: &mut GtaDrawable<Gen8>) -> &mut or_io::gta5::GrmModel<Gen8> {
    &mut d.lod_group.lods[0].models[0]
}

fn tree_of(d: &GtaDrawable<Gen8>) -> or_io::desc::Value {
    let pack = gates::format(EXT).description(Generation::Gen8).unwrap();
    or_io::decl::Owned::to_element(d, &or_io::decl::Cx::new(pack.plans().clone())).unwrap()
}

/// The first record of type `ty` in `tree`, depth first.
fn first_record<'a>(
    tree: &'a mut or_io::desc::Value,
    ty: &str,
) -> Option<&'a mut or_io::desc::Record> {
    use or_io::desc::Value;
    match tree {
        Value::Struct(r) => {
            if r.type_name() == ty {
                return Some(r);
            }
            r.children_mut().find_map(|(_, c)| first_record(c, ty))
        }
        Value::Ptr {
            target: Some(t), ..
        } => first_record(t, ty),
        Value::Array { elements, .. } => elements.iter_mut().find_map(|e| first_record(e, ty)),
        _ => None,
    }
}

/// One edit that plants a defect: on the owned value where it can state it, else on the first geometry's stated bytes.
enum Plant {
    Owned(fn(&mut GtaDrawable<Gen8>)),
    Stated(&'static [(usize, &'static [u8])]),
}

/// ⚠ Without this the refusals below would pass against a save that refuses everything.
#[test]
fn a_sound_constructed_drawable_saves() {
    use or_io::decl::File;
    if let Err(e) = sound().to_bytes() {
        panic!("a coherent drawable must save, and this said: {e}");
    }
}

/// An importer's likeliest mistake: one index naming a vertex the run does not have.
#[test]
fn a_save_refuses_an_index_past_the_vertex_run() {
    use or_io::decl::File;
    let mut broken = sound();
    geometry0(&mut broken).index_buffer.indices = vec![0, 1, 99];
    let why = broken
        .to_bytes()
        .expect_err("a save must refuse a mesh whose indices leave the run")
        .to_string();
    assert!(why.contains("an index past the vertex run"), "{why}");
    assert!(why.contains("level0/model0/geometry0"), "{why}");
}

/// ⚠ The counts an owned value derives (the stride, the index and primitive
/// counts, the palette's length) cannot be stated wrong on it, so those are
/// planted in the tree it saves through, which the same validation reads.
#[test]
fn every_mesh_defect_is_refused_at_save() {
    use or_io::decl::Shared;
    use or_io::gta5::drawable::MeshDefect;
    use or_io::gta5::skeleton::{CrBoneData, CrSkeletonData};
    use or_io::gta5::Mat34V;
    type G = Raw<GrmGeometry<Gen8>>;
    let cases: [(MeshDefect, Plant); 10] = [
        (
            MeshDefect::VertexCountPastBuffer,
            Plant::Owned(|d| geometry0(d).vertex_data = Shared::Own(vec![0; 48])),
        ),
        (
            MeshDefect::VertexStrideDisagrees,
            Plant::Stated(&[
                (offset_of!(G, vertex_stride), &[18]),
                (offset_of!(G, vertices_count), &[2, 0]),
            ]),
        ),
        (
            MeshDefect::IndexCountPastBuffer,
            Plant::Stated(&[
                (offset_of!(G, indices_count), &[6, 0, 0, 0]),
                (offset_of!(G, primitive_count), &[2, 0, 0, 0]),
            ]),
        ),
        (
            MeshDefect::PrimitiveCountDisagrees,
            Plant::Stated(&[(offset_of!(G, primitive_count), &[2, 0, 0, 0])]),
        ),
        (
            MeshDefect::IndexPastVertexRun,
            Plant::Owned(|d| geometry0(d).index_buffer.indices = vec![0, 1, 99]),
        ),
        (
            MeshDefect::PaletteCountDisagrees,
            Plant::Stated(&[(offset_of!(G, matrix_count), &[4, 0])]),
        ),
        (
            MeshDefect::PaletteNamesNoBone,
            Plant::Owned(|d| {
                d.skeleton = Some(Box::new(CrSkeletonData {
                    bones: vec![CrBoneData {
                        parent_index: -1,
                        next_index: -1,
                        ..Default::default()
                    }],
                    inverse_transforms: vec![Mat34V::default()],
                    default_transforms: vec![Mat34V::default()],
                    parent_indices: vec![-1],
                    ..Default::default()
                }));
                geometry0(d).matrix_palette = vec![9];
            }),
        ),
        (
            MeshDefect::AabbSlotsDisagree,
            Plant::Owned(|d| {
                model0(d).aabbs.clear();
                model0(d).shader_indices.clear();
            }),
        ),
        (
            MeshDefect::ShaderIndexCountDisagrees,
            Plant::Owned(|d| {
                model0(d).aabbs.clear();
                model0(d).shader_indices.clear();
            }),
        ),
        (
            MeshDefect::ShaderIndexPastGroup,
            Plant::Owned(|d| model0(d).shader_indices = vec![5]),
        ),
    ];
    assert!(
        (MeshDefect::ALL.iter()).all(|d| cases.iter().any(|(c, _)| c == d)),
        "every defect has a case"
    );
    let format = gates::format(EXT);
    for (defect, plant) in cases {
        let mut d = sound();
        let tree = match plant {
            Plant::Owned(edit) => {
                edit(&mut d);
                tree_of(&d)
            }
            Plant::Stated(words) => {
                let mut tree = tree_of(&d);
                let g = first_record(&mut tree, &G::name()).expect("a geometry");
                for (at, bytes) in words {
                    g.bytes_mut()[*at..*at + bytes.len()].copy_from_slice(bytes);
                }
                tree
            }
        };
        let why = match format.save(Generation::Gen8, &tree) {
            Ok(_) => panic!("{defect:?} saved"),
            Err(e) => e.to_string(),
        };
        assert!(why.contains(defect.label()), "{defect:?}: {why}");
    }
}

/// A saved file's decompressed bytes, and where its graphics half starts.
fn decompressed(file: &[u8]) -> (Vec<u8>, usize) {
    use or_io::container::resource::{ResourceEnvelope, Rsc7};
    let opened = Rsc7::open(file).expect("an RSC7 resource");
    (opened.data, opened.virtual_size as usize)
}

/// The first geometry of each level of the drawable at `at`, by its offset.
fn level_geometries(bytes: &[u8], virtual_size: usize, at: usize) -> Vec<usize> {
    let aligned = Aligned::from_bytes(bytes);
    let image = Image::new(aligned.bytes(), virtual_size);
    let d = View::<Raw<RmcDrawable<Gen8>>>::at_offset(image, at).unwrap();
    let mut out = Vec::new();
    for level in lods(image, &d).unwrap() {
        let model = level.models().unwrap().follow(0).unwrap().unwrap();
        let g = model.geometries().unwrap().follow(0).unwrap().unwrap();
        out.push(View::offset(&g));
    }
    out
}

/// Points the geometry at `to` at the vertex buffer, and with `both` the index buffer, of the geometry at `from`.
fn alias(bytes: &mut [u8], from: usize, to: usize, both: bool) {
    let mut fields = vec![offset_of!(Raw<GrmGeometry<Gen8>>, vertex_buffer)];
    if both {
        fields.push(offset_of!(Raw<GrmGeometry<Gen8>>, index_buffer));
    }
    for field in fields {
        let word: [u8; 8] = bytes[from + field..from + field + 8].try_into().unwrap();
        bytes[to + field..to + field + 8].copy_from_slice(&word);
    }
}

/// A drawable whose second level is a separate copy of its first.
fn two_levels() -> GtaDrawable<Gen8> {
    let mut d = sound();
    d.lod_group.lods[1] = d.lod_group.lods[0].clone();
    d
}

/// ⚠ The game rebases a buffer once per geometry pointing at it, so the
/// second fails the load; an owned save of such a file writes the aliasing
/// back, and only the refusal stands between it and the game.
#[test]
fn a_buffer_two_levels_point_at_is_refused_and_a_level_of_aliases_is_removed() {
    use or_io::decl::{File, Owned};
    use or_io::gta5::drawable::{self, aliased_lods, LodSlot};
    let (mut bytes, virtual_size) = decompressed(&two_levels().to_bytes().unwrap());
    let at = level_geometries(&bytes, virtual_size, 0);
    assert_eq!(at.len(), 2, "two levels, each its own geometry");
    {
        let aligned = Aligned::from_bytes(&bytes);
        let image = Image::new(aligned.bytes(), virtual_size);
        let buffers = drawable::buffers(image, 0, Generation::Gen8).unwrap();
        assert!(buffers.aliased().is_empty());
    }

    alias(&mut bytes, at[0], at[1], true);
    let aligned = Aligned::from_bytes(&bytes);
    let image = Image::new(aligned.bytes(), virtual_size);
    let aliased = (drawable::buffers(image, 0, Generation::Gen8).unwrap()).aliased();
    assert_eq!(
        aliased.len(),
        2,
        "the vertex and the index buffer: {aliased:?}"
    );
    for block in &aliased {
        let lods: Vec<LodSlot> = block.sites.iter().map(|s| s.lod).collect();
        assert_eq!(lods, [LodSlot::Level(0), LodSlot::Level(1)]);
    }

    let tree = gates::format(EXT).read(Generation::Gen8, image).unwrap();
    let mut opened = GtaDrawable::<Gen8>::from_element(&tree).unwrap();
    let why = opened
        .to_bytes()
        .expect_err("an aliased buffer")
        .to_string();
    assert!(
        why.contains("fixup walk") && why.contains("level1"),
        "{why}"
    );

    let root = View::<Raw<GtaDrawable<Gen8>>>::at_offset(image, 0).unwrap();
    let removed = aliased_lods(&root.base()).unwrap();
    assert_eq!(removed, [LodSlot::Level(1)]);
    opened.remove_lods(&removed);
    let (again, virtual_size) = decompressed(&opened.to_bytes().expect("repaired, it saves"));
    assert_eq!(
        level_geometries(&again, virtual_size, 0).len(),
        1,
        "the level that owns them stays"
    );
}

/// A level holding one buffer of its own keeps it, and its alias with it.
#[test]
fn a_level_with_a_buffer_of_its_own_is_not_removed() {
    use or_io::decl::File;
    use or_io::gta5::drawable::{self, aliased_lods};
    let (mut bytes, virtual_size) = decompressed(&two_levels().to_bytes().unwrap());
    let at = level_geometries(&bytes, virtual_size, 0);
    alias(&mut bytes, at[0], at[1], false);
    let aligned = Aligned::from_bytes(&bytes);
    let image = Image::new(aligned.bytes(), virtual_size);
    let aliased = (drawable::buffers(image, 0, Generation::Gen8).unwrap()).aliased();
    assert_eq!(aliased.len(), 1, "{aliased:?}");
    let root = View::<Raw<GtaDrawable<Gen8>>>::at_offset(image, 0).unwrap();
    assert!(aliased_lods(&root.base()).unwrap().is_empty());
}

/// ⚠ Two entries of a dictionary sharing a buffer is refused, and removing a
/// level of either would lose a drawable's only geometry, so none is removed.
#[test]
fn a_buffer_two_entries_point_at_is_refused_and_not_repaired() {
    use or_io::decl::File;
    use or_io::gta5::drawable::aliased_lods;
    use or_io::gta5::drawable_dict;
    let dict = PgDictionary::<GtaDrawable<Gen8>> {
        codes: vec![1, 2],
        entries: vec![Some(Box::new(sound())), Some(Box::new(sound()))],
        ..Default::default()
    };
    let (mut bytes, virtual_size) = decompressed(&dict.to_bytes().unwrap());
    let entries: Vec<usize> = {
        let aligned = Aligned::from_bytes(&bytes);
        let image = Image::new(aligned.bytes(), virtual_size);
        let root = View::<Raw<PgDictionary<GtaDrawable<Gen8>>>>::at_offset(image, 0).unwrap();
        let run = root.entries().unwrap();
        (0..run.len())
            .map(|i| View::offset(&run.follow(i).unwrap().unwrap()))
            .collect()
    };
    let a = level_geometries(&bytes, virtual_size, entries[0])[0];
    let b = level_geometries(&bytes, virtual_size, entries[1])[0];
    alias(&mut bytes, a, b, true);
    let aligned = Aligned::from_bytes(&bytes);
    let image = Image::new(aligned.bytes(), virtual_size);
    let aliased = (drawable_dict::buffers(image, 0, Generation::Gen8).unwrap()).aliased();
    assert_eq!(aliased.len(), 2, "{aliased:?}");
    let owners: Vec<usize> = aliased[0].sites.iter().map(|s| s.drawable).collect();
    assert_eq!(owners, [0, 1]);
    let why = (drawable_dict::validate(image, 0, Generation::Gen8).unwrap_err()).to_string();
    assert!(why.contains("fixup walk"), "{why}");
    for at in entries {
        let d = View::<Raw<GtaDrawable<Gen8>>>::at_offset(image, at).unwrap();
        assert!(aliased_lods(&d.base()).unwrap().is_empty());
    }
}
