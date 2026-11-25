use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::dlc::{ContentXml, DataFileEntry, DlcList, ExtraTitleUpdateData, ListItem, SetupData};
use crate::game::Game;
use crate::install::materials::GtaInstall;
use or_io::{Diagnostic, Diagnostics};

use or_io::container::rpf::crypto::RpfCryptoProvider;
use or_io::container::rpf::{RpfArchive, RpfEntry, RpfEntryKind};

use super::archive_device::ArchiveDevice;
use super::install::VfsBuildError;
use super::keys;
use super::mount::Vfs;
use super::overlay::{overlay_root, resolve_archive, ArchiveOrigin, MountOptions};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DlcCutoff {
    All,
    BaseGame,
    NameHash(String),
}

/// The size and modification time of a file on disk, which tell a changed archive from the one a mount opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    /// Bytes.
    pub size: u64,
    /// Nanoseconds since the Unix epoch.
    pub modified: u64,
}

impl FileStamp {
    /// The stamp of the file at `path`; `None` where there is no such file or it states no time.
    pub fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta.modified().ok()?;
        let modified = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
        meta.is_file().then_some(Self {
            size: meta.len(),
            // Nanoseconds, which a u64 holds for some centuries yet.
            modified: modified.as_nanos() as u64,
        })
    }
}

pub struct MountRoot {
    pub device: String,
    pub archive: Arc<RpfArchive>,
    pub container: String,
    /// Which copy of this archive was opened.
    ///
    /// ⚠ A partial overlay is the normal state, so this is a fact about each
    /// archive, recorded here rather than inferred from the container string.
    pub origin: ArchiveOrigin,
    /// The file on disk that holds this archive, for a nested one the file its outermost archive is.
    ///
    /// ⚠ Taken before the archive is opened: a file written after its mount was made has another
    /// stamp than the mount, so what is read through an old mount is never kept under the new file.
    pub stamp: Option<FileStamp>,
}

pub struct MountedPack {
    pub name_hash: String,
    pub device_name: String,
    pub setup: SetupData,
    pub data_files: Vec<DataFileEntry>,
    pub source_path: PathBuf,
}

pub struct MountedGame {
    pub vfs: Vfs,
    pub packs: Vec<MountedPack>,
    pub roots: Vec<MountRoot>,
    pub diagnostics: Diagnostics,
    pub game: Game,
}

/// What an archive's entry table already knows about a file.
///
/// Opening each file for its version costs: `openrage-cli versions --game`
/// reads 289,631 resource headers of a retail Legacy install in **370.9 s**.
/// A resource entry carries `system_info` and `graphics_info`, and the RSC7
/// version is the high nibble of each — the same arithmetic
/// `RpfArchive::read_prefix` uses to *rebuild* the header it hands
/// back — so the version is free at enumeration time;
/// `the_cheap_version_is_the_version_in_the_body` holds that the two agree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryFacts {
    /// `device:/path/inside.rpf/leaf.ydr`.
    pub logical: String,
    /// The chain of archives holding it.
    pub container: String,
    pub leaf: String,
    pub kind: RpfEntryKind,
    /// Bytes the file occupies in the archive.
    pub stored_size: u32,
    /// Bytes after inflating, for a binary entry. Equal to `stored_size` for a
    /// resource, which is never deflated.
    pub size: u32,
    pub compressed: bool,
    /// The resource's paging words, or `None` for a binary entry.
    pub paging: Option<(u32, u32)>,
}

impl EntryFacts {
    /// The RSC7 version, from the entry table alone.
    ///
    /// ⚠ `None` means *this is not a resource*, never *unknown*. A binary entry
    /// has no RSC7 header to have a version from, and returning `Some(0)` would
    /// make a `.dat` look like a resource of version zero.
    pub fn resource_version(&self) -> Option<u32> {
        let (system_info, graphics_info) = self.paging?;
        Some(((system_info >> 28) & 0xF) << 4 | ((graphics_info >> 28) & 0xF))
    }
}

impl MountedGame {
    /// What the mount holds: the tables of contents of its root archives and their names.
    ///
    /// ⚠ The nested archives its devices keep open, sixteen to a device, are not counted.
    pub fn held_bytes(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        let roots = self.roots.capacity() * std::mem::size_of::<MountRoot>();
        let held = |root: &MountRoot| {
            let table = seen
                .insert(Arc::as_ptr(&root.archive))
                .then(|| root.archive.table_bytes());
            table.unwrap_or(0) + root.device.capacity() + root.container.capacity()
        };
        roots + self.roots.iter().map(held).sum::<usize>()
    }

    pub fn for_each_file<F: FnMut(&str, &str, &str)>(&self, mut visit: F) {
        self.for_each_entry(|facts| visit(&facts.logical, &facts.container, &facts.leaf));
    }

    /// Every file in the mount, with what its entry table says about it.
    ///
    /// ⚠ **Descends nested archives, which is where almost everything is.**
    /// Measured over both retail installs: 9,801 of 10,142 archives — **96%** —
    /// are nested, so an index that stopped at the top level would describe 4% of
    /// an install.
    pub fn for_each_entry<F: FnMut(&EntryFacts)>(&self, mut visit: F) {
        for root in &self.roots {
            root.for_each_entry(&mut visit);
        }
    }
}

impl MountRoot {
    /// Every file of this root archive, its nested archives descended, as [`MountedGame::for_each_entry`] visits them.
    pub fn for_each_entry<F: FnMut(&EntryFacts)>(&self, mut visit: F) {
        walk_archive(
            &self.archive,
            &self.device,
            "",
            &self.container,
            0,
            &mut visit,
        );
    }
}

const MAX_NEST_DEPTH: u32 = 8;

/// One entry's facts, read from the table alone: no body read, no decryption,
/// no inflate.
pub(super) fn facts_for(
    archive: &RpfArchive,
    entry_index: u32,
    logical: String,
    container: &str,
    leaf: &str,
) -> Option<EntryFacts> {
    let entry = archive.entries.get(entry_index as usize)?;
    let (kind, stored_size, size, compressed, paging) = match entry {
        RpfEntry::Binary(binary) => (
            RpfEntryKind::Binary,
            binary.read_size(),
            binary.uncompressed_size,
            binary.is_compressed(),
            None,
        ),
        RpfEntry::Resource(resource) => (
            RpfEntryKind::Resource,
            resource.consumed_size,
            // A resource is never deflated, so its stored and unpacked sizes
            // are the same number.
            resource.consumed_size,
            false,
            Some((resource.system_info, resource.graphics_info)),
        ),
        RpfEntry::Directory(_) => return None,
    };
    Some(EntryFacts {
        logical,
        container: container.to_string(),
        leaf: leaf.to_string(),
        kind,
        stored_size,
        size,
        compressed,
        paging,
    })
}

fn walk_archive<F: FnMut(&EntryFacts)>(
    archive: &Arc<RpfArchive>,
    device: &str,
    path_prefix: &str,
    container: &str,
    depth: u32,
    visit: &mut F,
) {
    for node in &archive.nodes {
        if node.is_dir() {
            continue;
        }
        let path = node.path();
        let leaf = node.name();
        let logical = format!("{device}:/{path_prefix}{path}");

        if let Some(facts) = facts_for(archive, node.entry_index(), logical, container, leaf) {
            visit(&facts);
        }

        if depth < MAX_NEST_DEPTH && leaf.to_ascii_lowercase().ends_with(".rpf") {
            if let Ok(nested) = archive.open_nested(node.entry_index()) {
                let nested = Arc::new(nested);
                let nested_container = format!("{container}/{path}");
                let nested_prefix = format!("{path_prefix}{path}/");
                walk_archive(
                    &nested,
                    device,
                    &nested_prefix,
                    &nested_container,
                    depth + 1,
                    visit,
                );
            }
        }
    }
}

pub fn build_mounted_game(
    install: &GtaInstall,
    crypto: Arc<dyn RpfCryptoProvider>,
    cutoff: DlcCutoff,
) -> Result<MountedGame, VfsBuildError> {
    build_mounted_game_with(install, crypto, cutoff, MountOptions::default())
}

/// The mount, with the overlay resolution said out loud.
///
/// ⚠ **Every archive is resolved on its own.** A partial overlay — one archive
/// copied into `mods`, the rest not — is the normal state of a modded install,
/// so resolving the whole tree to one side describes an install nobody has.
pub fn build_mounted_game_with(
    install: &GtaInstall,
    crypto: Arc<dyn RpfCryptoProvider>,
    cutoff: DlcCutoff,
    options: MountOptions,
) -> Result<MountedGame, VfsBuildError> {
    let mut vfs = Vfs::new();
    let mut roots: Vec<MountRoot> = Vec::new();
    let mut diagnostics = Diagnostics::new();

    let game = Game::detect_from_root(&install.root);

    // ⚠ Membership is the install's own copy. An overlay holding `common.rpf`
    // beside a directory that holds nothing else is not an install, and
    // accepting one here would mount a `mods` folder as a game.
    let common_path = install.common_rpf();
    if !common_path.exists() {
        return Err(VfsBuildError::MissingCommon { path: common_path });
    }
    let opened = open_install_archive(
        &install.root,
        &common_path,
        options,
        &crypto,
        &mut diagnostics,
    )?;
    let common_dev =
        Arc::new(ArchiveDevice::new(Arc::clone(&opened.archive)).with_label(opened.label.clone()));
    vfs.mount("common:", common_dev.clone());
    vfs.mount("commoncrc:", common_dev);
    roots.push(MountRoot {
        device: "common".into(),
        archive: opened.archive,
        container: opened.label,
        origin: opened.origin,
        stamp: opened.stamp,
    });

    for path in glob_platform_packs(&install.root) {
        match open_install_archive(&install.root, &path, options, &crypto, &mut diagnostics) {
            Ok(opened) => {
                let dev = Arc::new(
                    ArchiveDevice::new(Arc::clone(&opened.archive))
                        .with_label(opened.label.clone()),
                );
                vfs.mount("platform:", dev.clone());
                vfs.mount("platformcrc:", dev);
                roots.push(MountRoot {
                    device: "platform".into(),
                    archive: opened.archive,
                    container: opened.label,
                    origin: opened.origin,
                    stamp: opened.stamp,
                });
            }
            Err(e) => diagnostics.push(
                Diagnostic::warning(
                    keys::DLC_PACK_UNREADABLE,
                    format!("archive did not open: {e:?}"),
                )
                .at(path.display().to_string()),
            ),
        }
    }

    let update_path = install.update_rpf();
    let update_mount: Option<UpdateMount> = if update_path.exists() {
        let opened = open_install_archive(
            &install.root,
            &update_path,
            options,
            &crypto,
            &mut diagnostics,
        )?;
        let update = opened.archive;
        let tu_common = Arc::new(
            ArchiveDevice::with_sub_root(Arc::clone(&update), "common")
                .with_label(opened.label.clone()),
        );
        vfs.mount("common:", tu_common.clone());
        vfs.mount("commoncrc:", tu_common);
        let tu_platform = Arc::new(
            ArchiveDevice::with_sub_root(Arc::clone(&update), "x64")
                .with_label(opened.label.clone()),
        );
        vfs.mount("platform:", tu_platform.clone());
        vfs.mount("platformcrc:", tu_platform);
        vfs.mount(
            "update:",
            Arc::new(ArchiveDevice::new(Arc::clone(&update)).with_label(opened.label.clone())),
        );
        roots.push(MountRoot {
            device: "update".into(),
            archive: Arc::clone(&update),
            container: opened.label.clone(),
            origin: opened.origin,
            stamp: opened.stamp,
        });
        Some(UpdateMount {
            archive: update,
            label: opened.label,
        })
    } else {
        None
    };

    // ⚠ On Enhanced it shares no path with `update.rpf`, on Legacy only a
    // byte-identical `x64/data/credits.ymt`, and its copies of `common.rpf`'s
    // and the platform archives' paths are the newer ones, so it goes on top
    // of both halves.
    if let Some(update2_path) = game
        .profile()
        .update2_archive()
        .map(|rel| install.root.join(rel))
        .filter(|path| path.exists())
    {
        let opened = open_install_archive(
            &install.root,
            &update2_path,
            options,
            &crypto,
            &mut diagnostics,
        )?;
        for (device, sub_root) in [("common", "common"), ("platform", "x64")] {
            let half = Arc::new(
                ArchiveDevice::with_sub_root(Arc::clone(&opened.archive), sub_root)
                    .with_label(opened.label.clone()),
            );
            vfs.mount(&format!("{device}:"), half.clone());
            vfs.mount(&format!("{device}crc:"), half);
        }
        vfs.mount(
            "update2:",
            Arc::new(
                ArchiveDevice::new(Arc::clone(&opened.archive)).with_label(opened.label.clone()),
            ),
        );
        roots.push(MountRoot {
            device: "update2".into(),
            archive: opened.archive,
            container: opened.label,
            origin: opened.origin,
            stamp: opened.stamp,
        });
    }

    // The loose `dlcpacks:` tree, overlay copies mounted last so they shadow.
    // A `.rpf` served here is bytes rather than an archive — `locate_dlc` is
    // what opens one — but leaving this mount install-only would have the two
    // disagree about which copy of a pack exists.
    for dir in dlcpacks_dirs(&install.root, options) {
        if dir.is_dir() {
            vfs.mount(
                "dlcpacks:",
                Arc::new(super::folder_device::FolderDevice::new(&dir)),
            );
        }
    }

    let mut packs: Vec<MountedPack> = Vec::new();

    if update_path.exists() {
        register_title_update(&vfs, &mut packs, &mut diagnostics);
    }

    if cutoff != DlcCutoff::BaseGame {
        mount_dlc(
            install,
            &crypto,
            &cutoff,
            game,
            update_mount.as_ref(),
            options,
            &mut vfs,
            &mut roots,
            &mut packs,
            &mut diagnostics,
        );
    }

    report_overlay(&install.root, options, &roots, &mut diagnostics);

    Ok(MountedGame {
        vfs,
        packs,
        roots,
        diagnostics,
        game,
    })
}

/// The title update, and the label it mounts under.
///
/// ⚠ The label cannot be a constant: with an overlay copy the archive really
/// is `mods/update/update.rpf`, and `locate` joins the label back onto the
/// install root to say where a file came from.
struct UpdateMount {
    archive: Arc<RpfArchive>,
    label: String,
}

struct OpenedArchive {
    archive: Arc<RpfArchive>,
    origin: ArchiveOrigin,
    /// Install-relative, and of the copy actually opened.
    label: String,
    stamp: Option<FileStamp>,
}

fn open_install_archive(
    install_root: &Path,
    install_path: &Path,
    options: MountOptions,
    crypto: &Arc<dyn RpfCryptoProvider>,
    diagnostics: &mut Diagnostics,
) -> Result<OpenedArchive, VfsBuildError> {
    let (path, origin) = resolve_archive(install_root, install_path, options);
    let stamp = FileStamp::of(&path);
    let archive = open_rpf_file(&path, Arc::clone(crypto))?;
    let label = file_label(install_root, &path);
    if origin.is_overlay() {
        diagnostics.push(
            Diagnostic::info(
                keys::ARCHIVE_FROM_OVERLAY,
                "read from the mods overlay rather than the install's own copy",
            )
            .at(label.clone()),
        );
    }
    Ok(OpenedArchive {
        archive,
        origin,
        label,
        stamp,
    })
}

fn dlcpacks_dirs(install_root: &Path, options: MountOptions) -> Vec<PathBuf> {
    let install = [
        install_root.join("x64").join("dlcpacks"),
        install_root.join("update").join("x64").join("dlcpacks"),
    ];
    if !options.reads_the_overlay() {
        return install.into();
    }
    let overlay = overlay_root(install_root);
    install
        .iter()
        .cloned()
        .chain(install.iter().filter_map(|dir| {
            dir.strip_prefix(install_root)
                .ok()
                .map(|rel| overlay.join(rel))
        }))
        .collect()
}

/// What the mount read, where the user can see it.
///
/// ⚠ **Zero is the interesting number.** An overlay that matched nothing is
/// what a user gets for putting it in the wrong place or spelling a path
/// wrong, and it is indistinguishable from having no overlay at all unless
/// something says so.
fn report_overlay(
    install_root: &Path,
    options: MountOptions,
    roots: &[MountRoot],
    diagnostics: &mut Diagnostics,
) {
    let overlay = overlay_root(install_root);
    if !overlay.is_dir() {
        return;
    }
    if !options.reads_the_overlay() {
        diagnostics.info(
            keys::OVERLAY_NOT_READ,
            format!(
                "a mods overlay exists at {} and overlay-first resolution is off; \
                 the install's own archives were read",
                overlay.display()
            ),
        );
        return;
    }
    let from_overlay = roots.iter().filter(|r| r.origin.is_overlay()).count();
    diagnostics.info(
        keys::OVERLAY_RESOLVED,
        format!(
            "mods overlay at {}: {from_overlay} of {} mounted archive(s) came from it",
            overlay.display(),
            roots.len()
        ),
    );
}

#[allow(clippy::too_many_arguments)]
fn mount_dlc(
    install: &GtaInstall,
    crypto: &Arc<dyn RpfCryptoProvider>,
    cutoff: &DlcCutoff,
    game: Game,
    update_mount: Option<&UpdateMount>,
    options: MountOptions,
    vfs: &mut Vfs,
    roots: &mut Vec<MountRoot>,
    packs: &mut Vec<MountedPack>,
    diagnostics: &mut Diagnostics,
) {
    let dlclist_bytes = vfs.open("common:/data/dlclist.xml");
    let Some(bytes) = dlclist_bytes else {
        diagnostics.warn(
            keys::DLCLIST_MISSING,
            "common:/data/dlclist.xml not found; no DLC was mounted",
        );
        return;
    };
    let list = match DlcList::parse(&bytes) {
        Ok(l) => l,
        Err(e) => {
            diagnostics.warn(
                keys::DLCLIST_PARSE_FAILED,
                format!("dlclist.xml did not parse; no DLC was mounted: {e}"),
            );
            return;
        }
    };

    let etud = vfs
        .open("update:/common/data/ExtraTitleUpdateData.meta")
        .and_then(|b| ExtraTitleUpdateData::parse(&b).ok())
        .unwrap_or_default();

    struct Discovered {
        setup: SetupData,
        archive: Arc<RpfArchive>,
        source_path: PathBuf,
        container: String,
        source: DlcSource,
        origin: ArchiveOrigin,
        stamp: Option<FileStamp>,
    }
    let mut discovered: Vec<Discovered> = Vec::new();
    let mut skipped_gen9 = 0usize;
    for entry in &list.paths {
        let name = dlc_pack_name(entry);
        if name.is_empty() {
            continue;
        }
        if !game.accepts_dlc_pack(&name) {
            skipped_gen9 += 1;
            continue;
        }
        let Some(located) = locate_dlc(install, roots, &name, crypto, options, diagnostics) else {
            continue;
        };
        let Ok(setup_bytes) = located.archive.read("setup2.xml") else {
            diagnostics.push(
                Diagnostic::warning(keys::SETUP2_MISSING, "setup2.xml missing; pack skipped")
                    .at(name.clone()),
            );
            continue;
        };
        let setup = match SetupData::parse(&setup_bytes) {
            Ok(s) => s,
            Err(e) => {
                diagnostics.push(
                    Diagnostic::warning(
                        keys::SETUP2_PARSE_FAILED,
                        format!("setup2.xml did not parse; pack skipped: {e}"),
                    )
                    .at(name.clone()),
                );
                continue;
            }
        };
        discovered.push(Discovered {
            setup,
            archive: located.archive,
            source_path: located.source_path,
            container: located.container,
            source: located.source,
            origin: located.origin,
            stamp: located.stamp,
        });
    }

    if skipped_gen9 > 0 {
        diagnostics.info(
            keys::GEN9_PACKS_SKIPPED,
            format!(
                "Legacy install: skipped {skipped_gen9} Enhanced-only (g9ec) pack(s) that the Legacy game does not load"
            ),
        );
    }

    discovered.sort_by(|a, b| a.setup.mount_cmp(&b.setup));

    if let DlcCutoff::NameHash(target) = cutoff {
        match discovered
            .iter()
            .position(|d| d.setup.name_hash.eq_ignore_ascii_case(target))
        {
            Some(idx) => discovered.truncate(idx + 1),
            None => diagnostics.warn(
                keys::DLC_TARGET_NOT_FOUND,
                format!("--dlc target '{target}' is not in dlclist; including all DLC"),
            ),
        }
    }

    for d in discovered {
        let device = if d.setup.device_name.is_empty() {
            format!("dlc_{}", d.setup.name_hash)
        } else {
            d.setup.device_name.clone()
        };
        let base_dev =
            Arc::new(ArchiveDevice::new(Arc::clone(&d.archive)).with_label(d.container.clone()));
        vfs.mount(&format!("{device}:"), base_dev.clone());
        vfs.mount(&format!("{device}CRC:"), base_dev);

        if let (Some(update), Some(patch_path)) = (update_mount, etud.mount_for(&device)) {
            if let Some(sub_root) = patch_path
                .strip_prefix("update:/")
                .or_else(|| patch_path.strip_prefix("update:"))
            {
                let sub_root = sub_root.trim_matches('/');
                let patch = Arc::new(
                    ArchiveDevice::with_sub_root(Arc::clone(&update.archive), sub_root)
                        .with_label(update.label.clone()),
                );
                vfs.mount(&format!("{device}:"), patch.clone());
                vfs.mount(&format!("{device}CRC:"), patch);
            }
        }

        let dat = if d.setup.dat_file.is_empty() {
            "content.xml".to_string()
        } else {
            d.setup.dat_file.clone()
        };
        let patched = etud.mount_for(&device).and_then(|base_path| {
            let sep = if base_path.ends_with('/') { "" } else { "/" };
            let full = format!("{base_path}{sep}{dat}");
            vfs.open(&full).map(|b| (b, full))
        });
        let content_bytes = match patched {
            Some((bytes, from)) => Some((bytes, from)),
            None => d
                .archive
                .read(&dat)
                .ok()
                .map(|b| (b, format!("{device}:/{dat}"))),
        };
        let mut data_files = Vec::new();
        match content_bytes {
            Some((cbytes, from)) => match ContentXml::parse(&cbytes) {
                Ok(content) => {
                    let (mut files, fte) = merge_registry(vfs, content, diagnostics);
                    apply_effective_disabled(&mut files, &fte);
                    data_files = files;
                }
                Err(e) => diagnostics.push(
                    Diagnostic::warning(
                        keys::CONTENT_XML_PARSE_FAILED,
                        format!("content.xml did not parse: {e}"),
                    )
                    .at(from.clone()),
                ),
            },
            None => diagnostics.push(
                Diagnostic::warning(keys::DATA_FILE_MISSING, format!("{dat} missing"))
                    .at(device.clone()),
            ),
        }

        roots.push(MountRoot {
            device: device.clone(),
            archive: Arc::clone(&d.archive),
            container: d.container.clone(),
            origin: d.origin,
            stamp: d.stamp,
        });

        for n in 1..=d.setup.sub_pack_count {
            let sub_name = format!("dlc{n}.rpf");
            match open_sub_pack(install, &d.source, &sub_name, crypto, options, diagnostics) {
                Some((sub_archive, sub_container, sub_origin, sub_stamp)) => {
                    // ⚠ Labelled: `locate` says where a file sits by its device's
                    // label, and an unlabelled one answers a path inside the
                    // archive that no walk from the install root finds.
                    //
                    // ⚠ On `{device}CRC:` too, as `dlc.rpf` is: a content file
                    // writes a pack's path against either.
                    let sub_dev = Arc::new(
                        ArchiveDevice::new(Arc::clone(&sub_archive))
                            .with_label(sub_container.clone()),
                    );
                    vfs.mount(&format!("{device}:"), sub_dev.clone());
                    vfs.mount(&format!("{device}CRC:"), sub_dev);
                    roots.push(MountRoot {
                        device: device.clone(),
                        archive: sub_archive,
                        container: sub_container,
                        origin: sub_origin,
                        stamp: sub_stamp,
                    });
                }
                None => diagnostics.push(
                    Diagnostic::warning(
                        keys::SUB_PACK_MISSING,
                        format!("sub-pack {sub_name} not found"),
                    )
                    .at(device.clone()),
                ),
            }
        }

        packs.push(MountedPack {
            name_hash: d.setup.name_hash.clone(),
            device_name: device,
            setup: d.setup,
            data_files,
            source_path: d.source_path,
        });
    }
}

fn register_title_update(vfs: &Vfs, packs: &mut Vec<MountedPack>, diagnostics: &mut Diagnostics) {
    let Some(bytes) = vfs.open("update:/content.xml") else {
        return;
    };
    let content = match ContentXml::parse(&bytes) {
        Ok(c) => c,
        Err(e) => {
            diagnostics.push(
                Diagnostic::warning(
                    keys::CONTENT_XML_PARSE_FAILED,
                    format!("content.xml did not parse: {e}"),
                )
                .at("update:"),
            );
            return;
        }
    };
    let (mut data_files, fte) = merge_registry(vfs, content, diagnostics);
    apply_effective_disabled(&mut data_files, &fte);
    for df in &mut data_files {
        df.filename = expand_platform(&df.filename);
    }
    packs.push(MountedPack {
        name_hash: "TitleUpdate".to_string(),
        device_name: "update".to_string(),
        setup: SetupData {
            name_hash: "TitleUpdate".to_string(),
            device_name: "update".to_string(),
            ..Default::default()
        },
        data_files,
        source_path: PathBuf::from("update/update.rpf"),
    });
}

fn expand_platform(path: &str) -> String {
    if !path.contains('%') {
        return path.to_string();
    }
    let lower = path.to_ascii_lowercase();
    if let Some(pos) = lower.find("%platform%") {
        let end = pos + "%platform%".len();
        format!("{}x64{}", &path[..pos], &path[end..])
    } else {
        path.to_string()
    }
}

fn merge_registry(
    vfs: &Vfs,
    content: ContentXml,
    diagnostics: &mut Diagnostics,
) -> (Vec<DataFileEntry>, Vec<ListItem>) {
    let mut files_to_enable = content.files_to_enable();
    let mut data = content.data_files;
    let included: Vec<String> = content
        .included_data_files
        .iter()
        .map(|i| i.value.clone())
        .chain(content.included_xml_files.iter().cloned())
        .collect();
    for inc in &included {
        let Some(bytes) = vfs.open(inc) else {
            diagnostics.warn(
                keys::DATA_FILE_MISSING,
                format!("included data file not found: {inc}"),
            );
            continue;
        };
        match ContentXml::parse(&bytes) {
            Ok(inner) => {
                files_to_enable.extend(inner.files_to_enable());
                data.extend(inner.data_files);
            }
            Err(e) => diagnostics.push(
                Diagnostic::warning(
                    keys::CONTENT_XML_PARSE_FAILED,
                    format!("included data file did not parse: {e}"),
                )
                .at(inc.clone()),
            ),
        }
    }
    (data, files_to_enable)
}

pub(crate) fn apply_effective_disabled(
    data_files: &mut [DataFileEntry],
    files_to_enable: &[ListItem],
) {
    if files_to_enable.is_empty() {
        return;
    }
    let enabled: std::collections::HashSet<String> = files_to_enable
        .iter()
        .map(|f| f.trim().to_ascii_lowercase())
        .collect();
    for df in data_files.iter_mut() {
        if df.disabled && enabled.contains(&df.filename.trim().to_ascii_lowercase()) {
            df.disabled = false;
        }
    }
}

#[derive(Clone)]
enum DlcSource {
    Disk(PathBuf),
    Nested {
        parent: Arc<RpfArchive>,
        /// Install-relative, as a mounted archive's container is.
        parent_container: String,
        dir: String,
        parent_origin: ArchiveOrigin,
        parent_stamp: Option<FileStamp>,
    },
}

struct LocatedDlc {
    archive: Arc<RpfArchive>,
    source_path: PathBuf,
    container: String,
    source: DlcSource,
    origin: ArchiveOrigin,
    stamp: Option<FileStamp>,
}

fn locate_dlc(
    install: &GtaInstall,
    roots: &[MountRoot],
    name: &str,
    crypto: &Arc<dyn RpfCryptoProvider>,
    options: MountOptions,
    diagnostics: &mut Diagnostics,
) -> Option<LocatedDlc> {
    let disk_candidates = [
        install
            .root
            .join("update")
            .join("x64")
            .join("dlcpacks")
            .join(name)
            .join("dlc.rpf"),
        install
            .root
            .join("x64")
            .join("dlcpacks")
            .join(name)
            .join("dlc.rpf"),
    ];
    // ⚠ Each install location is asked about its own overlay copy before the
    // next location is tried, rather than sweeping the overlay first. A pack
    // the overlay holds under `update/` does not outrank the install's copy
    // under `update/` — it *is* that copy.
    for candidate in disk_candidates {
        let (path, origin) = resolve_archive(&install.root, &candidate, options);
        if !path.is_file() {
            continue;
        }
        let stamp = FileStamp::of(&path);
        match open_rpf_file(&path, Arc::clone(crypto)) {
            Ok(a) => {
                let container = file_label(&install.root, &path);
                if origin.is_overlay() {
                    diagnostics.push(
                        Diagnostic::info(
                            keys::ARCHIVE_FROM_OVERLAY,
                            "read from the mods overlay rather than the install's own copy",
                        )
                        .at(container.clone()),
                    );
                }
                return Some(LocatedDlc {
                    archive: a,
                    source_path: path,
                    container,
                    // ⚠ The install-side path, not the one that was opened.
                    // Sub-packs resolve on their own, so a pack whose `dlc.rpf`
                    // is overlaid and whose `dlc1.rpf` is not still finds both.
                    source: DlcSource::Disk(candidate),
                    origin,
                    stamp,
                });
            }
            Err(e) => diagnostics.push(
                Diagnostic::warning(
                    keys::DLC_PACK_UNREADABLE,
                    format!("open {} failed: {e:?}", path.display()),
                )
                .at(name.to_string()),
            ),
        }
    }

    for root in roots {
        let prefixes: &[&str] = match root.device.as_str() {
            "platform" => &["dlcpacks"],
            "update" => &["dlcpacks", "dlc_patch"],
            _ => continue,
        };
        for prefix in prefixes {
            let dir = format!("{prefix}/{name}");
            let rel = format!("{dir}/dlc.rpf");
            let Some(idx) = root.archive.find(&rel) else {
                continue;
            };
            match root.archive.open_nested(idx) {
                Ok(nested) => {
                    let container = format!("{}/{rel}", root.container);
                    let source_path = install.root.join(&root.container).join(&rel);
                    return Some(LocatedDlc {
                        archive: Arc::new(nested),
                        source_path,
                        container,
                        source: DlcSource::Nested {
                            parent: Arc::clone(&root.archive),
                            parent_container: root.container.clone(),
                            dir,
                            parent_origin: root.origin,
                            parent_stamp: root.stamp,
                        },
                        // A pack inside an archive comes from wherever that
                        // archive did. There is no overlay copy of a path that
                        // only exists inside an `.rpf`.
                        origin: root.origin,
                        stamp: root.stamp,
                    });
                }
                Err(e) => diagnostics.push(
                    Diagnostic::warning(
                        keys::DLC_PACK_UNREADABLE,
                        format!("nested open {}/{rel} failed: {e}", root.container),
                    )
                    .at(name.to_string()),
                ),
            }
        }
    }

    diagnostics.push(
        Diagnostic::warning(keys::DLC_RPF_MISSING, "dlc.rpf not found on disk or nested")
            .at(name.to_string()),
    );
    None
}

fn open_sub_pack(
    install: &GtaInstall,
    source: &DlcSource,
    sub_name: &str,
    crypto: &Arc<dyn RpfCryptoProvider>,
    options: MountOptions,
    diagnostics: &mut Diagnostics,
) -> Option<(Arc<RpfArchive>, String, ArchiveOrigin, Option<FileStamp>)> {
    match source {
        DlcSource::Disk(dlc_path) => {
            let sibling = dlc_path.with_file_name(sub_name);
            let (path, origin) = resolve_archive(&install.root, &sibling, options);
            if !path.is_file() {
                return None;
            }
            let stamp = FileStamp::of(&path);
            let a = open_rpf_file(&path, Arc::clone(crypto)).ok()?;
            let label = file_label(&install.root, &path);
            if origin.is_overlay() {
                diagnostics.push(
                    Diagnostic::info(
                        keys::ARCHIVE_FROM_OVERLAY,
                        "read from the mods overlay rather than the install's own copy",
                    )
                    .at(label.clone()),
                );
            }
            Some((a, label, origin, stamp))
        }
        DlcSource::Nested {
            parent,
            parent_container,
            dir,
            parent_origin,
            parent_stamp,
        } => {
            let rel = format!("{dir}/{sub_name}");
            let nested = parent.open_nested(parent.find(&rel)?).ok()?;
            Some((
                Arc::new(nested),
                format!("{parent_container}/{rel}"),
                *parent_origin,
                *parent_stamp,
            ))
        }
    }
}

pub(crate) fn dlc_pack_name(entry: &str) -> String {
    let s = entry.replace('\\', "/");
    let s = s.trim().trim_end_matches('/');
    let s = if s.to_ascii_lowercase().ends_with("/dlc.rpf") {
        &s[..s.len() - "/dlc.rpf".len()]
    } else {
        s
    };
    let last = s.rsplit('/').next().unwrap_or(s);
    last.rsplit(':').next().unwrap_or(last).to_string()
}

fn glob_platform_packs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name.starts_with("x64") && name.ends_with(".rpf") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn file_label(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}

pub(crate) fn open_rpf_file(
    path: &Path,
    crypto: Arc<dyn RpfCryptoProvider>,
) -> Result<Arc<RpfArchive>, VfsBuildError> {
    RpfArchive::open_with(path, crypto)
        .map(Arc::new)
        .map_err(|source| VfsBuildError::OpenRpf {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::container::rpf::crypto::NoOpCryptoProvider;

    #[test]
    fn expand_platform_token() {
        assert_eq!(
            expand_platform("update:/%PLATFORM%/patch/data/effects/ptfx.rpf"),
            "update:/x64/patch/data/effects/ptfx.rpf"
        );
        assert_eq!(
            expand_platform("update:/common/data/x.meta"),
            "update:/common/data/x.meta"
        );
    }

    #[test]
    fn dlc_pack_name_strips_device_and_rpf() {
        assert_eq!(dlc_pack_name("dlcpacks:/mpbeach/"), "mpbeach");
        assert_eq!(
            dlc_pack_name("dlcpacks:/mpchristmas3/dlc.rpf"),
            "mpchristmas3"
        );
        assert_eq!(dlc_pack_name("mphipster"), "mphipster");
    }

    fn packs_each_edition_tries(game: Game) -> Vec<String> {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("common").join("data");
        std::fs::create_dir_all(&data).unwrap();
        let items: String = [
            "mpbeach",
            "mpg9ec",
            "patchday28ng",
            "patchdayg9ecng",
            "mpsum2",
            "mpsum2_g9ec",
            "mp2025_01_G9EC",
        ]
        .iter()
        .map(|n| format!("<Item>dlcpacks:/{n}/</Item>"))
        .collect();
        std::fs::write(
            data.join("dlclist.xml"),
            format!("<SMandatoryPacksData><Paths>{items}</Paths></SMandatoryPacksData>"),
        )
        .unwrap();

        let mut vfs = Vfs::new();
        vfs.mount(
            "common:",
            Arc::new(super::super::folder_device::FolderDevice::new(
                dir.path().join("common"),
            )),
        );
        let install = GtaInstall {
            root: dir.path().to_path_buf(),
        };
        let provider: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
        let mut diagnostics = Diagnostics::new();
        mount_dlc(
            &install,
            &provider,
            &DlcCutoff::All,
            game,
            None,
            MountOptions::default(),
            &mut vfs,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut diagnostics,
        );
        diagnostics
            .messages()
            .iter()
            .filter(|d| d.key == keys::DLC_RPF_MISSING)
            .filter_map(|d| d.location.clone())
            .collect()
    }

    #[test]
    fn each_gta_v_edition_mounts_the_packs_its_dlc_policy_accepts() {
        assert_eq!(
            packs_each_edition_tries(Game::GtaVLegacy),
            ["mpbeach", "patchday28ng", "mpsum2"]
        );
        assert_eq!(
            packs_each_edition_tries(Game::GtaVEnhanced),
            [
                "mpbeach",
                "mpg9ec",
                "patchday28ng",
                "patchdayg9ecng",
                "mpsum2",
                "mpsum2_g9ec",
                "mp2025_01_G9EC",
            ]
        );
    }

    #[test]
    fn base_game_cutoff_skips_dlc() {
        let install = GtaInstall {
            root: PathBuf::from("/nowhere-openrage-test"),
        };
        let provider: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
        let err = build_mounted_game(&install, provider, DlcCutoff::BaseGame);
        assert!(matches!(err, Err(VfsBuildError::MissingCommon { .. })));
    }
}
