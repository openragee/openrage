//! The readers a written file is reloaded through.
//!
//! A declared RSC7 format is read the way `openrage-cli search --verify` reads it, a DDS through its
//! own reader, a `.ymt` or a `.ymf` as the metadata document it is, and a `.ymap` or `.ytyp` as the typed root it is. A name nothing here reads is a file with
//! no reader to try.

use or_io::gta5::formats::Format;
use or_io::meta::{MetaInstance, Value};
use or_io::Generation;

/// A file a reader opened.
pub struct Reload {
    /// The reader that opened it, by what it read.
    pub reader: String,
    /// For a metadata document, the elements of the arrays its root holds.
    pub entries: Option<usize>,
}

/// The extensions [`reload`] opens with a reader of its own; every other one it opens is a declared RSC7 format.
const OWN: [&str; 6] = ["dds", "meta", "ymap", "ytyp", "ymt", "ymf"];

/// Whether a build opens a file of the extension `ext`, in lower case, again through a reader: the kinds of file a build knows.
pub fn reads(ext: &str) -> bool {
    OWN.contains(&ext) || or_io::gta5::formats::format(ext).is_some()
}

/// The reader that opened a file named `name`, or the reason it refused; `None` when no reader reads that name.
pub fn reload(name: &str, bytes: &[u8]) -> Option<Result<Reload, String>> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    // ⚠ The table decides, so a reader added below for a kind the table lacks is never reached.
    if !reads(&ext) {
        return None;
    }
    let read = |reader: String| Reload {
        reader,
        entries: None,
    };
    match ext.as_str() {
        "dds" => {
            return Some(
                or_io::dds::Dds::from_bytes(bytes)
                    .map(|_| read("Dds".to_string()))
                    .map_err(|e| e.to_string()),
            )
        }
        // ⚠ Only a weapon document: another `.meta` is a data file this build has no reader for, and it must not be failed for that.
        "meta" => {
            return or_io::meta::typed::weapon::reload(bytes).map(|read| {
                read.map(|(root, entries)| Reload {
                    reader: format!("{root} (XML)"),
                    entries: Some(entries),
                })
            })
        }
        // The world roots are read as the typed values the editor edits, which refuse a member of the wrong shape as the game would.
        "ymap" | "ytyp" => return Some(world_root(&ext, bytes)),
        "ymt" | "ymf" => {
            return Some(
                or_io::meta::MetaDocument::parse(or_io::meta::default_library(), bytes.to_vec())
                    .map(|doc| Reload {
                        reader: format!(
                            "{} ({})",
                            doc.instance.type_name,
                            doc.source_format.label()
                        ),
                        entries: Some(entries_of(&doc.instance)),
                    })
                    .map_err(|e| e.to_string()),
            )
        }
        _ => {}
    }
    let format = or_io::gta5::formats::format(&ext)?;
    Some(
        format
            .open_declared(bytes)
            .map(|target| read(label(format, target)))
            .map_err(|e| e.to_string()),
    )
}

/// A `.ymap` or `.ytyp` opened as the root it is, with the entities or the archetypes it holds.
fn world_root(ext: &str, bytes: &[u8]) -> Result<Reload, String> {
    use or_io::decl::File;
    use or_io::meta::typed::{CMapData, CMapTypes};
    let (root, entries) = match ext {
        "ymap" => CMapData::from_bytes(bytes)
            .map(|map| ("CMapData", map.entities.len()))
            .map_err(|e| e.to_string())?,
        _ => CMapTypes::from_bytes(bytes)
            .map(|types| ("CMapTypes", types.archetypes.len()))
            .map_err(|e| e.to_string())?,
    };
    let format = or_io::meta::detect_binary_format(bytes).map_or("XML", |f| f.label());
    Ok(Reload {
        reader: format!("{root} ({format})"),
        entries: Some(entries),
    })
}

/// The elements of the arrays that `root` holds, one level down.
fn entries_of(root: &MetaInstance) -> usize {
    root.fields
        .iter()
        .map(|(_, value)| match value {
            Value::Array(items) => items.len(),
            _ => 0,
        })
        .sum()
}

fn label(format: &Format, target: Generation) -> String {
    let root = format
        .root_name(target)
        .unwrap_or_else(|_| format.ext.to_string());
    format!("{root} ({})", target.edition_label())
}

/// Whether a file called `name` is a declared RSC7 format, which is laid out for one target; any other has one layout.
pub(crate) fn has_layout(name: &str) -> bool {
    let ext = name.rsplit_once('.').map_or("", |(_, ext)| ext);
    or_io::gta5::formats::format(ext).is_some()
}

/// The target a resource called `name` is laid out for, by its own RSC7 version.
pub fn layout(name: &str, bytes: &[u8]) -> Result<Generation, String> {
    let ext = name.rsplit_once('.').map_or("", |(_, ext)| ext);
    or_io::gta5::retarget::layout_of(&ext.to_ascii_lowercase(), bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_of_kinds_and_the_readers_are_one_set() {
        // ⚠ Not `meta`: only a weapon document has a reader, so bytes that are none are no reader's.
        for ext in OWN.iter().filter(|ext| **ext != "meta") {
            assert!(
                reload(&format!("a.{ext}"), b"no such file").is_some_and(|read| read.is_err()),
                "a .{ext} that is no .{ext} is refused by its reader"
            );
        }
        for format in or_io::gta5::formats::FORMATS {
            assert!(reads(format.ext), "{} is a declared format", format.ext);
            assert!(reload(&format!("a.{}", format.ext), b"no such file").is_some());
        }
        for ext in ["lua", "xml", "rpf", ""] {
            assert!(!reads(ext), "nothing reads a .{ext}");
            assert!(reload(&format!("a.{ext}"), b"no such file").is_none());
        }
    }
}
