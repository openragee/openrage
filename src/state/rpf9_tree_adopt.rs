//! A GTA VI tree whose Save as… wrote a copy becomes that copy: each row reads the copy's entry of
//! its path, every archive opened in it is read from the copy, and no row or archive is marked edited.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::RpfEntry;
use or_io::container::{RpfArchive, RpfCryptoProvider};
use or_io::gta6::hash::name_hash64;

use crate::state::document::{ArchiveId, RpfNodeId};
use crate::state::rpf_node::{ArchiveSource, RpfNodeBacking, RpfSessionNodeKind};
use crate::state::rpf_session::RpfEditSession;
use crate::state::{DocPayload, Document};

/// A written copy read back on the save's worker: its top archive, and each nested archive the
/// tree had opened, keyed by its chain of archive-relative paths.
pub(super) struct Reopened {
    top: Arc<RpfArchive>,
    nested: HashMap<Vec<String>, Arc<RpfArchive>>,
}

/// The entries of one archive of a copy, by path and, for a name its table holds only as a hash,
/// by the hash of the path the writer was given.
struct Index {
    paths: HashMap<String, u32>,
    hashes: HashMap<u64, u32>,
}

impl Index {
    fn of(written: &RpfArchive) -> Self {
        let paths = (written.nodes.iter())
            .map(|n| (n.path().to_ascii_lowercase(), n.entry_index()))
            .collect();
        let hashes = (written.nodes.iter())
            .filter_map(|n| Some((written.rpf9_row(n.entry_index())?.name64, n.entry_index())))
            .collect();
        Self { paths, hashes }
    }

    fn find(&self, path: &str) -> Option<u32> {
        (self.paths.get(&path.to_ascii_lowercase()))
            .or_else(|| self.hashes.get(&name_hash64(path.as_bytes())))
            .copied()
    }
}

/// The chain of archive-relative paths of every nested archive the tree `session` has opened,
/// each after the archive it is in.
pub(super) fn listed(session: &RpfEditSession) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    list_under(session, session.root_id(), &mut Vec::new(), &mut out);
    out
}

fn list_under(
    session: &RpfEditSession,
    root: RpfNodeId,
    chain: &mut Vec<String>,
    out: &mut Vec<Vec<String>>,
) {
    let prefix = prefix_of(session, root);
    let mut stack = session.children_of(root);
    while let Some(id) = stack.pop() {
        let Some(node) = session.node(id) else {
            continue;
        };
        if let RpfNodeBacking::Archive {
            archive_id: Some(_),
            source: ArchiveSource::Nested { .. },
            ..
        } = &node.backing
        {
            chain.push(relative(session, id, &prefix));
            out.push(chain.clone());
            list_under(session, id, chain, out);
            chain.pop();
            continue;
        }
        stack.extend(node.children.iter().copied());
    }
}

/// The copy at `out` read as an archive with the nested archives `listed` names, on the save's
/// worker; `None`, logged, where the copy does not open.
pub(super) fn reopened(
    out: &Path,
    crypto: Arc<dyn RpfCryptoProvider>,
    listed: &[Vec<String>],
) -> Option<Reopened> {
    let top = match RpfArchive::open_with(out, crypto) {
        Ok(copy) => Arc::new(copy),
        Err(error) => {
            let path = out.display();
            tracing::warn!(%path, %error, "a GTA VI copy does not reopen, so its tree keeps its source");
            return None;
        }
    };
    let mut nested: HashMap<Vec<String>, Arc<RpfArchive>> = HashMap::new();
    let mut indexes: HashMap<Vec<String>, Index> = HashMap::new();
    for chain in listed {
        let Some((last, parent)) = chain.split_last() else {
            continue;
        };
        let archive = match parent {
            [] => Arc::clone(&top),
            _ => match nested.get(parent) {
                Some(archive) => Arc::clone(archive),
                None => continue,
            },
        };
        let index = (indexes.entry(parent.to_vec())).or_insert_with(|| Index::of(&archive));
        // One that does not open here is opened by the adopt, which names it if it fails there too.
        if let Some(Ok(inner)) = index.find(last).map(|entry| archive.open_nested(entry)) {
            nested.insert(chain.clone(), Arc::new(inner));
        }
    }
    Some(Reopened { top, nested })
}

/// Make the tree `doc` the copy `written` at `out`, named for it; false, the tree left as it was,
/// where the copy holds no entry of a row's path.
pub(super) fn become_copy(doc: &mut Document, written: Reopened, out: &Path) -> bool {
    let DocPayload::Rpf(rpf) = &mut doc.payload else {
        return false;
    };
    // Adopted on a clone, so a row the copy lacks leaves the tree reading its source, edits and all.
    let mut session = rpf.session.clone();
    let root = session.root_id();
    let Some(RpfNodeBacking::ArchiveDir { archive_id }) =
        session.node(root).map(|n| n.backing.clone())
    else {
        return false;
    };
    let top = Arc::clone(&written.top);
    if let Err(why) = adopt(
        &mut session,
        root,
        archive_id,
        top,
        &mut Vec::new(),
        &written,
    ) {
        tracing::warn!(%why, "a GTA VI tree keeps reading the archive it was opened from");
        return false;
    }
    if let Some(slot) = session.archives.get_mut(&archive_id) {
        slot.fs_path = Some(out.to_path_buf());
    }
    rpf.session = session;
    doc.path = Some(out.to_path_buf());
    if let Some(stem) = out.file_stem().and_then(|s| s.to_str()) {
        doc.name = stem.to_owned();
    }
    true
}

fn prefix_of(session: &RpfEditSession, root: RpfNodeId) -> String {
    (session.path_of(root))
        .filter(|p| !p.is_empty())
        .map(|p| p + "/")
        .unwrap_or_default()
}

fn relative(session: &RpfEditSession, id: RpfNodeId, prefix: &str) -> String {
    let full = session.path_of(id).unwrap_or_default();
    full.strip_prefix(prefix).unwrap_or(&full).to_owned()
}

/// Point every row under `root` at its entry of the same path in `written`, which the archive
/// `archive_id` now reads, each archive opened in it reopened from the copy.
fn adopt(
    session: &mut RpfEditSession,
    root: RpfNodeId,
    archive_id: ArchiveId,
    written: Arc<RpfArchive>,
    chain: &mut Vec<String>,
    copy: &Reopened,
) -> Result<(), String> {
    let index = Index::of(&written);
    let prefix = prefix_of(session, root);
    if let Some(slot) = session.archives.get_mut(&archive_id) {
        slot.archive = Arc::clone(&written);
        slot.dirty = false;
    }
    let mut stack = session.children_of(root);
    while let Some(id) = stack.pop() {
        let path = relative(session, id, &prefix);
        let entry = (index.find(&path)).ok_or_else(|| format!("{path} is not in the copy"));
        let Some(node) = session.nodes.get_mut(&id) else {
            continue;
        };
        match &mut node.backing {
            RpfNodeBacking::Archive {
                archive_id: listed,
                source:
                    ArchiveSource::Nested {
                        parent_archive_id,
                        parent_entry_index,
                        opened,
                    },
                ..
            } => {
                let entry = entry?;
                *parent_archive_id = archive_id;
                *parent_entry_index = entry;
                *opened = None;
                if let Some(nested) = *listed {
                    chain.push(path.clone());
                    let inner = match copy.nested.get(chain.as_slice()) {
                        Some(inner) => Arc::clone(inner),
                        // ponytail: an archive opened after the save began is read here, on the
                        // thread that draws; the worker reads every one listed when it began.
                        None => Arc::new(
                            (written.open_nested(entry)).map_err(|e| format!("{path}: {e}"))?,
                        ),
                    };
                    adopt(session, id, nested, inner, chain, copy)?;
                    chain.pop();
                }
                continue;
            }
            RpfNodeBacking::ArchiveFile { .. } | RpfNodeBacking::InMemoryFile { .. } => {
                let entry = entry?;
                node.kind = match written.entries.get(entry as usize) {
                    Some(RpfEntry::Resource(_)) => RpfSessionNodeKind::ResourceFile,
                    _ => RpfSessionNodeKind::BinaryFile,
                };
                node.backing = RpfNodeBacking::ArchiveFile {
                    archive_id,
                    entry_index: entry,
                    override_bytes: None,
                };
            }
            RpfNodeBacking::ArchiveDir { archive_id: dir } => *dir = archive_id,
            RpfNodeBacking::InMemoryDir => node.backing = RpfNodeBacking::ArchiveDir { archive_id },
            _ => {}
        }
        stack.extend(node.children.iter().copied());
    }
    Ok(())
}
