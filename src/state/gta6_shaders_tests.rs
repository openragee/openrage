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
        wanted,
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
        wanted: vec![hash],
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
    shaders.tried.extend([7, 8]);
    let (_, first, _) = archive(dir.path());
    let (_, second, _) = archive(dir.path());
    keep(&mut shaders, vec![Arc::new(first)]);
    keep(&mut shaders, vec![Arc::new(second)]);
    assert_eq!(shaders.sources.len(), 1);
    assert_eq!(shaders.tried, HashSet::from([7]));
}
