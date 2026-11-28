//! A world file as XML, and back.
//!
//! The export is [`or_io::meta::canonical`]'s and carries no types; the import
//! takes them from the schema, lays the document out from its values and runs
//! the same structural refusal a binary save does before anything is encoded.
//! So a `.ymap` exported and imported twice is byte-identical the second
//! time, and a hand-edited one the game would misread never becomes bytes.
//!
//! ⚠ **An RBM class's `signature` word comes only from a donor.** No tool
//! derives one, so an import with no donor writes zero there; hand the file
//! being replaced to keep what the game's own parser carries.

use or_io::meta::binary::{write_meta_binary, BinaryFormat, MetaLayout};
use or_io::meta::canonical::{self, Converged, NotCanonical};
use or_io::meta::{default_library, MetaInstance, SchemaLibrary};

use crate::write::{refuse_if_broken, WriteError};

/// The XML a world file read with `layout` exports as.
pub fn export(root: &MetaInstance, layout: &MetaLayout) -> Result<String, WriteError> {
    if !crate::is_world_root(root) {
        return Err(WriteError::NotAWorldFile(root.type_name.clone()));
    }
    Ok(canonical::export(root, layout)?)
}

/// Encode an exported world file as `format`, refusing one the game would
/// misread.
pub fn import(
    xml: &[u8],
    format: BinaryFormat,
    donor: Option<&MetaLayout>,
) -> Result<Vec<u8>, WriteError> {
    import_with(default_library(), xml, format, donor)
}

/// [`import`] against a named schema library.
pub fn import_with(
    library: &SchemaLibrary,
    xml: &[u8],
    format: BinaryFormat,
    donor: Option<&MetaLayout>,
) -> Result<Vec<u8>, WriteError> {
    if let Some(why) = canonical::unavailable(library, format) {
        return Err(WriteError::NoSchema(why));
    }
    let (root, layout) = canonical::import_tree(library, xml, format, donor)?;
    if !crate::is_world_root(&root) {
        return Err(WriteError::NotAWorldFile(root.type_name));
    }
    refuse_if_broken(&root)?;
    Ok(write_meta_binary(format, Some(&layout), &root)?)
}

/// Export and import a world file twice through [`import`], and require the
/// second cycle to change nothing.
pub fn converge(bytes: &[u8]) -> Result<Converged, NotCanonical> {
    converge_with(default_library(), bytes)
}

/// [`converge`] against a named schema library.
pub fn converge_with(library: &SchemaLibrary, bytes: &[u8]) -> Result<Converged, NotCanonical> {
    canonical::converge_with(library, bytes, |xml, format| {
        import_with(library, xml, format, None)
            .map_err(|e| or_io::meta::MetaError::Other(e.to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::SchemaLibrary;

    /// ⚠ The snapshot is gitignored, so this is the state a clean checkout is
    /// in, and the refusal has to say why rather than name a missing type.
    #[test]
    fn with_no_schema_the_import_refuses_and_says_why() {
        let err = import_with(
            &SchemaLibrary::default(),
            b"<CMapData/>",
            BinaryFormat::Rbm,
            None,
        )
        .expect_err("no struct definitions");
        assert!(matches!(err, WriteError::NoSchema(_)), "{err}");
        assert!(err.to_string().contains("schema snapshot"), "{err}");
    }

    #[test]
    fn a_document_that_is_not_world_data_is_not_exported_as_one() {
        let layout = MetaLayout {
            format: BinaryFormat::Rbf,
            container: or_io::meta::binary::ContainerLayout::Rbf(Default::default()),
        };
        let err = export(&MetaInstance::new("CPedVariationInfo"), &layout).expect_err("not a map");
        assert!(matches!(err, WriteError::NotAWorldFile(_)), "{err}");
    }
}
