//! A flat loose drawable opens only through a byte-identical, named table entry.

use std::path::Path;

use or_io::gta6::headerless::raw_drawable_hint;
use or_job::Cancel;

use super::document::rpf9::entry::EntrySource;
use super::rpf9_texture_index::Tables;
use super::DocPayload;

pub(super) struct Opened {
    pub payload: DocPayload,
    pub source: EntrySource,
}

pub(super) fn load(
    path: &Path,
    scope: super::rpf9_texture_roots::Scope,
    provider: std::sync::Arc<dyn or_io::container::RpfCryptoProvider>,
) -> Result<super::LoadResult, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(error)?;
    let limit = 1 << 30;
    if file.metadata().map_err(error)?.len() > limit {
        return Err(error("the loose drawable exceeds its byte budget"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() as u64 > limit {
        return Err(error("the loose drawable exceeds its byte budget"));
    }
    if raw_drawable_hint(&bytes).map_err(error)?.is_some() {
        let cancel = Cancel::new();
        let tables = Tables::open(scope.roots(&cancel), &cancel, &or_job::Progress::new())
            .ok_or_else(|| tr!("Cancelled").to_string())?;
        let opened = open(path, &bytes, &tables, &cancel)?
            .ok_or_else(|| error("the loose drawable lost its GTA VI signature"))?;
        return Ok(super::LoadResult::Rpf9Loose {
            path: path.to_path_buf(),
            payload: opened.payload,
            source: opened.source,
        });
    }
    let (payload, resource) =
        super::document_io::parse_payload_from_path(path, super::DocumentKind::Ydr, provider)?;
    Ok(super::LoadResult::Success {
        path: path.to_path_buf(),
        payload,
        resource,
        binding: None,
    })
}

pub(super) fn open(
    path: &Path,
    bytes: &[u8],
    tables: &Tables,
    cancel: &Cancel,
) -> Result<Option<Opened>, String> {
    let Some(hint) = raw_drawable_hint(bytes).map_err(error)? else {
        return Ok(None);
    };
    let candidates = tables.entries_named(hint.name64, "ydr");
    if candidates.len() > 64 {
        return Err(error("too many table entries name this loose drawable"));
    }
    let mut compared = 0u64;
    let mut matched: Option<(EntrySource, Vec<u8>)> = None;
    let mut read_error = None;
    for source in candidates {
        if cancel.is_set() {
            return Err(tr!("Cancelled").to_string());
        }
        if source.decoded_len() != bytes.len() as u64 {
            continue;
        }
        compared = compared
            .checked_add(source.decoded_len().max(source.stored_len()))
            .filter(|total| *total <= 1 << 30)
            .ok_or_else(|| error("the loose drawable comparison exceeds its byte budget"))?;
        let candidate = match source.bytes_limited(1 << 30) {
            Ok(candidate) => candidate,
            Err(error) => {
                read_error.get_or_insert(error);
                continue;
            }
        };
        if candidate != bytes {
            continue;
        }
        let sections = source.section_lengths().ok_or_else(|| {
            error("the matching table entry does not state its resource section sizes")
        })?;
        if let Some((previous, _)) = &matched {
            if previous.section_lengths() != Some(sections) {
                return Err(error(
                    "matching table entries disagree on the resource section sizes",
                ));
            }
        } else {
            matched = Some((source, candidate));
        }
    }
    if cancel.is_set() {
        return Err(tr!("Cancelled").to_string());
    }
    let Some((source, candidate)) = matched else {
        return Err(read_error.unwrap_or_else(|| {
            error("this GTA VI drawable needs a byte-identical entry in a mounted or configured RPF9 archive to recover its section sizes")
        }));
    };
    let payload = source.payload_from_bytes_at(path, candidate)?;
    Ok(Some(Opened { payload, source }))
}

fn error(error: impl std::fmt::Display) -> String {
    tr!(
        "GTA VI's headerless resource reader does not read this body: {e}",
        e = error
    )
}

#[cfg(test)]
#[path = "rpf9_loose_tests.rs"]
mod tests;
