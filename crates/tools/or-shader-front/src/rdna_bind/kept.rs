//! What a session keeps for the next one: translations under a folder, and the effect containers
//! of an archive as they were read, so that opening a model again reads and translates nothing.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

use or_io::gta5::shaders::sgd5::Sgd5File;

use super::Native;
use crate::rdna::{Lowered, Options};
use crate::rdna_cache::{Cache, Cached};

/// The stored form of the containers: a change to what a container reads as takes a new number.
const CONTAINERS: u32 = 1;

static TRANSLATIONS: OnceLock<Cache> = OnceLock::new();

/// Keep translations in files under `dir` as well as in memory. The first call of a process
/// decides, and one made after a program was lowered changes nothing.
pub fn keep_translations_in(dir: impl Into<PathBuf>) {
    let _ = TRANSLATIONS.set(Cache::on_disk(dir));
}

/// `native` lowered as [`super::lower`] lowers it, once: a program lowered before, in this
/// session or one that kept its translations, is read back. `Err` is the refusal as it is said.
pub fn lower(
    native: &Native,
    half_slots: &std::collections::BTreeSet<u8>,
) -> Result<Lowered, String> {
    let mut options = Options::default();
    options.half_slots.extend(half_slots);
    let cache = TRANSLATIONS.get_or_init(Cache::in_memory);
    match &*cache.get_or_translate(&native.program, &options).0 {
        Cached::Translated(lowered) => Ok((**lowered).clone()),
        Cached::Refused { message, .. } => Err(message.clone()),
    }
}

/// The effect containers of an archive, each with its offset there, as the bytes a file holds.
pub fn encode_containers(containers: &[(u64, &Sgd5File)]) -> Vec<u8> {
    let mut out = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    // Neither the serialisation of plain data nor a write to memory fails.
    let _ = bincode::serialize(&(CONTAINERS, containers)).map(|bytes| out.write_all(&bytes));
    out.finish().unwrap_or_default()
}

/// What [`encode_containers`] wrote; `None` for bytes of another form or cut short.
pub fn decode_containers(bytes: &[u8]) -> Option<Vec<(u64, Sgd5File)>> {
    let mut plain = Vec::new();
    (flate2::read::DeflateDecoder::new(bytes).read_to_end(&mut plain)).ok()?;
    let (form, containers): (u32, Vec<(u64, Sgd5File)>) = bincode::deserialize(&plain).ok()?;
    (form == CONTAINERS).then_some(containers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dxil::ShaderKind;
    use crate::rdna::Program;

    #[test]
    fn the_containers_of_an_archive_read_back_as_they_were_kept() {
        let mut file = Sgd5File {
            head_word: 7,
            layouts: Vec::new(),
            globals: Default::default(),
            bank_counts: [Some(1), None, None, None, None, Some(0)],
            programs: Vec::new(),
            gaps: Vec::new(),
            effect_count: Some(0),
            effects: Vec::new(),
        };
        file.globals.names.push("a sampler".to_owned());
        let bytes = encode_containers(&[(0x9000, &file)]);
        assert_eq!(decode_containers(&bytes), Some(vec![(0x9000, file)]));
        assert_eq!(decode_containers(&bytes[..bytes.len() / 2]), None);
        assert_eq!(decode_containers(b"not a kept file"), None);
    }

    #[test]
    fn a_program_lowered_again_is_the_translation_kept() {
        // The one instruction that ends a program.
        let native = Native {
            program: Program {
                kind: ShaderKind::Pixel,
                slot: 5,
                words: vec![0xBF81_0000],
                wave32: false,
                inputs: Vec::new(),
                vertex_inputs: Vec::new(),
                position_read: 0,
                front_face: false,
                launch: None,
            },
            records: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        let first = lower(&native, &Default::default());
        let again = lower(&native, &Default::default());
        assert_eq!(
            first.as_ref().map(|l| format!("{:?}", l.module)),
            again.as_ref().map(|l| format!("{:?}", l.module))
        );
        assert_eq!(first.err(), again.err());
    }
}
