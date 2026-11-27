use super::*;
use or_io::gta5::shaders::sgd5::{Sgd5Pass, Sgd5Table};

fn dwords(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// A DXBC container of `parts`, each padded to whole dwords.
fn container(parts: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let table_end = 32 + 4 * parts.len();
    let (mut offsets, mut body) = (Vec::new(), Vec::new());
    for (tag, data) in parts {
        offsets.push((table_end + body.len()) as u32);
        body.extend_from_slice(*tag);
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(data);
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
    }
    let mut out = b"DXBC".to_vec();
    out.extend_from_slice(&[0x3C; 16]);
    out.extend(dwords(&[
        1,
        (table_end + body.len()) as u32,
        parts.len() as u32,
    ]));
    out.extend(dwords(&offsets));
    out.extend(body);
    out
}

/// A program of `kind` (0 pixel, 1 vertex) whose DXIL part holds an `XDAT` payload and whose one
/// input is `semantic`.
fn xdat(kind: u32, semantic: &[u8]) -> Vec<u8> {
    let mut dxil = dwords(&[(kind << 16) | 0x66, 8, 0x4C49_5844, 0x0106, 16, 8]);
    dxil.extend_from_slice(b"XDAT\x01\x02\x03\x04");
    let psv = dwords(&[28, 0, 0, 0, 0, 0, 0, 0, 0]);
    let mut isg = dwords(&[1, 8, 0, 40, 0, 0, 3, kind, 0x0F, 0]);
    isg.extend_from_slice(semantic);
    isg.push(0);
    container(&[(b"DXIL", dxil), (b"PSV0", psv), (b"ISG1", isg)])
}

/// `bytes` as one stored raw DEFLATE block.
fn stored(bytes: &[u8]) -> Vec<u8> {
    let len = bytes.len() as u16;
    let mut out = vec![0x01];
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(bytes);
    out
}

const BASE: u64 = 0x40;
const EFFECT: &[u8] = b"standard_t";

/// An archive holding one container at [`BASE`] with a vertex and a pixel program, and the
/// container as read: one effect whose only pass runs both.
fn archive(dir: &Path) -> (PathBuf, ShaderSource, [Sgd5Program; 2]) {
    let vertex = stored(&xdat(1, b"BLENDINDICES"));
    let pixel = stored(&xdat(0, b"TEXCOORD"));
    let (vertex_at, pixel_at) = (0x10, 0x10 + vertex.len() + 0x08);
    let mut bytes = vec![0xA5; BASE as usize + pixel_at + pixel.len() + 0x10];
    let place = |at: usize, stream: &[u8], bytes: &mut Vec<u8>| {
        let from = BASE as usize + at;
        bytes[from..from + stream.len()].copy_from_slice(stream);
    };
    place(vertex_at, &vertex, &mut bytes);
    place(pixel_at, &pixel, &mut bytes);
    let path = dir.join("shaders.rpf");
    std::fs::write(&path, &bytes).unwrap();

    let program = |bank: usize, at: usize, stream: &[u8], inflated: usize| Sgd5Program {
        bank: Some(bank),
        index: Some(0),
        name: format!("bank{bank}"),
        kind: 0,
        key: 0,
        at: at - 4,
        stream: at..at + stream.len(),
        inflated_len: inflated,
        variables: Sgd5Table::default(),
    };
    let programs = [
        program(0, vertex_at, &vertex, xdat(1, b"BLENDINDICES").len()),
        program(1, pixel_at, &pixel, xdat(0, b"TEXCOORD").len()),
    ];
    let effect = Sgd5Effect {
        name: String::from_utf8_lossy(EFFECT).into_owned(),
        at: 0,
        word: 0,
        programs: [vec![0], vec![0], vec![], vec![], vec![], vec![]],
        state_counts: [0; 4],
        techniques: Vec::new(),
        passes: vec![Sgd5Pass {
            name: "p0".into(),
            programs: [1, 1, 0, 0, 0, 0],
        }],
        bound: Vec::new(),
        parameters: Sgd5Table::default(),
        second: Sgd5Table::default(),
        layout_flag: 0,
        layout_refs: Vec::new(),
        layouts: Vec::new(),
    };
    let file = Sgd5File {
        head_word: 0,
        layouts: Vec::new(),
        globals: Sgd5Table::default(),
        bank_counts: [Some(1), Some(1), Some(0), Some(0), Some(0), Some(0)],
        programs: programs.to_vec(),
        gaps: Vec::new(),
        effect_count: Some(1),
        effects: vec![effect],
    };
    let source = ShaderSource {
        archive: path.clone(),
        base: BASE,
        file,
        whole: true,
    };
    (path, source, programs)
}

fn input(source: ShaderSource, wanted: Vec<u32>) -> Input {
    Input {
        sources: vec![Arc::new(source)],
        keys: HashMap::new(),
        known: HashSet::new(),
        wanted: wanted.into_iter().map(|hash| (hash, 0)).collect(),
        shading: HashMap::new(),
        stages: Stages::default(),
        install: None,
    }
}

#[test]
fn an_effect_s_programs_are_read_by_seek_translated_once_and_kept_as_a_pack_of_refusals() {
    let dir = tempfile::tempdir().unwrap();
    let (path, source, programs) = archive(dir.path());
    let hash = name_hash32(EFFECT);
    let absent = hash ^ 0x5555_AAAA;
    let found = work(
        input(source, vec![hash, absent]),
        &Cancel::new(),
        &Progress::new(),
    )
    .unwrap();

    // The keys the front end gives the two streams read alone.
    let bytes = std::fs::read(&path).unwrap();
    let key_of = |p: &Sgd5Program| {
        let from = BASE as usize + p.stream.start;
        translate_gta6_program(p, &bytes[from..from + p.stream.len()], None)
            .unwrap()
            .key
    };
    let (vertex, pixel) = (key_of(&programs[0]), key_of(&programs[1]));
    assert_ne!(vertex, pixel);
    let read: Vec<(&str, usize)> = (found.done.iter())
        .map(|(key, done)| (key.as_str(), done.program.source_bytes as usize))
        .collect();
    assert_eq!(
        read,
        vec![
            (vertex.as_str(), xdat(1, b"BLENDINDICES").len()),
            (pixel.as_str(), xdat(0, b"TEXCOORD").len())
        ]
    );
    assert_eq!(found.done[0].1.stage, ShaderStageKind::Vertex);
    assert_eq!(found.done[1].1.stage, ShaderStageKind::Pixel);
    assert!(found.keys.iter().any(|((p, b, at), (k, skinned))| {
        p == &path && *b == BASE && *at == programs[0].at && k == &vertex && *skinned
    }));

    // Its vertex program reads blend indices, so its one pass is the skinned technique's.
    assert_eq!(found.effects.len(), 1);
    let (named, effect) = &found.effects[0];
    assert_eq!(*named, hash);
    assert_eq!(effect.name.as_bytes(), EFFECT);
    let techniques: Vec<&str> = effect.techniques.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(techniques, vec!["drawskinned"]);
    let stages = &effect.techniques[0].passes[0].stages;
    assert_eq!(stages[&ShaderStageKind::Vertex], vertex);
    assert_eq!(stages[&ShaderStageKind::Pixel], pixel);

    let mut shaders = Shaders::default();
    assert_eq!(merge(&mut shaders, found), (0, 2));
    let (number, pack) = shaders.pack.clone().unwrap();
    assert_eq!(number, 1);
    assert_eq!(pack.root(), None);
    let Err(or_renderer::game_shaders::pack::PackError::NotTranslated { reason, .. }) =
        pack.module(&vertex)
    else {
        panic!("a program with no bitcode gave a module");
    };
    assert!(reason.contains("XDAT"), "{reason}");
    assert_eq!(pack.manifest().effects.len(), 1);

    let Status::Done { refused, first } = status_of(&shaders, &[hash, absent]) else {
        panic!("the effect read is not done");
    };
    assert_eq!(refused, 2);
    assert!(first.is_some_and(|why| why.contains("XDAT")));
    assert_eq!(status_of(&shaders, &[absent]), Status::NoSource);

    // A program read before is not read again: its key comes from what is kept.
    let (_, source, _) = archive(dir.path());
    let mut again = input(source, vec![hash]);
    again.keys = shaders.keys.clone();
    again.known = shaders.done.keys().cloned().collect();
    let found = work(again, &Cancel::new(), &Progress::new()).unwrap();
    assert!(found.done.is_empty() && found.keys.is_empty());
    assert_eq!(found.effects[0].1.techniques[0].passes[0].stages.len(), 2);
    assert_eq!(merge(&mut shaders, found), (0, 0));
    assert_eq!(shaders.pack.as_ref().map(|(n, _)| *n), Some(2));
}

#[test]
fn an_effect_read_from_its_container_is_kept_and_the_next_session_reads_no_container_for_it() {
    let dir = tempfile::tempdir().unwrap();
    let (path, source, _) = archive(dir.path());
    let hash = name_hash32(EFFECT);
    let first = work(input(source, vec![hash]), &Cancel::new(), &Progress::new()).unwrap();
    let kept = kept_effect(&path, hash).expect("a place to keep it");
    assert!(kept.is_file(), "the effect is not kept");

    // A session that knows no container, and whose archive holds none a scan would find.
    let later = Input {
        sources: Vec::new(),
        keys: HashMap::new(),
        known: HashSet::new(),
        wanted: vec![(hash, 0)],
        shading: HashMap::new(),
        stages: Stages::default(),
        install: Some(path.clone()),
    };
    let again = work(later, &Cancel::new(), &Progress::new()).unwrap();
    assert_eq!(again.searched, None, "the archive was searched");
    assert!(again.sources.is_empty());
    let keys =
        |found: &Found| -> Vec<String> { found.done.iter().map(|(key, _)| key.clone()).collect() };
    assert_eq!(keys(&again), keys(&first));
    assert_eq!(again.effects.len(), 1);
    assert_eq!(again.effects[0].1.techniques, first.effects[0].1.techniques);

    // The archive written again is another archive: what was kept of the old one is not its.
    std::fs::write(&path, [std::fs::read(&path).unwrap(), vec![0]].concat()).unwrap();
    assert_ne!(kept_effect(&path, hash), Some(kept));
}

#[test]
fn a_job_asked_to_stop_stops_before_the_next_program() {
    let dir = tempfile::tempdir().unwrap();
    let (_, source, _) = archive(dir.path());
    let cancel = Cancel::new();
    cancel.set();
    let ended = work(
        input(source, vec![name_hash32(EFFECT)]),
        &cancel,
        &Progress::new(),
    );
    assert_eq!(ended.err(), Some(stopped()));
}

#[test]
fn a_container_read_again_replaces_the_one_at_its_place_and_lets_unfound_effects_be_looked_for() {
    let dir = tempfile::tempdir().unwrap();
    let mut shaders = Shaders::default();
    shaders.effects.insert(7, Effect::default());
    shaders.tried.extend([(7, 0), (8, 0)]);
    let (_, first, _) = archive(dir.path());
    let (_, second, _) = archive(dir.path());
    keep(&mut shaders, vec![Arc::new(first)]);
    keep(&mut shaders, vec![Arc::new(second)]);
    assert_eq!(shaders.sources.len(), 1);
    assert_eq!(shaders.tried, HashSet::from([(7, 0)]));
}

/// The passes of a made-up effect: each the bytes of its mesh program's container and of its pixel
/// program's.
type Listed<'a> = &'a [(&'a [u8], &'a [u8])];

/// An archive holding one container at [`BASE`], and the container as read: each effect of
/// `effects` by its name, with one technique for each of its passes, a pass the bytes of its mesh
/// program's container and of its pixel program's.
fn shaded(dir: &Path, effects: &[(&str, Listed)]) -> ShaderSource {
    use or_io::gta5::shaders::sgd5::Sgd5Technique;
    let mut bytes = vec![0xA5; BASE as usize + 0x10];
    let mut programs: Vec<Sgd5Program> = Vec::new();
    let mut read = Vec::new();
    for (name, passes) in effects {
        let mut listed = [vec![], vec![], vec![], vec![], vec![], vec![]];
        for (bank, container) in passes
            .iter()
            .flat_map(|(mesh, pixel)| [(0, mesh), (1, pixel)])
        {
            let (stream, at) = (stored(container), bytes.len() - BASE as usize);
            bytes.extend_from_slice(&stream);
            bytes.extend_from_slice(&[0xA5; 8]);
            let index = programs.iter().filter(|p| p.bank == Some(bank)).count() as u32;
            listed[bank].push(index);
            programs.push(Sgd5Program {
                bank: Some(bank),
                index: Some(index),
                name: format!("bank{bank}"),
                kind: 0,
                key: 0,
                at: at - 4,
                stream: at..at + stream.len(),
                inflated_len: container.len(),
                variables: Sgd5Table::default(),
            });
        }
        let technique = |i: usize| Sgd5Technique {
            name: format!("t{i}"),
            flag: false,
            index: 0,
            passes: 1,
        };
        let pass = |i: usize| Sgd5Pass {
            name: format!("p{i}"),
            programs: [i as u8 + 1, i as u8 + 1, 0, 0, 0, 0],
        };
        read.push(Sgd5Effect {
            name: (*name).to_owned(),
            at: 0,
            word: 0,
            programs: listed,
            state_counts: [0; 4],
            techniques: (0..passes.len()).map(technique).collect(),
            passes: (0..passes.len()).map(pass).collect(),
            bound: Vec::new(),
            parameters: Sgd5Table::default(),
            second: Sgd5Table::default(),
            layout_flag: 0,
            layout_refs: Vec::new(),
            layouts: Vec::new(),
        });
    }
    let path = dir.join("shaders.rpf");
    std::fs::write(&path, &bytes).unwrap();
    let count = |bank: usize| programs.iter().filter(|p| p.bank == Some(bank)).count() as _;
    let bank_counts = [
        Some(count(0)),
        Some(count(1)),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
    ];
    ShaderSource {
        archive: path,
        base: BASE,
        file: Sgd5File {
            head_word: 0,
            layouts: Vec::new(),
            globals: Sgd5Table::default(),
            bank_counts,
            programs,
            gaps: Vec::new(),
            effect_count: Some(effects.len() as _),
            effects: read,
        },
        whole: true,
    }
}

/// A stand-in for the translator. A mesh program's container is `M`, the streams it reads alone
/// and, after a nought, those it reads besides as its pass's stage; a pixel program's is `P`, or
/// `X` for one that is refused. A stage's module holds one type, named by the container.
#[derive(Default)]
struct Told {
    read: AtomicUsize,
    pixels: AtomicUsize,
    passes: AtomicUsize,
    /// How many mesh programs are read at this moment, and the most that were at once.
    now: AtomicUsize,
    most: AtomicUsize,
    slow: bool,
}

use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

/// The blend the stand-in gives a forward pass, a pixel program's container `F`.
const LAID: or_cli::Gta6Blend = or_cli::Gta6Blend {
    colour: [
        or_cli::Gta6Factor::One,
        or_cli::Gta6Factor::OneMinusSourceAlpha,
    ],
    alpha: [or_cli::Gta6Factor::Zero, or_cli::Gta6Factor::One],
    writes: 0b0111,
};

impl Translate for Told {
    /// Whether the pixel stage is a forward one.
    type Pixel = bool;

    fn shades(&self, mesh: &[u8], pixel: &[u8]) -> bool {
        mesh[0] == b'M' && matches!(pixel[0], b'P' | b'X')
    }

    fn forward(&self, mesh: &[u8], pixel: &[u8]) -> bool {
        mesh[0] == b'M' && pixel[0] == b'F'
    }

    fn streams(&self, mesh: &[u8], _: &Sgd5Table, _: &Sgd5Table) -> Result<BTreeSet<u32>, String> {
        self.read.fetch_add(1, SeqCst);
        if self.slow {
            self.most
                .fetch_max(self.now.fetch_add(1, SeqCst) + 1, SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(60));
            self.now.fetch_sub(1, SeqCst);
        }
        let alone = mesh[1..].iter().take_while(|slot| **slot != 0);
        Ok(alone.map(|slot| u32::from(*slot)).collect())
    }

    fn pixel(&self, pixel: &[u8], _: &Sgd5Table, _: &Sgd5Table) -> Result<bool, String> {
        self.pixels.fetch_add(1, SeqCst);
        match pixel[0] {
            b'X' => Err("pixel program: refused".to_owned()),
            other => Ok(other == b'F'),
        }
    }

    fn mesh(
        &self,
        mesh: &[u8],
        _: &Sgd5Table,
        _: &Sgd5Table,
        forward: &bool,
    ) -> Result<Gta6ShadingPass, String> {
        self.passes.fetch_add(1, SeqCst);
        use wgpu::naga;
        let stage = |streams: BTreeSet<u32>| or_cli::Gta6Stage {
            approximations: Default::default(),
            module: {
                let mut module = naga::Module::default();
                let named = naga::Type {
                    name: Some(format!("{streams:?}")),
                    inner: naga::TypeInner::Scalar(naga::Scalar::F32),
                };
                module.types.insert(named, naga::Span::UNDEFINED);
                Arc::new(module)
            },
            feeds: Vec::new(),
            textures: Vec::new(),
            streams,
            rows: Vec::new(),
        };
        let streams = mesh[1..].iter().filter(|slot| **slot != 0);
        Ok(Gta6ShadingPass {
            mesh: stage(streams.map(|slot| u32::from(*slot)).collect()),
            pixel: stage(BTreeSet::new()),
            linked: Default::default(),
            forward: forward.then_some(LAID),
        })
    }
}

/// The streams a geometry holds, a bit a slot.
fn holding(slots: &[u32]) -> u32 {
    slots.iter().fold(0, |held, slot| held | 1 << slot)
}

/// A request for `worn` as a frame makes it: the job [`request`] starts for it, run to its end by
/// `translate` and taken into `s` as [`poll`] takes it. Whether it started one.
fn looked_for(s: &mut Shaders, translate: &Told, worn: &[Worn]) -> bool {
    let Some((input, wanted)) = asked(s, worn, || None) else {
        return false;
    };
    let found = work_with(translate, input, &Cancel::new(), &Progress::new());
    took(s, &wanted, found).unwrap();
    true
}

/// The vertex data of a geometry that holds a stream in each of `slots`: the table of streams at
/// its head.
fn vertex_data(slots: &[usize]) -> Vec<u8> {
    let mut vertex_data = vec![0u8; 4 * 23];
    for slot in slots {
        vertex_data[4 * slot] = 1;
    }
    vertex_data
}

/// The streams the mesh stage of the pass [`shading`] hands a geometry of `vertex_data` reads.
fn shaded_by(s: &Shaders, hash: u32, vertex_data: &[u8]) -> Option<Result<Vec<u32>, String>> {
    let pass = shading_in(s, hash, vertex_data)?;
    Some(pass.map(|pass| pass.mesh.streams.iter().copied().collect()))
}

#[test]
fn an_effect_asked_for_again_on_a_geometry_of_other_streams_starts_a_job_for_that_geometry_s_pass()
{
    let dir = tempfile::tempdir().unwrap();
    let passes: Listed = &[(b"M\x01\x02\x03", b"P"), (b"M\x01\x02\x03\x06\x08", b"P")];
    let mut s = Shaders::default();
    keep(
        &mut s,
        vec![Arc::new(shaded(dir.path(), &[("layered_t", passes)]))],
    );
    let hash = name_hash32(b"layered_t");
    let (plain, skinned) = (vertex_data(&[1, 2, 3]), vertex_data(&[1, 2, 3, 6, 8]));
    let worn = |vertex_data: &[u8]| (hash, gta6_held_streams(vertex_data));
    let told = Told::default();

    // A model of one geometry: a job, and its pass.
    assert!(looked_for(&mut s, &told, &[worn(&plain)]));
    assert_eq!(shaded_by(&s, hash, &plain), Some(Ok(vec![1, 2, 3])));
    assert_eq!(shaded_by(&s, hash, &skinned), None);
    // The next frame asks for the same and starts nothing.
    assert!(!looked_for(&mut s, &told, &[worn(&plain)]));
    // A model whose geometry wears the effect with other streams: a job, though one looked for
    // the effect, and each geometry is handed its own pass.
    assert!(looked_for(&mut s, &told, &[worn(&skinned)]));
    assert_eq!(shaded_by(&s, hash, &skinned), Some(Ok(vec![1, 2, 3, 6, 8])));
    assert_eq!(shaded_by(&s, hash, &plain), Some(Ok(vec![1, 2, 3])));
    assert!(!looked_for(&mut s, &told, &[worn(&plain), worn(&skinned)]));
    assert_eq!(told.passes.load(SeqCst), 2);
}

#[test]
fn two_geometries_that_wear_one_effect_with_other_streams_are_each_drawn_by_the_pass_asked_for_theirs(
) {
    use crate::render::gta6_meshlets::{meshlets_of, worn_by, Sources};
    use or_io::gta6::headerless::{Drawable, Geometry, Model, Palette, Shader};
    let dir = tempfile::tempdir().unwrap();
    let passes: Listed = &[(b"M\x01\x02\x03", b"P"), (b"M\x01\x02\x03\x06\x08", b"P")];
    let mut s = Shaders::default();
    keep(
        &mut s,
        vec![Arc::new(shaded(dir.path(), &[("layered_t", passes)]))],
    );
    let hash = name_hash32(b"layered_t");
    // A geometry of one meshlet of one primitive: the head of its index data, then the record.
    let index_data: Vec<u8> = [0x8000_0001u32, 0, 0, 0, !0, !0, !0]
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect();
    let geometry = |slots: &[usize]| Geometry {
        shader: 0,
        vertex_count: 0,
        channels: Vec::new(),
        triangles: Vec::new(),
        positions: Ok(Vec::new()),
        vertices: Default::default(),
        vertex_data: vertex_data(slots),
        index_data: index_data.clone(),
    };
    let model = Model {
        geometries: vec![Ok(geometry(&[1, 2, 3])), Ok(geometry(&[1, 2, 3, 6, 8]))],
        palette: Ok(Palette::Own),
        matrix_index: 0,
    };
    let drawable = Drawable {
        name: String::new(),
        box_min: [0.0; 3],
        box_max: [0.0; 3],
        shaders: vec![Shader {
            name_hash: hash,
            texture_keys: Vec::new(),
            parameters: Vec::new(),
        }],
        levels: [Some(vec![model]), None, None, None],
        lod_distances: [0.0; 4],
        bound: None,
        embedded: None,
        extra_skeletons: Vec::new(),
    };
    let models = drawable.levels[0].as_deref().unwrap();

    // What a frame asks a job for: the effect once for each set of streams a geometry holds.
    let worn = worn_by(&drawable, models);
    let (plain, skinned) = (holding(&[1, 2, 3]), holding(&[1, 2, 3, 6, 8]));
    assert_eq!(worn, [(hash, plain), (hash, skinned)]);
    let told = Told::default();
    assert!(looked_for(&mut s, &told, &worn));
    assert_eq!(told.passes.load(SeqCst), 2);

    // And what the frame is then handed: each geometry's surface by the pass of its own streams.
    let lost = BTreeSet::new();
    let sources = Sources {
        shading: &|hash, vertex_data| shading_in(&s, hash, vertex_data),
        map: &|_| Ok(None),
        layered: &|| None,
        lost: &lost,
        ped: &|| None,
    };
    let (meshlets, effects) = meshlets_of(&drawable, models, &sources).expect("its shader");
    assert_eq!((meshlets.rest.as_slice(), &meshlets.why), (&[][..], &None));
    assert_eq!(effects, [hash, hash]);
    // The stand-in names the one type of a mesh stage by the streams its pass reads.
    let reads = |at: usize| {
        let (_, named) = meshlets.surfaces[at].vertex.types.iter().next()?;
        named.name.clone()
    };
    assert_eq!(reads(0).as_deref(), Some("{1, 2, 3}"));
    assert_eq!(reads(1).as_deref(), Some("{1, 2, 3, 6, 8}"));
}

/// The streams the mesh stage of the pass a geometry holding `held` is handed reads.
fn handed(s: &Shaders, hash: u32, held: u32) -> Option<Result<Vec<u32>, String>> {
    Some(picked(s, hash, held)?.map(|pass| pass.mesh.streams.iter().copied().collect()))
}

/// Which passes of the effect are translated.
fn translated(s: &Shaders, hash: u32) -> Vec<bool> {
    let passes = s.shading[&hash].as_ref().unwrap();
    let whole = |pass: &Candidate| matches!(pass.pass, Some(Ok(_)));
    passes.iter().map(whole).collect()
}

#[test]
fn only_the_pass_a_geometry_picks_is_translated_and_a_second_geometry_still_gets_its_own() {
    let dir = tempfile::tempdir().unwrap();
    // Two passes for a geometry with no weights and two for one that has them.
    let passes: Listed = &[
        (b"M\x01\x02\x03", b"P"),
        (b"M\x01\x02\x03", b"P"),
        (b"M\x01\x02\x03\x06\x08", b"P"),
        (b"M\x01\x02\x03\x06\x08", b"P"),
    ];
    let mut s = Shaders::default();
    let source = shaded(dir.path(), &[("layered_t", passes)]);
    keep(&mut s, vec![Arc::new(source)]);
    let hash = name_hash32(b"layered_t");
    let (plain, skinned) = (holding(&[1, 2, 3]), holding(&[1, 2, 3, 6, 8]));
    let told = Told::default();
    let counts = |told: &Told| {
        (
            told.read.load(SeqCst),
            told.pixels.load(SeqCst),
            told.passes.load(SeqCst),
        )
    };

    looked_for(&mut s, &told, &[(hash, plain), (hash, plain)]);
    // Each mesh program's streams are read, and as many pairs are translated as are drawn.
    assert_eq!(counts(&told), (4, 1, 1));
    assert_eq!(translated(&s, hash), [true, false, false, false]);
    assert_eq!(handed(&s, hash, plain), Some(Ok(vec![1, 2, 3])));
    // A geometry no job has looked for a pass of is handed none: never another geometry's.
    assert_eq!(handed(&s, hash, skinned), None);
    let done = Status::Done {
        refused: 0,
        first: None,
    };
    assert_eq!(status_of(&s, &[hash]), done);

    let first = picked(&s, hash, plain).unwrap().unwrap();
    looked_for(&mut s, &told, &[(hash, plain), (hash, skinned)]);
    assert_eq!(counts(&told), (4, 2, 2), "the streams are read once");
    assert_eq!(translated(&s, hash), [true, false, true, false]);
    assert_eq!(handed(&s, hash, skinned), Some(Ok(vec![1, 2, 3, 6, 8])));
    let again = picked(&s, hash, plain).unwrap().unwrap();
    assert!(
        Arc::ptr_eq(&first, &again),
        "the first geometry's pass stands"
    );
}

#[test]
fn a_refused_pass_gives_way_to_the_one_drawn_were_every_pass_translated() {
    let dir = tempfile::tempdir().unwrap();
    let passes: Listed = &[
        (b"M\x01\x02\x03", b"X"),
        (b"M\x01\x02", b"P"),
        (b"M\x01\x02\x03", b"P"),
    ];
    let refused: Listed = &[(b"M\x01", b"X"), (b"M\x01\x02", b"X")];
    let mut s = Shaders::default();
    let source = shaded(dir.path(), &[("first_t", passes), ("none_t", refused)]);
    keep(&mut s, vec![Arc::new(source)]);
    let (hash, none) = (name_hash32(b"first_t"), name_hash32(b"none_t"));
    let told = Told::default();
    looked_for(&mut s, &told, &[(hash, holding(&[1, 2, 3])), (none, 0)]);
    // The first pass is refused by its pixel program; of the two left, the third reads more.
    assert_eq!(translated(&s, hash), [false, false, true]);
    assert_eq!(
        handed(&s, hash, holding(&[1, 2, 3])),
        Some(Ok(vec![1, 2, 3]))
    );
    // An effect every pass of which is refused says why, once each was tried.
    let why = "pixel program: refused".to_owned();
    assert_eq!(handed(&s, none, 0), Some(Err(why)));
    // No mesh stage is linked to a pixel stage that was refused.
    assert_eq!(told.passes.load(SeqCst), 1);
}

#[test]
fn a_mesh_program_that_reads_other_streams_in_its_pass_has_every_pass_translated_to_pick_by() {
    let dir = tempfile::tempdir().unwrap();
    // The first reads two more streams as its pass's stage than alone.
    let passes: Listed = &[
        (b"M\x01\x02\x03\x00\x06\x08", b"P"),
        (b"M\x01\x02\x03", b"P"),
    ];
    let mut s = Shaders::default();
    let source = shaded(dir.path(), &[("moved_t", passes)]);
    keep(&mut s, vec![Arc::new(source)]);
    let hash = name_hash32(b"moved_t");
    let told = Told::default();
    looked_for(&mut s, &told, &[(hash, holding(&[1, 2, 3]))]);
    assert_eq!(translated(&s, hash), [true, true]);
    assert_eq!(
        handed(&s, hash, holding(&[1, 2, 3])),
        Some(Ok(vec![1, 2, 3]))
    );
    let skinned = holding(&[1, 2, 3, 6, 8]);
    assert_eq!(handed(&s, hash, skinned), Some(Ok(vec![1, 2, 3, 6, 8])));
}

#[test]
fn two_effects_whose_passes_come_to_the_same_two_stages_draw_by_one_pass_of_the_device() {
    use or_renderer::game_shaders::meshlets::{MeshletPipelines, MeshletSurface};
    let dir = tempfile::tempdir().unwrap();
    let one: Listed = &[(b"M\x01", b"P")];
    let other: Listed = &[(b"M\x01\x02", b"P")];
    let names = ["one_t", "two_t", "other_t"];
    let mut s = Shaders::default();
    let source = shaded(
        dir.path(),
        &[(names[0], one), (names[1], one), (names[2], other)],
    );
    keep(&mut s, vec![Arc::new(source)]);
    let worn = |name: &&str| (name_hash32(name.as_bytes()), holding(&[1]));
    let wanted: Vec<Worn> = names.iter().map(worn).collect();
    looked_for(&mut s, &Told::default(), &wanted);
    // The passes a device builds are counted as it counts them: by the two stages of a surface.
    let mut built = MeshletPipelines::default();
    let mut passes = Vec::new();
    for (hash, held) in wanted {
        let pass = picked(&s, hash, held).unwrap().unwrap();
        let surface = MeshletSurface {
            vertex: Arc::clone(&pass.mesh.module),
            fragment: Arc::clone(&pass.pixel.module),
            feeds: Default::default(),
            meshlets: 1,
            primitives: 1,
            forward: None,
        };
        built.hold(&surface, true);
        passes.push(pass);
    }
    assert_eq!(
        built.len(),
        2,
        "passes of three effects, two of the same stages"
    );
    // Each effect's pass is still its own: what fills its bindings is the effect's.
    assert!(!Arc::ptr_eq(&passes[0], &passes[1]));
    // The third's mesh stage holds something else, and its pixel stage is the others'.
    assert!(!Arc::ptr_eq(&passes[0].mesh.module, &passes[2].mesh.module));
    assert!(Arc::ptr_eq(
        &passes[0].pixel.module,
        &passes[2].pixel.module
    ));
}

#[test]
fn the_effects_of_a_job_are_read_side_by_side_on_no_more_workers_than_the_machine_spares() {
    let dir = tempfile::tempdir().unwrap();
    let pass: Listed = &[(b"M\x01", b"P")];
    let names = ["a_t", "b_t", "c_t", "d_t", "e_t", "f_t"];
    let effects: Vec<(&str, Listed)> = names.iter().map(|name| (*name, pass)).collect();
    let mut s = Shaders::default();
    keep(&mut s, vec![Arc::new(shaded(dir.path(), &effects))]);
    let wanted: Vec<Worn> = (names.iter())
        .map(|name| (name_hash32(name.as_bytes()), holding(&[1])))
        .collect();
    let told = Told {
        slow: true,
        ..Told::default()
    };
    looked_for(&mut s, &told, &wanted);
    assert_eq!(told.passes.load(SeqCst), names.len());
    let (most, spared) = (told.most.load(SeqCst), or_job::workers());
    assert!(
        most <= spared,
        "{most} at once on a machine that spares {spared}"
    );
    assert!(most >= spared.min(2), "{most} at once: one after the other");
}

#[test]
fn a_forward_pass_draws_only_an_effect_that_has_no_pass_which_shades_a_model() {
    let dir = tempfile::tempdir().unwrap();
    let alone: Listed = &[(b"M\x01", b"F")];
    let both: Listed = &[(b"M\x01", b"F"), (b"M\x01\x02", b"P")];
    let mut s = Shaders::default();
    let source = shaded(dir.path(), &[("forward_t", alone), ("both_t", both)]);
    keep(&mut s, vec![Arc::new(source)]);
    let (forward, mixed) = (name_hash32(b"forward_t"), name_hash32(b"both_t"));
    let held = holding(&[1, 2]);
    looked_for(&mut s, &Told::default(), &[(forward, held), (mixed, held)]);
    let pass = |hash: u32| picked(&s, hash, held).unwrap().unwrap();
    // Alone, its forward pass draws it, laid on by its blend.
    assert_eq!(translated(&s, forward), [true]);
    assert_eq!(pass(forward).forward, Some(LAID));
    // Beside a pass that shades a model it is no candidate: that one draws.
    assert_eq!(translated(&s, mixed), [true]);
    assert_eq!(pass(mixed).forward, None);
    let streams: Vec<u32> = pass(mixed).mesh.streams.iter().copied().collect();
    assert_eq!(streams, [1, 2]);
}
