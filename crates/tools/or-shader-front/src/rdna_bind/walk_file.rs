//! A walk of the user's own build that a walk file names, lowered alone through the kept
//! translations: what a pass that runs it on a device is given, and how its rays are held to the
//! reference walk.
//!
//! Nothing of a build is in the tree: a walk file states the kept corpus, the program, the words
//! the walk starts and ends at and which register holds what, as `pass_rt_walk` reads one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use or_io::gta6::accel::{Cull, Opacity, Walk};

use super::kept::translations;
use crate::rdna::{self, Options, Region};
use crate::rdna_cache::Cached;
use crate::rdna_corpus::Corpus;

/// The variable that names the walk files, as a list of paths.
pub const WALKS: &str = "OPENRAGE_RDNA_WALKS";

/// The registers a lane's walk leaves, in the order its module stores them: the committed leaf
/// (nought for a miss), the distance, the instance, the turn count and the stack word.
const LEFT: [&str; 5] = ["leaf", "t", "instance", "count", "stack"];

/// One walk lowered alone, a lane an invocation.
#[derive(Debug, Clone)]
pub struct LoweredWalk {
    pub module: naga::Module,
    /// What each dword of an invocation's row of `registers_given` holds: the word the file states of its register, `zero` where none.
    pub given: Vec<String>,
    /// The dwords of an invocation's row of `registers_left`, the leaf first.
    pub left: usize,
    pub threads: [u32; 3],
    pub shared_dwords: u32,
    /// The walk the reference runs for the same rays.
    pub walk: Walk,
    /// The file it was read from, and the program it names.
    pub file: PathBuf,
    pub program: u32,
    /// The loops its lowering does not show to end ([`rdna::Lowered::loops_not_shown_to_end`]): a pass runs such a walk only with every loop held to a cap.
    pub loops_not_shown_to_end: u32,
}

/// What one walk file states.
struct Stated(BTreeMap<String, String>);

fn number(text: &str) -> Option<u32> {
    match text.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

impl Stated {
    fn read(file: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let lines = text
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'));
        let words = lines
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
            .collect();
        Ok(Self(words))
    }

    fn text(&self, key: &str) -> Result<&str, String> {
        (self.0.get(key).map(String::as_str))
            .ok_or_else(|| format!("the walk's file states no {key}"))
    }

    fn number(&self, key: &str) -> Result<u32, String> {
        number(self.text(key)?).ok_or_else(|| format!("the walk's file states {key} as no number"))
    }

    fn is(&self, key: &str, value: &str) -> bool {
        self.0.get(key).is_some_and(|stated| stated == value)
    }

    /// Whether it is a lane's own walk that ends at the first hit it commits: a shadow ray's.
    fn casts_shadows(&self) -> bool {
        !self.is("form", "wave") && self.is("first_hit", "true")
    }
}

/// Why a module is not written as WGSL: naga 29's WGSL writer stops on a ray query.
pub const NO_WGSL_OF_A_QUERY: &str =
    "the module holds a ray query, which naga's WGSL writer does not write";

/// Whether `module` holds a ray query or the device's structure it walks, as a walk lowered with [`Options::query`] does.
pub fn holds_ray_query(module: &naga::Module) -> bool {
    (module.types.iter()).any(|(_, ty)| {
        matches!(
            ty.inner,
            naga::TypeInner::RayQuery { .. } | naga::TypeInner::AccelerationStructure { .. }
        )
    })
}

/// The first walk a lane walks alone that ends at its first hit, of the file `named`, else of those [`WALKS`] lists; with `query`, its test of a node asks the device's ray query.
pub fn shadow_walk(named: Option<&Path>, query: bool) -> Result<LoweredWalk, String> {
    let files: Vec<PathBuf> = match named {
        Some(file) => vec![file.to_owned()],
        None => match std::env::var_os(WALKS) {
            Some(list) => std::env::split_paths(&list).collect(),
            None => return Err(format!("no walk file is named: {WALKS} is not set")),
        },
    };
    for file in files {
        let stated = Stated::read(&file)?;
        if stated.casts_shadows() {
            return lowered(&stated, file, query);
        }
    }
    Err(format!(
        "no walk file named is of a lane's walk that ends at its first hit ({WALKS})"
    ))
}

/// What the walk `stated` is lowered with: the arena, its region, and the device's query where `query`.
fn walk_options(stated: &Stated, query: bool) -> Result<Options, String> {
    let left: Result<Vec<String>, String> = (LEFT.iter())
        .map(|what| stated.text(what).map(str::to_owned))
        .collect();
    Ok(Options {
        arena: true,
        query,
        region: Some(Region {
            first: stated.number("first")?,
            exit: stated.number("exit")?,
            left: left?,
        }),
        ..Options::default()
    })
}

fn lowered(stated: &Stated, file: PathBuf, query: bool) -> Result<LoweredWalk, String> {
    if !stated.is("commits", "all") {
        return Err("the walk's own code decides a triangle that is not opaque by a table the pass does not fill".into());
    }
    let index = stated.number("program")?;
    let corpus = Path::new(stated.text("corpus")?);
    let mut corpus = Corpus::open(corpus).map_err(|e| format!("{}: {e}", corpus.display()))?;
    let bytes = (corpus.program(index as usize))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("the corpus holds no program {index}"))?;
    let block = match stated.0.contains_key("block") {
        true => stated.number("block")? as usize,
        false => 0,
    };
    let mut programs = rdna::unwrap(&bytes).map_err(|e| e.to_string())?;
    if block >= programs.len() {
        return Err(format!("the program holds no block {block}"));
    }
    let program = programs.swap_remove(block);
    let launch = program
        .launch
        .ok_or("the walk's program is no compute program")?;
    let options = walk_options(stated, query)?;
    let lowered = match &*translations().get_or_translate(&program, &options).0 {
        Cached::Translated(lowered) => (**lowered).clone(),
        Cached::Refused { message, .. } => return Err(message.clone()),
    };
    if lowered.subgroups {
        return Err(
            "the walk keeps its wave whole, which a lane an invocation does not run".into(),
        );
    }
    let loops_not_shown_to_end = lowered.loops_not_shown_to_end();
    let registers = lowered
        .region
        .ok_or("the lowering states no register of the walk")?;
    let roles: BTreeMap<&str, &str> = (stated.0.get("given").map_or("", String::as_str))
        .split_whitespace()
        .filter_map(|pair| pair.split_once(':'))
        .collect();
    let given = (registers.given.iter())
        .map(|register| {
            roles
                .get(register.as_str())
                .copied()
                .unwrap_or("zero")
                .to_owned()
        })
        .collect();
    let (ring, step) = (stated.number("ring")?, stated.number("step")?);
    let walk = Walk {
        ring: (ring + 4) / step.max(1),
        lane: 0,
        mask: stated.number("mask")? as u8,
        cull: match stated.text("cull")? {
            "back" => Cull::Back,
            "front" => Cull::Front,
            _ => Cull::None,
        },
        opacity: match stated.text("opacity")? {
            "opaque" => Opacity::Opaque,
            "nonopaque" => Opacity::NonOpaque,
            _ => Opacity::Instance,
        },
        first_hit: true,
        bound: stated.number("bound")?,
    };
    Ok(LoweredWalk {
        loops_not_shown_to_end,
        module: lowered.module,
        given,
        left: registers.left.len(),
        threads: launch.threads.map(|n| n.max(1)),
        shared_dwords: launch.shared_dwords,
        walk,
        file,
        program: index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shadow_ray_s_walk_is_a_lane_s_own_that_ends_at_its_first_hit() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| {
            let file = dir.path().join(name);
            std::fs::write(&file, text).unwrap();
            file
        };
        let wave = write("wave.walk", "form = wave\nfirst_hit = true\n");
        let closest = write("closest.walk", "# a comment\nfirst_hit = false\n");
        let stated = |file: &Path| Stated::read(file).unwrap().casts_shadows();
        assert!(!stated(&wave) && !stated(&closest));
        let first = write("first.walk", "first_hit = true\ncommits = table\n");
        assert!(stated(&first));
        // Taken, and refused by what its own code commits, before any corpus is read.
        let why = shadow_walk(Some(&first), false).unwrap_err();
        assert!(why.contains("table"), "{why}");
        let why = shadow_walk(Some(&closest), true).unwrap_err();
        assert!(why.contains(WALKS), "{why}");
        assert_eq!(number("0x7fc"), Some(0x7FC));
        assert_eq!(number("236"), Some(236));
    }

    #[test]
    fn a_walk_asked_through_the_device_s_query_is_lowered_with_the_query_and_one_not_without() {
        let mut words: BTreeMap<String, String> = LEFT
            .iter()
            .map(|what| ((*what).to_owned(), format!("v{}", what.len())))
            .collect();
        words.insert("first".into(), "0x10".into());
        words.insert("exit".into(), "0x40".into());
        let stated = Stated(words);
        let asked = walk_options(&stated, true).unwrap();
        let plain = walk_options(&stated, false).unwrap();
        // The lowering binds a `DeviceStructure` only for these options (`pass_rt_query`).
        assert!(asked.arena && asked.query);
        assert!(plain.arena && !plain.query);
        assert_eq!(asked.region, plain.region);
    }

    #[test]
    fn a_module_that_holds_a_ray_query_is_told_from_one_that_does_not() {
        let mut module = naga::Module::default();
        assert!(!holds_ray_query(&module));
        let ty = naga::Type {
            name: None,
            inner: naga::TypeInner::RayQuery {
                vertex_return: false,
            },
        };
        module.types.insert(ty, naga::Span::UNDEFINED);
        assert!(holds_ray_query(&module));
    }
}
