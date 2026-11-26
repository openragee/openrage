//! Assembling a DLC pack: the three documents, the archive, and the line
//! `dlclist.xml` needs.

use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::container::rpf::{RpfBuilder, RpfCryptoProvider, RpfEncryption};
use or_io::error::{IoError, Result};

use super::model::*;

/// Where a pack's own files live inside its archive, and what the game calls
/// each one.
#[derive(Debug, Clone)]
pub struct PackFile {
    /// Path inside `dlc.rpf`, e.g. `x64/levels/gta5/props/my_props.rpf`.
    pub archive_path: String,
    pub source: PathBuf,
    pub file_type: String,
    pub contents: String,
    pub persistent: bool,
    pub overlay: bool,
    /// Whether the pack registers the path and holds no file there: a name the game asks by, such as the `.ityp` of a `.ytyp` inside a nested archive.
    pub asked: bool,
}

impl PackFile {
    /// A file whose `fileType` its extension settles.
    ///
    /// ⚠ **Leaves `file_type` empty for an extension that settles nothing**,
    /// and [`DlcPackSpec::problems`] then refuses the pack rather than
    /// guessing. Guessing is the expensive failure here: the game ignores a
    /// registration whose type it does not expect, so the pack loads and one
    /// file quietly does not.
    pub fn new(archive_path: impl Into<String>, source: impl Into<PathBuf>) -> Self {
        let archive_path = archive_path.into();
        let file_type = file_type_for(&archive_path).unwrap_or_default().to_string();
        Self {
            persistent: file_type == "RPF_FILE",
            archive_path,
            source: source.into(),
            file_type,
            contents: String::new(),
            overlay: false,
            asked: false,
        }
    }

    /// A name the pack registers that is no file of it.
    pub fn asked(archive_path: impl Into<String>) -> Self {
        Self {
            asked: true,
            ..Self::new(archive_path, PathBuf::new())
        }
    }

    /// State what the file holds, as `CONTENTS_DLC_MAP_DATA` says of an archive of maps; an empty value states nothing.
    pub fn with_contents(mut self, contents: impl Into<String>) -> Self {
        self.contents = contents.into();
        self
    }

    pub fn with_file_type(mut self, file_type: impl Into<String>) -> Self {
        self.file_type = file_type.into();
        self.persistent = self.file_type == "RPF_FILE";
        self
    }
}

/// What the game calls a file, where its extension settles it.
///
/// ⚠ **Two extensions settle it and no others.** Over the `content.xml` of
/// every pack two retail installs ship — 184 packs, counted by
/// `what_a_shipped_dlc_pack_survives` — `.ityp` is `DLC_ITYP_REQUEST` 1,054
/// times out of 1,054 and `.rpf` is `RPF_FILE` 5,951 out of 6,015. `.meta`
/// and `.xml` have no majority spelling at all: 1,898 `.meta` entries spread
/// over types whose most common is `WEAPONINFO_FILE` at 280, and 328 `.xml`
/// entries whose most common is `OVERLAY_INFO_FILE` at 68. An extension-to-
/// type table covering those two would be invention, so this returns `None`
/// and the author states the type.
pub fn file_type_for(archive_path: &str) -> Option<&'static str> {
    let ext = archive_path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "ityp" => Some("DLC_ITYP_REQUEST"),
        "rpf" => Some("RPF_FILE"),
        _ => None,
    }
}

/// A pack as it is being authored, before it is written.
#[derive(Debug, Clone)]
pub struct DlcPackSpec {
    /// The pack's short name, e.g. `mymod`. Everything else derives from it.
    pub name: String,
    pub content_type: SetupContentType,
    pub order: i32,
    pub minor_order: i32,
    pub time_stamp: String,
    pub files: Vec<PackFile>,
}

impl DlcPackSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            content_type: SetupContentType::CompatPack,
            order: 255,
            minor_order: 0,
            time_stamp: "01/01/2024 00:00:00".to_string(),
            files: Vec::new(),
        }
    }

    pub fn device_name(&self) -> String {
        format!("dlc_{}", self.name)
    }

    /// The device a `content.xml` path is written against.
    ///
    /// ⚠ **`minorOrder` decides this and nothing else does.** The mount
    /// registers both `dlc_x:` and `dlc_xCRC:`, and a pack that writes its
    /// paths against the wrong one still mounts — it just registers files at a
    /// device the game will not resolve.
    pub fn content_device(&self) -> String {
        self.setup().content_device()
    }

    /// The `dlclist.xml` entry that makes the game load this pack.
    pub fn dlclist_entry(&self) -> String {
        format!("dlcpacks:/{}/", self.name)
    }

    /// The change set that enables everything this pack ships.
    fn change_set_name(&self) -> String {
        format!("{}_AUTOGEN", self.name.to_ascii_uppercase())
    }

    pub fn setup(&self) -> SetupData {
        SetupData {
            name_hash: self.name.clone(),
            device_name: self.device_name(),
            dat_file: "content.xml".to_string(),
            content_type: self.content_type,
            content_type_name: self.content_type.canonical_name().to_string(),
            order: self.order,
            minor_order: self.minor_order,
            is_level_pack: self.content_type == SetupContentType::LevelPack,
            sub_pack_count: 0,
            required_version: None,
            time_stamp: self.time_stamp.clone(),
            content_change_sets: Vec::new(),
            content_change_set_groups: vec![ContentChangeSetGroup {
                name_hash: "GROUP_STARTUP".to_string(),
                content_change_sets: vec![self.change_set_name().into()],
            }],
            startup_script: String::new(),
            script_callstack_size: 0,
            dependency_pack_hash: String::new(),
            platform_data_under_root: None,
        }
    }

    pub fn content(&self) -> ContentXml {
        let device = self.content_device();
        let registered: Vec<ListItem> = self
            .files
            .iter()
            .map(|f| format!("{device}:/{}", f.archive_path.trim_start_matches('/')).into())
            .collect();

        ContentXml {
            data_files: self
                .files
                .iter()
                .zip(&registered)
                .map(|(f, path)| DataFileEntry {
                    filename: path.value.clone(),
                    file_type: f.file_type.clone(),
                    overlay: f.overlay,
                    // ⚠ Shipped packs register every file disabled and switch
                    // it on from a change set. Writing `false` here loads the
                    // file before the change set runs, which is the ordering
                    // the change set exists to control.
                    disabled: true,
                    persistent: f.persistent,
                    contents: f.contents.clone(),
                    ..Default::default()
                })
                .collect(),
            disabled_files: Vec::new(),
            included_data_files: Vec::new(),
            included_xml_files: Vec::new(),
            content_change_sets: vec![ContentChangeSet {
                name: self.change_set_name(),
                files_to_enable: registered,
                ..Default::default()
            }],
            patch_files: Vec::new(),
        }
    }

    /// Everything the pack is, without touching the filesystem.
    pub fn documents(&self) -> PackDocuments {
        PackDocuments {
            setup2: self.setup().to_xml(),
            content: self.content().to_xml(),
            dlclist_entry: self.dlclist_entry(),
        }
    }

    /// Refusals a pack can earn before anything is written.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.name.trim().is_empty() {
            out.push("a pack needs a name; it becomes the folder, the device and the hash".into());
        }
        if !self
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            out.push(format!(
                "pack name `{}` is not [A-Za-z0-9_]; it is a device name and a folder name",
                self.name
            ));
        }
        if self.files.is_empty() {
            out.push("a pack with no files mounts and registers nothing".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for f in &self.files {
            let key = f.archive_path.to_ascii_lowercase();
            if !seen.insert(key) {
                out.push(format!("`{}` is added twice", f.archive_path));
            }
            if !f.asked && !f.source.is_file() {
                out.push(format!("`{}` is not a file", f.source.display()));
            }
            if f.file_type.trim().is_empty() {
                out.push(format!(
                    "`{}` needs a fileType; its extension does not settle one, and the \
                     game ignores a registration whose type it does not expect",
                    f.archive_path
                ));
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackDocuments {
    pub setup2: String,
    pub content: String,
    pub dlclist_entry: String,
}

/// Build `dlc.rpf` into `out_path`.
///
/// ⚠ **`out_path` is the archive, not the install.** Nothing here writes into
/// a game folder: a pack is built somewhere the author chose and installed by
/// a separate, reversible step.
pub fn build_pack(
    spec: &DlcPackSpec,
    out_path: &Path,
    encryption: RpfEncryption,
    crypto: Arc<dyn RpfCryptoProvider>,
) -> Result<PackDocuments> {
    build_pack_with(spec, out_path, encryption, crypto, &mut |_| true)
}

/// [`build_pack`], telling `go_on` the bytes written before each write; a `false` stops the build.
///
/// ⚠ A build that stops or fails once the archive is begun removes it: a
/// `dlc.rpf` cut short still mounts, as a pack missing what it registers.
pub fn build_pack_with(
    spec: &DlcPackSpec,
    out_path: &Path,
    encryption: RpfEncryption,
    crypto: Arc<dyn RpfCryptoProvider>,
    go_on: &mut dyn FnMut(u64) -> bool,
) -> Result<PackDocuments> {
    let problems = spec.problems();
    if !problems.is_empty() {
        return Err(IoError::InvalidData {
            context: format!("this pack would not load:\n  {}", problems.join("\n  ")),
        });
    }

    let docs = spec.documents();
    let mut builder = RpfBuilder::default().encrypted(encryption, crypto);
    for file in spec.files.iter().filter(|file| !file.asked) {
        builder.insert_file(&file.archive_path, &file.source)?;
    }
    // ⚠ Last: a path added twice is replaced, and a pack file named `setup2.xml`
    // must not replace the document the game reads the pack from.
    builder.insert_with("setup2.xml", docs.setup2.clone().into_bytes(), false)?;
    builder.insert_with("content.xml", docs.content.clone().into_bytes(), false)?;

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| IoError::FileWriteFailed {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let file = std::fs::File::create(out_path).map_err(|e| IoError::FileWriteFailed {
        path: out_path.to_path_buf(),
        source: e,
    })?;
    let mut writer = Watched {
        inner: std::io::BufWriter::new(file),
        written: 0,
        go_on,
    };
    let saved = builder.write_to(&mut writer);
    drop(writer);
    if let Err(e) = saved {
        let _ = std::fs::remove_file(out_path);
        return Err(e);
    }

    Ok(docs)
}

/// A writer that asks `go_on` before each write, with the bytes written so far.
struct Watched<'a, W> {
    inner: W,
    written: u64,
    go_on: &'a mut dyn FnMut(u64) -> bool,
}

impl<W: Write> Write for Watched<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // ⚠ Not `Interrupted`, which `write_all` retries rather than returns.
        if !(self.go_on)(self.written) {
            return Err(std::io::Error::other("the build was stopped"));
        }
        let n = self.inner.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl<W: Seek> Seek for Watched<'_, W> {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(to)
    }
}
