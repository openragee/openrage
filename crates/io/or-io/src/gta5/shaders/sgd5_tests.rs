use std::io::Write;

use super::*;

fn hash(s: &str) -> u32 {
    name_hash32(s.as_bytes())
}

fn name(s: &str) -> Vec<u8> {
    let mut out = vec![s.len() as u8 + 1];
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    out
}

fn le(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// A stand-in for a DXIL container: the magic, and its total size at 24.
fn container(len: usize) -> Vec<u8> {
    let mut out: Vec<u8> = (0..len).map(|i| (i * 7) as u8).collect();
    out[..4].copy_from_slice(b"DXBC");
    out[24..28].copy_from_slice(&(len as u32).to_le_bytes());
    out
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).expect("deflated");
    e.finish().expect("deflated")
}

fn record(code: u8, slot: u16, members: u8, first: u16, size: u32, hash: u32) -> Vec<u8> {
    let mut r = vec![code, 0];
    r.extend_from_slice(&slot.to_le_bytes());
    r.extend_from_slice(&[members, 0]);
    r.extend_from_slice(&first.to_le_bytes());
    r.extend_from_slice(&le(&[0, size, hash, hash]));
    r
}

/// `record` naming itself by the string `offset` bytes after its start.
fn named(mut record: Vec<u8>, offset: usize) -> Vec<u8> {
    record[8..10].copy_from_slice(&(offset as u16).to_le_bytes());
    record
}

/// A constant whose record states `hash` at +0x10 and `key` at +0x14.
fn constant(code: u8, count: u16, offset: u16, hash: u32, key: u32) -> Vec<u8> {
    let mut r = vec![code, 0];
    r.extend_from_slice(&count.to_le_bytes());
    r.extend_from_slice(&offset.to_le_bytes());
    r.extend_from_slice(&[0; 10]);
    r.extend_from_slice(&le(&[hash, key]));
    r
}

/// The bound-name section of [`effect`]'s one pass: its texture, then its constant buffer.
fn bound_words() -> Vec<u32> {
    vec![1, hash("diffusetex"), 0, 0, 1, hash("locals")]
}

/// A variable table with its u32 size first.
fn table(
    resources: &[Vec<u8>],
    constants: &[Vec<u8>],
    annotations: &[(u32, u32)],
    pool: &[u8],
) -> Vec<u8> {
    let mut t = Vec::new();
    for n in [resources.len(), constants.len(), annotations.len(), 0] {
        t.extend_from_slice(&(n as u16).to_le_bytes());
    }
    resources
        .iter()
        .chain(constants)
        .for_each(|r| t.extend_from_slice(r));
    annotations
        .iter()
        .for_each(|(h, v)| t.extend_from_slice(&le(&[*h, *v])));
    t.extend_from_slice(pool);
    let size = t.len() as u16;
    t[6..8].copy_from_slice(&size.to_le_bytes());
    let mut out = le(&[t.len() as u32]);
    out.extend(t);
    out
}

/// A binding layout of `records` records with its u32 size first.
fn layout(id: u32, records: u8) -> Vec<u8> {
    let mut l = le(&[24 + 8 * u32::from(records)]);
    l.extend_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
    l.extend_from_slice(&u16::from(records).to_le_bytes());
    l.extend_from_slice(&[0, records, 0, 0, 0, 0]);
    l.extend_from_slice(&le(&[id, 0]));
    for k in 0..u16::from(records) {
        l.extend_from_slice(&le(&[0xA000 + u32::from(k)]));
        l.extend_from_slice(&[1, 0]);
        l.extend_from_slice(&k.to_le_bytes());
    }
    l
}

fn program(name_: &str, key: u64, inflated: &[u8]) -> Vec<u8> {
    let stream = deflate(inflated);
    let size = 12 + stream.len() as u32;
    let mut p = name(name_);
    p.push(0x32);
    p.extend_from_slice(&le(&[size, FRAME, size, inflated.len() as u32]));
    p.extend_from_slice(&stream);
    let mut state = vec![0u8; PROGRAM_STATE];
    state[..8].copy_from_slice(&key.to_le_bytes());
    p.extend_from_slice(&state);
    p.extend_from_slice(&table(&[], &[], &[], &[]));
    p
}

const HEAD_ID: u32 = 0x0BAD_F00D;

fn effect() -> Vec<u8> {
    let mut e = name("fx");
    e.extend_from_slice(&le(&[0x111C]));
    for list in [&[0u32][..], &[0], &[], &[], &[], &[0]] {
        e.extend_from_slice(&le(&[list.len() as u32]));
        e.extend_from_slice(&le(list));
    }
    e.extend_from_slice(&[0; EFFECT_STATE]);
    e.extend_from_slice(&le(&[0, 0, 0, 0, 1]));
    // One technique named at 1 with one pass; the pass named at 6.
    e.extend_from_slice(&[1, 0, 0, 1]);
    let mut pass = vec![1, 1, 0, 0, 0, 1];
    pass.resize(PASS, 0xFF);
    pass[14..16].copy_from_slice(&6u16.to_le_bytes());
    e.extend_from_slice(&le(&[1]));
    e.extend_from_slice(&pass);
    let bound = bound_words();
    e.extend_from_slice(&le(&[bound.len() as u32]));
    e.extend_from_slice(&le(&bound));
    e.extend_from_slice(&le(&[LAYOUT_REFERENCE]));
    // A sampler, a texture and a constant buffer whose two members follow it; the second member
    // states another hash at +0x14 than at +0x10. The sampler names itself by the pool's first
    // string, past the five records from its own start.
    let resources = [
        named(record(0x23, 3, 0, 0, 0, hash("DiffuseSampler")), 5 * RECORD),
        record(0x02, 72, 0, 0, 0, hash("diffusetex")),
        record(CONSTANT_BUFFER, 1, 2, RECORD as u16, 0x40, hash("locals")),
    ];
    let constants = [
        constant(0x0C, 0, 0x30, hash("gloss"), hash("gloss")),
        constant(0x09, 0, 0x20, hash("shader_glowScale"), hash("GlowScale")),
    ];
    e.extend(table(&resources, &constants, &[], b"DiffuseSampler\0"));
    e.extend(table(&[], &[], &[(hash("UIName"), 7)], b"\0"));
    e.extend_from_slice(&le(&[0x0800_0002, LAYOUT_REFERENCE, HEAD_ID]));
    e.extend(layout(0x5150, 2));
    let names = b"\0Tech\0Pass\0";
    e.extend_from_slice(&le(&[names.len() as u32]));
    e.extend_from_slice(names);
    e
}

struct Built {
    bytes: Vec<u8>,
    /// Where each piece of the container starts: head, each program, the effect count, the effect.
    programs: Vec<usize>,
    effect_count: usize,
}

/// A container of three programs in banks 0, 1 and 5, `extra` more in bank 0, and one effect.
fn built(extra: usize) -> Built {
    let mut b = b"SGD5".to_vec();
    b.extend_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
    b.extend_from_slice(&le(&[1]));
    b.extend(layout(HEAD_ID, 3));
    b.extend(table(
        &[record(0x02, 0, 0, 0, 0, hash("gbuffer"))],
        &[],
        &[],
        b"\0",
    ));
    let mut programs = Vec::new();
    for (bank, n) in [1 + extra, 1, 0, 0, 0, 1].into_iter().enumerate() {
        b.extend_from_slice(&le(&[n as u32]));
        for k in 0..n {
            programs.push(b.len());
            let key = (bank * 100 + k) as u64;
            b.extend(program(
                &format!("P{bank}_{k}"),
                key,
                &container(300 + 50 * k),
            ));
        }
    }
    let effect_count = b.len();
    b.extend_from_slice(&le(&[1]));
    b.extend(effect());
    b.push(TRAILER);
    b.resize(b.len().next_multiple_of(TRAILER_PAGE), 0);
    Built {
        bytes: b,
        programs,
        effect_count,
    }
}

#[test]
fn a_container_reads_its_head_banks_programs_and_effects() {
    let built = built(0);
    let f = Sgd5File::open(&built.bytes).expect("read");
    assert_eq!(f.head_word, 0x0102_0304_0506_0708);
    assert_eq!(f.layouts.len(), 1);
    assert_eq!((f.layouts[0].id, f.layouts[0].bindings.len()), (HEAD_ID, 3));
    assert_eq!(f.globals.resources[0].name_hash, hash("gbuffer"));
    assert_eq!(
        f.bank_counts,
        [Some(1), Some(1), Some(0), Some(0), Some(0), Some(1)]
    );
    assert!(f.gaps.is_empty());
    let banks: Vec<_> = f
        .programs
        .iter()
        .map(|p| (p.bank, p.index, p.key))
        .collect();
    assert_eq!(
        banks,
        [
            (Some(0), Some(0), 0),
            (Some(1), Some(0), 100),
            (Some(5), Some(0), 500)
        ]
    );
    let p = f.program(1, 0).expect("bank 1's program");
    assert_eq!(p.name, "P1_0");
    assert_eq!(p.dxil(&built.bytes).expect("inflated"), container(300));

    assert_eq!(f.effect_count, Some(1));
    let e = &f.effects[0];
    assert_eq!((e.name.as_str(), e.word), ("fx", 0x111C));
    assert_eq!(e.programs[5], [0]);
    assert_eq!(
        e.techniques,
        [Sgd5Technique {
            name: "Tech".into(),
            flag: false,
            index: 0,
            passes: 1
        }]
    );
    assert_eq!(
        e.passes,
        [Sgd5Pass {
            name: "Pass".into(),
            programs: [1, 1, 0, 0, 0, 1]
        }]
    );
    assert_eq!(
        (e.layout_flag, e.layout_refs.as_slice(), e.layouts.len()),
        (8, &[HEAD_ID][..], 1)
    );
    let t = &e.parameters;
    assert_eq!(t.name_of(hash("diffusesampler")), Some("DiffuseSampler"));
    assert_eq!(t.name_of(hash("diffusetex")), None);
    assert_eq!(t.constants[0].cbuffer, Some(hash("locals")));
    assert_eq!(e.second.annotations, [(hash("UIName"), 7)]);
}

#[test]
fn a_program_in_a_piece_not_on_disk_is_named_as_that_and_the_read_steps_over_it() {
    let built = built(2);
    let (from, to) = (built.programs[1], built.programs[2]);
    // The piece replaces the second program and runs into the third, as a download leaves it.
    let mut bytes = built.bytes[..from].to_vec();
    bytes.resize(from + PIECE_PAGE, 0);
    bytes.extend_from_slice(&built.bytes[to..]);
    let f = Sgd5File::open(&bytes).expect("read around the piece");
    assert_eq!(
        f.gaps,
        [Sgd5Gap {
            from,
            resume: Some(from + PIECE_PAGE)
        }]
    );
    assert_eq!(f.program(0, 0).expect("before the piece").name, "P0_0");
    assert_eq!(
        f.program(0, 2)
            .expect("counted back from the bank's count")
            .name,
        "P0_2"
    );
    assert_eq!(
        f.program(0, 1),
        Err(Sgd5Error::ProgramNotOnDisk { bank: 0, index: 1 })
    );
    assert_eq!(
        f.program(0, 3),
        Err(Sgd5Error::NoSuchProgram { bank: 0, index: 3 })
    );
    assert_eq!(f.effects.len(), 1);
}

#[test]
fn effects_after_a_piece_that_holds_their_count_read_to_the_trailer() {
    let built = built(0);
    let from = built.programs[2] - 4;
    let effect_at = built.effect_count + 4;
    let mut bytes = built.bytes[..from].to_vec();
    bytes.resize(from + PIECE_PAGE, 0);
    bytes.extend_from_slice(&built.bytes[effect_at..]);
    let f = Sgd5File::open(&bytes).expect("read around the piece");
    assert_eq!(f.effect_count, None);
    assert_eq!(
        f.bank_counts[..5],
        [Some(1), Some(1), Some(0), Some(0), Some(0)]
    );
    assert_eq!(f.bank_counts[5], None);
    assert_eq!(
        f.program(5, 0),
        Err(Sgd5Error::ProgramNotOnDisk { bank: 5, index: 0 })
    );
    assert_eq!(f.effects[0].name, "fx");
    assert_eq!(
        f.gaps,
        [Sgd5Gap {
            from,
            resume: Some(from + PIECE_PAGE)
        }]
    );
}

#[test]
fn a_program_that_does_not_inflate_to_its_stated_size_is_refused() {
    let mut bytes = built(0).bytes;
    let at = built(0).programs[0] + name("P0_0").len() + 1 + 12;
    bytes[at..at + 4].copy_from_slice(&301u32.to_le_bytes());
    assert!(matches!(
        Sgd5File::open(&bytes),
        Err(Sgd5Error::Invalid { .. })
    ));
}

#[test]
fn open_for_reads_the_xbox_build_and_refuses_an_unmeasured_platform_by_name() {
    let bytes = built(0).bytes;
    assert!(Sgd5File::open_for(&bytes, Platform::XboxSeries).is_ok());
    for platform in [
        Platform::Ps5,
        Platform::Pc,
        Platform::Ps4,
        Platform::XboxOne,
    ] {
        match Sgd5File::open_for(&bytes, platform) {
            Err(Sgd5Error::PlatformNotMeasured { platform: refused }) => {
                assert_eq!(refused, platform);
                assert!(Sgd5Error::PlatformNotMeasured { platform }
                    .to_string()
                    .contains(platform.label()));
            }
            other => panic!("{platform:?} was not refused by name: {other:?}"),
        }
    }
}

#[test]
fn every_cut_of_a_container_is_refused_and_none_panics() {
    let built = built(0);
    let trailer = built
        .bytes
        .iter()
        .rposition(|b| *b == TRAILER)
        .expect("a trailer");
    for len in 0..=trailer {
        assert!(Sgd5File::open(&built.bytes[..len]).is_err(), "cut at {len}");
    }
    let mut huge = built.bytes.clone();
    let list = built.effect_count + 4 + name("fx").len() + 4;
    huge[list..list + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        Sgd5File::open(&huge),
        Err(Sgd5Error::Invalid { .. })
    ));
}

#[test]
fn a_reference_to_no_head_layout_and_bytes_after_the_trailer_are_refused() {
    let built = built(0);
    let mut bytes = built.bytes.clone();
    let id = HEAD_ID.to_le_bytes();
    let at = (built.effect_count..bytes.len())
        .find(|i| bytes[*i..].starts_with(&id))
        .expect("the reference");
    bytes[at] ^= 1;
    assert!(matches!(
        Sgd5File::open(&bytes),
        Err(Sgd5Error::Invalid { .. })
    ));
    let mut bytes = built.bytes.clone();
    let trailer = bytes
        .iter()
        .rposition(|b| *b == TRAILER)
        .expect("a trailer");
    bytes[trailer + 1] = 1;
    assert!(matches!(
        Sgd5File::open(&bytes),
        Err(Sgd5Error::Invalid { .. })
    ));
}

#[test]
fn saving_is_refused_and_no_program_is_handed_to_a_dxbc_translator() {
    let built = built(0);
    let f = Sgd5File::open(&built.bytes).expect("read");
    assert_eq!(f.save(), Err(Sgd5Error::ReadOnly));
    let dxil = f.programs[0].dxil(&built.bytes).expect("inflated");
    assert!(
        dxil.starts_with(b"DXBC"),
        "the container a caller gets, not a translation"
    );
    assert!(Sgd5Error::NotTranslated.to_string().contains("DXIL"));
}

#[test]
fn an_effect_s_parameters_are_defined_under_the_hashes_their_records_state() {
    let f = Sgd5File::open(&built(0).bytes).expect("read");
    let defs = shader_definitions("gta6", "gta6", &[f]);
    let shader = defs
        .shader(hash("FX"))
        .expect("keyed by the lower-cased name hash");
    assert_eq!(
        shader.parameters.len(),
        4,
        "a constant buffer is not itself a parameter"
    );
    let sampler = shader
        .parameter(hash("diffusesampler"))
        .expect("the sampler");
    assert_eq!(sampler.name, "DiffuseSampler");
    assert_eq!(sampler.register, 3);
    let texture = shader.parameter(hash("diffusetex")).expect("the texture");
    assert_eq!(texture.name, format!("hash_{:08X}", hash("diffusetex")));
    let gloss = shader.parameter(hash("gloss")).expect("the constant");
    assert_eq!(gloss.register, 3, "the 16-byte row its offset falls in");
    assert_eq!(gloss.cbuffer, Some(format!("hash_{:08X}", hash("locals"))));
    assert_eq!(gloss.default, None);
}

#[test]
fn only_a_member_of_a_buffer_whose_slot_has_bit_8_is_placed_where_a_drawable_holds_it() {
    let f = Sgd5File::open(&built(0).bytes).expect("read");
    let defs = shader_definitions("gta6", "gta6", std::slice::from_ref(&f));
    let shader = defs.shader(hash("fx")).expect("the effect");
    assert!(
        (shader.parameters.iter()).all(|p| p.drawable_place.is_none()),
        "slot 1 is a buffer the engine fills"
    );
    let mut held = f.clone();
    let buffer = &mut held.effects[0].parameters.resources[2];
    assert_eq!(buffer.code, CONSTANT_BUFFER);
    buffer.slot |= DRAWABLE_BUFFER_SLOT;
    let defs = shader_definitions("gta6", "gta6", &[held]);
    let shader = defs.shader(hash("fx")).expect("the effect");
    let place = |key: &str| drawable_parameter(shader, hash(key)).and_then(|p| p.drawable_place);
    assert_eq!(
        place("gloss"),
        Some((0, 0x30)),
        "its byte offset, not its row"
    );
    assert_eq!(place("GlowScale"), Some((0, 0x20)));
    assert_eq!(place("diffusetex"), None, "a texture has no buffer");
}

#[test]
fn a_drawable_s_parameter_joins_its_effect_by_the_hash_at_0x14_and_not_by_the_name_hash() {
    let f = Sgd5File::open(&built(0).bytes).expect("read");
    let defs = shader_definitions("gta6", "gta6", &[f]);
    let shader = defs.shader(hash("fx")).expect("the effect");
    let glow = drawable_parameter(shader, hash("GlowScale")).expect("joined by +0x14");
    assert_eq!(glow.name_hash, hash("shader_glowScale"));
    assert_eq!(glow.register, 2, "the 16-byte row of offset 0x20");
    assert!(
        drawable_parameter(shader, hash("shader_glowScale")).is_none(),
        "the name hash at +0x10 names no drawable parameter"
    );
    let gloss = drawable_parameter(shader, hash("gloss")).expect("+0x14 equals +0x10");
    assert_eq!(gloss.name_hash, hash("gloss"));
}

#[test]
fn gta_v_s_names_and_semantics_name_a_hash_they_hash_to() {
    let f = Sgd5File::open(&built(0).bytes).expect("read");
    let template = shader_definitions("gta6", "gta6", &[f]).shaders[0].parameters[0].clone();
    let param = |name: &str, semantic: Option<&str>| ShaderParamDef {
        name: name.into(),
        name_hash: hash(name),
        semantic: semantic.map(Into::into),
        semantic_hash: semantic.map(hash),
        ..template.clone()
    };
    let mut defs = ShaderDefinitions::new("gta5", "gen9");
    defs.shaders.push(ShaderDef {
        name: "default".into(),
        name_hash: hash("default"),
        effect: "default".into(),
        draw_bucket: None,
        vertex_type: 0,
        traits: Vec::new(),
        parameters: vec![
            param("DiffuseSampler", Some("DiffuseTex")),
            param("bumpiness", None),
        ],
    });
    let names = parameter_names(&defs);
    assert_eq!(names[&hash("diffusesampler")], "DiffuseSampler");
    assert_eq!(names[&hash("diffusetex")], "DiffuseTex");
    assert_eq!(names[&hash("bumpiness")], "bumpiness");
    assert_eq!(names.len(), 3);
    assert!(!names.contains_key(&hash("GlowScale")), "no name, no entry");
}

#[test]
fn a_pass_s_bound_lists_read_in_either_form_and_one_that_closes_as_neither_is_refused() {
    let built = built(0);
    let f = Sgd5File::open(&built.bytes).expect("read");
    let lists = [
        vec![hash("diffusetex")],
        vec![],
        vec![],
        vec![hash("locals")],
    ];
    assert_eq!(
        f.effects[0].bound,
        [Sgd5BoundPass {
            lists: lists.clone(),
            trailing: None
        }]
    );
    let plain = {
        let mut w = vec![bound_words().len() as u32];
        w.extend(bound_words());
        le(&w)
    };
    let at = (built.bytes.windows(plain.len()))
        .position(|w| w == plain)
        .expect("the section");
    let with = |last: u32| {
        let mut w = vec![bound_words().len() as u32 + 1];
        w.extend(bound_words());
        w.push(last);
        let mut bytes = built.bytes[..at].to_vec();
        bytes.extend(le(&w));
        bytes.extend_from_slice(&built.bytes[at + plain.len()..]);
        bytes
    };
    let f = Sgd5File::open(&with(0x101)).expect("the trailing form");
    assert_eq!(
        f.effects[0].bound,
        [Sgd5BoundPass {
            lists,
            trailing: Some(0x101)
        }]
    );
    assert!(matches!(
        Sgd5File::open(&with(0x201)),
        Err(Sgd5Error::Invalid { .. })
    ));
}

#[test]
fn a_layout_record_in_a_class_its_kind_does_not_map_to_is_refused() {
    let mut bytes = built(0).bytes;
    // The head layout's first record, kind 1 in class 1, made a sampler, whose class is 4.
    assert_eq!((class_of(1), class_of(3)), (Some(1), Some(4)));
    let kind = 0x10 + 4 + LAYOUT_HEAD + 4;
    assert_eq!(bytes[kind], 1);
    bytes[kind] = 3;
    assert!(matches!(
        Sgd5File::open(&bytes),
        Err(Sgd5Error::Invalid { .. })
    ));
}

#[test]
fn a_pool_name_is_the_printable_run_before_each_nul() {
    assert_eq!(pool_names(b"\x01abc\0\0xyz"), ["abc"]);
    let long = [b'a'; 70].iter().chain(b"\0").copied().collect::<Vec<_>>();
    assert_eq!(pool_names(&long), ["a".repeat(64)]);
}

#[test]
fn a_constant_s_default_is_its_type_s_words_at_its_offset_from_its_record_in_the_pool() {
    // Four constants then the pool at 8 + 4 × 24: a float, an integer triple, a float4 with no
    // default and a type whose size is not fixed.
    let pool_at = 8 + 4 * RECORD;
    let with_default = |code: u8, k: usize, target: usize| {
        let mut r = constant(code, 0, 4 * k as u16, hash("c") ^ k as u32, 0);
        let d = (target - (8 + RECORD * k)) as u16;
        r[10..12].copy_from_slice(&d.to_le_bytes());
        r
    };
    let constants = |first: usize| {
        [
            with_default(0x09, 0, first),
            with_default(0x03, 1, pool_at + 4),
            constant(0x0C, 0, 8, hash("none"), 0),
            with_default(0x0F, 3, pool_at + 16),
        ]
    };
    let pool = le(&[0.96f32.to_bits(), 0xE4E4_E4E4, 0xFFAA_5500, 0, 0x0F0F_0F0F]);
    let (_, t) = super::table(&table(&[], &constants(pool_at), &[], &pool), 0).expect("read");
    let defaults: Vec<_> = t.constants.iter().map(|c| c.default.clone()).collect();
    assert_eq!(
        defaults,
        [
            Some(vec![0.96f32.to_bits()]),
            Some(vec![0xE4E4_E4E4, 0xFFAA_5500, 0]),
            None,
            None
        ]
    );
    for refused in [pool_at - 4, pool_at + pool.len() - 2] {
        let bytes = table(&[], &constants(refused), &[], &pool);
        assert!(
            matches!(super::table(&bytes, 0), Err(Sgd5Error::Invalid { .. })),
            "a default at {refused} outside the pool"
        );
    }
    assert_eq!(
        (1..=0x0F).map(constant_units).collect::<Vec<_>>(),
        [
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            None,
            Some(16),
            None
        ]
    );
}

#[test]
fn a_definition_keeps_a_float_default_as_floats_and_an_integer_one_as_words() {
    let mut f = Sgd5File::open(&built(0).bytes).expect("read");
    let constants = &mut f.effects[0].parameters.constants;
    constants[1].default = Some(vec![0.5f32.to_bits()]);
    (constants[0].code, constants[0].default) = (0x03, Some(vec![0xE4E4_E4E4, 0xFFAA_5500, 0]));
    let defs = shader_definitions("gta6", "gta6", &[f]);
    let fx = defs.shader(hash("fx")).expect("the effect");
    let default = |key: &str| drawable_parameter(fx, hash(key)).and_then(|p| p.default.clone());
    assert_eq!(
        default("GlowScale"),
        Some(ShaderParamValue::Floats(vec![0.5]))
    );
    assert_eq!(
        default("gloss"),
        Some(ShaderParamValue::Uints(vec![0xE4E4_E4E4, 0xFFAA_5500, 0]))
    );
    let gloss = drawable_parameter(fx, hash("gloss")).expect("declared");
    assert!(holds_integers(gloss) && !holds_floats(gloss));
}

/// The three words of the remap table of `entries`, each a value and a flag.
fn remap_words(entries: [(u8, u8); 16]) -> [u32; 3] {
    let mut w = [0u32; 3];
    for (i, (value, flag)) in entries.into_iter().enumerate() {
        w[0] |= u32::from(value & 3) << (2 * i);
        w[1] |= u32::from(value >> 2) << (2 * i);
        w[2] |= u32::from(flag) << (2 * i);
    }
    w
}

#[test]
fn a_type_0x03_constant_reads_as_sixteen_entries_of_a_value_and_a_flag() {
    let identity: [(u8, u8); 16] = std::array::from_fn(|i| (i as u8, 0));
    assert!(is_identity_remap(&remap_words(identity)));
    assert_eq!(remap_table(remap_words(identity)), identity);
    let mut remapped = identity;
    remapped[0] = (3, 1);
    remapped[5] = (13, 1);
    remapped[15] = (15, 2);
    let words = remap_words(remapped);
    assert_eq!(remap_table(words), remapped);
    assert!(!is_identity_remap(&words));
    let mut flagged = identity;
    flagged[9].1 = 3;
    assert!(!is_identity_remap(&remap_words(flagged)));
    assert!(!is_identity_remap(&remap_words(identity)[..2]));
    assert_eq!(constant_units(0x00), Some(1));
}

#[test]
fn a_constant_of_type_0x00_0x0f_or_0x03_is_refused_where_its_room_does_not_fit_it() {
    // The member at 0x10 has room up to the next member, at `next`, of a 0x40-byte buffer.
    let reads = |code: u8, count: u16, next: u16| {
        let buffer = record(CONSTANT_BUFFER, 0, 2, RECORD as u16, 0x40, hash("locals"));
        let members = [
            constant(0x09, 0, next, hash("after"), 0),
            constant(code, count, 0x10, hash("it"), 0),
        ];
        super::table(&table(&[buffer], &members, &[], &[]), 0).is_ok()
    };
    for (code, count, next, fits) in [
        (0x03, 0, 0x1C, true),
        (0x03, 0, 0x18, false),
        (0x03, 2, 0x30, false),
        (0x00, 0, 0x14, true),
        (0x00, 1, 0x14, false),
        (0x0F, 4, 0x30, true),
        (0x0F, 3, 0x30, false),
        (0x0F, 0, 0x12, false),
        (0x0C, 0, 0x12, true),
    ] {
        assert_eq!(
            reads(code, count, next),
            fits,
            "type {code:#04X} [{count}] to {next:#X}"
        );
    }
    // A type-0x00 member that is not 4-aligned.
    let buffer = record(CONSTANT_BUFFER, 0, 1, RECORD as u16, 0x40, hash("locals"));
    let bytes = table(
        &[buffer],
        &[constant(0x00, 0, 0x12, hash("it"), 0)],
        &[],
        &[],
    );
    assert!(super::table(&bytes, 0).is_err());
}

#[test]
fn a_resource_is_named_by_the_string_its_record_points_at_and_one_stating_0_by_none() {
    // Three records, then the pool at 8 + 3 × 24: the first points at the pool's first string, the
    // second states 0 though the pool holds a string that hashes to it, and the third points at
    // that string under another hash.
    let pool_at = 8 + 3 * RECORD;
    let resources = [
        named(record(0x23, 1, 0, 0, 0, hash("gBumpSampler")), pool_at - 8),
        record(0x02, 2, 0, 0, 0, hash("other")),
        named(
            record(0x23, 3, 0, 0, 0, hash("gDetailSampler")),
            pool_at + 13 - (8 + 2 * RECORD),
        ),
    ];
    let bytes = table(&resources, &[], &[], b"gBumpSampler\0other\0");
    let (_, t) = super::table(&bytes, 0).expect("read");
    assert_eq!(
        t.resource_names,
        [
            Some("gBumpSampler".to_string()),
            None,
            Some("other".to_string())
        ]
    );
    assert_eq!(t.name_of(hash("gbumpsampler")), Some("gBumpSampler"));
    assert_eq!(
        t.name_of(hash("other")),
        None,
        "a pool string names no record that does not point at it"
    );
    assert_eq!(t.name_of(hash("gDetailSampler")), Some("other"));
}

#[test]
fn a_resource_name_outside_its_table_s_pool_or_with_no_nul_is_refused_by_the_record_s_hash() {
    let pool_at = 8 + RECORD;
    let read = |offset: usize, pool: &[u8]| {
        let resource = named(record(0x23, 1, 0, 0, 0, hash("gBumpSampler")), offset);
        super::table(&table(&[resource], &[], &[], pool), 0)
    };
    assert!(read(pool_at - 8, &b"gBumpSampler\0"[..]).is_ok());
    for (offset, pool, what) in [
        (
            pool_at - 8 + 64,
            &b"gBumpSampler\0"[..],
            "past the table's end",
        ),
        (4, &b"gBumpSampler\0"[..], "inside its own record"),
        (
            pool_at - 8,
            &b"gBumpSampler"[..],
            "no NUL before the table's end",
        ),
        (pool_at - 8, &b"\0"[..], "an empty name"),
    ] {
        match read(offset, pool) {
            Err(Sgd5Error::Invalid { at, why }) => {
                assert_eq!(at, 4 + 8, "{what}: refused at the record");
                assert!(
                    why.contains(&format!("{:08X}", hash("gBumpSampler"))),
                    "{what}: {why}"
                );
            }
            other => panic!("{what} was read: {other:?}"),
        }
    }
}
