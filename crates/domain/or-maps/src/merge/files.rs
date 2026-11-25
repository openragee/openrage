//! The merge over files: three files of one kind, and another copy merged into a metadata source.

use super::*;

fn read<T: File>(which: &'static str, root: &'static str, bytes: &[u8]) -> Result<T, Error> {
    T::from_bytes(bytes).map_err(|why| Error::Read {
        which,
        root,
        why: why.to_string(),
    })
}

/// Three files of one kind, `extension` saying which (`ymap` or `ytyp`), merged, and the merged file as it saves.
pub fn merge_files(
    extension: &str,
    base: &[u8],
    ours: &[u8],
    theirs: &[u8],
    policy: &mut dyn FnMut(&Conflict) -> Take,
) -> Result<Merge<Vec<u8>>, Error> {
    fn saved<T: File>(merge: Merge<T>) -> Result<Merge<Vec<u8>>, Error> {
        let merged = match &merge.merged {
            Some(root) => Some(
                root.to_bytes()
                    .map_err(|why| Error::Invalid(why.to_string()))?,
            ),
            None => None,
        };
        Ok(Merge {
            merged,
            report: merge.report,
        })
    }
    match extension {
        CMapData::EXTENSION => {
            let root = "map file";
            saved(merge_maps(
                &read("the base", root, base)?,
                &read("ours", root, ours)?,
                &read("theirs", root, theirs)?,
                policy,
            )?)
        }
        CMapTypes::EXTENSION => {
            let root = "type file";
            saved(merge_types(
                &read("the base", root, base)?,
                &read("ours", root, ours)?,
                &read("theirs", root, theirs)?,
                policy,
            )?)
        }
        other => Err(Error::Kind(other.to_string())),
    }
}

/// What merging another copy into a source did.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceMerge {
    pub report: Report,
    /// The XML of the source as it was and as the merge left it; none when a conflict stopped the merge, and the source is then as it was.
    pub edited: Option<Edited>,
}

/// Merge `theirs`, another copy of the file the metadata source in `dir` cooks to, into that source: the base is the file the source keeps, ours is what it cooks to, and the merged file is written as its XML.
pub fn merge_source(
    dir: &Path,
    theirs: &[u8],
    policy: &mut dyn FnMut(&Conflict) -> Take,
) -> Result<SourceMerge, Error> {
    let said = |problems: Vec<or_source::Problem>| {
        let said: Vec<String> = problems.iter().map(ToString::to_string).collect();
        said.join("; ")
    };
    let source = or_source::asset::Source::open(dir).map_err(|p| Error::Source(p.to_string()))?;
    let file = or_source::metadata::cooked_name(&source.manifest)
        .map_err(|p| Error::Source(p.to_string()))?;
    let extension = file.rsplit_once('.').map_or("", |(_, e)| e);
    if !matches!(extension, CMapData::EXTENSION | CMapTypes::EXTENSION) {
        return Err(Error::Kind(extension.to_string()));
    }
    let ours = or_source::metadata::compile(&source)
        .map_err(|p| Error::Source(format!("the source does not cook: {}", said(p))))?;
    let base = or_source::metadata::compile_base(&source)
        .map_err(|p| Error::Source(format!("the base of the source does not cook: {}", said(p))))?;
    let merge = merge_files(extension, &base.bytes, &ours.bytes, theirs, policy)?;
    let edited = match &merge.merged {
        Some(bytes) => {
            let replaced =
                or_source::metadata::replace(dir, bytes).map_err(|p| Error::Write(said(p)))?;
            Some(Edited {
                file: replaced.file,
                before: replaced.before,
                after: replaced.after,
            })
        }
        None => None,
    };
    Ok(SourceMerge {
        report: merge.report,
        edited,
    })
}
