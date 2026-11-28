//! `openrage-cli project check-resource`: any resource folder read as a server will read it, with no project.
//!
//! ⚠ It writes nothing and answers with its exit code: 0 where no rule found an error, 1 where one did. A warning and a note never
//! fail it, because a name the folder does not hold may be the game's or another resource's.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use or_project::resource::{check, Install, Options, Report, FILE_MEMORY_LIMIT};
use or_world_gta5::{ArchetypeAssetIndex, Catalogue, Kind};

use crate::project::{parse, Failure};
use crate::wardrobe_fix::without;
use Failure::{Run, Usage};

/// What a mounted install answers about a name, from the tables `depends` reads of it.
struct Mounted<'a> {
    catalogue: &'a Catalogue,
    archetypes: &'a ArchetypeAssetIndex,
}

impl Install for Mounted<'_> {
    fn declares_archetype(&self, name: u32) -> bool {
        self.archetypes.get(name).is_some()
    }

    fn holds_model(&self, name: u32) -> bool {
        let models = [Kind::Drawable, Kind::Fragment, Kind::DrawableDict];
        self.catalogue.find_any(name, &models).is_some()
    }

    fn holds_texture_dictionary(&self, name: u32) -> bool {
        self.catalogue.find(name, Kind::TextureDict).is_some()
    }
}

/// The ceilings `--limits` states, as `figure=count` pairs.
///
/// ⚠ A figure the check does not weigh is refused: taken, it would weigh nothing and the check would read as passed.
fn limits_of(stated: Option<&String>) -> Result<BTreeMap<String, u64>, Failure> {
    let mut limits = BTreeMap::new();
    for pair in stated.iter().flat_map(|l| l.split(',')) {
        let (figure, count) = pair
            .split_once('=')
            .ok_or_else(|| Usage(format!("--limits takes figure=count pairs, not {pair}")))?;
        let figure = figure.trim();
        if figure != FILE_MEMORY_LIMIT {
            return Err(Usage(format!(
                "--limits takes the figure {FILE_MEMORY_LIMIT}, not {figure}"
            )));
        }
        let count = count
            .trim()
            .parse()
            .map_err(|_| Usage(format!("--limits takes a count, not {count}")))?;
        limits.insert(figure.to_string(), count);
    }
    Ok(limits)
}

/// What the rules find in the resource `folder`, the install at `game` asked about each name the folder lacks.
pub fn check_resource_folder(
    folder: &Path,
    game: Option<&Path>,
    keys: Option<&str>,
    limits: BTreeMap<String, u64>,
) -> Result<Report, String> {
    if !folder.is_dir() {
        return Err(format!("{} is not a folder", folder.display()));
    }
    let index = match game {
        Some(root) => Some(crate::depends::load(root, keys)?),
        None => None,
    };
    let mounted = index.as_ref().map(|index| Mounted {
        catalogue: &index.catalogue,
        archetypes: index.archetypes(),
    });
    let options = Options {
        limits,
        install: mounted.as_ref().map(|asked| asked as &dyn Install),
    };
    Ok(check(folder, &options))
}

/// `project check-resource`: what the rules find in a resource folder, as text or as JSON.
pub fn check_resource(args: &[String]) -> Result<ExitCode, Failure> {
    let (rest, bare) = without(args, &["--json"]);
    let (positional, flags) = parse(&rest, &["--game", "--keys", "--limits"])?;
    let [folder] = positional.as_slice() else {
        return Err(Usage("expected one <resource folder>".into()));
    };
    let limits = limits_of(flags.get("--limits"))?;
    let report = check_resource_folder(
        Path::new(folder),
        flags.get("--game").map(Path::new),
        flags.get("--keys").map(String::as_str),
        limits,
    )
    .map_err(Run)?;
    if bare[0] {
        let document = serde_json::to_value(&report).map_err(|e| Run(e.to_string()))?;
        crate::wardrobe_fix::print_json(vec![document]);
    } else {
        for line in report.lines() {
            println!("{line}");
        }
    }
    Ok(if report.has_error() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::container::rpf::RpfEntryKind;
    use or_io::meta::instance::HashedString;
    use or_io::meta::{MetaInstance, Value};
    use or_title::vfs::EntryFacts;

    fn hash(name: &str) -> u32 {
        or_io::string_hash(name.as_bytes())
    }

    #[test]
    fn a_mounted_install_answers_by_the_kind_of_file_and_by_the_archetypes_it_declares() {
        let mut catalogue = Catalogue::new();
        for leaf in [
            "bench.ydr",
            "gate.yft",
            "props.ydd",
            "wood.ytd",
            "bench.ybn",
        ] {
            catalogue.index(&EntryFacts {
                logical: format!("x64a.rpf/{leaf}"),
                container: "x64a.rpf".into(),
                leaf: leaf.into(),
                kind: RpfEntryKind::Resource,
                stored_size: 32,
                size: 32,
                compressed: false,
                paging: Some((0x8000_0000, 0x1000_0000)),
            });
        }
        let mut archetype = MetaInstance::new("CBaseArchetypeDef");
        archetype.set("name", Value::Hash(HashedString::new("bench_a")));
        archetype.set("assetName", Value::Hash(HashedString::new("bench")));
        let mut types = MetaInstance::new("CMapTypes");
        types.set("name", Value::Hash(HashedString::new("props")));
        types.set(
            "archetypes",
            Value::Array(vec![Value::Pointer(Some(Box::new(archetype)))]),
        );
        let mut archetypes = ArchetypeAssetIndex::new();
        archetypes.add(or_world_gta5::MapTypes::new(&types).expect("a type file"));
        let asked = Mounted {
            catalogue: &catalogue,
            archetypes: &archetypes,
        };

        for model in ["bench", "gate", "props"] {
            assert!(asked.holds_model(hash(model)), "{model}");
        }
        assert!(
            !asked.holds_model(hash("wood")),
            "a texture dictionary is no model"
        );
        assert!(asked.holds_texture_dictionary(hash("wood")));
        assert!(
            !asked.holds_texture_dictionary(hash("bench")),
            "a model and a bound of the name are no texture dictionary"
        );
        assert!(asked.declares_archetype(hash("bench_a")));
        assert!(
            !asked.declares_archetype(hash("bench")),
            "the asset an archetype names is not an archetype"
        );
    }

    #[test]
    fn the_limits_are_pairs_of_a_figure_and_a_count() {
        let stated = format!(" {FILE_MEMORY_LIMIT} = 16777216 ");
        let limits = limits_of(Some(&stated)).ok().expect("one pair");
        assert_eq!(
            limits,
            BTreeMap::from([(FILE_MEMORY_LIMIT.to_string(), 16_777_216)])
        );
        assert!(limits_of(None).ok().is_some_and(|none| none.is_empty()));
        for refused in [
            "file_memory",
            "file_memory=many",
            "file_memory=-1",
            "file_memroy=1",
            "file_memory=1,files=3",
        ] {
            assert!(
                matches!(limits_of(Some(&refused.to_string())), Err(Usage(_))),
                "{refused}"
            );
        }
    }
}
