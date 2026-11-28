//! Copying an archetype's dependency set out of a mount, whole or not at all.

use std::path::{Path, PathBuf};

use or_title::MountedGame;

use crate::resolve::{DependencySet, Edge, Found};
use or_world_gta5::Via;

/// The file an export writes beside the copies, one line per edge.
pub const MANIFEST: &str = "manifest.txt";

/// Why an export wrote nothing.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("not exported: archetype {0:#010X} names no asset")]
    NoAsset(u32),
    #[error("not exported: the install holds no {}", list(.0))]
    Incomplete(Vec<Edge>),
    #[error("not exported: {0} is in the entry table but would not open")]
    Unreadable(String),
    #[error("writing {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn list(edges: &[Edge]) -> String {
    (edges.iter())
        .map(|e| format!("{:?} {:?} {:#010X}", e.via, e.kind, e.name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// One file an export wrote, and every edge that put it in the set.
#[derive(Debug, Clone)]
pub struct Exported {
    pub leaf: String,
    pub logical: String,
    pub why: Vec<Edge>,
}

/// Copy every file `set` names into `out`, with a [`MANIFEST`].
///
/// ⚠ Every edge must resolve and every file open before anything is written:
/// an export missing one texture looks like it worked. The manifest is
/// written last, so a folder an I/O error cut short has none.
pub fn export(
    set: &DependencySet,
    mount: &MountedGame,
    out: &Path,
) -> Result<Vec<Exported>, ExportError> {
    if set.asset_edge().is_none() {
        return Err(ExportError::NoAsset(set.of));
    }
    let missing: Vec<Edge> = (set.edges.iter())
        .filter(|e| !e.is_resolved())
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(ExportError::Incomplete(missing));
    }

    let mut files: Vec<(Exported, Vec<u8>)> = Vec::new();
    for edge in &set.edges {
        let Some(logical) = edge.logical() else {
            continue;
        };
        if let Some((file, _)) = files.iter_mut().find(|(f, _)| f.logical == logical) {
            file.why.push(edge.clone());
            continue;
        }
        let bytes = mount
            .vfs
            .open(logical)
            .ok_or_else(|| ExportError::Unreadable(logical.to_string()))?;
        let leaf = logical.rsplit(['/', '\\']).next().unwrap_or(logical);
        files.push((
            Exported {
                leaf: leaf.to_string(),
                logical: logical.to_string(),
                why: vec![edge.clone()],
            },
            bytes,
        ));
    }

    let write = |path: PathBuf, bytes: &[u8]| {
        std::fs::write(&path, bytes).map_err(|source| ExportError::Write { path, source })
    };
    std::fs::create_dir_all(out).map_err(|source| ExportError::Write {
        path: out.to_path_buf(),
        source,
    })?;
    let mut manifest = format!(
        "# archetype {:#010X}\n# file\tedge\tkind\tname\tlogical path\twhy\n",
        set.of
    );
    for (file, bytes) in &files {
        write(out.join(&file.leaf), bytes)?;
        for edge in &file.why {
            let inside = match edge.found {
                Found::Entry(_) => " (an entry inside this file)",
                _ => "",
            };
            manifest.push_str(&format!(
                "{}\t{:?}\t{:?}\t{:#010X}\t{}\t{}{inside}\n",
                file.leaf,
                edge.via,
                edge.kind,
                edge.name,
                file.logical,
                why(edge.via)
            ));
        }
    }
    write(out.join(MANIFEST), manifest.as_bytes())?;
    Ok(files.into_iter().map(|(f, _)| f).collect())
}

/// Why an edge's file is in a set, in the words the manifest gives.
pub fn why(via: Via) -> &'static str {
    match via {
        Via::ArchetypeAsset => "the model, which holds its own skeleton where it has one",
        Via::ArchetypeTextureDictionary => "the textures the archetype names",
        Via::ArchetypeDrawableDictionary => "the drawable dictionary holding the model",
        Via::ArchetypePhysicsDictionary => "the collision",
        Via::ArchetypeClipDictionary => "the clips that animate the model",
        Via::DeclaringTypeFile => "the type file declaring the archetype",
        Via::TextureDictionaryParent => "a dictionary the textures fall through to",
        Via::ManifestHdTexture => "the high-detail textures a manifest binds to the asset",
        Via::HighDetailCompanion => "the high-detail companion, by filename convention",
        Via::ManifestMapDataGroup => "a static bound the manifest names for the map",
        Via::MapStemConvention { .. } => "a static bound named for the map, by filename convention",
        Via::ManifestInterior => "a bound the manifest names for the interior",
    }
}

/// Where a convention bound sits against its map's streaming box, in the words `depends of` prints; `None` for any other edge.
pub fn verdict(via: Via) -> Option<&'static str> {
    match via {
        Via::MapStemConvention {
            inside_streaming_box: Some(true),
        } => Some("inside the map's streaming box"),
        Via::MapStemConvention {
            inside_streaming_box: Some(false),
        } => Some("OUTSIDE the map's streaming box"),
        Via::MapStemConvention {
            inside_streaming_box: None,
        } => Some("not cross-checked, a body did not read"),
        _ => None,
    }
}
