//! A modder's edit, made by the editor an OpenRage verb calls, landed in a GTA VI patched copy: for
//! one small row of a real build per kind, the edited body goes through `plan_patch` into a copy of
//! the span of the archive it needs, is read back through `RpfArchive`, and must be the edit; per
//! kind it prints whether it fitted in place, or the refusal's name and by how many bytes.
//!
//! The kinds and their editors: a text table's text changed at its own byte length (`to_section_over`,
//! which the CLI's `--rewrite` and the app save through, and `to_section`, their fallback); a script
//! program's push immediate (`set_int`, as `script compiled --set-int`); a scalar field of a PSO
//! in table form (`write_adding_strings`, as the app saves it) and in P-form
//! (`write_meta_binary`) and of a headerless metadata document (`write_sections`); a texture
//! dictionary's texture replaced by itself re-encoded (`write_dictionary_textures`, as
//! `rpf scan-texture` writes it), zlib rows only; a drawable's vertex moved onto another vertex at
//! the same vertex count (`write_geometry`, as `mesh import` writes it). A text table's row is also
//! saved as the app saves it (`to_section_over` into a block-mapped virtual section with no
//! physical one), and that plan must be a patch, not a rebuild. A panic, a read that is not the
//! edit, a kind with no row found, or no edit of any kind reaching the patch writer fails the test.
//! A metadata XML document is not tried: its splice of an edited value lives in the app, which
//! this test cannot reach.
//!
//! Bounded: the walk stops at 120 s or once each kind has 4 rows; a row holds at most 512 KiB
//! inflated and its stored bytes end in the first 64 MiB of its file, which is what is copied; each
//! kind tries its rows in turn until its editor makes an edit. On the Xbox build 1.0.2.1 every
//! P-form row (57, by a node mirror of every table) is a root row of class 0x0B whose stored bytes
//! end past 400 MiB, so the P-form rows are sought first, in root tables only, within 512 MiB, and
//! a P-form attempt copies up to that much twice. The optimal parse runs only on a
//! changed chunk no level of the tree's deflater fits. Install-gated; it skips without:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use or_io::authoring::{encode_texture_with_options, TextureBuildOptions};
use or_io::codec::texture::decode_level;
use or_io::container::rpf::gta6_chunk_table::entry_chunks;
use or_io::container::rpf::gta6_kinds::{kind_label, PLAIN_FILE};
use or_io::container::rpf::{
    gta6_patch::{plan_patch_with, OPTIMAL_CAP},
    open_at, plan_patch, read_rpf9_body, write_planned_copy, write_rpf9_resource, BodyPatch,
    Gta6Class, Gta6Context, Gta6Entry, Gta6Mount, GtaViKeys, NameResolver, PatchRefusal,
    Rpf9BodyKind, Rpf9Form, Rpf9Role, Rpf9Sections, RpfArchive, GTA6_SNAPSHOT_ENV, RPF9_SLOT,
};
use or_io::gta6::headerless::{
    read_dictionary, read_drawable, read_drawable_with_bound, write_dictionary_textures,
    write_geometry, Entry, GeometryAt, GeometryEdit, HeldTexture,
};
use or_io::gta6::script::compiled::{disasm, Compiled};
use or_io::gta6::script::compiled_write::set_int;
use or_io::gta6::text::{read as read_text, TextItem, TextTable};
use or_io::meta::binary::hash_index::title_names;
use or_io::meta::binary::pso::table_strings::write_adding_strings;
use or_io::meta::binary::rbm::gta6 as headerless_meta;
use or_io::meta::binary::{parse_meta_binary_full, ContainerLayout};
use or_io::meta::{write_meta_binary, Diagnostics, MetaInstance, SchemaLibrary, Value};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// A sampled row's stored bytes end within this many bytes of its file's start: the span copied.
const PREFIX_CAP: u64 = 64 << 20;
/// The span a P-form row, sought in root tables only, is copied within.
const P_FORM_PREFIX_CAP: u64 = 512 << 20;
/// The most a sampled row holds inflated.
const ROW_CAP: u64 = 512 << 10;
const CANDIDATES: usize = 4;
const NESTED_PER_ARCHIVE: usize = 8;
const MAX_DEPTH: usize = 3;
const WALK_BUDGET: Duration = Duration::from_secs(120);

const TEXT_TABLE: &str = "text table";
const SCRIPT: &str = "script program";
const HEADERLESS_META: &str = "headerless metadata";
const TEXTURES: &str = "texture dictionary";
const DRAWABLE: &str = "drawable";
const PSO_TABLE: &str = "PSO table form";
const PSO_P: &str = "PSO P-form";
/// A text table's row saved as the app saves it, tried on the text table's rows.
const APP_TEXT: &str = "text table, the app's save";
/// The optimal-parse cap the app's save plans with.
const APP_OPTIMAL_CAP: usize = 256 << 10;
/// The 20-byte rows' kinds, as the type table labels them.
const HEADERLESS: [&str; 5] = [TEXT_TABLE, SCRIPT, HEADERLESS_META, TEXTURES, DRAWABLE];
const KINDS: [&str; 7] = [
    TEXT_TABLE,
    SCRIPT,
    PSO_TABLE,
    PSO_P,
    HEADERLESS_META,
    TEXTURES,
    DRAWABLE,
];

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// Every `*.rpf` directly under `root`, then every `pack/*/*.rpf`, sorted.
fn build_archives(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut rpfs_in = |dir: &Path| {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rpf") {
                out.push(path);
            }
        }
    };
    rpfs_in(root);
    for pack in std::fs::read_dir(root.join("pack"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if pack.path().is_dir() {
            rpfs_in(&pack.path());
        }
    }
    out.sort();
    out
}

/// A row to edit: its file, the starts of the nested archives down to it, its archive's header, its
/// row and where its stored bytes end.
struct Candidate {
    file: PathBuf,
    chain: Vec<u64>,
    base: u64,
    entry: u32,
    end: u64,
}

struct Walk<'a> {
    keys: &'a GtaViKeys,
    file: PathBuf,
    began: Instant,
    found: BTreeMap<&'static str, Vec<Candidate>>,
}

impl Walk<'_> {
    fn wants(&self, kind: &str) -> bool {
        self.found.get(kind).map_or(0, Vec::len) < CANDIDATES
    }

    fn done(&self) -> bool {
        self.began.elapsed() > WALK_BUDGET || KINDS.iter().all(|k| !self.wants(k))
    }
}

/// The end of `e`'s stored bytes, where the patch writer reads them.
fn stored_end(mount: &Gta6Mount, e: &Gta6Entry) -> Option<u64> {
    let start = mount.absolute_start(e)?;
    if let Some(stored) = e.stored {
        return Some(start + stored);
    }
    let nested = mount.archive.entry_extra(e.index as usize)?.nested?;
    let (at, len) = nested.ciphered_body_extent(start);
    Some(at + len)
}

/// A PSO's form: `PMAP` after `PSIN` is the P-form, else the table form; `None` for no PSO.
fn pso_form(body: &[u8]) -> Option<&'static str> {
    if !body.starts_with(b"PSIN") {
        return None;
    }
    let psin = body.get(4..8)?;
    let psin = u32::from_be_bytes([psin[0], psin[1], psin[2], psin[3]]) as usize;
    if body.get(psin..psin + 4) == Some(&b"PMAP"[..]) {
        Some(PSO_P)
    } else {
        Some(PSO_TABLE)
    }
}

/// What `e`'s chunk table states it holds inflated, `None` where it has no table.
fn inflated(mount: &Gta6Mount, e: &Gta6Entry) -> Option<u64> {
    match entry_chunks(&mount.archive, e.index as usize) {
        Ok(Some(table)) => Some(table.iter().map(|c| u64::from(c.inflated)).sum()),
        _ => None,
    }
}

/// The kind of edit `e` is sampled for, `None` where it is none of them or too large.
fn kind_of(w: &Walk, file: &mut File, mount: &Gta6Mount, e: &Gta6Entry) -> Option<&'static str> {
    if inflated(mount, e)? > ROW_CAP {
        return None;
    }
    let row = mount.archive.entry_extra(e.index as usize)?.nested;
    if let Some(row) = row {
        let label = kind_label(None, Some(row.resource_type));
        return HEADERLESS.into_iter().find(|k| *k == label && w.wants(k));
    }
    if !(w.wants(PSO_TABLE) || w.wants(PSO_P)) {
        return None;
    }
    let label = kind_label(e.class, None);
    if label != "PSO" && label != PLAIN_FILE {
        return None;
    }
    let body = mount.read_entry_decrypted(&mut *file, e, w.keys).ok()?;
    pso_form(&body).filter(|k| w.wants(k))
}

/// The P-form rows of the root archive `mount` whose stored bytes end within
/// [`P_FORM_PREFIX_CAP`], whatever their class byte.
fn walk_p_form(w: &mut Walk, file: &mut File, mount: &Gta6Mount) {
    for e in &mount.archive.entries {
        if !w.wants(PSO_P) || w.began.elapsed() > WALK_BUDGET {
            return;
        }
        let row20 =
            (mount.archive.entry_extra(e.index as usize)).is_some_and(|x| x.nested.is_some());
        if row20 || e.kind() == Some(Gta6Class::NestedArchive) {
            continue;
        }
        let Some(end) = stored_end(mount, e).filter(|&end| end <= P_FORM_PREFIX_CAP) else {
            continue;
        };
        if inflated(mount, e).is_none_or(|n| n > ROW_CAP) {
            continue;
        }
        let Ok(body) = mount.read_entry_decrypted(&mut *file, e, w.keys) else {
            continue;
        };
        if pso_form(&body) == Some(PSO_P) {
            let candidate = Candidate {
                file: w.file.clone(),
                chain: Vec::new(),
                base: mount.base,
                entry: e.index,
                end,
            };
            w.found.entry(PSO_P).or_default().push(candidate);
        }
    }
}

fn walk(w: &mut Walk, file: &mut File, mount: &Gta6Mount, chain: &[u64], depth: usize) {
    let mut nested = 0;
    for e in &mount.archive.entries {
        if w.done() {
            return;
        }
        if e.kind() == Some(Gta6Class::NestedArchive) {
            let near = mount.absolute_start(e).is_some_and(|s| s < PREFIX_CAP);
            if near && depth < MAX_DEPTH && nested < NESTED_PER_ARCHIVE {
                nested += 1;
                if let (Some(start), Ok(inner)) =
                    (e.start, mount.open_nested(&mut *file, e, w.keys))
                {
                    let mut chain = chain.to_vec();
                    chain.push(start);
                    walk(w, file, &inner, &chain, depth + 1);
                }
            }
            continue;
        }
        let Some(end) = stored_end(mount, e).filter(|&end| end <= PREFIX_CAP) else {
            continue;
        };
        let Some(kind) = kind_of(w, file, mount, e) else {
            continue;
        };
        let candidate = Candidate {
            file: w.file.clone(),
            chain: chain.to_vec(),
            base: mount.base,
            entry: e.index,
            end,
        };
        w.found.entry(kind).or_default().push(candidate);
    }
}

fn refusal_name(r: &PatchRefusal) -> String {
    let debug = format!("{r:?}");
    (debug.split(|c: char| !c.is_alphanumeric()).next())
        .unwrap_or_default()
        .to_owned()
}

/// How far past what fits a refusal is, where it states one.
fn over(r: &PatchRefusal) -> String {
    match *r {
        PatchRefusal::ChunkDoesNotFit {
            deflated,
            optimal,
            ceiling,
            ..
        } => {
            let best = i64::try_from(deflated.min(optimal)).unwrap_or(i64::MAX);
            format!(", {} bytes past the ceiling", best - i64::from(ceiling))
        }
        PatchRefusal::Length { got, want, .. } => {
            format!(
                ", {:+} bytes against the row's length",
                got as i64 - want as i64
            )
        }
        _ => String::new(),
    }
}

/// The body whose sections were `before` with `after` in their place: the head before the
/// sections, as `original` holds it, then the new sections.
fn body_of(original: &[u8], before: &Rpf9Sections, after: &Rpf9Sections) -> Vec<u8> {
    let head = original.len() - before.virtual_section.len() - before.physical.len();
    [
        &original[..head],
        &after.virtual_section[..],
        &after.physical[..],
    ]
    .concat()
}

/// A 20-byte row's sections, the role its head places and whether it is a zlib row, read from its
/// stored bytes as the scan verbs read them, which must be the body the row reads as.
fn sections_of(
    mount: &Gta6Mount,
    src: &mut File,
    keys: &GtaViKeys,
    e: &Gta6Entry,
    original: &[u8],
) -> Result<(Rpf9Sections, Option<Rpf9Role>, bool), String> {
    let row = (mount.archive.entry_extra(e.index as usize))
        .and_then(|x| x.nested)
        .ok_or("it is no 20-byte row")?;
    let stored = match e.cipher {
        Some(_) => {
            (mount.read_entry_deciphered_stored(&mut *src, e, keys)).map_err(|e| e.to_string())?
        }
        None => {
            let start = mount.absolute_start(e).ok_or("it has no start")?;
            let mut stored = vec![0u8; row.stored_size().saturating_sub(RPF9_SLOT) as usize];
            src.seek(SeekFrom::Start(start + RPF9_SLOT))
                .and_then(|_| src.read_exact(&mut stored))
                .map_err(|e| e.to_string())?;
            stored
        }
    };
    let (find, bytes) = read_rpf9_body(Cursor::new(&stored), 0, original.len() as u64)
        .map_err(|e| format!("the resource reader refuses it: {e}"))?;
    if bytes != original {
        return Err("the resource reader and the row's read give other bytes".into());
    }
    let (role, zlib) = match find.kind {
        Rpf9BodyKind::Zlib(run) => (run.signature.and_then(|s| s.role()), true),
        _ => (None, false),
    };
    let sections =
        Rpf9Sections::of(&find, &bytes).ok_or("the resource reader finds no sections")?;
    let rest = sections.virtual_section.len() + sections.physical.len();
    if rest > original.len() || body_of(original, &sections, &sections) != original {
        return Err("its sections are not its body after a head".into());
    }
    Ok((sections, role, zlib))
}

/// The first integer, float or boolean of `instance`, depth first, changed; the field's path.
fn nudge(instance: &mut MetaInstance) -> Option<String> {
    (instance.fields.iter_mut())
        .find_map(|(name, value)| nudge_value(value).map(|rest| format!("{name}{rest}")))
}

fn nudge_value(value: &mut Value) -> Option<String> {
    match value {
        Value::Int(n) => *n ^= 1,
        Value::UInt(n) => *n ^= 1,
        Value::Bool(b) => *b = !*b,
        // Negation changes a float at any width; zero becomes one.
        Value::Float(f) if f.is_finite() => *f = if *f == 0.0 { 1.0 } else { -*f },
        Value::Struct(s) | Value::Pointer(Some(s)) => return nudge(s).map(|p| format!(".{p}")),
        Value::Array(items) => {
            return (items.iter_mut().enumerate())
                .find_map(|(i, item)| nudge_value(item).map(|p| format!("[{i}]{p}")))
        }
        _ => return None,
    }
    Some(String::new())
}

type Edits = Vec<(String, Result<Vec<u8>, String>)>;

/// The text table `section` holds with one plain text's first ASCII letter's case changed, and
/// that text's key.
fn flipped(section: &[u8]) -> Result<(TextTable, u32), String> {
    let (mut table, _) = read_text(section).map_err(|e| e.to_string())?;
    let changed = (table.entries.iter_mut()).find_map(|e| match &mut e.item {
        TextItem::Text(text) => {
            let at = text.bytes().position(|b| b.is_ascii_alphabetic())?;
            let mut bytes = std::mem::take(text).into_bytes();
            // A letter's case is its 0x20 bit: the text keeps its byte length.
            bytes[at] ^= 0x20;
            *text = String::from_utf8(bytes).expect("an ASCII letter's case keeps UTF-8");
            Some(e.key)
        }
        TextItem::Variants(_) => None,
    });
    let key = changed.ok_or("no plain text of it holds an ASCII letter")?;
    Ok((table, key))
}

fn text_edits(original: &[u8], s: &Rpf9Sections) -> Result<Edits, String> {
    let (table, key) = flipped(&s.virtual_section)?;
    let laid = |v: Result<Vec<u8>, String>| {
        v.map(|v| {
            let after = Rpf9Sections {
                virtual_section: v,
                ..s.clone()
            };
            body_of(original, s, &after)
        })
    };
    let anew = table.to_section().map_err(|e| e.to_string());
    let over = (table.to_section_over(&s.virtual_section)).map_err(|e| e.to_string());
    let what = format!("a letter's case of text {key:#010X}");
    Ok(vec![
        (
            format!("{what}, to_section (the fallback when to_section_over refuses)"),
            laid(anew),
        ),
        (
            format!("{what}, to_section_over (the CLI's --rewrite and the app's save)"),
            laid(over),
        ),
    ])
}

/// The edit of [`text_edits`] saved as the app's text table document saves it: over its own section,
/// as a block-mapped virtual section with no physical one (`Gta6TextDocument::written`,
/// `edited_sections`).
fn app_text_edit(s: &Rpf9Sections) -> Result<Edits, String> {
    let (table, key) = flipped(&s.virtual_section)?;
    // The app holds the loose form and plans the body that form reads back to.
    let written = (table.to_section_over(&s.virtual_section))
        .map_err(|e| e.to_string())
        .and_then(|virtual_section| {
            let after = Rpf9Sections {
                form: Rpf9Form::BlockMapped,
                virtual_section,
                physical: Vec::new(),
            };
            let loose = write_rpf9_resource(&after).map_err(|e| e.to_string())?;
            let (find, bytes) =
                read_rpf9_body(Cursor::new(&loose), 0, u64::MAX).map_err(|e| e.to_string())?;
            Rpf9Sections::of(&find, &bytes)
                .map(|_| bytes)
                .ok_or_else(|| "the loose form reads as no resource".to_owned())
        });
    let how = format!("a letter's case of text {key:#010X}, saved as the app saves a text table");
    Ok(vec![(how, written)])
}

fn script_edit(original: &[u8]) -> Result<Edits, String> {
    let program = Compiled::open(original).map_err(|e| e.to_string())?;
    for ins in program.instructions() {
        let Ok(ins) = ins else { break };
        if !disasm::mnemonic(ins.opcode).is_some_and(|name| name.starts_with("push")) {
            continue;
        }
        for value in [1, 0] {
            let mut body = original.to_vec();
            if set_int(&mut body, ins.offset, value).is_ok() && body != original {
                let how = format!("set_int {value} at code offset {:#X}", ins.offset);
                return Ok(vec![(how, Ok(body))]);
            }
        }
    }
    Err("its code holds no push of one immediate value before its first unread instruction".into())
}

fn pso_edit(library: &SchemaLibrary, keys: &GtaViKeys, original: &[u8]) -> Result<Edits, String> {
    let mut diags = Diagnostics::lenient();
    let (mut instance, layout) =
        parse_meta_binary_full(library, original, &mut diags).map_err(|e| e.to_string())?;
    let field = nudge(&mut instance).ok_or("it holds no integer, float or boolean field")?;
    let table = matches!(&layout.container, ContainerLayout::Pso(p) if p.is_table_form());
    // As the app saves: a table form's strings added with no key first, with the string keys after.
    let written = if table {
        write_adding_strings(&layout, &instance, None)
            .or_else(|first| match keys.has_string_keys() {
                true => write_adding_strings(&layout, &instance, Some(keys)),
                false => Err(first),
            })
            .map_err(|e| e.to_string())
    } else {
        write_meta_binary(layout.format, Some(&layout), &instance).map_err(|e| e.to_string())
    };
    let writer = if table {
        "write_adding_strings"
    } else {
        "write_meta_binary"
    };
    Ok(vec![(format!("field {field}, {writer}"), written)])
}

fn meta_edit(original: &[u8], s: &Rpf9Sections) -> Result<Edits, String> {
    let (mut instance, layout) =
        headerless_meta::open(&s.virtual_section, &s.physical).map_err(|e| e.to_string())?;
    let ContainerLayout::Rbm(rbm) = &layout.container else {
        return Err("it opens as no RBM layout".into());
    };
    let field = nudge(&mut instance).ok_or("it holds no integer, float or boolean field")?;
    let written = headerless_meta::write_sections(rbm, &instance).map(|(v, p)| {
        let after = Rpf9Sections {
            form: s.form,
            virtual_section: v,
            physical: p,
        };
        body_of(original, s, &after)
    });
    Ok(vec![(
        format!("field {field}, write_sections"),
        written.map_err(|e| e.to_string()),
    )])
}

/// `t`'s pixels as its own top level decoded and encoded again in its format and mips, where that
/// gives other bytes of the same length.
fn reencoded(read: &or_io::gta6::Texture, t: &HeldTexture) -> Option<Vec<u8>> {
    let image = decode_level(read, 0).ok()?;
    let options = TextureBuildOptions {
        mip_levels: Some(u32::from(t.mips)),
        allow_npot: true,
        srgb: t.srgb,
        ..TextureBuildOptions::default()
    };
    let e =
        encode_texture_with_options(t.format, image.width, image.height, &image, &options).ok()?;
    (e.mip_count == t.mips && e.data.len() == t.pixels.len() && e.data != t.pixels)
        .then_some(e.data)
}

fn texture_edit(original: &[u8], s: &Rpf9Sections) -> Result<Edits, String> {
    let read = read_dictionary(&s.virtual_section, Some(&s.physical)).map_err(|e| e.to_string())?;
    let (mut held, mut chosen) = (Vec::new(), None);
    for (_, entry) in read.entries {
        let t = match entry {
            Ok(Entry::Texture(t)) => t,
            Ok(_) => return Err("it holds entries that are no textures".into()),
            // An entry that does not read is kept as the body holds it.
            Err(_) => continue,
        };
        let flat = t.dimension != 3;
        let tex = t.texture.ok();
        let format = tex.as_ref().map(|x| x.format);
        let mut texture = HeldTexture {
            name: t.name,
            width: t.width,
            height: t.height,
            format: format.map(|f| f.format()).unwrap_or_default(),
            srgb: format.is_some_and(|f| f.is_srgb()),
            mips: tex.as_ref().map_or(t.mip_levels, |x| x.levels as u8),
            pixels: tex.as_ref().map(|x| x.pixels.clone()).unwrap_or_default(),
        };
        let flat = flat && tex.as_ref().is_some_and(|x| x.faces <= 1);
        if chosen.is_none() && flat {
            if let Some(pixels) = tex.as_ref().and_then(|x| reencoded(x, &texture)) {
                texture.pixels = pixels;
                chosen = Some(format!(
                    "{}x{} {:?}, {} mips",
                    texture.width, texture.height, texture.format, texture.mips
                ));
            }
        }
        held.push(texture);
    }
    let what =
        chosen.ok_or("no 2D texture of it reads and re-encodes to other bytes of its length")?;
    let mut after = s.clone();
    let written =
        write_dictionary_textures(&mut after, &held).map(|()| body_of(original, s, &after));
    Ok(vec![(
        format!("a texture ({what}) replaced by itself re-encoded, write_dictionary_textures"),
        written.map_err(|e| e.to_string()),
    )])
}

fn vertex_edit(original: &[u8], s: &Rpf9Sections, role: Option<Rpf9Role>) -> Result<Edits, String> {
    let role = role.ok_or("its head places no drawable")?;
    let drawable = match role {
        Rpf9Role::Drawable => read_drawable(&s.virtual_section, Some(&s.physical)),
        Rpf9Role::DrawableWithBound => {
            read_drawable_with_bound(&s.virtual_section, Some(&s.physical)).map(|d| d.drawable)
        }
        other => return Err(format!("its role is {}", other.label())),
    }
    .map_err(|e| e.to_string())?;
    let models = drawable.levels[0]
        .as_ref()
        .ok_or("its first level is empty")?;
    for (model, m) in models.iter().enumerate() {
        for (geometry, g) in m.geometries.iter().enumerate() {
            let Some(held) = g.as_ref().ok().and_then(|g| g.positions.as_ref().ok()) else {
                continue;
            };
            let Some(apart) = held.iter().position(|p| Some(p) != held.first()) else {
                continue;
            };
            let g = g.as_ref().expect("its positions read");
            let mut positions = held.clone();
            positions[0] = held[apart];
            let edit = GeometryEdit {
                positions,
                triangles: g.triangles.clone(),
                ..GeometryEdit::default()
            };
            let at = GeometryAt {
                level: 0,
                model,
                geometry,
            };
            let mut after = s.clone();
            let written = write_geometry(&mut after, role, None, at, &edit)
                .map(|_| body_of(original, s, &after))
                .map_err(|e| e.to_string());
            let how = format!(
                "vertex 0 of geometry {geometry} of model {model} moved onto vertex {apart}, \
                 write_geometry"
            );
            return Ok(vec![(how, written)]);
        }
    }
    Err("no geometry of its first level holds two vertices apart whose positions read".into())
}

/// The body of entry `entry` of the archive at `base`, read through `RpfArchive` from `copy`.
fn read_back(copy: &Path, chain: &[u64], base: u64, entry: u32) -> Vec<u8> {
    let mut archive = Arc::new(RpfArchive::open(copy).expect("the patched copy opens"));
    for &start in chain {
        let nested = (0..archive.entries.len() as u32)
            .find(|&i| {
                archive.rpf9_place(i).is_none()
                    && archive
                        .binary_entry(i)
                        .is_some_and(|b| b.file_offset == start)
            })
            .expect("a nested archive is a folder of the tree");
        archive = Arc::new(
            archive
                .open_nested(nested)
                .expect("the nested archive opens"),
        );
    }
    let file = (0..archive.entries.len() as u32)
        .find(|&i| {
            archive
                .rpf9_place(i)
                .is_some_and(|p| (p.base, p.entry) == (base, entry))
        })
        .expect("the row is a file of the tree");
    archive.read_entry(file).expect("the patched row reads")
}

/// `body` planned into the span `prefix` holds and, where it fits, written to `copy` and read back;
/// the refusal said as the error.
fn land(
    prefix: &Path,
    copy: &Path,
    keys: &GtaViKeys,
    c: &Candidate,
    body: &[u8],
    cap: usize,
) -> Result<String, (String, bool)> {
    let mut src = File::open(prefix).expect("the span opens");
    let patch = BodyPatch {
        base: c.base,
        entry: c.entry,
        body: body.to_vec(),
    };
    let began = Instant::now();
    let plan = match plan_patch_with(&mut src, keys, &patch, cap, &mut |_, _| true) {
        Ok(plan) => plan,
        Err(r) => {
            let length = matches!(r, PatchRefusal::Length { .. });
            return Err((
                format!("refused, {}{}: {r}", refusal_name(&r), over(&r)),
                length,
            ));
        }
    };
    let took = began.elapsed().as_millis();
    let chunks = plan.chunks.len();
    let changed = plan.chunks.iter().filter(|k| k.changed).count();
    let optimal = plan.chunks.iter().filter(|k| k.optimal).count();
    let report =
        write_planned_copy(prefix, copy, keys, vec![plan]).expect("a planned patch writes");
    let back = read_back(copy, &c.chain, c.base, c.entry);
    assert!(
        back == body,
        "the patched copy reads back otherwise than the edit"
    );
    std::fs::remove_file(copy).expect("the copy is removed");
    Ok(format!(
        "fitted in place: {changed} of {chunks} chunk(s) changed, {optimal} by the optimal parse, \
         {} bytes written, planned in {took} ms; read back through RpfArchive, it is the edit",
        report.bytes_written
    ))
}

/// Try `c` for `kind`: its span copied, its own body planned unchanged, its editor's edits landed;
/// the lines said and how many edits reached the patch writer.
fn attempt(
    kind: &str,
    c: &Candidate,
    keys: &GtaViKeys,
    library: &SchemaLibrary,
    scratch: &Path,
) -> Result<(Vec<String>, usize), String> {
    let (prefix, copy) = (scratch.join("span.rpf"), scratch.join("patched.rpf"));
    let mut from = File::open(&c.file).expect("the archive opens");
    let mut to = File::create(&prefix).expect("the span is written");
    std::io::copy(&mut (&mut from).take(c.end), &mut to).expect("the span is copied");
    drop(to);

    let mut src = File::open(&prefix).expect("the span opens");
    let mount = Gta6Mount {
        base: c.base,
        archive: open_at(&mut src, c.base, keys).map_err(|e| e.to_string())?,
    };
    let e = &mount.archive.entries[c.entry as usize];
    let original = (mount.read_entry_decrypted(&mut src, e, keys))
        .map_err(|e| format!("it does not read: {e}"))?;
    let own = BodyPatch {
        base: c.base,
        entry: c.entry,
        body: original.clone(),
    };
    match plan_patch(&mut src, keys, &own) {
        Ok(plan) if plan.chunks.iter().all(|k| !k.changed) => {}
        Ok(_) => return Err("its own body plans a changed chunk".into()),
        Err(r) => {
            return Err(format!(
                "its own body is refused, {}: {r}",
                refusal_name(&r)
            ))
        }
    }
    let edits = match kind {
        SCRIPT => script_edit(&original)?,
        PSO_TABLE | PSO_P => pso_edit(library, keys, &original)?,
        _ => {
            let (s, role, zlib) = sections_of(&mount, &mut src, keys, e, &original)?;
            match kind {
                TEXT_TABLE => text_edits(&original, &s)?,
                APP_TEXT => app_text_edit(&s)?,
                HEADERLESS_META => meta_edit(&original, &s)?,
                TEXTURES if !zlib => {
                    return Err("it is no zlib row; BCPack rows are not tried here".into())
                }
                TEXTURES => texture_edit(&original, &s)?,
                _ => vertex_edit(&original, &s, role)?,
            }
        }
    };
    // A row whose every edit its editor refuses leaves the kind to its next row.
    let made = (edits.iter())
        .filter(|(_, body)| body.as_ref().is_ok_and(|body| *body != original))
        .count();
    if made == 0 {
        let said: Vec<String> = (edits.into_iter())
            .map(|(how, body)| match body {
                Err(why) => format!("{how}: the editor refuses it: {why}"),
                Ok(_) => format!("{how}: the edit changed no byte"),
            })
            .collect();
        return Err(said.join("; "));
    }
    let ciphered = if e.cipher.is_some() {
        "enciphered"
    } else {
        "clear"
    };
    let mut lines = vec![format!(
        "row {} of the archive at {:#x}, {} bytes, {ciphered}",
        c.entry,
        c.base,
        original.len()
    )];
    // The app plans with its own optimal-parse cap, the CLI with the patch writer's.
    let cap = if kind == APP_TEXT {
        APP_OPTIMAL_CAP
    } else {
        OPTIMAL_CAP
    };
    for (how, body) in edits {
        let said = match body {
            Err(why) => format!("the editor refuses it: {why}"),
            Ok(body) if body == original => "the edit changed no byte".to_owned(),
            Ok(body) => match land(&prefix, &copy, keys, c, &body, cap) {
                Ok(said) => said,
                // Only a body of another length is the app's save shape gone wrong; a chunk that
                // does not fit is the deflater's outcome, counted as any other kind's.
                Err((refused, true)) if kind == APP_TEXT => panic!(
                    "the app's save of row {} at {:#x} plans a rebuilt archive, not a patch: \
                     {refused}",
                    c.entry, c.base
                ),
                Err((refused, _)) => refused,
            },
        };
        lines.push(format!("  {how}: {said}"));
    }
    Ok((lines, made))
}

#[test]
fn an_edit_made_by_openrages_editors_lands_in_a_patched_copy_or_is_refused_by_name() {
    let what = "GTA VI edits by OpenRage's editors into a patched copy";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}"),
        );
        return;
    };
    let keys = Arc::new(GtaViKeys::from_file(&snapshot).expect("the key snapshot parses"));
    // `RpfArchive` reads a GTA VI archive with the keys the process was given.
    Gta6Context::give(Gta6Context {
        keys: Arc::clone(&keys),
        names: Arc::new(NameResolver::new()),
    });
    let _names = title_names();
    let library = SchemaLibrary::default();
    let archives = build_archives(&build);
    assert!(!archives.is_empty(), "no .rpf under {}", build.display());

    let began = Instant::now();
    let mut w = Walk {
        keys: &keys,
        file: PathBuf::new(),
        began,
        found: BTreeMap::new(),
    };
    for path in &archives {
        if !w.wants(PSO_P) || w.began.elapsed() > WALK_BUDGET {
            break;
        }
        let mut file = File::open(path).expect("open the archive");
        let Ok(root) = Gta6Mount::open_root(&mut file, &keys) else {
            continue;
        };
        w.file = path.clone();
        walk_p_form(&mut w, &mut file, &root);
    }
    for path in &archives {
        if w.done() {
            break;
        }
        let mut file = File::open(path).expect("open the archive");
        let Ok(root) = Gta6Mount::open_root(&mut file, &keys) else {
            continue;
        };
        w.file = path.clone();
        walk(&mut w, &mut file, &root, &[], 0);
    }
    let walked = began.elapsed();
    let found = w.found;

    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut reached = 0;
    let mut none_found = Vec::new();
    for kind in KINDS.into_iter().chain([APP_TEXT]) {
        let rows = if kind == APP_TEXT { TEXT_TABLE } else { kind };
        let candidates = found.get(rows).map_or(&[][..], Vec::as_slice);
        if candidates.is_empty() {
            none_found.push(kind);
        }
        let mut notes = Vec::new();
        let mut done = false;
        for c in candidates {
            match attempt(kind, c, &keys, &library, scratch.path()) {
                Ok((lines, made)) => {
                    reached += made;
                    println!("{kind}: {}", lines.join("\n"));
                    done = true;
                    break;
                }
                Err(why) => notes.push(format!("row {} at {:#x}: {why}", c.entry, c.base)),
            }
        }
        if !done {
            println!(
                "{kind}: no edit made, of {} row(s) found: {}",
                candidates.len(),
                notes.join("; ")
            );
        }
    }
    println!(
        "walked in {} s, {} s in all",
        walked.as_secs(),
        began.elapsed().as_secs()
    );
    assert!(
        none_found.is_empty(),
        "no row found within the walk's bounds for: {}",
        none_found.join(", ")
    );
    assert!(reached > 0, "no edit of any kind reached the patch writer");
}
