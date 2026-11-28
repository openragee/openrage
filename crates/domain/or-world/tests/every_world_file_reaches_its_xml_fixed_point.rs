//! Every world file a retail install ships, exported to XML and imported
//! twice through [`or_world::xml`], is byte-identical the second time.
//!
//! Point `OPENRAGE_GAME_ROOT` at one or more install roots, `;`-separated.
//!
//! With no schema snapshot this checks the refusal instead: every import has
//! to refuse with the reason rather than with a missing type per file.
//!
//! ⚠ The property is `C1 == C2`, never `C1 == O`: the import lays a document
//! out from its values, and an RBM class's `signature` word is zero without a
//! donor. A first cycle that also changed the XML dropped something, and that
//! fails too: the members Enhanced adds are in [`or_io::meta::shipped`], and every
//! type table in both installs is held to what it says there.

mod support;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_io::meta::binary::layout::ContainerLayout;
use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::binary::pso::schema::{PsoDataType, PsoSchemaItem};
use or_io::meta::binary::rbm::schema::PsoType;
use or_io::meta::binary::synth::name_hash;
use or_io::meta::canonical::NotCanonical;
use or_io::meta::shipped::gen9_members;
use or_io::meta::{ArrayKind, Diagnostics, FieldType};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_title::install::crawl::{self, ControlFlow};

const WHAT: &str = "every_world_file_reaches_its_xml_fixed_point";

/// How many failing documents a report names.
const REPORTED: usize = 12;

/// Per mille of world files that may be refused before the run stops being
/// evidence: what it admits is a file whose type table predates a member the
/// PSC corpus declares, and the report names each one.
const REFUSED_PER_MILLE: usize = 1;

#[derive(Default)]
struct Install {
    enhanced: bool,
    seen: usize,
    converged: usize,
    dropped: usize,
    refused: usize,
    /// Type tables describing a type Enhanced extends, with and without its
    /// members.
    extended: usize,
    unextended: usize,
    /// `OccludeModel::verts`-shaped descriptors: a counted pointer array.
    pointer_arrays: usize,
}

#[derive(Default)]
struct Census {
    installs: BTreeMap<String, Install>,
    by_format: BTreeMap<&'static str, usize>,
    undeclared: BTreeMap<String, (usize, String)>,
    reshaped: BTreeMap<String, (usize, String)>,
    lost: Vec<String>,
    refused: BTreeMap<String, (usize, String)>,
    moved: Vec<String>,
    unreadable: Vec<String>,
    /// A type table that holds Enhanced's members otherwise than
    /// [`or_io::meta::shipped::GEN9_MEMBERS`] states them, or a pointer array
    /// otherwise than synthesis lays one out.
    misdescribed: BTreeMap<String, (usize, String)>,
}

impl Census {
    fn total(&self, f: impl Fn(&Install) -> usize) -> usize {
        self.installs.values().map(f).sum()
    }

    fn report(&self) -> String {
        let mut out = String::new();
        for (root, i) in &self.installs {
            out.push_str(&format!(
                "{root}: {} world files; {} converged, {} of them dropping a member the schema \
                 does not declare; {} refused; {} type tables with Enhanced's members and {} \
                 without; {} counted pointer arrays\n",
                i.seen,
                i.converged,
                i.dropped,
                i.refused,
                i.extended,
                i.unextended,
                i.pointer_arrays
            ));
        }
        for (why, (n, example)) in &self.misdescribed {
            out.push_str(&format!("  misdescribed {n:>6}  {why}  (e.g. {example})\n"));
        }
        out.push_str(&format!(
            "containers {:?}; lost something else {}; not a fixed point {}; unreadable {}",
            self.by_format,
            self.lost.len(),
            self.moved.len(),
            self.unreadable.len(),
        ));
        for (member, (n, example)) in &self.undeclared {
            out.push_str(&format!(
                "\n  undeclared {n:>6}  {member}  (e.g. {example})"
            ));
        }
        for (member, (n, example)) in &self.reshaped {
            out.push_str(&format!(
                "\n  stored as another kind {n:>6}  {member}  (e.g. {example})"
            ));
        }
        for (why, (n, example)) in self.refused.iter().take(REPORTED) {
            out.push_str(&format!("\n  refused {n:>6}  {why}  (e.g. {example})"));
        }
        for leaf in self.lost.iter().take(REPORTED) {
            out.push_str(&format!("\n  lost: {leaf}"));
        }
        for m in self.moved.iter().take(REPORTED) {
            out.push_str(&format!("\n  moved: {m}"));
        }
        out
    }

    fn require_exercised(&self, what: &str, got: usize, floor: usize) {
        assert!(
            got >= floor,
            "{what}: {got} is under the floor of {floor}, so this check passed without \
             exercising anything.\n{}",
            self.report()
        );
    }

    fn fold(&mut self, install: &str, leaf: &str, bytes: &[u8]) {
        if let Some(format) = or_io::meta::detect_binary_format(bytes) {
            *self.by_format.entry(format.label()).or_default() += 1;
        }
        self.fold_type_table(install, leaf, bytes);
        let tally = self.installs.entry(install.to_string()).or_default();
        match or_world::xml::converge(bytes) {
            Ok(converged) => {
                tally.converged += 1;
                for member in &converged.reshaped {
                    let slot = self
                        .reshaped
                        .entry(member.clone())
                        .or_insert((0, leaf.to_string()));
                    slot.0 += 1;
                }
                if converged.values_moved && converged.undeclared.is_empty() {
                    self.lost.push(leaf.to_string());
                } else if converged.values_moved {
                    tally.dropped += 1;
                    for member in converged.undeclared {
                        let slot = self
                            .undeclared
                            .entry(member)
                            .or_insert((0, leaf.to_string()));
                        slot.0 += 1;
                    }
                }
            }
            Err(e @ NotCanonical::Moved { .. }) => self.moved.push(format!("{leaf}: {e}")),
            Err(e @ NotCanonical::Unreadable(_)) => self.unreadable.push(format!("{leaf}: {e}")),
            Err(e) => {
                tally.refused += 1;
                let why: String = e.to_string().chars().take(160).collect();
                let slot = self.refused.entry(why).or_insert((0, leaf.to_string()));
                slot.0 += 1;
            }
        }
    }
}

/// One member descriptor, as far as this census asks about it.
struct Member {
    hash: u32,
    word: bool,
    /// `(counting member's index, alignment power)` for a counted pointer array.
    counted_pointer: Option<(u32, u8)>,
}

impl Census {
    fn misdescribed(&mut self, why: String, leaf: &str) {
        self.misdescribed
            .entry(why)
            .or_insert((0, leaf.to_string()))
            .0 += 1;
    }

    /// Hold every type table to [`or_io::meta::shipped::GEN9_MEMBERS`] and to the
    /// pointer arrays synthesis would lay out from the snapshot.
    fn fold_type_table(&mut self, install: &str, leaf: &str, bytes: &[u8]) {
        let mut diags = Diagnostics::lenient();
        let Ok((_, layout)) = parse_meta_binary_full(support::schema(), bytes, &mut diags) else {
            return;
        };
        let classes: Vec<(u32, Vec<Member>)> = match &layout.container {
            ContainerLayout::Rbm(r) => r
                .model
                .classes
                .values()
                .map(|c| {
                    let members = c
                        .entries
                        .iter()
                        .map(|e| Member {
                            hash: e.name_hash,
                            word: e.data_type == PsoType::U32,
                            counted_pointer: matches!(
                                e.data_type,
                                PsoType::ArrayPointer32WithCount | PsoType::ArrayPointer64WithCount
                            )
                            .then(|| (e.array_count(), e.array_align_power())),
                        })
                        .collect();
                    (c.name_hash, members)
                })
                .collect(),
            ContainerLayout::Pso(p) => p
                .schema
                .items
                .iter()
                .filter_map(|item| match item {
                    PsoSchemaItem::Struct(s) => Some((
                        s.name_hash,
                        s.entries
                            .iter()
                            .map(|e| Member {
                                hash: e.name_hash,
                                word: e.data_type == PsoDataType::UInt,
                                counted_pointer: None,
                            })
                            .collect(),
                    )),
                    PsoSchemaItem::Enum(_) => None,
                })
                .collect(),
            ContainerLayout::Rbf(_) => return,
        };

        let library = support::schema();
        let index = support::hash_index();
        for (class_hash, members) in classes {
            let class = index.name_or_literal(class_hash);
            // ⚠ A file names a type by its PSC `name=` where it has one —
            // `OccludeModel`, where the snapshot keys `rage::OccludeModel`.
            let Some(resolved) = library
                .resolve_any(&class)
                .or_else(|| library.resolve_any(&format!("rage::{class}")))
            else {
                continue;
            };
            for m in &members {
                let Some((count, align)) = m.counted_pointer else {
                    continue;
                };
                if let Some(tally) = self.installs.get_mut(install) {
                    tally.pointer_arrays += 1;
                }
                let declared = resolved
                    .fields
                    .iter()
                    .find(|f| name_hash(&f.name) == m.hash);
                let Some(FieldType::Array {
                    kind:
                        ArrayKind::Pointer {
                            size_var,
                            align: bytes_align,
                        },
                    ..
                }) = declared.map(|f| &f.ty)
                else {
                    self.misdescribed(
                        format!(
                            "{class}.{}: a counted pointer the snapshot does not declare as one",
                            index.name_or_literal(m.hash)
                        ),
                        leaf,
                    );
                    continue;
                };
                let counted_by = members.get(count as usize).map(|c| c.hash);
                let wanted_power = bytes_align.checked_ilog2().unwrap_or(0) as u8;
                if counted_by != size_var.as_deref().map(name_hash) || align != wanted_power {
                    self.misdescribed(
                        format!(
                            "{class}.{}: counted by {:?} aligned 2^{align}, where the PSC says {size_var:?} aligned {bytes_align}",
                            index.name_or_literal(m.hash),
                            counted_by.map(|h| index.name_or_literal(h))
                        ),
                        leaf,
                    );
                }
            }

            for added in gen9_members() {
                let Some(owner) = resolved.chain.iter().find(|d| d.name == added.name) else {
                    continue;
                };
                let at: Vec<Option<usize>> = added
                    .fields
                    .iter()
                    .map(|f| members.iter().position(|m| m.hash == name_hash(&f.name)))
                    .collect();
                if at.iter().all(Option::is_none) {
                    if let Some(tally) = self.installs.get_mut(install) {
                        tally.unextended += 1;
                    }
                    continue;
                }
                if let Some(tally) = self.installs.get_mut(install) {
                    tally.extended += 1;
                }
                let Some(at) = at.into_iter().collect::<Option<Vec<usize>>>() else {
                    self.misdescribed(
                        format!("{class}: some of {}'s Enhanced members", added.name),
                        leaf,
                    );
                    continue;
                };
                // Appended to the base: after every member it declares, and
                // before anything a derived type adds.
                let after_the_base = owner.fields.iter().all(|f| {
                    members
                        .iter()
                        .position(|m| m.hash == name_hash(&f.name))
                        .is_none_or(|p| p < at[0])
                });
                let last = at[at.len() - 1];
                let before_the_derived = resolved
                    .chain
                    .iter()
                    .skip_while(|d| d.name != owner.name)
                    .skip(1)
                    .flat_map(|d| d.fields.iter())
                    .all(|f| {
                        members
                            .iter()
                            .position(|m| m.hash == name_hash(&f.name))
                            .is_none_or(|p| p > last)
                    });
                let consecutive = at.windows(2).all(|w| w[1] == w[0] + 1);
                let words = at.iter().all(|i| members[*i].word);
                if !after_the_base || !before_the_derived || !consecutive || !words {
                    self.misdescribed(
                        format!(
                            "{class}: {}'s Enhanced members are not u32s appended after its last declared member",
                            added.name
                        ),
                        leaf,
                    );
                }
            }
        }
    }

    fn merge(&mut self, other: Census) {
        self.installs.extend(other.installs);
        for (k, n) in other.by_format {
            *self.by_format.entry(k).or_default() += n;
        }
        for (map, theirs) in [
            (&mut self.undeclared, other.undeclared),
            (&mut self.reshaped, other.reshaped),
            (&mut self.refused, other.refused),
            (&mut self.misdescribed, other.misdescribed),
        ] {
            for (k, (n, example)) in theirs {
                map.entry(k).or_insert((0, example)).0 += n;
            }
        }
        self.lost.extend(other.lost);
        self.moved.extend(other.moved);
        self.unreadable.extend(other.unreadable);
    }
}

/// One install's census, or `None` when it yields no keys.
fn census_of(root: &Path) -> Option<Census> {
    let provider = crypto_for(root)?;
    let label = root.display().to_string();
    let mut census = Census::default();
    census.installs.entry(label.clone()).or_default().enhanced =
        Game::detect_from_root(root) == Some(Game::GtaVEnhanced);
    let mut warnings = Vec::new();
    let _ = crawl::crawl(root, provider, &mut warnings, &mut |found| {
        let leaf = found.leaf.to_ascii_lowercase();
        if !(leaf.ends_with(".ymap") || leaf.ends_with(".ytyp")) {
            return ControlFlow::Continue(());
        }
        if let Some(tally) = census.installs.get_mut(&label) {
            tally.seen += 1;
        }
        match found.read() {
            Ok(bytes) => census.fold(&label, found.leaf, &bytes),
            Err(e) => census
                .unreadable
                .push(format!("{}: extract: {e}", found.leaf)),
        }
        ControlFlow::Continue(())
    });
    Some(census)
}

fn crypto_for(root: &Path) -> Option<Arc<dyn RpfCryptoProvider>> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    Some(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))))
}

#[test]
fn every_world_file_reaches_its_xml_fixed_point() {
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(
            WHAT,
            format_args!("no install root; set {GAME_ROOT_ENV} to a ;-separated list"),
        );
        return;
    }
    // ⚠ One thread per install and one document live per thread: a run over
    // both retail installs is 44,965 files and two and a half hours serially.
    let parts: Vec<Census> = std::thread::scope(|scope| {
        let workers: Vec<_> = roots
            .iter()
            .map(|root| scope.spawn(move || census_of(root)))
            .collect();
        workers
            .into_iter()
            .filter_map(|w| w.join().expect("a census thread panicked"))
            .collect()
    });
    let mut census = Census::default();
    for part in parts {
        census.merge(part);
    }
    if census.installs.is_empty() {
        skip_install(
            WHAT,
            "no install root yielded an executable to take keys from",
        );
        return;
    }
    eprintln!("{}", census.report());

    let seen = census.total(|i| i.seen);
    census.require_exercised("exporting a world file", seen, 10_000);
    assert!(census.unreadable.is_empty(), "{}", census.report());
    assert!(census.moved.is_empty(), "{}", census.report());

    if support::schema().structs.is_empty() {
        // Every world file is PSO or RBM, and both need struct definitions.
        let clean = census
            .refused
            .keys()
            .all(|why| why.contains("schema snapshot"));
        assert!(
            clean && census.total(|i| i.refused) == seen,
            "with no schema snapshot every import has to refuse and say why:\n{}",
            census.report()
        );
        return;
    }

    census.require_exercised(
        "a world file at its fixed point",
        census.total(|i| i.converged),
        10_000,
    );
    assert!(census.lost.is_empty(), "{}", census.report());
    assert!(
        census.total(|i| i.refused) <= seen * REFUSED_PER_MILLE / 1000,
        "{}",
        census.report()
    );

    // What these two catch, measured: 670 occluder maps storing
    // `OccludeModel::verts` as another kind of value, and 20,376 Enhanced
    // files dropping a member the PSC corpus does not declare.
    assert!(census.reshaped.is_empty(), "{}", census.report());
    assert_eq!(census.total(|i| i.dropped), 0, "{}", census.report());

    assert!(census.misdescribed.is_empty(), "{}", census.report());
    for (root, install) in &census.installs {
        if install.enhanced {
            census.require_exercised(
                &format!("{root}: a type table carrying Enhanced's members"),
                install.extended,
                10_000,
            );
        } else {
            assert_eq!(
                install.extended,
                0,
                "{root} is Legacy and carries Enhanced's members:\n{}",
                census.report()
            );
        }
    }
    census.require_exercised(
        "a counted pointer array",
        census.total(|i| i.pointer_arrays),
        500,
    );
}
