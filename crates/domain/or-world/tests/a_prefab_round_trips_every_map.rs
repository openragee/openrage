//! Every map a retail install ships, captured as a prefab and placed back,
//! at its own origin and turned somewhere else, and read back from the bytes.
//!
//! Point `OPENRAGE_GAME_ROOT` at one or more install roots, `;`-separated.
//! `OPENRAGE_EXEC_STRIDE=N` takes every Nth map.
//!
//! The turned placement checks the quaternion asymmetry on shipped rotations
//! rather than on a quarter turn. A catalogue is one title's: the two
//! installs' entity member sets differ, so a prefab of one is refused by a
//! map of the other.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use glam::{Quat, Vec3};
use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::{Diagnostics, MetaInstance, Value};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_world::map_data::{Entity, MapData};
use or_world::prefab::{self, same_rotation, Catalogue, EntityShape, PrefabError};

const WHAT: &str = "a_prefab_round_trips_every_map";

/// A centimetre: `(p - o) + o` in `f32` at 8 km from the origin is a few
/// tenths of a millimetre off.
const CM: f32 = 0.01;

#[derive(Default)]
struct Report {
    label: String,
    stride: usize,
    maps: u64,
    shapes: BTreeMap<usize, u64>,
    duplicate_guids: u64,
    captured: u64,
    entities: u64,
    in_place: u64,
    turned: u64,
    guids_unique: u64,
    read_back: u64,
    write_refused: BTreeMap<String, u64>,
    catalogued: u64,
    catalogue_refused: Vec<String>,
    catalogue_shape: Option<EntityShape>,
    failures: Vec<String>,
    variants: Vec<Variant>,
    sample: Option<MetaInstance>,
}

/// One entity member set and the maps carrying it.
struct Variant {
    shape: EntityShape,
    maps: u64,
    formats: BTreeSet<String>,
    example: String,
}

impl Report {
    /// The member set most of the install's maps carry.
    fn shape(&self) -> Option<&EntityShape> {
        self.variants
            .iter()
            .max_by_key(|v| v.maps)
            .map(|v| &v.shape)
    }

    fn variants_text(&self) -> String {
        let main = self.shape();
        self.variants
            .iter()
            .map(|v| {
                format!(
                    "{} maps ({:?}){}",
                    v.maps,
                    v.formats,
                    match main {
                        Some(m) if std::ptr::eq(m, &v.shape) => String::new(),
                        Some(m) => format!(
                            ", {} member(s) apart from the main set, e.g. {}",
                            m.differences(&v.shape).len(),
                            v.example
                        ),
                        None => String::new(),
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn text(&self) -> String {
        format!(
            "{} (every {} map)\n  {} maps with entities; entity member sets {:?} (members: maps), \
             as variants: {}\n  {} carry a duplicate non-zero guid\n  {} captured ({} entities); placed at their origin \
             exactly on {}, turned on {}; guids unique after two placements on {}; read back from \
             the bytes on {}; the writer refused {:?}\n  {} filed in one catalogue, {} refused by it as another member \
             set's: {:?}\n  failures {}:\n    {}",
            self.label,
            self.stride,
            self.maps,
            self.shapes,
            self.variants_text(),
            self.duplicate_guids,
            self.captured,
            self.entities,
            self.in_place,
            self.turned,
            self.guids_unique,
            self.read_back,
            self.write_refused,
            self.catalogued,
            self.catalogue_refused.len(),
            self.catalogue_refused,
            self.failures.len(),
            self.failures.iter().take(12).cloned().collect::<Vec<_>>().join("\n    "),
        )
    }
}

fn crypto_for(root: &Path) -> Option<Arc<dyn RpfCryptoProvider>> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    Some(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))))
}

/// The map's own fields with nothing in any array, built without copying one.
fn empty_like(root: &MetaInstance) -> MetaInstance {
    let mut out = MetaInstance::new(root.type_name.clone());
    for (name, value) in &root.fields {
        out.fields.push((name.clone(), prefab::emptied(value)));
    }
    out
}

fn entities(root: &MetaInstance) -> Vec<Entity<'_>> {
    MapData::new(root)
        .map(|m| m.entities().collect())
        .unwrap_or_default()
}

fn measure(root: &Path, stride: usize) -> Option<Report> {
    let provider = crypto_for(root)?;
    let install = or_title::GtaInstall::from_path(root)?;
    let game = or_title::build_mounted_game(&install, provider, or_title::DlcCutoff::All).ok()?;
    let mut winners: BTreeMap<String, String> = BTreeMap::new();
    game.for_each_file(|logical, _container, leaf| {
        let lower = leaf.to_ascii_lowercase();
        if lower.ends_with(".ymap") {
            winners.insert(lower, logical.to_string());
        }
    });

    let mut report = Report {
        label: root.display().to_string(),
        stride,
        ..Report::default()
    };
    let mut catalogue = Catalogue::new(root.display().to_string());
    let turn = 0.7f32;
    let rotation = Quat::from_rotation_z(turn);

    let mut with_entities = 0usize;
    for path in winners.values() {
        let Some(bytes) = game.vfs.open(path) else {
            continue;
        };
        let mut diags = Diagnostics::lenient();
        let Ok((source, layout)) = parse_meta_binary_full(support::schema(), &bytes, &mut diags)
        else {
            continue;
        };
        let Some(map) = MapData::new(&source) else {
            continue;
        };
        if map.entity_count() == 0 {
            continue;
        }
        report.maps += 1;
        with_entities += 1;
        if let Some(shape) = EntityShape::of_map(&source) {
            *report.shapes.entry(shape.len()).or_default() += 1;
            let format = layout.format.label().to_string();
            match report
                .variants
                .iter_mut()
                .find(|v| v.shape.agrees_with(&shape))
            {
                Some(v) => {
                    v.shape.absorb(&shape);
                    v.maps += 1;
                    v.formats.insert(format);
                }
                None => report.variants.push(Variant {
                    shape,
                    maps: 1,
                    formats: BTreeSet::from([format]),
                    example: path.clone(),
                }),
            }
        }
        let guids: Vec<u32> = map
            .entities()
            .map(|e| e.guid())
            .filter(|g| *g != 0)
            .collect();
        if guids.iter().collect::<BTreeSet<_>>().len() != guids.len() {
            report.duplicate_guids += 1;
        }
        if !(with_entities - 1).is_multiple_of(stride) {
            continue;
        }

        let selection = prefab::capturable(&source);
        if selection.is_empty() {
            continue;
        }
        let src: Vec<Entity<'_>> = map.entities().collect();
        let origin = src[selection[0]].position();
        let captured = match prefab::capture(&source, &selection, map.name(), origin) {
            Ok(p) => p,
            Err(e) => {
                report.failures.push(format!("{path}: capture: {e}"));
                continue;
            }
        };
        report.captured += 1;
        report.entities += selection.len() as u64;
        match catalogue.add(path.clone(), &captured) {
            Ok(()) => report.catalogued += 1,
            Err(PrefabError::WrongCatalogue(_)) => report.catalogue_refused.push(path.clone()),
            Err(e) => report.failures.push(format!("{path}: catalogue: {e}")),
        }
        if report.sample.is_none()
            && selection.len() <= 16
            && selection.iter().all(|i| !src[*i].is_interior_instance())
        {
            report.sample = Some(captured.clone());
        }

        let position_in_selection: BTreeMap<usize, usize> =
            selection.iter().enumerate().map(|(k, i)| (*i, k)).collect();
        let mut target = empty_like(&source);
        if let Err(e) = prefab::place(&mut target, &captured, origin, 0.0) {
            report.failures.push(format!("{path}: place: {e}"));
            continue;
        }
        let back = entities(&target);
        let in_place = back.len() == selection.len()
            && selection.iter().enumerate().all(|(k, i)| {
                let (a, b) = (&src[*i], &back[k]);
                a.archetype() == b.archetype()
                    && a.position().abs_diff_eq(b.position(), CM)
                    && same_rotation(a.rotation(), b.rotation(), 1e-5)
                    && b.parent_index() == a.parent_index().map(|p| position_in_selection[&p])
            });
        if in_place {
            report.in_place += 1;
        } else {
            report.failures.push(format!("{path}: not back in place"));
        }

        let at = origin + Vec3::new(5.0, 5.0, 0.0);
        let placed = match prefab::place(&mut target, &captured, at, turn) {
            Ok(p) => p,
            Err(e) => {
                report.failures.push(format!("{path}: turned place: {e}"));
                continue;
            }
        };
        let back = entities(&target);
        let turned = selection.iter().enumerate().all(|(k, i)| {
            let (a, b) = (&src[*i], &back[placed.base + k]);
            let want = rotation * (a.position() - origin) + at;
            b.position().abs_diff_eq(want, CM)
                && same_rotation(b.rotation(), rotation * a.rotation(), 1e-5)
        });
        if turned {
            report.turned += 1;
        } else {
            report.failures.push(format!("{path}: turned wrongly"));
        }
        let placed_guids: Vec<u32> = back.iter().map(|e| e.guid()).filter(|g| *g != 0).collect();
        if placed_guids.iter().collect::<BTreeSet<_>>().len() == placed_guids.len() {
            report.guids_unique += 1;
        } else {
            report
                .failures
                .push(format!("{path}: two placements share a guid"));
        }

        let expected: Vec<u32> = back.iter().map(|e| e.archetype()).collect();
        // A writer that cannot lay the result out refuses with its reason
        // and writes nothing; that is counted, not called a failure.
        let encoded = match or_world::write::encode(&target, &layout) {
            Ok(b) => b,
            Err(or_world::write::WriteError::Meta(e)) => {
                *report.write_refused.entry(e.to_string()).or_default() += 1;
                continue;
            }
            Err(e) => {
                report.failures.push(format!("{path}: encode: {e}"));
                continue;
            }
        };
        let mut diags = Diagnostics::lenient();
        match parse_meta_binary_full(support::schema(), &encoded, &mut diags) {
            Ok((read, _)) => {
                let got: Vec<u32> = entities(&read).iter().map(|e| e.archetype()).collect();
                if got == expected {
                    report.read_back += 1;
                } else {
                    report
                        .failures
                        .push(format!("{path}: the bytes carry other entities"));
                }
            }
            Err(e) => report.failures.push(format!("{path}: read back: {e}")),
        }
    }
    report.catalogue_shape = catalogue.shape().cloned();
    Some(report)
}

#[test]
fn a_prefab_round_trips_every_map() {
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(
            WHAT,
            format_args!(
                "no install root; set {GAME_ROOT_ENV} to a ;-separated list of install \
                 directories. World files live inside encrypted archives."
            ),
        );
        return;
    }
    if support::schema().struct_("CMapData").is_none() {
        skip_install(
            WHAT,
            "no schema snapshot is staged, and a placement grows an array, which reaches the \
             file only through synthesis; see crates/io/or-io/src/meta/README.md",
        );
        return;
    }
    let stride: usize = std::env::var("OPENRAGE_EXEC_STRIDE")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(1);
    let reports: Vec<Report> = roots.iter().filter_map(|r| measure(r, stride)).collect();
    if reports.is_empty() {
        skip_install(WHAT, "no install root mounted");
        return;
    }
    for r in &reports {
        eprintln!("{}", r.text());
    }
    for r in &reports {
        let text = r.text();
        assert!(r.captured >= 100 / r.stride as u64, "{text}");
        assert!(r.failures.is_empty(), "{text}");
        for got in [r.in_place, r.turned, r.guids_unique] {
            assert_eq!(got, r.captured, "{text}");
        }
        let refused: u64 = r.write_refused.values().sum();
        assert!(refused * 100 <= r.captured, "{text}");
        assert_eq!(r.read_back + refused, r.captured, "{text}");
        // The catalogue is the main member set's, and refuses only the rest.
        let (Some(main), Some(filed)) = (r.shape(), &r.catalogue_shape) else {
            panic!("no member set\n{text}");
        };
        assert!(main.agrees_with(filed), "{text}");
        let minority: u64 = r
            .variants
            .iter()
            .filter(|v| !v.shape.agrees_with(main))
            .map(|v| v.maps)
            .sum();
        assert!(r.catalogue_refused.len() as u64 <= minority, "{text}");
        assert_eq!(
            r.catalogued + r.catalogue_refused.len() as u64,
            r.captured,
            "{text}"
        );
    }

    if let [a, b, ..] = reports.as_slice() {
        let (Some(sample), Some(a_shape), Some(b_shape)) = (&a.sample, a.shape(), b.shape()) else {
            panic!("no sample prefab to carry across");
        };
        assert!(
            !a_shape.agrees_with(b_shape),
            "the two installs share an entity member set"
        );
        eprintln!(
            "the installs' entity types differ in {} member(s)",
            a_shape.differences(b_shape).len()
        );
        let plain = or_io::hash::literal_string_hash(b"CEntityDef");
        let mut foreign = MetaInstance::new("CMapData");
        let mut ent = MetaInstance::new("CEntityDef");
        for (_, member) in b_shape.members().filter(|(ty, _)| *ty == plain) {
            ent.set(format!("0x{member:08X}"), Value::UInt(0));
        }
        foreign.set("name", Value::UInt(1));
        foreign.set(
            "entities",
            Value::Array(vec![Value::Pointer(Some(Box::new(ent)))]),
        );
        let refused = prefab::place(&mut foreign, sample, Vec3::ZERO, 0.0);
        assert!(
            matches!(refused, Err(PrefabError::TitleMismatch { .. })),
            "{refused:?}"
        );
    }
}
