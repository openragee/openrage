//! What a DLC pack says about itself: `dlclist.xml`, `setup2.xml`,
//! `content.xml`, and the title-update mount table — read, written and
//! assembled.

mod mode;
mod model;
mod pack;
mod read;
pub(crate) mod tree;
mod write;
pub(crate) mod xml;

pub use mode::{ChangeSetsInMode, MapMode};
pub use model::{
    ContentChangeSet, ContentChangeSetDef, ContentChangeSetGroup, ContentDef, ContentXml,
    DataFileEntry, DlcList, ExecutionConditions, ExtraTitleUpdateData, IncludedChangeset, ListItem,
    MapChangeSet, RegisteredFile, SetupContentType, SetupData, TitleUpdateMount,
};
pub use pack::{build_pack, build_pack_with, file_type_for, DlcPackSpec, PackDocuments, PackFile};
pub use xml::Result;

#[cfg(test)]
mod tests;
