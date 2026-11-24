use std::path::PathBuf;
use std::sync::Arc;

use or_io::decl::Repair;
use or_io::gta5::pose_db::PoseMatcherDatabase;
use or_io::gta5::text::Gxt2;
use or_io::gta5::File as _;

use super::document::{self, DocPayload, DocumentKind, RpfNodeId};
use super::payload_handlers;
use super::prefs::RpfOutputMode;
use super::rpf_node::RpfNodeBacking;
use super::rpf_session::{RpfDocument, RpfEditSession};
use crate::ui::panels::hierarchy::gta6_save;

/// The message the four enumerating matches in this file end with.
///
/// ⚠ [`DocumentKind`] has an open variant, so these matches are not
/// exhaustive and a kind one of them forgets lands here at runtime rather than
/// failing to compile. Only kinds with no [`DocumentKind::resource_type`]
/// should arrive; [`tests::a_named_kind_never_falls_through_these_matches`]
/// checks that, building its string through this function.
fn unhandled(kind: DocumentKind, what: &str) -> String {
    let extension = match kind {
        DocumentKind::Registered(resource) => resource.extension(),
        named => named.extension(),
    };
    match or_io::gta5::formats::format(extension) {
        Some(format) => format!("no {what} for {}", format.noun),
        None => format!("no {what} for a .{extension} file"),
    }
}

/// Why a `.fxc` has no write path, said once so the offer and the attempt say
/// the same thing.
const READ_ONLY_FXC: &str =
    "a .fxc holds compiled shader bytecode; OpenRage reads one and does not write one";

/// Why a document of `kind` is never written, for the kinds that never are: what greys Save and Save as… over one.
pub fn never_written(kind: DocumentKind) -> Option<&'static str> {
    match kind {
        DocumentKind::Fxc => Some(READ_ONLY_FXC),
        DocumentKind::Bik => Some(READ_ONLY_BIK),
        DocumentKind::Rpf9 => Some(crate::i18n::tr(READ_ONLY_RPF9)),
        DocumentKind::Gta6AudioData => Some(crate::i18n::tr(READ_ONLY_GTA6_AUDIO_DATA)),
        _ => None,
    }
}

const READ_ONLY_BIK: &str =
    "a .bik is previewed and played; OpenRage writes no video, so it does not write one";

const NO_BLANK_GFX: &str =
    "a .gfx is opened or imported from an SWF; OpenRage does not author an empty one";

const NO_BLANK_MRF: &str =
    "a .mrf is opened and saved as it was read; OpenRage does not author an empty one";

const NO_BLANK_DDS: &str =
    "a .dds is opened and its pixels replaced from an image; OpenRage does not author an empty one";

const NO_BLANK_AWC: &str =
    "an .awc is opened and its streams replaced; OpenRage does not author an empty one";

const NO_BLANK_WORLD_DAT: &str =
    "a binary world .dat is opened from the game; OpenRage does not author an empty one";
const READ_ONLY_HEX: &str = "a file no reader opens is shown as its bytes, read-only";
const NO_BLANK_TTPF: &str = tr_key!(
    "a GTA VI texture pack is opened, its textures replaced and saved; OpenRage does not author an empty one"
);
const TTPF_FROM_DISK: &str =
    tr_key!("a GTA VI texture pack opens from its own file, which is read a texture at a time");
const READ_ONLY_RPF9: &str = tr_key!(
    "a GTA VI archive is scanned for the bodies stored in the clear and shown; OpenRage does not write one"
);
const RPF9_FROM_DISK: &str =
    tr_key!("a GTA VI archive opens from its own file, which is scanned a slice at a time");
const READ_ONLY_GTA6_ANIM: &str = tr_key!(
    "a GTA VI animation resource opened from a scan is saved once edited, as a loose resource; any other is read and shown"
);
const GTA6_ANIM_FROM_SCAN: &str = tr_key!(
    "a GTA VI animation resource opens only from a scan of the archive that holds it, or from the audio stream that stores it"
);

/// A GTA VI animation resource stored whole in an RSC8 of version 65, as an audio stream stores one.
fn stored_gta6_anim(data: &[u8]) -> std::result::Result<DocPayload, String> {
    use or_io::container::rpf::rsc8::Rsc8;
    let rsc = Rsc8::open(data)
        .ok()
        .filter(|rsc| rsc.resource_id() == or_io::gta6::anim::RSC8_VERSION)
        .ok_or_else(|| crate::i18n::tr(GTA6_ANIM_FROM_SCAN).to_string())?;
    let section = &rsc.data()[..rsc.virtual_size().min(rsc.data().len())];
    // A body stored as it is, as every audio stream's is, keeps its section, so its edits save.
    let kept = (data.get(5).is_some_and(|c| c & 0x1F == 0x1F)).then(|| {
        Arc::new(or_io::container::rpf::Rpf9Sections {
            form: or_io::container::rpf::Rpf9Form::StoredRsc8 { config: data[7] },
            virtual_section: section.to_vec(),
            physical: Vec::new(),
        })
    });
    or_io::gta6::anim::read_stored(section)
        .map(|mut resource| {
            resource.sections = kept;
            DocPayload::Gta6Anim(Arc::new(resource))
        })
        .map_err(|e| {
            tr!(
                "GTA VI's animation reader does not read this body: {e}",
                e = e
            )
        })
}
const READ_ONLY_GTA6_DRAWABLE: &str = tr_key!(
    "a GTA VI drawable is written only once its materials are edited, as a loose resource of its own"
);
const GTA6_DRAWABLE_FROM_SCAN: &str =
    tr_key!("a GTA VI drawable opens only from a scan of the archive that holds it");
const GTA6_TEXT_SAVE: &str = tr_key!(
    "a GTA VI text table opens from a scan or a loose resource, and saves as a loose resource or as GTA V's text table"
);
const READ_ONLY_GTA6_AUDIO_DATA: &str = tr_key!(
    "GTA VI audio game data is read only: a new or renamed item needs a name in its encrypted name table"
);
const EXPRESSIONS_FROM_SCAN: &str = tr_key!(
    "a GTA VI expression set opens only from a scan of the archive that holds it, and Save As writes the body as it is stored"
);
const GTA6_SAVE_FROM_FILE: &str = tr_key!(
    "a GTA VI save opens from a file that starts with JSAV or ASAV, and no blank one is made"
);
const NO_FILE_COMPARE: &str =
    "a comparison is made from two files and shown, and is never a file of its own";

const NO_BLANK_DAT_TABLE: &str =
    "a cache file, metadata.dat or the audio sector grid is opened from the game; OpenRage does not author an empty one";
const NO_BLANK_REL: &str =
    "a .rel is opened and saved as it was read; OpenRage does not author an empty one";
const NO_BLANK_NAME_TABLE: &str =
    "a .nametable names what its .dat holds; OpenRage opens one and does not author an empty one";

pub(super) fn resource_image_for(
    kind: DocumentKind,
    data: &[u8],
) -> Option<Arc<document::ResourceImage>> {
    kind.resource_type()?;

    let info =
        <or_io::container::resource::Rsc7 as or_io::container::resource::ResourceEnvelope>::open(
            data,
        )
        .ok()?;
    let virtual_size = info.virtual_size as usize;

    tracing::debug!(
        kind = kind.label(),
        compressed = data.len(),
        decompressed = info.data.len(),
        virtual_size,
        "kept the resource image"
    );

    Some(Arc::new(document::ResourceImage {
        data: info.data,
        virtual_size,
        seal: info.seal,
    }))
}

/// The built-in description of a document's declared format at the target its RSC7 version names, and that target's root type.
pub fn described_root(doc: &super::Document) -> Option<(Arc<or_io::desc::Pack>, String)> {
    // ⚠ The format first: a kind with none (a folder, a game root) never has an image to ask for.
    let format = or_io::gta5::formats::format(doc.kind.extension())?;
    let Some(image) = doc.resource.as_ref() else {
        doc.ask_for_image();
        return None;
    };
    described_for(format, image)
}

/// [`described_root`] for `format` and an image no document holds.
pub(super) fn described_for(
    format: &or_io::gta5::formats::Format,
    image: &document::ResourceImage,
) -> Option<(Arc<or_io::desc::Pack>, String)> {
    let target = format.target_of_version(image.seal.version()).ok()?;
    let described = format
        .description(target)
        .and_then(|pack| Ok((pack, format.root_name(target)?)));
    match described {
        Ok(described) => Some(described),
        Err(error) => {
            tracing::warn!(ext = format.ext, %error, "a declared format has no description");
            None
        }
    }
}

pub(super) fn parse_payload_from_path(
    path: &std::path::Path,
    kind: DocumentKind,
    crypto: std::sync::Arc<dyn or_io::container::RpfCryptoProvider>,
) -> std::result::Result<(DocPayload, Option<Arc<document::ResourceImage>>), String> {
    if let Some((archive, offset)) = super::rpf9_scan::scanned_body(path) {
        return Ok((super::rpf9_scan::read_body(path, archive, offset)?, None));
    }
    if matches!(kind, DocumentKind::Text) {
        let data = std::fs::read(path).map_err(|e| format!("Read error: {e}"))?;
        return Ok((text_payload(path, data), None));
    }
    // ⚠ Before the whole read: a GTA VI archive can be tens of gigabytes, and is scanned instead, as is the tab of one read again.
    if matches!(kind, DocumentKind::Rpf | DocumentKind::Rpf9) {
        // Its table, where a key opens it, lists it in the archive explorer; else a toast says the scan does.
        super::rpf9_scan::rpf9_table::offer(path);
        if let Some(scan) = super::rpf9_scan::Rpf9ScanDocument::open(path)? {
            return Ok((DocPayload::Rpf9(Arc::new(scan)), None));
        }
    }
    // ⚠ Never read whole: a pack can be gigabytes, and a texture is read only when it is shown.
    if matches!(kind, DocumentKind::Ttpf) {
        let ttpf = super::ttpf_document::TtpfDocument::open(path)
            .map_err(|e| format!("Parse error: {e}"))?;
        return Ok((DocPayload::Ttpf(Arc::new(ttpf)), None));
    }
    let data = std::fs::read(path).map_err(|e| format!("Read error: {e}"))?;
    let payload = parse_payload(path, kind, &data, crypto)?;
    Ok((payload, resource_image_for(kind, &data)))
}

/// The target an RSC7 file's version names for the format its extension names, and the version.
pub(super) fn read_file_generation(path: &std::path::Path) -> Option<(or_io::Generation, u32)> {
    use std::io::Read;

    let mut header = [0u8; 16];
    let mut file = std::fs::File::open(path).ok()?;
    // A short read is not an error here: a file too small to hold an RSC7
    // header is simply not a paged resource.
    let read = file.read(&mut header).ok()?;
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let version = rsc7_version(&header[..read])?;
    Some((target_of(&ext, version)?, version))
}

fn rsc7_version(data: &[u8]) -> Option<u32> {
    use or_io::container::resource::{ResourceEnvelope, Rsc7};
    Rsc7::peek(data).ok().map(|seal| seal.version())
}

/// What `or_io::open` reads a file of `ext` at `version` as.
fn target_of(ext: &str, version: u32) -> Option<or_io::Generation> {
    or_io::gta5::formats::format(ext)?
        .target_of_version(version)
        .ok()
}

pub(super) fn parse_payload(
    path: &std::path::Path,
    kind: DocumentKind,
    data: &[u8],
    crypto: std::sync::Arc<dyn or_io::container::RpfCryptoProvider>,
) -> std::result::Result<DocPayload, String> {
    use or_io::container::rpf::RpfArchive;

    if let Some(save) = sniffed_save(data) {
        return Ok(save);
    }
    if let Some(resource) = kind.resource_type() {
        let target = rsc7_version(data)
            .and_then(|version| target_of(resource.extension(), version))
            .unwrap_or(or_io::Generation::Gen8);
        let handler = payload_handlers::handler_for(resource, target)
            .ok_or_else(|| format!("No reader registered for {resource} ({})", target.slug()))?;
        return (handler.parse)(path, data);
    }

    match kind {
        DocumentKind::Gxt2 => Gxt2::from_bytes(data)
            .map(DocPayload::Gxt2)
            .map_err(|e| format!("Parse error: {}", e)),
        DocumentKind::Ypdb => PoseMatcherDatabase::from_bytes(data)
            .map(DocPayload::Ypdb)
            .map_err(|e| format!("Parse error: {}", e)),
        DocumentKind::Fxc => or_io::gta5::shaders::fxc::GrcEffect::parse(data)
            .map(|effect| {
                DocPayload::Fxc(document::FxcDocument {
                    effect,
                    bytes: data.to_vec(),
                })
            })
            .map_err(|e| format!("Parse error: {}", e)),
        DocumentKind::Rpf => {
            let name = path.file_name().and_then(|n| n.to_str());
            RpfArchive::from_bytes_with(name, data.to_vec(), crypto)
                .map_err(|e| e.to_string())
                .and_then(RpfDocument::new)
                .map(DocPayload::Rpf)
                .map_err(|e| format!("Parse error: {e}"))
        }
        DocumentKind::GameRoot => Err(
            "GameRoot documents are auto-created from the install folder, not loaded from disk"
                .to_string(),
        ),
        DocumentKind::Folder => Err(
            "Folder documents are created via File ▸ Open folder…, not loaded from disk as files"
                .to_string(),
        ),
        DocumentKind::Bik => Ok(DocPayload::Bik(crate::video::BikDocument::new(data))),
        DocumentKind::Awc => crate::audio::AwcDocument::open(data).map(DocPayload::Awc),
        DocumentKind::HeightMap => or_io::height_map::HeightMap::open(data)
            .map(DocPayload::HeightMap)
            .map_err(|e| format!("Parse error: {e}")),
        DocumentKind::DistantLights => or_io::distant_lights::DistantLights::open(data)
            .map(DocPayload::DistantLights)
            .map_err(|e| format!("Parse error: {e}")),
        DocumentKind::WaterMap => or_io::water_map::WaterMap::open(data)
            .map(DocPayload::WaterMap)
            .map_err(|e| format!("Parse error: {e}")),
        DocumentKind::WorldBlanket => or_io::world_blanket::WorldBlanket::open(data)
            .map(DocPayload::WorldBlanket)
            .map_err(|e| format!("Parse error: {e}")),
        DocumentKind::NavPrecalc => or_io::nav_precalc::NavPrecalc::open(data)
            .map(DocPayload::NavPrecalc)
            .map_err(|e| format!("Parse error: {e}")),
        DocumentKind::DatTable => document::DatTable::open(path, data).map(DocPayload::DatTable),
        DocumentKind::Rel => document::RelDocument::open(path, data).map(DocPayload::Rel),
        DocumentKind::NameTable => document::name_table_text(data).map(DocPayload::NameTable),
        DocumentKind::Gfx => or_io::gfx::GfxMovie::open(data)
            .map(DocPayload::Gfx)
            .map_err(|e| format!("Parse error: {e}")),
        DocumentKind::MoveNetwork => or_io::mrf::MvNetworkDef::open(data)
            .map(DocPayload::MoveNetwork)
            .map_err(|e| format!("Parse error: {e}")),
        DocumentKind::Dds => or_io::dds::Dds::from_bytes(data)
            .map_err(|e| e.to_string())
            .and_then(|dds| {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("texture");
                super::dds_document::open(name, &dds)
            })
            .map(DocPayload::Dds)
            .map_err(|e| format!("Parse error: {e}")),
        DocumentKind::Text => Ok(text_or_hex(path, data.to_vec())),
        DocumentKind::Meta => Ok(DocPayload::Meta(document::MetaDocument::from_bytes(
            data.to_vec(),
        ))),
        DocumentKind::Hex | DocumentKind::Gta6Save if gta6_save::is_save(data) => {
            gta6_save::open(data)
        }
        DocumentKind::Hex if is_text(data) => {
            Ok(DocPayload::Text(document::TextDocument::decode(data)))
        }
        DocumentKind::Hex => Ok(DocPayload::Hex(document::HexDocument {
            bytes: data.to_vec(),
            banner: no_reader(path),
        })),
        DocumentKind::Compare => Err(NO_FILE_COMPARE.to_string()),
        DocumentKind::Ttpf => Err(crate::i18n::tr(TTPF_FROM_DISK).to_string()),
        DocumentKind::Rpf9 => Err(crate::i18n::tr(RPF9_FROM_DISK).to_string()),
        DocumentKind::Gta6Anim => stored_gta6_anim(data),
        DocumentKind::Gta6Drawable => Err(crate::i18n::tr(GTA6_DRAWABLE_FROM_SCAN).to_string()),
        DocumentKind::Gta6Text => Err(crate::i18n::tr(GTA6_TEXT_SAVE).to_string()),
        DocumentKind::Gta6AudioData => Err(crate::i18n::tr(READ_ONLY_GTA6_AUDIO_DATA).to_string()),
        DocumentKind::Gta6Expressions => Err(crate::i18n::tr(EXPRESSIONS_FROM_SCAN).to_string()),
        DocumentKind::Gta6Save => Err(crate::i18n::tr(GTA6_SAVE_FROM_FILE).to_string()),
        other => Err(unhandled(other, "reader")),
    }
}

/// The GTA VI save `data` start as and read as, under whatever name they came: a save has no
/// extension of its own, and one that does not read is left to the reader its name picks.
fn sniffed_save(data: &[u8]) -> Option<DocPayload> {
    (gta6_save::is_save(data).then(|| gta6_save::open(data).ok())).flatten()
}

/// The text `bytes` decode to, or their hex view under the reason they are not text.
/// Bytes a text extension names: a metadata document where they read as one, else text or bytes.
pub(super) fn text_payload(path: &std::path::Path, data: Vec<u8>) -> DocPayload {
    if let Some(save) = sniffed_save(&data) {
        return save;
    }
    match DocumentKind::Text.refine_with_bytes(&data) {
        DocumentKind::Meta => DocPayload::Meta(document::MetaDocument::from_bytes(data)),
        _ => text_or_hex(path, data),
    }
}

fn text_or_hex(path: &std::path::Path, bytes: Vec<u8>) -> DocPayload {
    match document::TextDocument::open(path, &bytes) {
        Ok(text) => DocPayload::Text(text),
        Err(banner) => DocPayload::Hex(document::HexDocument { bytes, banner }),
    }
}

/// Whether bytes no extension names are text: a UTF-16 byte order mark, or UTF-8 with no NUL in the first 8 KiB.
pub(super) fn is_text(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return true;
    }
    !bytes[..bytes.len().min(8192)].contains(&0)
        && std::str::from_utf8(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)).is_ok()
}

/// The banner over a file whose name no reader claims.
pub(super) fn no_reader(path: &std::path::Path) -> String {
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    format!("OpenRage has no reader for {name}, so it opens as its bytes")
}

pub(super) fn serialize_payload(
    payload: &DocPayload,
    rpf_mode: RpfOutputMode,
    crypto: std::sync::Arc<dyn or_io::container::RpfCryptoProvider>,
) -> std::result::Result<Vec<u8>, String> {
    if let Some(handler) = payload_handlers::handler_for_payload(payload) {
        return handler.save(payload);
    }

    match payload {
        DocPayload::Gxt2(v) => v.to_bytes().map_err(|e| e.to_string()),
        DocPayload::Ypdb(v) => v.to_bytes().map_err(|e| e.to_string()),
        // Read-only: the DXBC blobs in a `.fxc` are `fxc.exe`'s output, and
        // `or-shader-tool` bakes rather than rewrites them.
        DocPayload::Fxc(_) => Err(READ_ONLY_FXC.to_string()),
        DocPayload::Bik(_) => Err(READ_ONLY_BIK.to_string()),
        DocPayload::Awc(awc) => awc.to_bytes(),
        DocPayload::HeightMap(map) => map.save().map_err(|e| e.to_string()),
        DocPayload::DistantLights(lights) => lights.save().map_err(|e| e.to_string()),
        DocPayload::WaterMap(map) => map.save().map_err(|e| e.to_string()),
        DocPayload::WorldBlanket(blanket) => Ok(blanket.save()),
        DocPayload::NavPrecalc(precalc) => precalc.save().map_err(|e| e.to_string()),
        DocPayload::DatTable(table) => table.save(),
        DocPayload::Rel(rel) => rel.to_bytes(),
        DocPayload::NameTable(text) => document::name_table_bytes(text),
        DocPayload::Gfx(movie) => movie.save().map_err(|e| e.to_string()),
        DocPayload::MoveNetwork(network) => network.save().map_err(|e| e.to_string()),
        DocPayload::Dds(texture) => super::dds_document::save(texture),
        DocPayload::Rpf(v) => {
            serialize_archive_records(v.session.file_records()?, rpf_mode, crypto)
        }
        DocPayload::Text(t) => Ok(t.encode()),
        DocPayload::Meta(m) => m.to_bytes().map_err(|e| format!("meta serialize: {e}")),
        DocPayload::Hex(_) => Err(READ_ONLY_HEX.to_string()),
        DocPayload::Ttpf(ttpf) => ttpf.save(),
        DocPayload::Rpf9(_) => Err(crate::i18n::tr(READ_ONLY_RPF9).to_string()),
        // An edited resource no scan holds, as an audio stream's, saves as the loose resource it is.
        DocPayload::Gta6Anim(r) => match r.sections.as_deref().filter(|_| r.edited) {
            Some(sections) => {
                or_io::container::rpf::write_rpf9_resource(sections).map_err(|e| e.to_string())
            }
            None => Err(crate::i18n::tr(READ_ONLY_GTA6_ANIM).to_string()),
        },
        DocPayload::Gta6Drawable(_) => Err(crate::i18n::tr(READ_ONLY_GTA6_DRAWABLE).to_string()),
        DocPayload::Gta6Text(_) => Err(crate::i18n::tr(GTA6_TEXT_SAVE).to_string()),
        DocPayload::Gta6AudioData(_) => Err(crate::i18n::tr(READ_ONLY_GTA6_AUDIO_DATA).to_string()),
        DocPayload::Gta6Expressions(_) => Err(crate::i18n::tr(EXPRESSIONS_FROM_SCAN).to_string()),
        DocPayload::Gta6Save(save) => save.to_bytes(),
        DocPayload::Compare(_) => Err(NO_FILE_COMPARE.to_string()),
        other => Err(unhandled(other.kind(), "writer")),
    }
}

/// Whether OpenRage rewrites a file of this name when it saves one.
///
/// ⚠ An archive is left out: rebuilding one re-encrypts it in the output mode
/// the settings name, which is a choice about the archive and not a fix to it.
pub(super) fn has_canonical_form(name: &str) -> bool {
    DocumentKind::from_path(std::path::Path::new(name)).is_some_and(|k| k != DocumentKind::Rpf)
}

/// [`serialize_payload`], rewriting a count a resource states against its run as `repair` allows, and each count it rewrote.
pub(super) fn serialize_payload_repairing(
    payload: &DocPayload,
    rpf_mode: RpfOutputMode,
    crypto: std::sync::Arc<dyn or_io::container::RpfCryptoProvider>,
    repair: Repair,
) -> std::result::Result<(Vec<u8>, Vec<String>), String> {
    if let Some(handler) = payload_handlers::handler_for_payload(payload) {
        return (handler.serialize)(payload, repair);
    }
    serialize_payload(payload, rpf_mode, crypto).map(|bytes| (bytes, Vec::new()))
}

/// The bytes saving `data`, opened as `name` and left unedited, writes; `None` where nothing reads and writes it.
pub(super) fn canonical_form(name: &str, data: &[u8]) -> Option<Vec<u8>> {
    canonical_form_repairing(name, data).map(|(bytes, _)| bytes)
}

/// [`canonical_form`] with OpenRage's fixes applied, which repairs a count only losslessly, and each count it rewrote.
pub(super) fn canonical_form_repairing(name: &str, data: &[u8]) -> Option<(Vec<u8>, Vec<String>)> {
    if !has_canonical_form(name) {
        return None;
    }
    let path = std::path::Path::new(name);
    let kind = DocumentKind::from_path(path)?.refine_with_bytes(data);
    let crypto: Arc<dyn or_io::container::RpfCryptoProvider> =
        Arc::new(or_io::container::NoOpCryptoProvider::new());
    let payload = parse_payload(path, kind, data, Arc::clone(&crypto)).ok()?;
    serialize_payload_repairing(&payload, RpfOutputMode::Open, crypto, Repair::Lossless).ok()
}

pub(super) fn serialize_archive_records(
    records: Vec<(String, Vec<u8>, bool)>,
    rpf_mode: RpfOutputMode,
    crypto: Arc<dyn or_io::container::RpfCryptoProvider>,
) -> std::result::Result<Vec<u8>, String> {
    use or_io::container::rpf::{NoOpCryptoProvider, RpfEncryption};

    let (encryption, keys): (RpfEncryption, Arc<dyn or_io::container::RpfCryptoProvider>) =
        match rpf_mode {
            RpfOutputMode::Open => (RpfEncryption::Open, Arc::new(NoOpCryptoProvider::new())),
            RpfOutputMode::Aes => {
                if crypto.supports(RpfEncryption::Aes) {
                    (RpfEncryption::Aes, crypto)
                } else {
                    tracing::warn!("AES output requested but no keys loaded; falling back to OPEN");
                    (RpfEncryption::Open, Arc::new(NoOpCryptoProvider::new()))
                }
            }
            RpfOutputMode::Ng => {
                if crypto.supports(RpfEncryption::Ng) {
                    (RpfEncryption::Ng, crypto)
                } else {
                    tracing::warn!("NG output requested but no keys loaded; falling back to OPEN");
                    (RpfEncryption::Open, Arc::new(NoOpCryptoProvider::new()))
                }
            }
        };
    archive_from_records(records, encryption, keys)
}

/// An archive of `records` under `encryption`, where a path ending in `/` is an empty folder.
pub(super) fn archive_from_records(
    records: Vec<(String, Vec<u8>, bool)>,
    encryption: or_io::container::rpf::RpfEncryption,
    keys: Arc<dyn or_io::container::RpfCryptoProvider>,
) -> std::result::Result<Vec<u8>, String> {
    let mut builder = or_io::container::rpf::RpfBuilder::default().encrypted(encryption, keys);
    for (path, bytes, _is_resource) in records {
        let added = match path.strip_suffix('/') {
            Some(folder) => builder.mkdir(folder),
            None => builder.insert_with(&path, bytes, should_compress_path(&path)),
        };
        added.map_err(|e| e.to_string())?;
    }
    builder.to_bytes().map_err(|e| e.to_string())
}

/// A blank payload built from a compiled default.
pub(super) fn empty_payload_for_kind(kind: DocumentKind) -> Option<DocPayload> {
    if let Some(resource) = kind.resource_type() {
        if let Some(handler) = payload_handlers::handler_for(resource, or_io::Generation::Gen8) {
            return Some((handler.empty)());
        }
    }

    empty_non_resource_payload(kind)
}

/// The non-paged kinds, which have no description and so always have a
/// compiled blank.
///
/// ⚠ `None` rather than a text document for a kind this match does not name:
/// a text fallback gives `File ▸ New ▸ Whatever` a tab that claims to be a
/// `.whatever` and is a text file.
fn empty_non_resource_payload(kind: DocumentKind) -> Option<DocPayload> {
    Some(match kind {
        DocumentKind::Gxt2 => DocPayload::Gxt2(Gxt2::new()),
        DocumentKind::Ypdb => DocPayload::Ypdb(Default::default()),
        // Quietly, unlike the catch-all below, which would log every frame
        // the menu is drawn.
        DocumentKind::Fxc
        | DocumentKind::Bik
        | DocumentKind::Awc
        | DocumentKind::HeightMap
        | DocumentKind::DistantLights
        | DocumentKind::WaterMap
        | DocumentKind::WorldBlanket
        | DocumentKind::NavPrecalc
        | DocumentKind::DatTable
        | DocumentKind::Rel
        | DocumentKind::NameTable
        | DocumentKind::Gfx
        | DocumentKind::MoveNetwork
        | DocumentKind::Dds
        | DocumentKind::Ttpf
        | DocumentKind::Rpf9
        | DocumentKind::Gta6Anim
        | DocumentKind::Gta6Drawable
        | DocumentKind::Gta6Text
        | DocumentKind::Gta6AudioData
        | DocumentKind::Gta6Expressions
        | DocumentKind::Gta6Save
        | DocumentKind::Hex
        | DocumentKind::Compare => return None,
        DocumentKind::Rpf | DocumentKind::GameRoot | DocumentKind::Folder => {
            DocPayload::Rpf(RpfDocument {
                session: RpfEditSession::empty(),
                expanded: {
                    let mut s = std::collections::HashSet::new();
                    s.insert(0);
                    s
                },
                focused_node: None,
                is_game_root: matches!(kind, DocumentKind::GameRoot),
                entry_documents: std::collections::HashMap::new(),
            })
        }
        DocumentKind::Text => DocPayload::Text(document::TextDocument::empty()),
        DocumentKind::Meta => DocPayload::Meta(document::MetaDocument::empty()),
        other => {
            tracing::error!("{}", unhandled(other, "blank payload"));
            return None;
        }
    })
}

/// Why this build cannot author a `kind`, or `None` if it can.
///
/// ⚠ **The create-new menu must ask this rather than reimplement it.** Two
/// copies of "can we author this" drift into an offer whose click refuses.
/// `nothing_offered_would_refuse_if_clicked` holds the offer and the attempt
/// to one question.
pub(super) fn why_not_authorable(kind: DocumentKind) -> Option<String> {
    // ⚠ No handler is a refusal: `None` from this function means *can author*,
    // so a `?` here would offer a registered format nothing implements.
    if let Some(resource) = kind.resource_type() {
        if payload_handlers::handler_for(resource, or_io::Generation::Gen8).is_none() {
            return Some(format!(
                "nothing in this build implements .{}",
                kind.extension()
            ));
        }
    }
    // A non-paged kind is asked too: `.fxc` is read-only, so answering `None`
    // here would offer a create-new row whose click refuses.
    authoring_verdict(kind)
}

/// The remembered outcome of actually authoring a blank `kind`, memoised
/// because the menu is drawn every frame and a blank `.ydd` is real work.
fn authoring_verdict(kind: DocumentKind) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    /// The value is the refusal, or `None` for success.
    type Tried = Mutex<HashMap<DocumentKind, Option<String>>>;
    static TRIED: OnceLock<Tried> = OnceLock::new();

    let cache = TRIED.get_or_init(|| Mutex::new(HashMap::new()));

    if let Ok(seen) = cache.lock() {
        if let Some(verdict) = seen.get(&kind) {
            return verdict.clone();
        }
    }

    // ⚠ `author_blank` rather than `empty_file_bytes_for_kind`, which asks *this*
    // function first — and would recurse.
    let verdict = author_blank(kind).err();
    if let Ok(mut seen) = cache.lock() {
        seen.insert(kind, verdict.clone());
    }
    verdict
}

pub(super) fn empty_file_bytes_for_kind(kind: DocumentKind) -> Result<Vec<u8>, String> {
    author_blank(kind)
}

/// Author a blank `kind`, for real.
///
/// The menu's verdict from [`why_not_authorable`] is this function's error, so
/// this must not ask that function back.
fn author_blank(kind: DocumentKind) -> Result<Vec<u8>, String> {
    use or_io::container::RpfBuilder;

    if let Some(resource) = kind.resource_type() {
        if let Some(handler) = payload_handlers::handler_for(resource, or_io::Generation::Gen8) {
            return handler.save(&(handler.empty)());
        }
    }

    match kind {
        DocumentKind::Gxt2 => Gxt2::new().to_bytes().map_err(|e| e.to_string()),
        DocumentKind::Ypdb => PoseMatcherDatabase::default()
            .to_bytes()
            .map_err(|e| e.to_string()),
        DocumentKind::Fxc => Err(READ_ONLY_FXC.to_string()),
        DocumentKind::Bik => Err(READ_ONLY_BIK.to_string()),
        DocumentKind::Awc => Err(NO_BLANK_AWC.to_string()),
        DocumentKind::HeightMap
        | DocumentKind::DistantLights
        | DocumentKind::WaterMap
        | DocumentKind::WorldBlanket
        | DocumentKind::NavPrecalc => Err(NO_BLANK_WORLD_DAT.to_string()),
        DocumentKind::DatTable => Err(NO_BLANK_DAT_TABLE.to_string()),
        DocumentKind::Rel => Err(NO_BLANK_REL.to_string()),
        DocumentKind::NameTable => Err(NO_BLANK_NAME_TABLE.to_string()),
        DocumentKind::Gfx => Err(NO_BLANK_GFX.to_string()),
        DocumentKind::MoveNetwork => Err(NO_BLANK_MRF.to_string()),
        DocumentKind::Dds => Err(NO_BLANK_DDS.to_string()),
        DocumentKind::Hex => Err(READ_ONLY_HEX.to_string()),
        DocumentKind::Ttpf => Err(crate::i18n::tr(NO_BLANK_TTPF).to_string()),
        DocumentKind::Rpf9 => Err(crate::i18n::tr(READ_ONLY_RPF9).to_string()),
        DocumentKind::Gta6Anim => Err(crate::i18n::tr(READ_ONLY_GTA6_ANIM).to_string()),
        DocumentKind::Gta6Drawable => Err(crate::i18n::tr(READ_ONLY_GTA6_DRAWABLE).to_string()),
        DocumentKind::Gta6Text => Err(crate::i18n::tr(GTA6_TEXT_SAVE).to_string()),
        DocumentKind::Gta6AudioData => Err(crate::i18n::tr(READ_ONLY_GTA6_AUDIO_DATA).to_string()),
        DocumentKind::Gta6Expressions => Err(crate::i18n::tr(EXPRESSIONS_FROM_SCAN).to_string()),
        DocumentKind::Gta6Save => Err(crate::i18n::tr(GTA6_SAVE_FROM_FILE).to_string()),
        DocumentKind::Compare => Err(NO_FILE_COMPARE.to_string()),
        DocumentKind::Rpf => RpfBuilder::default().to_bytes().map_err(|e| e.to_string()),
        DocumentKind::GameRoot => {
            Err("GameRoot documents have no on-disk byte form; cannot embed in an RPF".to_string())
        }
        DocumentKind::Folder => {
            Err("Folder documents have no on-disk byte form; cannot embed in an RPF".to_string())
        }
        DocumentKind::Text => Ok(document::TextDocument::empty().encode()),
        DocumentKind::Meta => Ok(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n".to_vec()),
        other => Err(unhandled(other, "empty byte form")),
    }
}

pub(super) fn unique_child_name(
    session: &RpfEditSession,
    parent: RpfNodeId,
    stem: &str,
    ext: &str,
) -> String {
    let siblings: std::collections::HashSet<String> = session
        .children_of(parent)
        .into_iter()
        .filter_map(|id| session.node(id).map(|n| n.name.to_ascii_lowercase()))
        .collect();
    // ⚠ A folder on disk the tree has not scanned holds names the tree does not list.
    let disk = session.node(parent).and_then(|n| match &n.backing {
        RpfNodeBacking::DiskFolder { fs_path, .. } => Some(fs_path.as_path()),
        _ => None,
    });
    let free = |name: &str| {
        !siblings.contains(&name.to_ascii_lowercase())
            && disk.is_none_or(|dir| dir.join(name).symlink_metadata().is_err())
    };
    let candidate = format!("{stem}.{ext}");
    if free(&candidate) {
        return candidate;
    }
    for n in 1u32..10_000 {
        let c = format!("{stem}{n}.{ext}");
        if free(&c) {
            return c;
        }
    }
    format!("{stem}.{ext}")
}

pub(super) fn should_compress_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    !(lower.ends_with(".bik")
        || lower.ends_with(".awc")
        || lower.ends_with(".gxt")
        || lower.ends_with(".rpf"))
}

/// Write `data` to `path`, keeping a copy of whatever was there **first**.
///
/// ⚠ The `.bak` is taken only when none exists, so it holds the file from
/// before OpenRage touched it. Copying on every save leaves it holding the
/// previous edit, and the original is gone by the second save. A stale `.bak`
/// is therefore never refreshed.
pub(super) fn atomic_write_with_backup(path: &PathBuf, data: &[u8]) -> std::io::Result<()> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("bin")
        .to_string();

    if path.exists() {
        let backup_path = path.with_extension(format!("{}.bak", ext));
        if !backup_path.exists() {
            if let Err(e) = std::fs::copy(path, &backup_path) {
                tracing::warn!(
                    error = %e,
                    backup_path = %backup_path.display(),
                    "Failed to create backup file (continuing with save)"
                );
            }
        }
    }

    let temp_path = path.with_extension(format!("{}.tmp", ext));
    std::fs::write(&temp_path, data)?;

    #[cfg(target_os = "windows")]
    if path.exists() {
        std::fs::remove_file(path)?;
    }

    std::fs::rename(&temp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_crypto() -> Arc<dyn or_io::container::RpfCryptoProvider> {
        Arc::new(or_io::container::NoOpCryptoProvider::new())
    }

    /// An archive's path in a folder that goes when the first of the pair drops.
    fn scratch(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("openrage-bak-{name}-"))
            .tempdir()
            .expect("scratch dir");
        let path = dir.path().join("archive.rpf");
        (dir, path)
    }

    /// The backup holds what was there **before OpenRage**, across any number
    /// of saves.
    ///
    /// ⚠ Three saves and a check of the bytes: a copy-every-time backup passes
    /// a two-save test that only checks a `.bak` exists.
    #[test]
    fn a_backup_is_taken_once_and_holds_the_original() {
        let (_scratch, path) = scratch("once");
        std::fs::write(&path, b"shipped").expect("seed");

        atomic_write_with_backup(&path, b"edit-one").expect("first save");
        atomic_write_with_backup(&path, b"edit-two").expect("second save");
        atomic_write_with_backup(&path, b"edit-three").expect("third save");

        let backup = path.with_extension("rpf.bak");
        assert_eq!(
            std::fs::read(&path).expect("the file"),
            b"edit-three",
            "the last save must be what is on disk"
        );
        assert_eq!(
            std::fs::read(&backup).expect("the backup"),
            b"shipped",
            "the backup must still hold the bytes from before the first save. \
             Holding `edit-two` means the copy is happening every time, which \
             cuts the seatbelt at the second use — the state this was fixed from."
        );
    }

    /// No original, no backup — and no error either.
    #[test]
    fn writing_a_file_that_did_not_exist_leaves_no_backup() {
        let (_scratch, path) = scratch("fresh");
        atomic_write_with_backup(&path, b"new").expect("write");

        assert_eq!(std::fs::read(&path).expect("the file"), b"new");
        assert!(
            !path.with_extension("rpf.bak").exists(),
            "a file that never existed has no earlier state to preserve, and a \
             `.bak` holding nothing would be indistinguishable from one holding \
             a real original"
        );
    }

    /// The temporary file does not survive a successful write.
    #[test]
    fn the_atomic_temporary_does_not_outlive_the_write() {
        let (_scratch, path) = scratch("tmp");
        std::fs::write(&path, b"before").expect("seed");
        atomic_write_with_backup(&path, b"after").expect("write");

        assert!(
            !path.with_extension("rpf.tmp").exists(),
            "a leftover .tmp beside every saved archive is litter a user has to \
             clean up and cannot tell from a real file"
        );
    }

    /// What replaces the compiler's exhaustiveness check on the four matches
    /// in this file.
    ///
    /// ⚠ A new [`DocumentKind`] variant breaks the build only in `document`
    /// (measured: four errors, all four there), never here: it falls into
    /// each match's catch-all and becomes a format
    /// that is offered in the Open dialog, opens to an error and creates
    /// nothing.
    #[test]
    fn a_named_kind_never_falls_through_these_matches() {
        for &kind in DocumentKind::NAMED {
            let probe = std::path::PathBuf::from(format!("probe.{}", kind.extension()));

            // Empty bytes are not a valid file of any format, so a real
            // reader is expected to refuse them. What must not happen is
            // being refused because no arm names the kind.
            if let Err(error) = parse_payload(&probe, kind, &[], no_crypto()) {
                assert_ne!(
                    error,
                    unhandled(kind, "reader"),
                    "{kind:?} falls through `parse_payload`, so opening one \
                     fails with a message instead of reading the file"
                );
            }

            if let Err(error) = empty_file_bytes_for_kind(kind) {
                assert_ne!(
                    error,
                    unhandled(kind, "empty byte form"),
                    "{kind:?} falls through `empty_file_bytes_for_kind`, so \
                     `New ▸` inside an archive cannot create one"
                );
            }

            // A paged format's blank is its handler's, and a kind
            // `why_not_authorable` refuses (`.fxc`) is never offered, so neither
            // owes a compiled blank here.
            if kind.resource_type().is_none() {
                if why_not_authorable(kind).is_some() {
                    continue;
                }
                let blank = empty_payload_for_kind(kind).unwrap_or_else(|| {
                    panic!(
                        "{kind:?} falls through `empty_non_resource_payload`, \
                         so `File ▸ New` opens nothing for it"
                    )
                });
                if let Err(error) = serialize_payload(&blank, RpfOutputMode::Open, no_crypto()) {
                    assert_ne!(
                        error,
                        unhandled(blank.kind(), "writer"),
                        "{kind:?} falls through `serialize_payload`, so it \
                         opens and cannot be saved"
                    );
                }
            }
        }
    }

    /// The catch-all names the file in words, or by its extension, and never by a kind code.
    ///
    /// ⚠ Nothing here may ask the live registry about `Registered(Ypm)`:
    /// `state::adding_a_format` registers a synthetic handler over that
    /// resource, so such an assertion passes or fails on test order.
    #[test]
    fn a_kind_these_matches_do_not_name_gets_the_catch_all_in_words() {
        use or_io::container::resource::ResourceType;
        assert_eq!(
            unhandled(DocumentKind::Registered(ResourceType::Ybd), "writer"),
            "no writer for a bounds dictionary"
        );
        assert_eq!(
            unhandled(DocumentKind::Registered(ResourceType::Ypm), "reader"),
            "no reader for a .ypm file"
        );
    }

    /// A save has no extension, so its first bytes open it whatever its name says it is.
    #[test]
    fn a_gta_vi_save_is_opened_by_its_first_bytes_under_any_name() {
        // A story save of no block: its head, and an index of nothing.
        let mut save = b"JSAV".to_vec();
        for word in [3u32, 0, 0] {
            save.extend_from_slice(&word.to_le_bytes());
        }
        for (name, kind) in [
            ("SAVE01", DocumentKind::Hex),
            ("SAVE01.bak", DocumentKind::Hex),
            ("save.dds", DocumentKind::Dds),
            ("save.meta", DocumentKind::Meta),
        ] {
            let payload = parse_payload(std::path::Path::new(name), kind, &save, no_crypto());
            assert_eq!(
                payload.map(|p| p.kind()),
                Ok(DocumentKind::Gta6Save),
                "{name}"
            );
        }
        let text = std::path::Path::new("save.txt");
        assert_eq!(
            text_payload(text, save.clone()).kind(),
            DocumentKind::Gta6Save
        );
        // A version no reader reads is no save: its name's reader takes it, and no name says why.
        save[4] = 9;
        assert!(sniffed_save(&save).is_none());
        assert_ne!(
            text_payload(text, save.clone()).kind(),
            DocumentKind::Gta6Save
        );
        let unnamed = std::path::Path::new("SAVE01");
        assert!(parse_payload(unnamed, DocumentKind::Hex, &save, no_crypto()).is_err());
        assert!(sniffed_save(b"JSA").is_none());
    }
}
