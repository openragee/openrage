//! Every shipped archetype whose drawable the mount can find, held against it:
//! the oracle for each field `or_world::generate` derives.
//!
//! Point `OPENRAGE_GAME_ROOT` at one or more install roots, `;`-separated.
//! `OPENRAGE_ARCHETYPE_LIMIT` caps how many archetypes an install compares.

mod support;

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use glam::Vec3;
use or_io::container::resource::{ResourceEnvelope, Rsc7};
use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_io::decl::{File, Gen8, Gen9};
use or_io::gta5::{GtaDrawable, GtaFragType, PgDictionary};
use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::Diagnostics;
use or_io::Generation;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_world::generate::{Agreement, AssetKind, Closeness, DrawableFacts};
use or_world::map_types::{ArchetypeKind, MapTypes};

const WHAT: &str = "what_an_archetype_takes_from_its_drawable";

/// What one drawable says, plus the LOD distances the archetype's `lodDist`
/// is compared with.
#[derive(Clone)]
struct Read {
    facts: DrawableFacts,
    lod_distances: [f32; 4],
    /// A fragment's own sphere, beside its common drawable's.
    fragment_sphere: Option<(Vec3, f32)>,
}

macro_rules! read_of {
    ($lod:expr, $name:expr, $kind:expr, $dict:expr) => {{
        let g = &$lod;
        let b = &g.bounds_box;
        let s = &g.bounds_sphere.center_and_radius;
        Read {
            facts: DrawableFacts {
                name: $name,
                kind: $kind,
                dictionary: $dict,
                bb_min: Vec3::new(b.min.x, b.min.y, b.min.z),
                bb_max: Vec3::new(b.max.x, b.max.y, b.max.z),
                bs_centre: Vec3::new(s.x, s.y, s.z),
                bs_radius: s.w,
            },
            lod_distances: g.lod_distances,
            fragment_sphere: None,
        }
    }};
}

/// Every drawable a file holds, keyed by entry hash (0 for a single drawable).
fn read_file(ext: &str, bytes: &[u8]) -> Result<Vec<(u32, Read)>, String> {
    let resource = match ext {
        "ydr" => or_io::container::resource::ResourceType::Ydr,
        "ydd" => or_io::container::resource::ResourceType::Ydd,
        _ => or_io::container::resource::ResourceType::Yft,
    };
    let gen9 = Rsc7::peek(bytes)
        .ok()
        .and_then(|header| resource.generation_of_version(header.version()))
        == Some(Generation::Gen9);
    let e = |e: or_io::Error| e.to_string();
    macro_rules! dictionary {
        ($t:ty) => {{
            let dict = PgDictionary::<GtaDrawable<$t>>::from_bytes(bytes).map_err(e)?;
            let kind = AssetKind::DrawableDictionary;
            dict.codes
                .iter()
                .zip(&dict.entries)
                .filter_map(|(h, d)| {
                    let d = d.as_deref()?;
                    Some((*h, read_of!(d.lod_group, String::new(), kind, None)))
                })
                .collect()
        }};
    }
    macro_rules! fragment {
        ($t:ty) => {{
            let f = GtaFragType::<$t>::from_bytes(bytes).map_err(e)?;
            let Some(d) = f.common_drawable.as_deref() else {
                return Ok(Vec::new());
            };
            let mut r = read_of!(d.lod_group, String::new(), AssetKind::Fragment, None);
            let s = &f.bounding_sphere;
            r.fragment_sphere = Some((Vec3::new(s.x, s.y, s.z), s.w));
            vec![(0, r)]
        }};
    }
    Ok(match (ext, gen9) {
        ("ydr", false) => {
            let d = GtaDrawable::<Gen8>::from_bytes(bytes).map_err(e)?;
            vec![(
                0,
                read_of!(d.lod_group, String::new(), AssetKind::Drawable, None),
            )]
        }
        ("ydr", true) => {
            let d = GtaDrawable::<Gen9>::from_bytes(bytes).map_err(e)?;
            vec![(
                0,
                read_of!(d.lod_group, String::new(), AssetKind::Drawable, None),
            )]
        }
        ("ydd", false) => dictionary!(Gen8),
        ("ydd", true) => dictionary!(Gen9),
        (_, false) => fragment!(Gen8),
        (_, true) => fragment!(Gen9),
    })
}

#[derive(Default)]
struct PerKind {
    archetypes: usize,
    no_file: usize,
    no_entry: usize,
    unreadable: usize,
    compared: usize,
    bounds_agree: usize,
    /// The winning copy disagreed and an overridden one agreed.
    another_copy_agrees: usize,
    /// Neither agreed, and the box has the drawable's extent at another place.
    displaced: usize,
    several_copies: usize,
    closeness: BTreeMap<(&'static str, Closeness), usize>,
    name_is_asset: usize,
    txd_is_asset: usize,
    /// `lodDist` against the drawable's four LOD distances.
    lod_dist: BTreeMap<&'static str, usize>,
    fragment_sphere_agrees: usize,
    disagreeing: Vec<String>,
}

#[derive(Default)]
struct Census {
    installs: Vec<String>,
    type_files: usize,
    refused: Vec<String>,
    unread_assets: Vec<String>,
    kinds: BTreeMap<AssetKind, PerKind>,
    unstated_kind: usize,
}

impl Census {
    fn per(&mut self, kind: AssetKind) -> &mut PerKind {
        self.kinds.entry(kind).or_default()
    }

    fn compared(&self) -> usize {
        self.kinds.values().map(|k| k.compared).sum()
    }

    fn bounds_agree(&self) -> usize {
        self.kinds.values().map(|k| k.bounds_agree).sum()
    }

    fn report(&self) -> String {
        let mut out = format!(
            "installs: {}\n.ytyp read {}, refused {}; archetypes with no stated asset kind {}\n",
            self.installs.join(", "),
            self.type_files,
            self.refused.len(),
            self.unstated_kind,
        );
        for (kind, k) in &self.kinds {
            out.push_str(&format!(
                "{kind:?}: {} archetypes; no file {}, no entry {}, unreadable {}; compared {} \
                 ({} with more than one copy), bounds within 1 cm {}, only an overridden copy \
                 agrees {}, same extent elsewhere {}; name = assetName {}, txd = assetName {}\n",
                k.archetypes,
                k.no_file,
                k.no_entry,
                k.unreadable,
                k.compared,
                k.several_copies,
                k.bounds_agree,
                k.another_copy_agrees,
                k.displaced,
                k.name_is_asset,
                k.txd_is_asset,
            ));
            for ((field, c), n) in &k.closeness {
                out.push_str(&format!("    {field:<9} {c:?}: {n}\n"));
            }
            for (rel, n) in &k.lod_dist {
                out.push_str(&format!("    lodDist {rel}: {n}\n"));
            }
            if *kind == AssetKind::Fragment {
                out.push_str(&format!(
                    "    fragment sphere agrees with bsCentre/bsRadius: {}\n",
                    k.fragment_sphere_agrees
                ));
            }
            for d in &k.disagreeing {
                out.push_str(&format!("    e.g. {d}\n"));
            }
        }
        for u in self.unread_assets.iter().take(10) {
            out.push_str(&format!("unreadable: {u}\n"));
        }
        out
    }

    fn require_exercised(&self, what: &str, got: usize, floor: usize) {
        assert!(
            got >= floor,
            "{what}: {got} is under the floor of {floor}, so this check passed \
             without exercising anything.\n{}",
            self.report()
        );
    }
}

fn crypto_for(root: &Path) -> Option<Arc<dyn RpfCryptoProvider>> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    Some(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))))
}

type Cache = HashMap<String, Option<Vec<(u32, Read)>>>;

fn read_cached<'c>(
    cache: &'c mut Cache,
    game: &or_title::MountedGame,
    ext: &str,
    path: &str,
    unread: &mut Vec<String>,
) -> Option<&'c Vec<(u32, Read)>> {
    cache
        .entry(path.to_string())
        .or_insert_with(|| {
            let bytes = game.vfs.open(path)?;
            match read_file(ext, &bytes) {
                Ok(r) => Some(r),
                Err(e) => {
                    unread.push(format!("{path}: {e}"));
                    None
                }
            }
        })
        .as_ref()
}

fn lod_relation(lod_dist: f32, distances: [f32; 4]) -> &'static str {
    const NAMES: [&str; 4] = ["high", "medium", "low", "very low"];
    for (i, d) in distances.iter().enumerate() {
        if *d == lod_dist {
            return NAMES[i];
        }
    }
    "none of the four"
}

fn census() -> Option<Census> {
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(
            WHAT,
            format_args!("no install root; set {GAME_ROOT_ENV} to a ;-separated list"),
        );
        return None;
    }
    let limit: Option<usize> = std::env::var("OPENRAGE_ARCHETYPE_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok());
    let library = support::schema();
    let mut census = Census::default();

    for root in &roots {
        let Some(provider) = crypto_for(root) else {
            continue;
        };
        let Some(install) = or_title::GtaInstall::from_path(root) else {
            continue;
        };
        let Ok(game) = or_title::build_mounted_game(&install, provider, or_title::DlcCutoff::All)
        else {
            continue;
        };
        census.installs.push(root.display().to_string());

        // Every copy of each name is kept, in the order the mount lists them,
        // so a disagreement can be told apart from having read the wrong copy.
        let mut assets: HashMap<(&'static str, u32), Vec<String>> = HashMap::new();
        let mut type_files: BTreeMap<String, String> = BTreeMap::new();
        game.for_each_file(|logical, _container, leaf| {
            let lower = leaf.to_ascii_lowercase();
            let Some((stem, ext)) = lower.rsplit_once('.') else {
                return;
            };
            let ext: &'static str = match ext {
                "ydr" => "ydr",
                "ydd" => "ydd",
                "yft" => "yft",
                "ytyp" => {
                    type_files.insert(lower.clone(), logical.to_string());
                    return;
                }
                _ => return,
            };
            assets
                .entry((ext, or_io::string_hash(stem.as_bytes())))
                .or_default()
                .push(logical.to_string());
        });

        let mut cache: Cache = HashMap::new();
        let mut compared_here = 0usize;
        'files: for logical in type_files.values() {
            let Some(bytes) = game.vfs.open(logical) else {
                census.refused.push(format!("{logical}: not in the mount"));
                continue;
            };
            let mut diags = Diagnostics::lenient();
            let root = match parse_meta_binary_full(library, &bytes, &mut diags) {
                Ok((root, _)) => root,
                Err(e) => {
                    census.refused.push(format!("{logical}: {e}"));
                    continue;
                }
            };
            let Some(types) = MapTypes::new(&root) else {
                continue;
            };
            census.type_files += 1;
            for arch in types.archetypes() {
                if arch.kind() == ArchetypeKind::Mlo {
                    continue;
                }
                let Some(kind) = AssetKind::of(&arch) else {
                    census.unstated_kind += 1;
                    continue;
                };
                let (ext, file_hash, entry) = match kind {
                    AssetKind::Drawable => ("ydr", arch.asset_name(), 0),
                    AssetKind::Fragment => ("yft", arch.asset_name(), 0),
                    AssetKind::DrawableDictionary => {
                        ("ydd", arch.drawable_dictionary(), arch.asset_name())
                    }
                    AssetKind::Assetless => continue,
                };
                census.per(kind).archetypes += 1;
                let Some(copies) = assets.get(&(ext, file_hash)).cloned() else {
                    census.per(kind).no_file += 1;
                    continue;
                };
                let winner = copies.last().expect("a name is only indexed with a copy");
                let Some(reads) =
                    read_cached(&mut cache, &game, ext, winner, &mut census.unread_assets)
                else {
                    census.per(kind).unreadable += 1;
                    continue;
                };
                let Some((_, read)) = reads.iter().find(|(h, _)| *h == entry) else {
                    census.per(kind).no_entry += 1;
                    continue;
                };
                let read = read.clone();
                let Some(agreement) = Agreement::of(&arch, &read.facts) else {
                    continue;
                };
                let same_extent = arch.bounds().is_some_and(|b| {
                    Closeness::of(b.max - b.min, read.facts.bb_max - read.facts.bb_min)
                        <= Closeness::Centimetre
                });
                let mut another_copy_agrees = false;
                if !agreement.bounds_agree() {
                    for other in &copies[..copies.len() - 1] {
                        let found =
                            read_cached(&mut cache, &game, ext, other, &mut census.unread_assets)
                                .and_then(|r| r.iter().find(|(h, _)| *h == entry).cloned());
                        if let Some((_, r)) = found {
                            if Agreement::of(&arch, &r.facts).is_some_and(|a| a.bounds_agree()) {
                                another_copy_agrees = true;
                                break;
                            }
                        }
                    }
                }
                let per = census.per(kind);
                per.compared += 1;
                per.several_copies += usize::from(copies.len() > 1);
                if agreement.bounds_agree() {
                    per.bounds_agree += 1;
                } else if another_copy_agrees {
                    per.another_copy_agrees += 1;
                } else if same_extent {
                    per.displaced += 1;
                } else if per.disagreeing.len() < 5 {
                    per.disagreeing.push(format!(
                        "{logical} {:#010X}: stored {:?}, drawable {:?}",
                        arch.name(),
                        arch.bounds(),
                        read.facts
                    ));
                }
                for (field, c) in [
                    ("bbMin", agreement.bb_min),
                    ("bbMax", agreement.bb_max),
                    ("bsCentre", agreement.bs_centre),
                    ("bsRadius", agreement.bs_radius),
                ] {
                    *per.closeness.entry((field, c)).or_default() += 1;
                }
                per.name_is_asset += usize::from(agreement.name_is_asset);
                per.txd_is_asset += usize::from(agreement.txd_is_asset);
                *per.lod_dist
                    .entry(lod_relation(arch.lod_dist(), read.lod_distances))
                    .or_default() += 1;
                if let (Some((centre, radius)), Some(bounds)) =
                    (read.fragment_sphere, arch.bounds())
                {
                    let agrees = Closeness::of(bounds.centre, centre) <= Closeness::Centimetre
                        && (bounds.radius - radius).abs() <= 0.01;
                    per.fragment_sphere_agrees += usize::from(agrees);
                }
                compared_here += 1;
                if limit.is_some_and(|l| compared_here >= l) {
                    break 'files;
                }
            }
        }
    }

    if census.installs.is_empty() {
        skip_install(WHAT, "no install root mounted");
        return None;
    }
    Some(census)
}

/// The claims `or_world::generate` rests on, each over what was compared.
///
/// Measured over both retail installs: 317,514 archetypes compared out of
/// 3,414 type files, every drawable they name found and read. `name` is
/// `assetName` on all of them. The drawable's box is within a centimetre of the stored one on 179,750 — 6,671 of 8,977
/// fragments, 92,816 of 154,848 drawables, 80,263 of 153,689 dictionary
/// entries — and 49,366 more store the drawable's extent displaced, where the
/// archetype's frame is not the drawable's and nothing in the drawable says
/// so. Only 19 disagreements are resolved by reading an overridden copy, so
/// the mount's choice of copy is not the cause. `lodDist` equals one of a
/// drawable's four LOD distances on 10 of 308,537 non-fragment archetypes,
/// which is why it is an authoring decision.
#[test]
fn what_an_archetype_takes_from_its_drawable() {
    let Some(census) = census() else {
        return;
    };
    eprintln!("{}", census.report());
    // One edition compares about 158,000; the floor sits under that so a
    // single install still runs and an empty mount cannot pass.
    census.require_exercised("comparing an archetype", census.compared(), 100_000);
    assert!(census.refused.is_empty(), "{}", census.report());
    assert!(
        census.unread_assets.is_empty(),
        "a drawable a shipped archetype names did not read\n{}",
        census.report()
    );

    let named_as_asset: usize = census.kinds.values().map(|k| k.name_is_asset).sum();
    assert_eq!(
        named_as_asset,
        census.compared(),
        "an archetype's name is its asset's, which is what the generator writes\n{}",
        census.report()
    );

    assert!(
        census.bounds_agree() * 2 > census.compared(),
        "the drawable's own box is what most shipped archetypes carry, and the \
         generator copies it\n{}",
        census.report()
    );

    let (lod_equal, lod_total) = census
        .kinds
        .iter()
        .filter(|(k, _)| **k != AssetKind::Fragment)
        .flat_map(|(_, v)| v.lod_dist.iter())
        .fold((0, 0), |(eq, all), (rel, n)| {
            (
                eq + if *rel == "none of the four" { 0 } else { *n },
                all + n,
            )
        });
    assert!(
        lod_equal * 100 < lod_total,
        "lodDist followed a drawable LOD distance on {lod_equal} of {lod_total}; it is \
         treated as an authoring decision because it almost never does\n{}",
        census.report()
    );
}

#[test]
#[cfg_attr(
    all(windows, debug_assertions),
    ignore = "Cranelift cannot unwind a panic on Windows: run it with `cargo test --release`"
)]
fn the_floor_rejects_a_census_that_compared_nothing() {
    let hollow = Census::default();
    let rejected = std::panic::catch_unwind(|| {
        hollow.require_exercised("control", hollow.compared(), 100_000)
    })
    .is_err();
    assert!(rejected, "an empty census passed its own floor");
}
