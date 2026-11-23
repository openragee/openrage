//! GTA VI's programs through the DXIL front end, and through the native one and its binder where
//! a program holds native code: the names and constant buffers an `SGD5` table gives a program,
//! the key a pack stores it by, and `rpf scan-shaders`, which counts how every program of an
//! effect container translates, by outcome and by refusal.

use std::collections::BTreeMap;
use std::fs;

use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind};
use or_io::gta5::shaders::sgd5::{Sgd5File, Sgd5Program, Sgd5Table};
use or_io::gta5::shaders::sgd5_pack::constant_buffers;
use or_shader_front::dxil::container::{Container, ContainerError, PsvResourceKind};
use or_shader_front::dxil::plan::hash_name;
use or_shader_front::dxil::{translate, Naming, Refusal, Register, ResourceClass, Translated};
use or_shader_front::rdna_bind::{self, Unbound};
use sha1::{Digest, Sha1};

pub use or_shader_front::dxil::DXIL_FRONT;

/// `SGD2`'s resource type codes of what binds a shader-resource register; the others bind no
/// `PSV0` class anyone has matched.
const SRV_CODES: [u8; 5] = [0x01, 0x02, 0x05, 0x07, 0x42];
/// The bytes a program's stream may hold before it is refused unread.
const MAX_STREAM: usize = 64 << 20;

/// Why a GTA VI program is not translated: by the DXIL front end, or, where it holds native code, by
/// the native front end and its binder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gta6Refusal {
    Dxil(Refusal),
    Native(Unbound),
}

impl Gta6Refusal {
    /// The word a census counts by: the DXIL front end's, or `native:` and the binder's.
    pub fn name(&self) -> String {
        match self {
            Self::Dxil(refusal) => refusal.name(),
            Self::Native(unbound) => format!("native:{}", unbound.name()),
        }
    }
}

impl std::fmt::Display for Gta6Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dxil(refusal) => std::fmt::Display::fmt(refusal, f),
            Self::Native(unbound) => std::fmt::Display::fmt(unbound, f),
        }
    }
}

/// What one GTA VI program came to through the front end its payload takes.
#[derive(Debug)]
pub struct Gta6Program {
    /// The SHA-1 of its inflated container, as hex: its key in a pack.
    pub key: String,
    pub source_bytes: u32,
    /// Whether its inputs read blend indices, which only a skinned pass's vertex program does.
    pub skinned: bool,
    pub translated: Result<Translated, Gta6Refusal>,
}

/// `program` translated from its DEFLATE stream `stream`, read alone; `Err` where it does not
/// inflate. `effect` is the first table of an effect that runs it, which states the hash a
/// drawable names each of a native program's parameters by.
pub fn translate_gta6_program(
    program: &Sgd5Program,
    stream: &[u8],
    effect: Option<&Sgd5Table>,
) -> Result<Gta6Program, String> {
    if stream.len() > MAX_STREAM {
        return Err(format!(
            "program `{}` states a {}-byte stream, past the {MAX_STREAM} bytes one is read to",
            program.name,
            stream.len()
        ));
    }
    let container = program.inflate(stream).map_err(|e| e.to_string())?;
    let inputs = Container::read(&container).and_then(|c| c.inputs());
    let skinned = inputs.is_ok_and(|inputs| {
        (inputs.iter()).any(|e| e.semantic.eq_ignore_ascii_case("BLENDINDICES"))
    });
    Ok(Gta6Program {
        key: Sha1::digest(&container)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        source_bytes: u32::try_from(container.len()).unwrap_or(u32::MAX),
        skinned,
        translated: match translate(&container, &naming(&program.variables, &container)) {
            Err(Refusal::Container(ContainerError::NativeCode { .. })) => {
                rdna_bind::bind(&container, &program.variables, effect).map_err(Gta6Refusal::Native)
            }
            other => other.map_err(Gta6Refusal::Dxil),
        },
    })
}

/// A variable's name: the GTA V engine name its +0x10 or +0x14 hash is of, else `0x` and the +0x14 hash.
fn variable_name(hash: u32, semantic: u32) -> Option<String> {
    let engine = hash_name(hash);
    Some(match engine.starts_with("0x") {
        true => hash_name(semantic),
        false => engine,
    })
}

/// What `table` names in the program `container` holds. A record states a register and no space:
/// a texture's name goes by class and register, for the plan to place where one space holds it,
/// and a constant buffer by its full register where the container's `PSV0` holds it in one space.
fn naming(table: &Sgd5Table, container: &[u8]) -> Naming {
    let mut naming = Naming::default();
    for r in (table.resources.iter()).filter(|r| SRV_CODES.contains(&r.code)) {
        let key = (ResourceClass::Srv, u32::from(r.slot));
        naming.by_register.insert(key, hash_name(r.semantic_hash));
    }
    let psv = Container::read(container).and_then(|c| c.psv());
    let cbvs: Vec<(u32, u32)> = (psv.iter().flat_map(|p| &p.resources))
        .filter(|r| r.kind == PsvResourceKind::Cbv)
        .map(|r| (r.space, r.lower))
        .collect();
    for (slot, buffer) in constant_buffers(table, &variable_name) {
        let mut spaces = (cbvs.iter()).filter(|(_, lower)| *lower == u32::from(slot));
        let (Some((space, _)), None) = (spaces.next(), spaces.next()) else {
            continue;
        };
        let register = Register {
            class: ResourceClass::Cbv,
            space: *space,
            index: u32::from(slot),
        };
        naming.resources.insert(register, buffer.name.clone());
        naming.constant_buffers.insert(register, buffer);
    }
    naming
}

/// How the programs of a container came out: translated, refused by name, or not read.
#[derive(Debug, Default, PartialEq, Eq)]
struct Census {
    translated: usize,
    refused: BTreeMap<String, usize>,
    unread: usize,
}

fn census(file: &Sgd5File, bytes: &[u8]) -> Census {
    let mut census = Census::default();
    for program in &file.programs {
        let read = bytes
            .get(program.stream.clone())
            .map(|s| translate_gta6_program(program, s, None));
        match read {
            Some(Ok(Gta6Program {
                translated: Ok(_), ..
            })) => census.translated += 1,
            Some(Ok(Gta6Program {
                translated: Err(refusal),
                ..
            })) => *census.refused.entry(refusal.name()).or_default() += 1,
            Some(Err(_)) | None => census.unread += 1,
        }
    }
    census
}

/// `rpf scan-shaders`: every program of the `SGD5` container at an offset through the DXIL front
/// end, printed as counts by outcome and by refusal and never as program text or bytes.
pub(crate) fn cmd_scan_shaders(args: &[String]) -> Result<(), String> {
    let (limit, args) = super::rpf::split_limit(args)?;
    let [path, offset] = args.as_slice() else {
        return Err("usage: rpf scan-shaders <archive.rpf> <offset> [--limit <bytes>]".to_string());
    };
    let offset = super::rpf::parse_offset(offset)?;
    let file = fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let (find, bytes) = read_rpf9_body(&file, offset, limit).map_err(|e| e.to_string())?;
    if !matches!(find.kind, Rpf9BodyKind::Sgd5 { .. }) {
        return Err(format!(
            "the body at {offset:#X} is not an SGD5 effect container"
        ));
    }
    let container = Sgd5File::open(&bytes)
        .map_err(|e| format!("the SGD5 container at {offset:#X} does not read: {e}"))?;
    let census = census(&container, &bytes);
    let refused: usize = census.refused.values().sum();
    println!(
        "  programs      {}: {} translated, {refused} refused, {} not read",
        container.programs.len(),
        census.translated,
        census.unread
    );
    for (reason, count) in &census.refused {
        println!("    refused {reason}: {count}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta5::shaders::sgd5::{Sgd5Constant, Sgd5Resource};
    use or_io::gta5::shaders::xdat::{XdatProgram, XdatStage};

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
        out.extend_from_slice(&[0x5A; 16]);
        out.extend(dwords(&[
            1,
            (table_end + body.len()) as u32,
            parts.len() as u32,
        ]));
        out.extend(dwords(&offsets));
        out.extend(body);
        out
    }

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    /// A vertex stage's `FSSM` package: slot 4 used, its 64 bytes of code three made-up words,
    /// `s_endpgm` and `s_code_end` padding.
    fn package() -> Vec<u8> {
        let mut m = vec![0u8; 0x310 + 64 + 16];
        let len = m.len();
        put(&mut m, 0, b"FSSM");
        put(&mut m, 4, &dwords(&[0x101, (len - 16) as u32, 0, 6, 0]));
        for slot in 0..12 {
            let value: u64 = if slot == 4 {
                0xB0
            } else {
                0xDEAD_BEEF_DEAD_BEEF
            };
            put(&mut m, 0x18 + 8 * slot, &value.to_le_bytes());
        }
        put(&mut m, 0x78, &dwords(&[0x2_0002]));
        // The descriptor, at the base (+0x10) plus 0xB0: code offset and size, next offset, stage word.
        put(
            &mut m,
            0xC0,
            &[0x300_u64.to_le_bytes(), 64_u64.to_le_bytes()].concat(),
        );
        put(&mut m, 0xC0 + 0x28, &(0x300_u64 + 64).to_le_bytes());
        put(&mut m, 0xC0 + 0x40, &dwords(&[0x2_0004]));
        let mut code = vec![0x7E02_0280, 0x7E04_0281, 0x7E06_0282, 0xBF81_0000];
        code.resize(16, 0xBF9F_0000);
        put(&mut m, 0x310, &dwords(&code));
        m
    }

    /// A vertex program whose DXIL part holds an `XDAT` payload in its real layout, which reads
    /// blend indices and binds constant buffer 4 of space 2.
    fn xdat_program() -> Vec<u8> {
        let dxil = XdatProgram {
            program_version: (1 << 16) | 0x66,
            digest: [0x3C; 16],
            stages: vec![XdatStage {
                slot: 4,
                stage_word: 0x2_0004,
                package: package(),
                code: 0x310..0x350,
            }],
            stub_words: 0x1A5,
        }
        .write()
        .unwrap();
        let psv = dwords(&[28, 0, 0, 0, 0, 0, 0, 0, 1, 16, 2, 2, 4, 4]);
        let mut isg = dwords(&[1, 8, 0, 40, 0, 0, 1, 3, 0x0F, 0]);
        isg.extend_from_slice(b"BLENDINDICES\0");
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

    fn table() -> Sgd5Table {
        let resource = |code: u8, slot: u16, name_hash: u32, semantic_hash: u32| Sgd5Resource {
            code,
            flags: 0,
            slot,
            members: 1,
            buffer_size: 32,
            name_hash,
            semantic_hash,
        };
        Sgd5Table {
            resources: vec![
                resource(0x30, 4, 0x0C0C_0001, 0x0C0C_1001),
                resource(0x02, 1, 0x0707_0002, 0x0707_1002),
                // A code no `PSV0` class is matched to: not named.
                resource(0x14, 2, 0x0909_0003, 0x0909_1003),
                // A constant buffer `PSV0` does not bind: not named.
                resource(0x30, 6, 0x0D0D_0004, 0x0D0D_1004),
            ],
            constants: vec![Sgd5Constant {
                code: 0x0C,
                count: 0,
                offset: 16,
                default_offset: 0,
                default: None,
                name_hash: 0x0E0E_0005,
                semantic_hash: 0x0E0E_1005,
                cbuffer: Some(0x0C0C_0001),
            }],
            ..Sgd5Table::default()
        }
    }

    #[test]
    fn a_program_s_table_names_its_textures_by_register_and_its_buffers_by_the_space_psv0_holds() {
        let naming = naming(&table(), &xdat_program());
        assert_eq!(
            naming.by_register,
            [((ResourceClass::Srv, 1), hash_name(0x0707_1002))].into()
        );
        let register = Register {
            class: ResourceClass::Cbv,
            space: 2,
            index: 4,
        };
        assert_eq!(naming.resources.len(), 1);
        assert_eq!(naming.resources[&register], hash_name(0x0C0C_1001));
        let buffer = &naming.constant_buffers[&register];
        assert_eq!(
            (buffer.name.as_str(), buffer.size),
            (hash_name(0x0C0C_1001).as_str(), 32)
        );
        let variables: Vec<(String, u32)> = (buffer.variables.iter())
            .map(|v| (v.name.clone(), v.offset))
            .collect();
        assert_eq!(variables, vec![(hash_name(0x0E0E_1005), 16)]);
    }

    #[test]
    fn a_program_of_native_code_is_keyed_by_its_container_and_refused_by_name() {
        let inflated = xdat_program();
        let stream = stored(&inflated);
        let program = Sgd5Program {
            bank: Some(0),
            index: Some(3),
            name: "vs_x".into(),
            kind: 0,
            key: 0,
            at: 0,
            stream: 0..stream.len(),
            inflated_len: inflated.len(),
            variables: table(),
        };
        let done = translate_gta6_program(&program, &stream, None).unwrap();
        let key: String = Sha1::digest(&inflated)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(done.key, key);
        assert_eq!(done.source_bytes as usize, inflated.len());
        assert!(done.skinned);
        // Native code goes to the native front end, which is what refuses these made-up words.
        let refusal = done.translated.unwrap_err();
        assert!(matches!(refusal, Gta6Refusal::Native(_)), "{refusal}");
        assert!(refusal.name().starts_with("native:"), "{}", refusal.name());
        assert!(!refusal.to_string().is_empty());

        // A stream cut short does not inflate.
        assert!(translate_gta6_program(&program, &stream[..stream.len() - 1], None).is_err());
    }

    #[test]
    fn a_census_counts_each_program_by_its_outcome() {
        let inflated = xdat_program();
        let stream = stored(&inflated);
        let mut bytes = vec![0xEE; 8];
        bytes.extend_from_slice(&stream);
        let program = |at: usize, len: usize| Sgd5Program {
            bank: Some(1),
            index: Some(at as u32),
            name: format!("ps_{at}"),
            kind: 0,
            key: 0,
            at,
            stream: at..at + len,
            inflated_len: inflated.len(),
            variables: Sgd5Table::default(),
        };
        let file = Sgd5File {
            head_word: 0,
            layouts: Vec::new(),
            globals: Sgd5Table::default(),
            bank_counts: [None; 6],
            programs: vec![
                program(8, stream.len()),
                program(8, stream.len()),
                // Its stream is not the program's.
                program(0, stream.len()),
                // Past the end.
                program(8, stream.len() + 100),
            ],
            gaps: Vec::new(),
            effect_count: Some(0),
            effects: Vec::new(),
        };
        let census = census(&file, &bytes);
        assert_eq!((census.translated, census.unread), (0, 2));
        assert_eq!(census.refused.values().sum::<usize>(), 2);
        assert!(census.refused.keys().all(|w| w.starts_with("native:")));
    }
}
