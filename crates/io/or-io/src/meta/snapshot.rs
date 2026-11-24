//! The default schema library, which the host installs from a snapshot at
//! run time, and what happens without one.
//!
//! The snapshot is a binary index of the metadata schema: which structs exist,
//! what fields they have, and what those fields are called. It is what turns
//! `0x2C7A9E31` into `entityFlags` in the inspector.
//!
//! It is derived from a PSC corpus that is not ours to redistribute, so it is
//! not committed, a clean clone has none, and no build compiles one in: the
//! editor and the CLI find it through `or_title::schema` and install it here.
//! Metadata still reads, writes and round-trips byte-for-byte without it,
//! because the container carries its own type table; what is lost is the
//! names, which show as hashes except in the world metadata family
//! [`crate::meta::binary::public_names`] names. An installed snapshot wins wherever
//! both answer. [`has_default_library`] reports which situation you are in.
//!
//! ⚠ "The container carries its own type table" is a claim about the *write*
//! path too, and it was false until a suite ran with no snapshot staged: a
//! snapshot-less build saved `AT_FRG` as its own name hash and RBM narrowed
//! that to a byte, in 3,176 of 6,072 real fixtures.
//! [`crate::meta::binary::enum_text`] spells an unresolved name and an unlisted value
//! differently and inverts through the file's own enum table.
//!
//! See `crates/io/or-io/src/meta/README.md` for how to produce one.

use std::path::{Path, PathBuf};
use std::sync::{LazyLock, OnceLock};

use crate::meta::error::{MetaError, Result};
use crate::meta::schema::SchemaLibrary;
use crate::meta::snapshot_discovery::{fingerprint_of, stamp_path, GENERATOR_SOURCE_DIR};

struct Installed {
    library: SchemaLibrary,
    origin: Option<PathBuf>,
}

/// ⚠ Set once: every cache keyed on the default (the hash index, the shipped
/// definitions) would otherwise answer for a library that is gone.
static INSTALLED: OnceLock<Installed> = OnceLock::new();

/// A second library, read for its names alone: another title's, whose structures may be named as
/// the default's are and laid out otherwise.
static NAMES: OnceLock<SchemaLibrary> = OnceLock::new();

static EMPTY: LazyLock<SchemaLibrary> = LazyLock::new(SchemaLibrary::default);

/// Whether a snapshot came out of the `build_library` this source tree has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotAge {
    Current,
    /// Written before the stamp existed, so it cannot say.
    Unstamped,
    Stale {
        recorded: String,
        builder: String,
    },
    /// Not running from a source tree, so there is no generator to compare.
    Unjudged,
}

impl SnapshotAge {
    /// What to tell a reader, or nothing when there is nothing to tell.
    pub fn concern(&self) -> Option<String> {
        match self {
            SnapshotAge::Current | SnapshotAge::Unjudged => None,
            SnapshotAge::Unstamped => Some(format!(
                "the schema snapshot carries no generator stamp, so whether it matches this \
                 tree's schema/ ({}) is unknown. `just snapshot <psc-root>` settles it.",
                builder_stamp().unwrap_or("?")
            )),
            SnapshotAge::Stale { recorded, builder } => Some(format!(
                "the schema snapshot was built by schema/ {recorded} and this tree's is \
                 {builder}. It is still in use, so anything it decides is the older \
                 generator's answer. Rebuild it with `just snapshot <psc-root>`."
            )),
        }
    }
}

/// The digest of this source tree's `src/meta/schema`, or `None` outside a source tree.
///
/// ⚠ Read at run time because nothing is compiled in: a digest of the
/// sources on disk, so a changed parser marks every older snapshot stale
/// without a version being raised by hand.
pub fn builder_stamp() -> Option<&'static str> {
    static STAMP: LazyLock<Option<String>> = LazyLock::new(|| {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .join(GENERATOR_SOURCE_DIR)
            .is_dir()
            .then(|| fingerprint_of(manifest_dir))
    });
    STAMP.as_deref()
}

/// Whether the snapshot at `snapshot` was built by this tree's `src/meta/schema`.
///
/// ⚠ A stale snapshot gives a plausible answer, not an error: nothing in the
/// file records which `build_library` wrote it, so the stamp beside it is the
/// only witness.
pub fn snapshot_age(snapshot: &Path) -> SnapshotAge {
    let Some(builder) = builder_stamp() else {
        return SnapshotAge::Unjudged;
    };
    let recorded = std::fs::read_to_string(stamp_path(snapshot)).unwrap_or_default();
    match recorded.trim() {
        "" => SnapshotAge::Unstamped,
        r if r == builder => SnapshotAge::Current,
        r => SnapshotAge::Stale {
            recorded: r.to_string(),
            builder: builder.to_string(),
        },
    }
}

/// Read and decode the snapshot at `path`.
pub fn load_snapshot(path: &Path) -> Result<SchemaLibrary> {
    let bytes = std::fs::read(path)?;
    let mut library = decode(&bytes).map_err(|e| {
        MetaError::Snapshot(format!(
            "{} does not decode as format {}: {e}. A snapshot written by a tree with \
             another type model lives under another format directory",
            path.display(),
            crate::meta::snapshot_discovery::SNAPSHOT_FORMAT
        ))
    })?;
    library.apply_field_hints();
    Ok(library)
}

/// Install `library` as the default; the first install in a process wins.
pub fn install_default_library(library: SchemaLibrary) -> &'static SchemaLibrary {
    &INSTALLED
        .get_or_init(|| Installed {
            library,
            origin: None,
        })
        .library
}

/// Install the snapshot at `path` as the default, unless one is installed.
pub fn install_snapshot(path: &Path) -> Result<&'static SchemaLibrary> {
    if let Some(installed) = INSTALLED.get() {
        return Ok(&installed.library);
    }
    let library = load_snapshot(path)?;
    if let Some(concern) = snapshot_age(path).concern() {
        tracing::warn!("{concern}");
    }
    let installed = Installed {
        library,
        origin: Some(path.to_path_buf()),
    };
    Ok(&INSTALLED.get_or_init(|| installed).library)
}

/// Install the snapshot at `path` for its names alone, behind the default's; the first wins.
///
/// ⚠ Before the first name is asked for: the hash index is built once.
pub fn install_names_snapshot(path: &Path) -> Result<&'static SchemaLibrary> {
    if let Some(held) = NAMES.get() {
        return Ok(held);
    }
    let library = load_snapshot(path)?;
    Ok(NAMES.get_or_init(|| library))
}

pub(crate) fn names_library() -> Option<&'static SchemaLibrary> {
    NAMES.get()
}

pub(crate) fn installed() -> Option<&'static SchemaLibrary> {
    INSTALLED.get().map(|i| &i.library)
}

/// The installed default, or an empty library when the host installed none.
pub fn default_library() -> &'static SchemaLibrary {
    installed().unwrap_or(&EMPTY)
}

pub fn has_default_library() -> bool {
    INSTALLED.get().is_some()
}

/// The file the default was installed from, so a mislocated snapshot can be
/// told from a missing one.
pub fn default_origin() -> Option<&'static Path> {
    INSTALLED.get().and_then(|i| i.origin.as_deref())
}

pub fn decode(bytes: &[u8]) -> Result<SchemaLibrary> {
    bincode::deserialize::<SchemaLibrary>(bytes).map_err(|e| MetaError::Snapshot(e.to_string()))
}

pub fn encode(lib: &SchemaLibrary) -> Result<Vec<u8>> {
    bincode::serialize(lib).map_err(|e| MetaError::Snapshot(e.to_string()))
}

#[cfg(test)]
mod tests {
    use crate::meta::schema::*;
    use crate::meta::snapshot_discovery::SNAPSHOT_FORMAT;

    /// ⚠ **Exhaustive on purpose, and never with `..`.** A variant or a field
    /// added to anything a snapshot serialises stops this compiling, and the
    /// fix is to list it *and* raise [`SNAPSHOT_FORMAT`] — otherwise the new
    /// generator writes a file every older lane reads from the same place
    /// and cannot decode.
    #[test]
    fn the_snapshot_format_names_every_shape() {
        fn field_type(t: &FieldType) {
            match t {
                FieldType::Bool
                | FieldType::U8
                | FieldType::U16
                | FieldType::U32
                | FieldType::U64
                | FieldType::S8
                | FieldType::S16
                | FieldType::S32
                | FieldType::S64
                | FieldType::Float
                | FieldType::Float16
                | FieldType::Double
                | FieldType::Vector2
                | FieldType::Vector3
                | FieldType::Vector4
                | FieldType::Vec2V
                | FieldType::Vec3V
                | FieldType::Vec4V
                | FieldType::Mat33V
                | FieldType::Mat34V
                | FieldType::Mat44V
                | FieldType::Matrix33
                | FieldType::Matrix34
                | FieldType::Matrix44
                | FieldType::Color32
                | FieldType::Enum(_)
                | FieldType::Struct(_)
                | FieldType::Pad(_)
                | FieldType::Unknown(_) => {}
                FieldType::String(kind) => match kind {
                    StringKind::Plain
                    | StringKind::AtHashString
                    | StringKind::AtHashValue
                    | StringKind::AtString
                    | StringKind::AtFinalHashString => {}
                },
                FieldType::Bitset { storage, values: _ } => match storage {
                    BitsetStorage::Fixed8
                    | BitsetStorage::Fixed16
                    | BitsetStorage::Fixed32
                    | BitsetStorage::Fixed64
                    | BitsetStorage::Variable => {}
                },
                FieldType::Pointer { base: _, policy } => match policy {
                    PointerPolicy::Owner
                    | PointerPolicy::Reference
                    | PointerPolicy::ExternalNamed => {}
                },
                FieldType::Array { elem, kind } => {
                    field_type(elem);
                    match kind {
                        ArrayKind::AtArray
                        | ArrayKind::AtFixedArray(_)
                        | ArrayKind::AtPtrArray
                        | ArrayKind::Member(_)
                        | ArrayKind::Pointer {
                            size_var: _,
                            align: _,
                        } => {}
                    }
                }
                FieldType::Map { key, value } => {
                    field_type(key);
                    field_type(value);
                }
            }
        }
        fn field(f: &Field) {
            let Field {
                name: _,
                ty,
                init,
                min: _,
                max: _,
                step: _,
                ui_key: _,
                hide_widgets: _,
                description: _,
                hint,
            } = f;
            field_type(ty);
            match init {
                None
                | Some(
                    Literal::Bool(_)
                    | Literal::Int(_)
                    | Literal::Float(_)
                    | Literal::Str(_)
                    | Literal::Enum(_)
                    | Literal::Other(_),
                ) => {}
            }
            match hint {
                FieldHint::None
                | FieldHint::ModelName
                | FieldHint::TextureName
                | FieldHint::ClipSetName
                | FieldHint::EffectRuleName
                | FieldHint::SoundName
                | FieldHint::ScenarioName
                | FieldHint::BoneTag
                | FieldHint::VehicleMod => {}
            }
        }
        let SchemaLibrary { structs, enums } = SchemaLibrary::default();
        for s in structs.values() {
            let StructDef {
                name: _,
                base: _,
                abstract_: _,
                simple: _,
                fields,
                source_path: _,
            } = s;
            fields.iter().for_each(field);
        }
        for e in enums.values() {
            let EnumDef {
                name: _,
                values,
                generate_bitset: _,
                source_path: _,
            } = e;
            for EnumValue {
                name: _,
                description: _,
                value: _,
            } in values
            {}
        }
        assert_eq!(
            SNAPSHOT_FORMAT, "v3",
            "the listing above is the v3 type model; a change to it is a new format"
        );
    }
}
