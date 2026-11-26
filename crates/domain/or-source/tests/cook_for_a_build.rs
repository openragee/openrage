//! What a build asks of a source folder: what it will cook to, before it cooks, and the cook itself.

mod common;
mod model_fixtures;

use std::fs;
use std::path::Path;

use common::*;
use or_io::codec::texture::GrcTextureFormat;
use or_source::asset::{sha256_hex, Source, MANIFEST};
use or_source::cook::{cook_source, describe};

fn bark(parent: &Path) -> std::path::PathBuf {
    let spec = vec![Spec::new("bark", GrcTextureFormat::DXT5, ramp(32, 32, 1))];
    extract(parent, "bark_set", &legacy_dictionary(&spec))
}

fn edit_manifest(dir: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let path = dir.join(MANIFEST);
    let mut text: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut text);
    fs::write(path, serde_json::to_string_pretty(&text).unwrap()).unwrap();
}

#[test]
fn a_textures_source_says_what_it_cooks_to_before_it_cooks() {
    let work = tempfile::tempdir().unwrap();
    let dir = bark(work.path());
    let said = describe(&dir).expect("a textures source is described");
    assert_eq!(said.kind, "textures");
    assert_eq!(said.name, "bark_set");
    assert_eq!(said.extension, "ytd");
    let size = |p: std::path::PathBuf| fs::metadata(p).unwrap().len();
    let payload: u64 = fs::read_dir(dir.join("textures"))
        .unwrap()
        .flatten()
        .map(|e| e.metadata().unwrap().len())
        .sum::<u64>()
        + size(dir.join(MANIFEST))
        + size(dir.join("textures.json"));
    assert!(payload > 0);
    assert_eq!(
        said.bytes, payload,
        "the plan counts the files the cook reads"
    );
    assert!(
        said.game.is_empty(),
        "an authored source is no file of the game's"
    );
}

#[test]
fn a_source_from_the_game_names_the_file_it_was_taken_from() {
    let work = tempfile::tempdir().unwrap();
    let dir = bark(work.path());
    let original = sha256_hex(b"the compiled original");
    edit_manifest(&dir, |m| {
        m["origin"]["from_game"] = true.into();
        m["origin"]["sha256"] = original.clone().into();
    });
    assert_eq!(describe(&dir).unwrap().game, vec![original]);
}

#[test]
fn a_platform_but_win64_and_a_kind_nobody_cooks_are_each_refused_in_one_sentence() {
    let work = tempfile::tempdir().unwrap();
    let dir = bark(work.path());
    edit_manifest(&dir, |m| m["platform"] = "xbox360".into());
    let why = describe(&dir).unwrap_err();
    assert!(why.contains("xbox360") && why.contains("win64"), "{why}");
    assert!(!why.contains('\n'));

    edit_manifest(&dir, |m| {
        m["platform"] = "win64".into();
        m["kind"] = "cloth".into();
    });
    let why = describe(&dir).unwrap_err();
    assert!(why.contains("this is a cloth source"), "{why}");
    for cooked in ["bounds", "paths"] {
        assert!(why.contains(cooked), "{cooked}: {why}");
    }
}

#[test]
fn a_cook_writes_the_file_a_compile_makes_into_the_scratch_folder() {
    let work = tempfile::tempdir().unwrap();
    let dir = bark(work.path());
    let scratch = work.path().join("scratch");
    let output = cook_source(&dir, &scratch).expect("the source cooks");
    let id = Source::open(&dir).unwrap().manifest.id;
    assert_eq!(output, scratch.join(id).join("bark_set.ytd"));
    assert_eq!(fs::read(&output).unwrap(), compile(&dir));
}

#[test]
fn a_broken_image_fails_the_cook_and_names_itself_and_writes_nothing() {
    let work = tempfile::tempdir().unwrap();
    let dir = bark(work.path());
    fs::write(dir.join("textures").join("bark.png"), b"not an image").unwrap();
    let scratch = work.path().join("scratch");
    let why = cook_source(&dir, &scratch).unwrap_err();
    assert!(why.iter().any(|w| w.contains("bark")), "{why:?}");
    assert!(
        !scratch.exists()
            || fs::read_dir(&scratch)
                .unwrap()
                .flatten()
                .all(|e| { fs::read_dir(e.path()).unwrap().flatten().next().is_none() }),
        "a failed cook left a file"
    );
}

#[test]
fn a_rule_the_texture_cannot_honour_fails_the_cook_and_names_the_texture_and_the_rule() {
    let work = tempfile::tempdir().unwrap();
    let dir = bark(work.path());
    // The DDS is DXT5 and has no image beside it, so a format of another kind would be skipped if it were not refused.
    edit_rules(&dir, |rules| {
        entry(rules, "bark")["encode"] = serde_json::json!({"format": "DXT1"});
    });
    let scratch = work.path().join("scratch");
    let why = cook_source(&dir, &scratch).unwrap_err();
    assert!(
        why.iter().any(|w| w.contains("texture bark")
            && w.contains("format")
            && w.contains("textures.json")),
        "{why:?}"
    );
    assert!(!scratch.exists() || fs::read_dir(&scratch).unwrap().flatten().next().is_none());
}

/// How many files of each extension the corpus check imports, spread over the staged ones.
const SAMPLE: usize = 6;

/// The cook of the source in `folder`, as the bytes a build would ship.
fn cooked(folder: &Path, scratch: &Path) -> Vec<u8> {
    let output =
        cook_source(folder, scratch).unwrap_or_else(|why| panic!("{}: {why:?}", folder.display()));
    fs::read(output).unwrap()
}

/// Imports `bytes` as the game's file `name` into a project of its own and returns the source folder.
fn import_from_the_game(project: &Path, name: &str, bytes: &[u8]) -> Option<std::path::PathBuf> {
    use or_source::game_import::{import, Facts, Input, Options, Verdict};
    let facts = Facts {
        from_game: true,
        virtual_path: Some(format!("x64a.rpf/{name}")),
        game_build: None,
        ..Facts::default()
    };
    let report = import(
        project,
        &[Input::Bytes { name, bytes, facts }],
        &Options::default(),
    );
    match &report.files[0].verdict {
        Verdict::Made { folder } => Some(folder.clone()),
        _ => None,
    }
}

/// What a build asks of one imported file: is its cook the game's file, before an edit and after.
fn check_the_games_file_until_it_is_edited(file: &str, bytes: &[u8]) -> Option<bool> {
    let project = tempfile::tempdir().unwrap();
    let folder = import_from_the_game(project.path(), file, bytes)?;
    let said = describe(&folder).unwrap_or_else(|why| panic!("{file}: {why}"));
    assert!(
        said.game.contains(&sha256_hex(bytes)),
        "{file}: the game's own file is not one the build leaves out"
    );
    let first = cooked(&folder, &project.path().join("first"));
    assert!(
        said.game.contains(&sha256_hex(&first)),
        "{file}: an import nobody touched cooks to a file the build does not take for the game's, \
         so it would ship"
    );
    if !edit_the_source(&folder) {
        return Some(false);
    }
    let second = cooked(&folder, &project.path().join("second"));
    assert_ne!(first, second, "{file}: the edit did not reach the cook");
    let after = describe(&folder).unwrap();
    assert!(
        !after.game.contains(&sha256_hex(&second)),
        "{file}: an edited source still cooks to what the build takes for the game's file"
    );
    Some(true)
}

#[test]
fn a_source_is_the_games_file_to_a_build_until_it_is_edited_whatever_bytes_the_game_ships() {
    use model_fixtures::{drawable, geometry, ydd, ydr};
    let ytd = legacy_dictionary(&[Spec::new("bark", GrcTextureFormat::DXT5, ramp(32, 32, 1))]);
    let ydr = ydr(drawable("jacket", vec![geometry(6, false, 0.0)], None));
    let ydd = ydd(vec![(
        "jbib_000_u",
        drawable("jbib_000_u", vec![geometry(6, false, 0.0)], None),
    )]);
    for (file, bytes) in [("bark.ytd", ytd), ("jacket.ydr", ydr), ("jbib_u.ydd", ydd)] {
        let shipped = as_the_game_ships_it(&bytes);
        assert_eq!(
            check_the_games_file_until_it_is_edited(file, &shipped),
            Some(true),
            "{file} was not imported and edited"
        );
    }
}

#[test]
fn a_corpus_file_taken_from_the_game_is_the_games_file_to_a_build_until_it_is_edited() {
    let (mut untouched, mut edited) = (0, 0);
    for extension in ["ytd", "ydr", "ydd"] {
        let Some(files) = or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), extension)
            .all_targets()
            .find_or_skip(&format!(
                "a corpus .{extension} is the game's file until it is edited"
            ))
        else {
            continue;
        };
        let step = (files.len() / SAMPLE).max(1);
        for path in files.iter().step_by(step).take(SAMPLE) {
            let file = path.file_name().unwrap().to_string_lossy().into_owned();
            if let Some(was_edited) =
                check_the_games_file_until_it_is_edited(&file, &fs::read(path).unwrap())
            {
                untouched += 1;
                edited += usize::from(was_edited);
            }
        }
    }
    eprintln!("{untouched} corpus file(s) cook to the game's file untouched; {edited} edited");
    assert!(
        untouched == 0 || edited > 0,
        "no corpus file could be edited"
    );
}

/// Makes a source of a game file's bytes, under a name, in a folder.
type Extract = fn(&[u8], &str, &Path) -> Result<std::path::PathBuf, Vec<or_source::Problem>>;

/// Every staged file of `extension` made a source by `extract`, and what each source cooks to against `canonical`.
fn every_corpus_file_becomes_a_source_that_cooks_to_its_canonical_save(
    extension: &str,
    what: &str,
    extract: Extract,
    canonical: fn(&[u8]) -> Option<Vec<u8>>,
) {
    let Some(files) =
        or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), extension).find_or_skip(what)
    else {
        return;
    };
    let (mut made, mut refused) = (0usize, Vec::new());
    for path in &files {
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = fs::read(path).unwrap();
        let Some(canonical) = canonical(&bytes) else {
            refused.push(format!("{file}: the file does not open and save"));
            continue;
        };
        let work = tempfile::tempdir().unwrap();
        match extract(&bytes, "subject", work.path()) {
            Ok(folder) => {
                let cooked = cooked(&folder, &work.path().join("scratch"));
                assert!(
                    cooked == canonical,
                    "{file}: the cook of its source is not the canonical save of the file"
                );
                made += 1;
            }
            Err(why) => refused.push(format!("{file}: {why:?}")),
        }
    }
    eprintln!(
        "{made} of {} staged .{extension} became a source that cooks to its canonical save; {} refused",
        files.len(),
        refused.len()
    );
    assert!(refused.is_empty(), "{refused:#?}");
}

#[test]
fn every_corpus_ybn_becomes_a_source_that_cooks_to_its_canonical_save() {
    use or_io::decl::File;
    use or_io::gta5::bounds::PhBoundAny;
    every_corpus_file_becomes_a_source_that_cooks_to_its_canonical_save(
        "ybn",
        "every corpus .ybn becomes a source that cooks to its canonical save",
        |bytes, name, parent| {
            let how = or_source::textures::Extraction {
                from_game: true,
                virtual_path: None,
            };
            or_source::bounds::extract(bytes, name, parent, &how)
        },
        |bytes| PhBoundAny::from_bytes(bytes).ok()?.to_bytes().ok(),
    );
}

#[test]
fn every_corpus_ynd_becomes_a_source_that_cooks_to_its_canonical_save() {
    use or_io::decl::File;
    use or_io::gta5::CPathRegion;
    every_corpus_file_becomes_a_source_that_cooks_to_its_canonical_save(
        "ynd",
        "every corpus .ynd becomes a source that cooks to its canonical save",
        |bytes, name, parent| {
            let how = or_source::textures::Extraction {
                from_game: true,
                virtual_path: None,
            };
            or_source::paths::extract(bytes, name, parent, &how)
        },
        |bytes| CPathRegion::from_bytes(bytes).ok()?.to_bytes().ok(),
    );
}

#[test]
fn every_node_of_a_corpus_ynd_is_addressed_by_its_place_in_one_region() {
    use or_io::decl::File;
    use or_io::gta5::CPathRegion;
    let Some(files) = or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), "ynd")
        .find_or_skip("every node of a corpus .ynd is addressed by its place")
    else {
        return;
    };
    let (mut nodes, mut elsewhere) = (0usize, Vec::new());
    for path in &files {
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let region = CPathRegion::from_bytes(&fs::read(path).unwrap())
            .unwrap_or_else(|why| panic!("{file}: {why}"));
        let own = region.nodes.first().map(|node| node.address.region);
        for (place, node) in region.nodes.iter().enumerate() {
            nodes += 1;
            if usize::from(node.address.index) != place || Some(node.address.region) != own {
                elsewhere.push(format!(
                    "{file}: node {place} is {}:{}",
                    node.address.region, node.address.index
                ));
            }
        }
    }
    eprintln!(
        "{nodes} nodes of {} staged .ynd are each addressed by their place in the region of their file",
        files.len()
    );
    assert!(
        elsewhere.is_empty(),
        "a source refuses an address that is not the place of its node, and these files hold one: {elsewhere:#?}"
    );
}

/// What a polygon's record holds apart from where its vertices and its neighbours sit: its kind, its material by
/// the game's value, the words of the vertices it stands on, its sixteen bytes with each index left as the bit
/// above it, and what the bound holds for each of those vertices.
type Polygon = (u32, u64, Vec<[i16; 3]>, Vec<u8>, Vec<u32>);

/// The polygon of a triangle across each of its edges.
type Across = [Option<Polygon>; 3];

/// Every soup under `bound` by the address a source gives it.
fn soups_under<'a>(
    bound: &'a or_io::gta5::bounds::PhBoundAny,
    address: String,
    out: &mut Vec<(String, &'a or_io::gta5::bounds::PhBoundGeometry)>,
) {
    use or_io::gta5::bounds::PhBoundAny;
    match bound {
        PhBoundAny::PhBoundGeometry(g) => out.push((address, g)),
        PhBoundAny::PhBoundBVH(b) => out.push((address, &b.base)),
        PhBoundAny::PhBoundComposite(c) => {
            for (i, child) in c.bounds.iter().enumerate() {
                if let Some(child) = child {
                    soups_under(child, format!("{address}_child{i}"), out);
                }
            }
        }
        _ => {}
    }
}

/// How often each polygon is held by `g`, a triangle with the polygons across its edges.
fn polygons_of(
    g: &or_io::gta5::bounds::PhBoundGeometry,
) -> std::collections::BTreeMap<(Polygon, Option<Across>), usize> {
    use or_io::gta5::bounds::{NO_NEIGHBOUR, VERTEX_INDEX_MASK};
    let polygon = |at: usize| -> Polygon {
        let p = &g.polygons[at];
        let (stands_on, _) = g.polygon_vertices(p).expect("a polygon of a kind");
        let above = !VERTEX_INDEX_MASK;
        let mut indices = p.vertex_indices;
        let mut neighbours = p.neighboring_polygons;
        // Where each kind keeps the vertices it stands on: a triangle and a box in the words after the first four bytes.
        match p.primitive() {
            0 => {
                indices = indices.map(|v| v & above);
                neighbours = [0; 3];
            }
            3 => {
                indices = indices.map(|v| v & above);
                neighbours[0] &= above;
            }
            _ => indices[2] = 0,
        }
        let mut area = p.area.to_bits();
        if matches!(p.primitive(), 1 | 2 | 4) {
            area &= 0xffff;
        }
        let mut bytes = area.to_le_bytes().to_vec();
        bytes.extend(indices.iter().flat_map(|v| v.to_le_bytes()));
        bytes.extend(neighbours.iter().flat_map(|v| v.to_le_bytes()));
        let material = g
            .poly_mat_index_list
            .get(at)
            .map_or(u64::MAX, |slot| g.material_ids[usize::from(*slot)].id);
        let words = stands_on
            .iter()
            .map(|v| {
                let v = &g.compressed_vertices[*v];
                [v.x, v.y, v.z]
            })
            .collect();
        let held = stands_on
            .iter()
            .filter_map(|v| g.vertex_attribs.get(*v).map(|c| c.color))
            .collect();
        (p.primitive(), material, words, bytes, held)
    };
    let mut out = std::collections::BTreeMap::new();
    for (at, p) in g.polygons.iter().enumerate() {
        let across = (p.primitive() == 0).then(|| {
            p.neighboring_polygons
                .map(|n| (n != NO_NEIGHBOUR).then(|| polygon(usize::from(n))))
        });
        *out.entry((polygon(at), across)).or_insert(0) += 1;
    }
    out
}

/// The game's own check of a saved bound, over the file's image.
fn validate(file: &[u8]) -> Result<(), String> {
    use or_io::container::resource::{ResourceEnvelope, Rsc7};
    let opened = Rsc7::open(file).map_err(|e| e.to_string())?;
    let aligned = or_io::decl::Aligned::from_bytes(&opened.data);
    let image = or_io::desc::Image::new(aligned.bytes(), opened.virtual_size as usize);
    or_io::gta5::bounds::validate(image, 0).map_err(|e| e.to_string())
}

#[test]
fn a_triangle_taken_out_of_a_corpus_ybn_cooks_to_a_bound_that_holds_one_fewer_or_is_refused_by_name(
) {
    use or_io::decl::File;
    use or_io::gta5::bounds::PhBoundAny;
    let Some(files) = or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), "ybn")
        .find_or_skip("a triangle taken out of a corpus .ybn")
    else {
        return;
    };
    let (mut cooked, mut no_triangle, mut by_name) = (0usize, 0usize, Vec::new());
    let (mut soups_edited, mut holding_attributes) = (0usize, 0usize);
    for path in &files {
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = fs::read(path).unwrap();
        let work = tempfile::tempdir().unwrap();
        let how = or_source::textures::Extraction {
            from_game: true,
            virtual_path: None,
        };
        let folder = or_source::bounds::extract(&bytes, "subject", work.path(), &how)
            .unwrap_or_else(|why| panic!("{file}: {why:?}"));
        if !folder.join(or_source::bounds::MODEL).is_file() {
            no_triangle += 1;
            continue;
        }
        // The last triangle of every node goes, where the primitive it is in holds another.
        let mut edited: Vec<String> = Vec::new();
        edit_bounds_glb(&folder, "subject", |scene| {
            for mesh in &mut scene.objects[0].meshes {
                let Some(primitive) = mesh.primitives.last_mut() else {
                    continue;
                };
                if primitive.indices.len() > 3 {
                    primitive.indices.truncate(primitive.indices.len() - 3);
                    edited.push(mesh.name.clone());
                }
            }
        });
        let made = match or_source::bounds::compile(&Source::open(&folder).unwrap()) {
            Ok(made) => made,
            Err(why) => {
                let why = why
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ");
                assert!(
                    why.contains("which follows its vertices one for one")
                        || why.contains("two vertices that differ in their"),
                    "{file}: refused for a reason that names nothing it holds: {why}"
                );
                by_name.push(format!("{file}: {why}"));
                continue;
            }
        };
        validate(&made.bytes).unwrap_or_else(|why| panic!("{file}: {why}"));
        let (base, bound) = (
            PhBoundAny::from_bytes(&bytes).unwrap(),
            PhBoundAny::from_bytes(&made.bytes).unwrap(),
        );
        let (mut was, mut is) = (Vec::new(), Vec::new());
        soups_under(&base, "bound".to_string(), &mut was);
        soups_under(&bound, "bound".to_string(), &mut is);
        assert_eq!(was.len(), is.len(), "{file}");
        for ((address, was), (_, is)) in was.iter().zip(&is) {
            let (held, holds) = (polygons_of(was), polygons_of(is));
            let more =
                |a: &std::collections::BTreeMap<(Polygon, Option<Across>), usize>,
                 b: &std::collections::BTreeMap<(Polygon, Option<Across>), usize>| {
                    let mut out: Vec<(Polygon, Option<Across>)> = Vec::new();
                    for (polygon, count) in a {
                        let other = b.get(polygon).copied().unwrap_or(0);
                        out.extend(std::iter::repeat_n(
                            polygon.clone(),
                            count.saturating_sub(other),
                        ));
                    }
                    out
                };
            let (gone, come) = (more(&held, &holds), more(&holds, &held));
            if !edited.contains(address) {
                assert!(
                    gone.is_empty() && come.is_empty(),
                    "{file} {address}: nothing of its node changed and its polygons did"
                );
                continue;
            }
            soups_edited += 1;
            holding_attributes += usize::from(!was.vertex_attribs.is_empty());
            // What is gone and has not come back with other neighbours is the triangle that was taken out.
            let mut taken: Vec<&Polygon> = gone.iter().map(|(polygon, _)| polygon).collect();
            for (polygon, _) in &come {
                let at = taken
                    .iter()
                    .position(|gone| *gone == polygon)
                    .unwrap_or_else(|| {
                        panic!("{file} {address}: a polygon the base does not hold: {polygon:?}")
                    });
                taken.swap_remove(at);
            }
            assert_eq!(
                taken.len(),
                1,
                "{file} {address}: {} polygons went with one triangle",
                taken.len()
            );
            assert_eq!(taken[0].0, 0, "{file} {address}: what went is no triangle");
            // Every other polygon that changed is a triangle that stated the taken one across an edge.
            for (polygon, across) in &gone {
                let beside = across
                    .as_ref()
                    .is_some_and(|across| across.iter().flatten().any(|n| n == taken[0]));
                assert!(
                    polygon == taken[0] || beside,
                    "{file} {address}: a triangle away from the one taken out changed: {polygon:?}"
                );
            }
        }
        cooked += 1;
    }
    eprintln!(
        "{cooked} of {} staged .ybn cook with a triangle taken out of each of their nodes, {soups_edited} nodes in all, {holding_attributes} of them holding vertex attributes; {no_triangle} hold no triangle; {} refused by name: {by_name:#?}",
        files.len(),
        by_name.len()
    );
    assert!(
        cooked > 0 || files.len() == no_triangle,
        "no staged .ybn took the edit"
    );
}

#[test]
fn every_soup_of_a_corpus_ybn_restated_with_what_it_holds_cooks_to_the_canonical_save() {
    use or_io::container::resource::{ResourceEnvelope, Rsc7};
    use or_io::decl::{Aligned, File};
    use or_io::desc::Image;
    use or_io::gta5::bounds::PhBoundAny;
    use or_source::bounds::edit::leaves;
    use or_source::bounds::Inputs;
    let what =
        "every soup of a corpus .ybn restated with what it holds cooks to the canonical save";
    let Some(files) =
        or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), "ybn").find_or_skip(what)
    else {
        return;
    };
    let how = or_source::textures::Extraction {
        from_game: true,
        virtual_path: None,
    };
    let (mut soups, mut polygons) = (0usize, 0usize);
    // What taking the first polygon out of a soup comes to: built again, or refused, by the reason the compile gives.
    let (mut built_again, mut refused) = (0usize, std::collections::BTreeMap::new());
    for path in &files {
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = fs::read(path).unwrap();
        let canonical = PhBoundAny::from_bytes(&bytes).unwrap().to_bytes().unwrap();
        let work = tempfile::tempdir().unwrap();
        let folder = or_source::bounds::extract(&bytes, "subject", work.path(), &how)
            .unwrap_or_else(|why| panic!("{file}: {why:?}"));
        let inputs = Inputs::read(&Source::open(&folder).unwrap()).unwrap();
        let bound = PhBoundAny::from_bytes(&inputs.compile().unwrap().bytes).unwrap();
        for leaf in leaves(&bound).unwrap_or_else(|why| panic!("{file}: {why}")) {
            let Some(soup) = &leaf.soup else { continue };
            soups += 1;
            polygons += soup.polygons.len();
            let again = inputs
                .restated(&leaf.address, soup)
                .unwrap_or_else(|why| panic!("{file}, {}: {why:?}", leaf.address));
            let cooked = again
                .compile()
                .unwrap_or_else(|why| panic!("{file}, {}: {why:?}", leaf.address));
            assert!(
                cooked.bytes == canonical,
                "{file}, {}: restated with what it holds, the source cooks to another file",
                leaf.address
            );
            if soup.polygons.len() < 2 {
                continue;
            }
            let mut fewer = soup.clone();
            fewer.polygons.remove(0);
            let taken = inputs
                .restated(&leaf.address, &fewer)
                .unwrap_or_else(|why| panic!("{file}, {}: {why:?}", leaf.address));
            match taken.compile() {
                Ok(made) => {
                    // The check of what the game would misread, over the image of the file.
                    let opened = Rsc7::open(&made.bytes).unwrap();
                    let aligned = Aligned::from_bytes(&opened.data);
                    let image = Image::new(aligned.bytes(), opened.virtual_size as usize);
                    or_io::gta5::bounds::validate(image, 0)
                        .unwrap_or_else(|why| panic!("{file}, {}: {why}", leaf.address));
                    let now = PhBoundAny::from_bytes(&made.bytes).unwrap();
                    let held = leaves(&now)
                        .unwrap()
                        .into_iter()
                        .find(|l| l.address == leaf.address)
                        .and_then(|l| l.soup)
                        .map_or(0, |s| s.polygons.len());
                    assert_eq!(held, fewer.polygons.len(), "{file}, {}", leaf.address);
                    built_again += 1;
                }
                Err(why) => {
                    let said = why
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ");
                    let reason = ["a shrunk hull", "an octant map", "for each of its vertices"]
                        .into_iter()
                        .find(|reason| said.contains(reason))
                        .unwrap_or_else(|| panic!("{file}, {}: {said}", leaf.address));
                    *refused.entry(reason).or_insert(0usize) += 1;
                }
            }
        }
    }
    eprintln!(
        "{soups} soup(s) of {polygons} polygon(s) in {} staged .ybn restated to the canonical save; with one polygon taken out {built_again} were built again and the rest refused: {refused:?}",
        files.len()
    );
    assert!(soups > 0, "no staged .ybn holds a polygon soup");
}

#[test]
fn the_staged_versions_of_an_area_hold_polygons_of_their_own() {
    use or_io::decl::File;
    use or_io::gta5::bounds::PhBoundAny;
    use or_source::bounds::edit::{leaves, Polygon};
    use std::collections::{BTreeMap, HashMap};
    /// How far apart one corner may be held by two files, in metres.
    const SAME_CORNER: f32 = 0.01;
    /// The side of a cell of the grid polygons are looked up in, in metres: wider than two polygons alike hold their middles apart.
    const CELL: f32 = 0.25;
    const { assert!(CELL > 2.0 * SAME_CORNER) };
    let what = "the staged versions of an area hold polygons of their own";
    let Some(files) =
        or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), "ybn").find_or_skip(what)
    else {
        return;
    };
    // Whether two files hold one polygon: the same kind on the same points, in any order.
    let alike = |a: &Polygon, b: &Polygon| {
        let near = |p: &[f32; 3], q: &[f32; 3]| {
            (0..3).map(|k| (p[k] - q[k]).powi(2)).sum::<f32>().sqrt() <= SAME_CORNER
        };
        let mut spare: Vec<&[f32; 3]> = b.points.iter().collect();
        a.kind == b.kind
            && (a.radius - b.radius).abs() <= SAME_CORNER
            && a.points.len() == b.points.len()
            && a.points.iter().all(|p| {
                let found = spare.iter().position(|q| near(p, q));
                found.map(|at| spare.swap_remove(at)).is_some()
            })
    };
    // The soups of a file and the polygons of every one of them, where the file has them.
    let polygons_of = |path: &Path| -> (usize, Vec<Polygon>) {
        let bound = PhBoundAny::from_bytes(&fs::read(path).unwrap()).unwrap();
        let (mut soups, mut held) = (0, Vec::new());
        for leaf in leaves(&bound).unwrap() {
            assert!(
                leaf.placed_by.iter().all(|m| *m
                    == [
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                        [0.0, 0.0, 0.0]
                    ]),
                "{}: a soup is placed by its composite, so its points are not the file's",
                path.display()
            );
            soups += usize::from(leaf.soup.is_some());
            held.extend(leaf.soup.into_iter().flat_map(|soup| soup.polygons));
        }
        (soups, held)
    };
    // The cell the middle of a polygon is in.
    let cell = |polygon: &Polygon| -> [i64; 3] {
        let count = polygon.points.len().max(1) as f32;
        std::array::from_fn(|k| {
            let middle = polygon.points.iter().map(|p| p[k]).sum::<f32>() / count;
            (middle / CELL).floor() as i64
        })
    };
    // How many of `sought` a file that holds `held` holds: every polygon is looked for, among those whose middle is a cell away at most.
    let held_of = |sought: &[Polygon], held: &[Polygon]| -> usize {
        let mut grid: HashMap<[i64; 3], Vec<&Polygon>> = HashMap::new();
        for polygon in held {
            grid.entry(cell(polygon)).or_default().push(polygon);
        }
        (sought.iter())
            .filter(|p| {
                let [x, y, z] = cell(p);
                (-1..=1).any(|i| {
                    (-1..=1).any(|j| {
                        (-1..=1).any(|k| {
                            (grid.get(&[x + i, y + j, z + k]).into_iter().flatten())
                                .any(|q| alike(p, q))
                        })
                    })
                })
            })
            .count()
    };
    // The files of each area: the name with `hi@` or `ma@` before it is a version of the same area, and the name alone is the third.
    let mut areas: BTreeMap<String, Vec<(String, &std::path::PathBuf)>> = BTreeMap::new();
    for path in &files {
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let area = (stem.strip_prefix("hi@"))
            .or_else(|| stem.strip_prefix("ma@"))
            .unwrap_or(&stem)
            .to_string();
        areas.entry(area).or_default().push((stem, path));
    }
    let (mut pairs, mut looked_for, mut shared) = (0usize, 0usize, 0usize);
    for (area, versions) in areas.iter().filter(|(_, versions)| versions.len() > 1) {
        // No two files of the corpus are one version of an area: what tells them apart is the prefix alone.
        let mut stems: Vec<&str> = versions.iter().map(|(stem, _)| stem.as_str()).collect();
        stems.sort_unstable();
        stems.dedup();
        assert_eq!(stems.len(), versions.len(), "{area}");
        assert!(
            stems.iter().all(
                |stem| [area.clone(), format!("hi@{area}"), format!("ma@{area}")]
                    .contains(&stem.to_string())
            ),
            "{stems:?}"
        );
        for (stem, of) in versions {
            let (soups, held) = polygons_of(of);
            // The measure finds what is there: a file holds every polygon of its own.
            assert_eq!(held_of(&held, &held), held.len(), "{stem}");
            for (other, theirs) in versions.iter().filter(|(other, _)| other != stem) {
                let found = held_of(&held, &polygons_of(theirs).1);
                eprintln!(
                    "{stem} ({soups} soup(s)): {found} of its {} polygon(s) are held by {other}",
                    held.len()
                );
                // ⚠ What the collision operations of a map rest on: were a version a copy of another, an operation could ask them
                // to be alike.
                assert!(found < held.len(), "{other} holds every polygon of {stem}");
                pairs += 1;
                looked_for += held.len();
                shared += found;
            }
        }
    }
    eprintln!(
        "{shared} of {looked_for} polygons of a staged version of an area are held by another version of it, over {pairs} ordered pair(s) of versions"
    );
    assert!(pairs > 0, "no staged area has two versions to compare");
}
