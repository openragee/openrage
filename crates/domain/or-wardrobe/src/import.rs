//! The import: the model of a pack made from the files of an existing resource, in place.
//!
//! The garments are the models and textures whose names the resolver reads, each pinned at the
//! number its name carries, because that number is the position a server's saved outfits already
//! use, and the cloth of a model is the `.yld` that has its name. What the pack's own `.ymt` states
//! is kept, so planning the import writes metadata that means
//! what the original did. Nothing is moved or copied, and nothing is guessed: a file the resolver
//! cannot place, a file that has no garment to belong to, and a fragment file or a metadata
//! file no reader opens are listed as left out.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use or_io::decl::File;
use or_io::meta::{
    CAlternateVariations, CCreatureMetaData, CPedVariationInfo, FirstPersonAlternateData,
};

use crate::mask;
use crate::metadata::{hashed, text_of};
use crate::model::{self, Carried, Expression, Garment, Variation, Wardrobe};
use crate::names::{Component, Ped, Prop, Skin, Slot};
use crate::resolve::{resolve, Resolved, Role, Unread};
use crate::shop::{self, Offered, ShopReport};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
    #[error("{0}")]
    Model(#[from] model::Error),
    #[error("no model or texture of a freemode ped is named in {0}")]
    Nothing(String),
    #[error("{0} already holds a wardrobe.json, which an import never replaces")]
    Exists(String),
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The name of the pack; the collections' own name where they carry one.
    pub pack: Option<String>,
}

/// Why a picture that names no texture of a garment is left out, which a Log lists once for all of them.
pub const STRAY_PICTURE: &str = "an image is a texture of a garment only when its name says which, such as `jbib_diff_000_a_uni.png`";

/// A file the import did not use, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    pub path: String,
    pub why: String,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub models: usize,
    /// Models after the name of another, `<model>_1.ydd`, kept with the model they belong to.
    pub alternates: usize,
    /// Cloth files kept with the model they are named after.
    pub cloth: usize,
    pub textures: usize,
    /// Places the `.ymt` states that no model fills, held as garments with no model.
    pub reserved: usize,
    pub left_out: Vec<LeftOut>,
    /// Garments whose metadata states `ownsCloth` and that no cloth file was found for.
    pub cloth_missing: Vec<LeftOut>,
    /// What the shop metadata offered said: what resolved, what points at nothing, what was carried.
    pub shop: ShopReport,
    /// The alternate variations read: the sets they made and what they point at.
    pub switches: crate::alternate::Read,
    /// The first person alternates read.
    pub first_person: crate::firstperson::Read,
    /// The `.ymt` files the pack was read from, whose content a build writes again: one the import left out for want of a ped is not here.
    pub metadata: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Imported {
    pub wardrobe: Wardrobe,
    pub report: Report,
}

/// The extensions of a pack's files that nothing here imports or ships: fragments, which a freemode component has none of.
const UNBUILT: [&str; 1] = ["yft"];

type Key = (Ped, Slot, u32);

/// A model or a texture the resolver placed.
struct Found {
    path: String,
    read: Resolved,
}

/// Read the pack in `folder`.
pub fn import(folder: &Path, options: &Options) -> Result<Imported, Error> {
    let mut paths = Vec::new();
    walk(folder, folder, &mut paths)?;
    paths.sort();

    let mut report = Report::default();
    let mut found: Vec<Found> = Vec::new();
    let mut variations: BTreeMap<(Ped, String), (String, CPedVariationInfo)> = BTreeMap::new();
    let mut creatures: Vec<(String, CCreatureMetaData)> = Vec::new();
    for path in paths.clone() {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".ymt") {
            let bytes =
                std::fs::read(folder.join(&path)).map_err(|e| Error::Io(path.clone(), e))?;
            if let Ok(info) = CPedVariationInfo::from_bytes(&bytes) {
                match collection_of_ymt(&path) {
                    Some((ped, collection)) => {
                        variations.insert((ped, collection), (path, info));
                    }
                    None => report.left_out.push(LeftOut {
                        path,
                        why: "a variation metadata file whose name states no ped".into(),
                    }),
                }
            } else if let Ok(creature) = CCreatureMetaData::from_bytes(&bytes) {
                creatures.push((path, creature));
            } else {
                report.left_out.push(LeftOut {
                    path,
                    why: "a .ymt that is neither variation metadata nor creature metadata".into(),
                });
            }
            continue;
        }
        if !(lower.ends_with(".ydd") || lower.ends_with(".ytd") || lower.ends_with(".yld")) {
            if let Some(ext) = UNBUILT
                .into_iter()
                .find(|e| lower.ends_with(&format!(".{e}")))
            {
                report.left_out.push(LeftOut {
                    path,
                    why: format!("a .{ext} file, which a pack here neither imports nor ships"),
                });
            }
            continue;
        }
        if let Some(why) = protected(&folder.join(&path)) {
            report.left_out.push(LeftOut { path, why });
            continue;
        }
        let read = resolve(&path);
        if let Some(why) = unplaceable(&read) {
            report.left_out.push(LeftOut { path, why });
        } else {
            found.push(Found { path, read });
        }
    }
    if found.is_empty() && variations.is_empty() {
        return Err(Error::Nothing(folder.display().to_string()));
    }

    // The collection of each ped: the one most names carry.
    let mut votes: BTreeMap<Ped, BTreeMap<String, usize>> = BTreeMap::new();
    for f in &found {
        if let (Some(ped), Some(collection)) = (f.read.ped, &f.read.collection) {
            *votes
                .entry(ped)
                .or_default()
                .entry(collection.clone())
                .or_default() += 1;
        }
    }
    for (ped, collection) in variations.keys() {
        *votes
            .entry(*ped)
            .or_default()
            .entry(collection.clone())
            .or_default() += 1;
    }
    let collections: BTreeMap<Ped, String> = votes
        .iter()
        .filter_map(|(ped, c)| {
            c.iter()
                .max_by_key(|(_, n)| **n)
                .map(|(name, _)| (*ped, name.clone()))
        })
        .collect();

    let pack = options
        .pack
        .clone()
        .or_else(|| pack_of(&collections))
        .unwrap_or_else(|| "pack".to_string());
    let mut wardrobe = Wardrobe::new(&pack);
    for (ped, collection) in &collections {
        if *collection != format!("mp_{}_{pack}", ped.letter()) {
            wardrobe.collections.insert(*ped, collection.clone());
        }
    }

    // Models and textures by place.
    let mut places: BTreeMap<Key, Garment> = BTreeMap::new();
    let mut textures: BTreeMap<Key, Vec<(u8, Found)>> = BTreeMap::new();
    let mut alternates: BTreeMap<Key, Vec<(u32, String)>> = BTreeMap::new();
    let mut cloths: BTreeMap<Key, Vec<(Option<Skin>, String)>> = BTreeMap::new();
    for f in found {
        let ped = f.read.ped.expect("placeable names a ped");
        let slot = f.read.slot.expect("placeable names a slot");
        let position = f.read.position.expect("placeable names a position");
        match (&f.read.collection, collections.get(&ped)) {
            (Some(own), Some(held)) if own != held => {
                report.left_out.push(LeftOut {
                    path: f.path,
                    why: format!("it is of collection `{own}`, and the pack of {ped} is `{held}`"),
                });
                continue;
            }
            // A pack whose names carry a collection does not take a file whose name carries none: a replacement of the game's own is that.
            (None, Some(held)) => {
                report.left_out.push(LeftOut {
                    path: f.path,
                    why: format!("its name carries no collection, and the files of {ped} here are of `{held}`"),
                });
                continue;
            }
            _ => {}
        }
        let key = (ped, slot, position);
        match f.read.role {
            Some(Role::Model) if !f.read.extra.is_empty() => report.left_out.push(LeftOut {
                path: f.path,
                why: format!(
                    "a model with `{}` after its name, which nothing here builds",
                    f.read.extra.join("_")
                ),
            }),
            Some(Role::Cloth) if !f.read.extra.is_empty() || f.read.alternate.is_some() => {
                report.left_out.push(LeftOut {
                    path: f.path,
                    why: "a cloth with more after its name than a model's has, which the game does not find".into(),
                })
            }
            Some(Role::Cloth) => cloths.entry(key).or_default().push((f.read.skin, f.path)),
            Some(Role::Model) if f.read.alternate.is_some() => alternates
                .entry(key)
                .or_default()
                .push((f.read.alternate.unwrap_or_default(), f.path)),
            Some(Role::Model) => {
                if places.contains_key(&key) {
                    report.left_out.push(LeftOut {
                        path: f.path,
                        why: "a second model for one place".into(),
                    });
                    continue;
                }
                let mut g = Garment::new();
                g.ped = Some(ped);
                g.slot = Some(slot);
                g.position = Some(position);
                g.skin = f.read.skin.unwrap_or(Skin::Universal);
                g.model = Some(f.path);
                places.insert(key, g);
                report.models += 1;
            }
            Some(Role::Texture) => textures.entry(key).or_default().push((
                f.read.variant.expect("placeable texture names a variation"),
                f,
            )),
            None => {}
        }
    }
    for (key, mut list) in alternates {
        list.sort();
        match places.get_mut(&key) {
            Some(garment) => {
                report.alternates += list.len();
                garment.alternates = list.into_iter().map(|(_, path)| path).collect();
            }
            None => report
                .left_out
                .extend(list.into_iter().map(|(_, path)| LeftOut {
                    path,
                    why: "an alternate with no model at its place".into(),
                })),
        }
    }
    for (key, list) in cloths {
        let left = |path, why: &str| LeftOut {
            path,
            why: why.to_string(),
        };
        let Some(garment) = places.get_mut(&key) else {
            report.left_out.extend(
                list.into_iter()
                    .map(|(_, path)| left(path, "a cloth with no model at its place")),
            );
            continue;
        };
        let mut list = list.into_iter();
        let Some((skin, path)) = list.next() else {
            continue;
        };
        if key.1.is_prop() {
            report.left_out.push(left(
                path,
                "a prop owns no cloth, so the game does not look for one",
            ));
        } else if skin.is_some_and(|s| s != garment.skin) {
            report.left_out.push(left(
                path,
                "its name states another skin than the model's, and the game finds a cloth by the model's name",
            ));
        } else {
            report.cloth += 1;
            garment.cloth = Some(path);
        }
        report
            .left_out
            .extend(list.map(|(_, path)| left(path, "a second cloth for one place")));
    }
    for (key, mut list) in textures {
        list.sort_by_key(|(letter, f)| (*letter, f.path.clone()));
        let Some(garment) = places.get_mut(&key) else {
            report
                .left_out
                .extend(list.into_iter().map(|(_, f)| LeftOut {
                    path: f.path,
                    why: "a texture with no model at its place".into(),
                }));
            continue;
        };
        for (_, f) in list {
            report.textures += 1;
            garment.variations.push(Variation {
                texture: Some(f.path),
                tint: f.read.tint.filter(|t| *t != garment.skin.tint()),
                ..Variation::default()
            });
        }
    }

    // What the pack's own metadata states.
    for ((ped, _), (path, info)) in &variations {
        state_variation(*ped, info, &mut places, &mut report, path);
    }
    for (path, creature) in &creatures {
        state_creature(creature, &mut places, &mut report, path);
    }
    report.metadata = variations
        .values()
        .map(|(path, _)| path.clone())
        .chain(creatures.iter().map(|(path, _)| path.clone()))
        .collect();

    wardrobe.garments = places.into_values().collect();
    read_the_shop(folder, &paths, &mut wardrobe, &mut report);
    read_the_alternates(folder, &paths, &mut wardrobe, &mut report);
    keep_the_collections_whole(&mut wardrobe);
    Ok(Imported { wardrobe, report })
}

/// Whether the first kilobytes of a text file hold `needle`, which is how a metadata file names its root.
fn head_holds(path: &Path, needle: &str) -> bool {
    use std::io::Read;
    let mut head = vec![0u8; 2048];
    std::fs::File::open(path)
        .and_then(|mut file| file.read(&mut head))
        .is_ok_and(|n| String::from_utf8_lossy(&head[..n]).contains(needle))
}

/// Read the alternate variations and the first person alternates among `paths`: the sets and the numbers the pack's garments state.
///
/// ⚠ After the garments, since a source is a garment only when the pack holds one at the place the file names.
fn read_the_alternates(
    folder: &Path,
    paths: &[String],
    wardrobe: &mut Wardrobe,
    report: &mut Report,
) {
    for shown in paths {
        let lower = shown.to_ascii_lowercase();
        if !(lower.ends_with(".meta") || lower.ends_with(".xml")) {
            continue;
        }
        let path = folder.join(shown);
        let (variations, first_person) = (
            head_holds(&path, "<CAlternateVariations"),
            head_holds(&path, "<FirstPersonAlternateData"),
        );
        if !variations && !first_person {
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                report.left_out.push(LeftOut {
                    path: shown.clone(),
                    why: format!("it cannot be read: {e}"),
                });
                continue;
            }
        };
        let read = match (variations, first_person) {
            (true, _) => CAlternateVariations::from_bytes(&bytes).map(|root| {
                crate::alternate::apply(wardrobe, &root, shown, &mut report.switches);
            }),
            (_, true) => FirstPersonAlternateData::from_bytes(&bytes).map(|root| {
                crate::firstperson::apply(wardrobe, &root, shown, &mut report.first_person);
            }),
            _ => continue,
        };
        match read {
            Ok(()) => report.metadata.push(shown.clone()),
            Err(e) => report.left_out.push(LeftOut {
                path: shown.clone(),
                why: format!("it does not read as alternate variations: {e}"),
            }),
        }
    }
    report.left_out.append(&mut report.switches.left_out);
    report.left_out.append(&mut report.first_person.left_out);
}

/// A pack read is one collection of each ped, and the build writes it as it was: the cap of its slots is raised to the most it holds, up to what a collection takes.
///
/// ⚠ Without this a server's collection of 200 jackets would be written as two the next time it is built, and every saved outfit that names the old numbers would change.
fn keep_the_collections_whole(wardrobe: &mut Wardrobe) {
    let needed = (wardrobe.garments.iter())
        .filter(|g| g.ped.is_some() && g.slot.is_some())
        .filter_map(|g| g.position)
        .map(|p| p.saturating_add(1))
        .max()
        .unwrap_or(0);
    if needed > crate::settings::DEFAULT_CAP {
        let cap = u64::from(needed.min(crate::settings::MAX_CAP));
        wardrobe
            .rules
            .set(crate::settings::SLOT_CAP, |s| s.threshold = Some(cap));
    }
}

/// Read the shop metadata among `paths`, keep the ids it states and carry the files a manifest registers with the shop.
fn read_the_shop(folder: &Path, paths: &[String], wardrobe: &mut Wardrobe, report: &mut Report) {
    let offered: Vec<Offered> = paths
        .iter()
        .map(|p| Offered {
            shown: p.clone(),
            path: folder.join(p),
        })
        .collect();
    let found = shop::discover(&offered);
    // An import places a garment where its name says, so an entry finds it by the number it states.
    let at = |w: &Wardrobe, i: usize| {
        let g = &w.garments[i];
        Some((g.ped?, g.slot?, g.position?))
    };
    report.shop = shop::apply(wardrobe, &found.shops, &at);
    report.shop.unread.extend(found.unread);
    report.metadata.extend(found.consumed);
    for (file, line) in found.carried {
        report.shop.carried.push(LeftOut {
            path: file.clone(),
            why: format!(
                "registered as {} by `{}`, carried unchanged",
                line.kind, line.directive
            ),
        });
        wardrobe.shop.carried.push(Carried {
            file,
            directive: line.directive,
            kind: line.kind,
        });
    }
}

/// A garment by its model, or by its place where it holds one with none.
fn place_name(garment: &Garment, ped: Ped, slot: Slot, position: u32) -> String {
    garment
        .model
        .clone()
        .unwrap_or_else(|| format!("{ped} {slot} {position:03}"))
}

/// Write the import beside its files as `wardrobe.json`, never over one.
pub fn write(folder: &Path, imported: &Imported) -> Result<(), Error> {
    if folder.join(model::FILE).exists() {
        return Err(Error::Exists(folder.display().to_string()));
    }
    imported.wardrobe.write(folder).map_err(Error::from)
}

/// The directories an import does not look in: the project's own, and a source.
fn skipped(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "build" | "vfs") || name.ends_with(".src")
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), Error> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::Io(dir.display().to_string(), e))?;
    for entry in entries.flatten() {
        let path: PathBuf = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if skipped(&name) {
            continue;
        }
        // A link is neither: it may lead out of the folder, or back into it.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            walk(base, &path, out)?;
        } else if kind.is_file() {
            if let Ok(rel) = path.strip_prefix(base) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    Ok(())
}

/// Why a file is not read when it opens with `FXAP`, the container of a protected asset, and not with an RSC7 header; none when it does not.
pub(crate) fn protected(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .ok()?;
    (&magic == or_project::resource::ESCROW_MARKER).then(|| "a protected asset: it opens with FXAP and not with an RSC7 header, and OpenRage does not read protected assets".to_string())
}

/// Why a name cannot be placed in a pack, or none.
fn unplaceable(read: &Resolved) -> Option<String> {
    let lacks: Vec<&str> = read
        .unread
        .iter()
        .map(|u| match u {
            Unread::Role => "what kind of file it is",
            Unread::Ped => "the gender",
            Unread::Slot => "the slot",
            Unread::Position => "the number",
            Unread::Variant => "the letter of the texture",
        })
        .collect();
    (!lacks.is_empty()).then(|| format!("its name does not state {}", lacks.join(", ")))
}

/// The ped and collection a variation metadata file's name states: `mp_m_freemode_01_<collection>.ymt`.
fn collection_of_ymt(path: &str) -> Option<(Ped, String)> {
    let file = path.rsplit('/').next()?.to_ascii_lowercase();
    let lower = file.strip_suffix(".ymt")?;
    Ped::ALL.into_iter().find_map(|ped| {
        lower
            .strip_prefix(&format!("{}_", ped.model()))
            .filter(|c| !c.is_empty())
            .map(|c| (ped, c.to_string()))
    })
}

/// The pack both peds' collections name: `mp_m_<pack>` and `mp_f_<pack>`.
fn pack_of(collections: &BTreeMap<Ped, String>) -> Option<String> {
    let packs: Vec<&str> = collections
        .iter()
        .filter_map(|(ped, c)| c.strip_prefix(&format!("mp_{}_", ped.letter())))
        .collect();
    let first = packs.first()?;
    (packs.len() == collections.len() && packs.iter().all(|p| p == first))
        .then(|| (*first).to_string())
}

fn differs<T: PartialEq>(value: T, default: T) -> Option<T> {
    (value != default).then_some(value)
}

fn mods(value: &[f32]) -> Option<[f32; 5]> {
    let held: [f32; 5] = value.try_into().ok()?;
    (held != [0.0; 5]).then_some(held)
}

fn audio(value: &or_io::meta::instance::HashedString) -> Option<String> {
    differs(value.hash, hashed("NONE").hash).map(|_| text_of(value))
}

/// Keep what a variation metadata states of the drawables the pack has a place for, and hold a place for each it does not.
fn state_variation(
    ped: Ped,
    info: &CPedVariationInfo,
    places: &mut BTreeMap<Key, Garment>,
    report: &mut Report,
    path: &str,
) {
    for component in Component::ALL {
        let Some(&data) = info.avail_comp.get(usize::from(component.id())) else {
            continue;
        };
        let Some(data) = info.a_component_data3.get(usize::from(data)) else {
            continue;
        };
        let slot = Slot::Component(component);
        for (position, drawable) in data.a_drawbl_data3.iter().enumerate() {
            let position = position as u32;
            let garment = places.entry((ped, slot, position)).or_insert_with(|| {
                report.reserved += 1;
                let mut g = Garment::new();
                g.ped = Some(ped);
                g.slot = Some(slot);
                g.position = Some(position);
                g
            });
            if let Some(skin) = drawable.a_tex_data.first().map(|t| t.tex_id) {
                if garment.is_reserved() {
                    garment.skin = if skin == Skin::Race.tex_id() {
                        Skin::Race
                    } else {
                        Skin::Universal
                    };
                }
            }
            garment.meta.prop_mask = differs(drawable.prop_mask, mask::default_for(garment.skin));
            let alternates = u8::try_from(garment.alternates.len()).unwrap_or(u8::MAX);
            garment.meta.num_alternatives = differs(drawable.num_alternatives, alternates);
            // ⚠ A garment has cloth when one is linked, and `ownsCloth` is written from that. So a cloth the metadata did not state is not linked (the game did not use it, and it stays a loose file), and a flag with no cloth file is a garment missing its cloth.
            match (drawable.cloth_data.owns_cloth, garment.cloth.is_some()) {
                (true, false) => {
                    garment.missing_cloth = true;
                    report.cloth_missing.push(LeftOut {
                        path: place_name(garment, ped, slot, position),
                        why: format!(
                            "{path} states ownsCloth and no cloth file is beside the model"
                        ),
                    });
                }
                (false, true) => {
                    let ignored = garment.cloth.take().unwrap_or_default();
                    report.cloth = report.cloth.saturating_sub(1);
                    report.left_out.push(LeftOut {
                        path: ignored,
                        why: format!("{path} does not state ownsCloth, so the game does not use this cloth: Link it in the Garment panel to turn it on"),
                    });
                }
                _ => {}
            }
            let held = garment.variations.len();
            if garment.is_reserved() {
                garment.variations = drawable
                    .a_tex_data
                    .iter()
                    .map(|_| Variation::default())
                    .collect();
            } else if held != drawable.a_tex_data.len() {
                report.left_out.push(LeftOut {
                    path: path.to_string(),
                    why: format!(
                        "{} {position:03} states {} textures and the pack has {held} files for it",
                        slot.name(),
                        drawable.a_tex_data.len()
                    ),
                });
            }
            let skin = garment.skin;
            for (v, t) in garment.variations.iter_mut().zip(&drawable.a_tex_data) {
                v.tex_id = differs(t.tex_id, skin.tex_id());
                v.distribution = differs(t.distribution, 255);
            }
            let component_info = info.comp_infos.iter().find(|i| {
                i.ped_xml_comp_idx == component.id() && u32::from(i.ped_xml_drawbl_idx) == position
            });
            if let Some(i) = component_info {
                let m = &mut garment.meta;
                m.ped_xml_audio_id = audio(&i.ped_xml_audio_id);
                m.ped_xml_audio_id2 = audio(&i.ped_xml_audio_id2);
                m.ped_xml_expression_mods = mods(&i.ped_xml_expression_mods);
                m.flags = differs(i.flags, 0);
                m.inclusions = differs(i.inclusions, 0);
                m.exclusions = differs(i.exclusions, 0);
                m.ped_xml_vfx_comps = differs(i.ped_xml_vfx_comps, 0);
                m.ped_xml_flags = differs(i.ped_xml_flags, 0);
            }
        }
    }
    for meta in &info.prop_info.a_prop_meta_data {
        let (Some(prop), position) = (Prop::from_anchor(meta.anchor_id), u32::from(meta.prop_id))
        else {
            // ⚠ Anchors 9 to 12 (the feet and the two placeholder hands) are named by the documents' tables and stated by no staged document, so no slot here holds them.
            report.left_out.push(LeftOut {
                path: path.to_string(),
                why: format!(
                    "a prop at anchor {} ({}), which no staged document states and no slot here places",
                    meta.anchor_id,
                    or_io::meta::Anchor(meta.anchor_id).name().unwrap_or("not an anchor of the engine's")
                ),
            });
            continue;
        };
        let slot = Slot::Prop(prop);
        let garment = places.entry((ped, slot, position)).or_insert_with(|| {
            report.reserved += 1;
            let mut g = Garment::new();
            g.ped = Some(ped);
            g.slot = Some(slot);
            g.position = Some(position);
            g
        });
        if garment.is_reserved() {
            garment.variations = meta.tex_data.iter().map(|_| Variation::default()).collect();
        }
        for (i, (v, t)) in garment
            .variations
            .iter_mut()
            .zip(&meta.tex_data)
            .enumerate()
        {
            v.tex_id = differs(t.tex_id, u8::try_from(i).unwrap_or(u8::MAX));
            v.distribution = differs(t.distribution, 255);
            v.inclusions = differs(t.inclusions, 0);
            v.exclusions = differs(t.exclusions, 0);
            v.inclusion_id = differs(t.inclusion_id, 0);
            v.exclusion_id = differs(t.exclusion_id, 0);
        }
        let m = &mut garment.meta;
        m.audio_id = audio(&meta.audio_id);
        m.expression_mods = mods(&meta.expression_mods);
        m.render_flags = differs(meta.render_flags, 0);
        m.prop_flags = differs(meta.prop_flags, 0);
        m.flags = differs(u32::from(meta.flags), 0);
        m.stickyness = differs(meta.stickyness, 0);
    }
}

/// Give each expression the garments at its place, of both peds, because the file is shared.
fn state_creature(
    creature: &CCreatureMetaData,
    places: &mut BTreeMap<Key, Garment>,
    report: &mut Report,
    path: &str,
) {
    let mut put = |slot: Option<Slot>, position: i32, e: Expression| {
        let mut placed = false;
        if let (Some(slot), Ok(position)) = (slot, u32::try_from(position)) {
            for ped in Ped::ALL {
                if let Some(g) = places.get_mut(&(ped, slot, position)) {
                    g.expressions.push(e.clone());
                    placed = true;
                }
            }
        }
        if !placed {
            report.left_out.push(LeftOut {
                path: path.to_string(),
                why: format!("an expression for a drawable the pack lacks (slot {slot:?}, position {position})"),
            });
        }
    };
    for c in &creature.ped_comp_expressions {
        let slot = u8::try_from(c.ped_comp_id)
            .ok()
            .and_then(Component::from_id)
            .map(Slot::Component);
        put(
            slot,
            c.ped_comp_var_index,
            Expression {
                index: c.ped_comp_expression_index,
                tracks: c.tracks.clone(),
                ids: c.ids.clone(),
                types: c.types.clone(),
                components: c.components.clone(),
            },
        );
    }
    for p in &creature.ped_prop_expressions {
        let slot = u8::try_from(p.ped_prop_id)
            .ok()
            .and_then(Prop::from_anchor)
            .map(Slot::Prop);
        put(
            slot,
            p.ped_prop_var_index,
            Expression {
                index: p.ped_prop_expression_index,
                tracks: p.tracks.clone(),
                ids: p.ids.clone(),
                types: p.types.clone(),
                components: p.components.clone(),
            },
        );
    }
}

#[cfg(test)]
mod escrow_tests {
    /// ⚠ The crates below or-project cannot name its marker, so the one or-source spells is held to it here, where both are in reach.
    #[test]
    fn every_reader_of_a_locked_file_knows_it_by_the_same_bytes() {
        assert_eq!(
            or_source::models::PROTECTED,
            or_project::resource::ESCROW_MARKER.as_slice()
        );
        let dir = tempfile::tempdir().unwrap();
        let sealed = dir.path().join("sealed.ydd");
        let bytes = [or_source::models::PROTECTED, b" and what follows"].concat();
        std::fs::write(&sealed, bytes).unwrap();
        assert!(super::protected(&sealed).is_some());
        std::fs::write(&sealed, b"RSC7 and what follows").unwrap();
        assert!(super::protected(&sealed).is_none());
    }
}
