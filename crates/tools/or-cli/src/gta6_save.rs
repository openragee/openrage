//! `openrage-cli gta6-save`: what a GTA VI save container states, a value of a story save read
//! or replaced into a new file, and the key a name hashes to.

use std::collections::{BTreeSet, HashMap};
use std::process::ExitCode;

use or_io::gta6::save::{self, AsavHeader, Block, Jsav, SaveKind, Slayer1};

const USAGE: &str = "usage: gta6-save info <file>\n       \
     gta6-save get <file> <block> <path>\n       \
     gta6-save set <file> <block> <path> <json> <out_file>\n       \
     gta6-save key <name>...\n\
     <block> is a block's id in hex (0x...) or its name; <path> is steps joined by '/', each a \
     name, a key as the file writes it ($0x..., $x) or an array index, from inside the block";

pub fn run(args: &[String]) -> ExitCode {
    let done = match args.split_first() {
        Some((verb, [file])) if verb == "info" => info(file),
        Some((verb, [file, block, path])) if verb == "get" => get(file, block, path),
        Some((verb, [file, block, path, json, out])) if verb == "set" => {
            set(file, block, path, json, out)
        }
        Some((verb, names)) if verb == "key" && !names.is_empty() => {
            for name in names {
                println!("{}  {name}", save::key_text(name));
            }
            Ok(())
        }
        _ => return crate::usage_error(USAGE),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::from(1)
        }
    }
}

fn read(file: &str) -> Result<Vec<u8>, String> {
    std::fs::read(file).map_err(|e| format!("{file}: {e}"))
}

fn header_lines(h: &AsavHeader) -> Vec<String> {
    let kind = match h.kind {
        1 => "a packed story save",
        2 => "settings",
        3 => "a profile record",
        _ => "a kind not measured",
    };
    vec![
        format!("  kind          {} ({kind})", h.kind),
        format!("  title         {}", h.title),
        format!("  slot          {}", h.slot),
        format!("  time          {} s since 1970, UTC", h.time),
        format!(
            "  payload       at {:#X}; size stated {}, stored size stated {}",
            h.payload_offset, h.size, h.stored_size
        ),
        format!(
            "  checksum      {:#010X}, the RAGE joaat of the header; recomputed on save",
            h.checksum
        ),
        format!(
            "  not named     the words at 0x0C and 0x10: {:#X} and {:#X}",
            h.word_0c, h.word_10
        ),
    ]
}

/// The names the metadata names hold, GTA VI's parser names among them, each by the hash a save
/// writes for it.
fn key_names() -> HashMap<u32, String> {
    crate::meta::schema();
    let _title = or_io::meta::binary::hash_index::title_names();
    (or_io::meta::binary::hash_index::default_hash_index().names())
        .map(|name| (save::key_hash(name), name.to_owned()))
        .collect()
}

/// The distinct hashed keys the blocks' texts write, and how many of them `names` names.
fn key_tally<'a>(
    texts: impl IntoIterator<Item = &'a str>,
    names: &HashMap<u32, String>,
) -> (usize, usize) {
    let mut keys = BTreeSet::new();
    for text in texts {
        // A key is `"$0x`, eight hex digits and `"`, then a colon.
        for (at, _) in text.match_indices("\"$0x") {
            let key = text.get(at + 1..at + 12).and_then(save::parse_key);
            let after = text.get(at + 12..).unwrap_or_default();
            if after
                .strip_prefix('"')
                .is_some_and(|rest| rest.trim_start().starts_with(':'))
            {
                keys.extend(key);
            }
        }
    }
    let named = keys.iter().filter(|key| names.contains_key(key)).count();
    (named, keys.len())
}

fn block_line(group: &str, block: &Block, names: &HashMap<u32, String>) -> String {
    let keys = match block.keys() {
        Some((_, inner)) if inner == block.hash => "keyed by its id".to_string(),
        Some((_, inner)) => format!("keyed {inner:#010X}, not its id"),
        None => "its outer keys are not two hashes".to_string(),
    };
    let name = names.get(&block.hash).map_or("", String::as_str);
    format!(
        "    {group} {:#010X} {name}  flags {}, {} bytes; {keys}",
        block.hash,
        block.flags,
        block.text.len()
    )
}

fn info(file: &str) -> Result<(), String> {
    let bytes = read(file)?;
    match SaveKind::of(&bytes).ok_or_else(|| save::SaveError::NotASave.to_string())? {
        SaveKind::Story => {
            let story = Jsav::open(&bytes).map_err(|e| e.to_string())?;
            println!(
                "JSAV version {}: a story save of {} code blocks and {} script blocks",
                save::JSAV_VERSION,
                story.code.len(),
                story.script.len()
            );
            let same = story.save().is_ok_and(|written| written == bytes);
            let rebuild = if same {
                "byte-identical"
            } else {
                "differs from the file"
            };
            println!("  rebuild       {rebuild}");
            let names = key_names();
            for block in &story.code {
                println!("{}", block_line("code  ", block, &names));
            }
            for block in &story.script {
                println!("{}", block_line("script", block, &names));
            }
            let (named, keys) = key_tally(story.blocks().map(|b| b.text.as_str()), &names);
            println!(
                "  keys          {named} of its {keys} distinct hashed keys are the hash, case kept, of a name the metadata names hold"
            );
        }
        SaveKind::Record => {
            let record = save::Asav::open(&bytes).map_err(|e| e.to_string())?;
            println!(
                "ASAV version {}: a header and {} payload bytes",
                save::ASAV_VERSION,
                record.payload.len()
            );
            for line in header_lines(&record.header) {
                println!("{line}");
            }
            let json = (record.text())
                .is_some_and(|t| serde_json::from_str::<serde_json::Value>(t).is_ok());
            let payload = if json {
                "JSON text"
            } else {
                "not JSON text: not decoded"
            };
            println!("  payload is    {payload}");
        }
        SaveKind::Packed => {
            let packed = Slayer1::read(&bytes).map_err(|e| e.to_string())?;
            println!(
                "SLAYER1 version {}: a packed copy named {}, {} payload bytes stated",
                save::ASAV_VERSION,
                packed.name,
                packed.payload_size
            );
            for line in header_lines(&packed.header) {
                println!("{line}");
            }
            println!(
                "  not read      its body, compressed or encrypted; the plain JSAV beside it \
                 holds the same save"
            );
        }
    }
    Ok(())
}

/// The id `block` names: hex digits after `0x`, else the hash of a name.
fn block_id(block: &str) -> Result<u32, String> {
    match block
        .strip_prefix("0x")
        .or_else(|| block.strip_prefix("0X"))
    {
        Some(hex) => u32::from_str_radix(hex, 16).map_err(|e| format!("{block}: {e}")),
        None => Ok(save::key_hash(block)),
    }
}

/// The keys from the block's root to `path`: the block's two outer keys, then each step, a name hashed.
fn steps(block: &Block, path: &str) -> Vec<String> {
    let outer = block.keys().map(|(group, inner)| [group, inner]);
    let outer = outer
        .into_iter()
        .flatten()
        .map(|hash| format!("$0x{hash:08x}"));
    let inner = path.split('/').filter(|s| !s.is_empty()).map(|step| {
        let written = step.starts_with('$') || step.bytes().all(|b| b.is_ascii_digit());
        if written {
            step.to_string()
        } else {
            save::key_text(step)
        }
    });
    outer.chain(inner).collect()
}

fn opened(file: &str, block: &str) -> Result<(Jsav, u32), String> {
    let story = Jsav::open(&read(file)?).map_err(|e| format!("{file}: {e}"))?;
    Ok((story, block_id(block)?))
}

fn no_block(id: u32) -> String {
    format!("the save has no block {id:#010X}")
}

fn get(file: &str, block: &str, path: &str) -> Result<(), String> {
    let (mut story, id) = opened(file, block)?;
    let block = story.block_mut(id).ok_or_else(|| no_block(id))?;
    let steps = steps(block, path);
    let steps: Vec<&str> = steps.iter().map(String::as_str).collect();
    let value =
        (block.get(&steps)).ok_or_else(|| format!("no value lies at {}", steps.join("/")))?;
    println!("{value}");
    Ok(())
}

fn set(file: &str, block: &str, path: &str, json: &str, out: &str) -> Result<(), String> {
    let (mut story, id) = opened(file, block)?;
    let block = story.block_mut(id).ok_or_else(|| no_block(id))?;
    let steps = steps(block, path);
    let steps: Vec<&str> = steps.iter().map(String::as_str).collect();
    block.set(&steps, json).map_err(|e| e.to_string())?;
    let bytes = story.save().map_err(|e| e.to_string())?;
    std::fs::write(out, &bytes).map_err(|e| format!("{out}: {e}"))?;
    println!(
        "wrote {out}: {} bytes, block {id:#010X} edited at {}",
        bytes.len(),
        steps.join("/")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_a_save_writes_are_counted_once_each_and_by_the_names_that_hash_to_them() {
        let names = HashMap::from([(save::key_hash("ZqHealth"), "ZqHealth".to_string())]);
        let health = save::key_text("ZqHealth");
        let texts = [
            format!("{{\"{health}\":1,\"$0x0000002a\":{{\"{health}\":2}}}}"),
            // A reserved key, a key cut short and a value written as a key is are no keys.
            "{\"$x\":\"$0x0000002b\",\"$0x12\":0,\"$0x0000002c\":0}".to_string(),
        ];
        assert_eq!(key_tally(texts.iter().map(String::as_str), &names), (1, 3));
        assert_eq!(key_tally([], &names), (0, 0));
    }

    #[test]
    fn a_path_s_names_are_hashed_and_its_written_keys_and_indices_kept() {
        let group = save::key_text("Group");
        let text = format!("{{\"{group}\":{{\"$0x0000002a\":{{}}}}}}");
        // A story save of that one block, laid out by hand.
        let mut bytes = b"JSAV".to_vec();
        for word in [3u32, 1, 0, 0x2A, 1, 0, 0x30, text.len() as u32] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.resize(0x30, 0);
        bytes.extend_from_slice(text.as_bytes());
        bytes.resize(bytes.len().next_multiple_of(16), 0);
        let story = Jsav::open(&bytes).expect("opens");
        assert_eq!(
            steps(&story.code[0], "Position/$x/3/"),
            [
                group,
                "$0x0000002a".to_string(),
                save::key_text("Position"),
                "$x".to_string(),
                "3".to_string()
            ]
        );
        assert_eq!(block_id("0x2A"), Ok(0x2A));
        assert_eq!(block_id("Name"), Ok(save::key_hash("Name")));
        let names = HashMap::from([(0x2A, "ZqStructure".to_string())]);
        let line = block_line("code  ", &story.code[0], &names);
        assert!(line.ends_with("keyed by its id") && line.contains(" ZqStructure "));
    }
}
