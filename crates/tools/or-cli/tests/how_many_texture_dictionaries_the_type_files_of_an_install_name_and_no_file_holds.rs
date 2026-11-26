//! How many of the texture dictionaries the archetypes of an install's own type files name are held by no file of the install: what
//! `project check-resource --game` would say of each type file the game ships, asked with the rule's own question
//! (`or_project::resource::unheld_texture_dictionaries`) of the catalogue the command asks. It is the figure the severity of
//! `texture-dictionary-missing` is decided on.
//!
//! Point `OPENRAGE_GAME_ROOT` at an install root. Without one this skips, which is not a pass.
//!
//! ⚠ The install is opened read-only and nothing here writes into one. It prints counts, and never a name of the game.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use or_depends::{Catalogue, Kind};
use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider, RpfCryptoProvider};
use or_io::decl::File;
use or_io::meta::typed::{CBaseArchetypeDefAny, CMapTypes};
use or_project::content::rule::Severity;
use or_project::resource::{rule_of, unheld_texture_dictionaries};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::{build_mounted_game, DlcCutoff, Game, GtaInstall, MountedGame};

const WHAT: &str =
    "how_many_texture_dictionaries_the_type_files_of_an_install_name_and_no_file_holds";

/// The share of the game's own files a rule may fire on and still warn: past it, the rule says of a resource what is true of the game.
const A_TENTH: f64 = 0.1;

const MODELS: [Kind; 3] = [Kind::Drawable, Kind::Fragment, Kind::DrawableDict];

fn mount(root: &Path) -> Option<MountedGame> {
    let exe = Game::detect_from_root(root).find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    let crypto: Arc<dyn RpfCryptoProvider> =
        Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
    build_mounted_game(&GtaInstall::from_path(root)?, crypto, DlcCutoff::All).ok()
}

#[derive(Default)]
struct Figures {
    type_files: usize,
    unread: usize,
    dictionaries_held: usize,
    /// The type files with an archetype that names a dictionary no file holds: the files the rule fires on.
    firing: usize,
    /// The archetypes that name a dictionary, and those whose dictionary no file holds.
    naming: usize,
    naming_unheld: usize,
    /// The dictionaries named, each once, those no file holds, and those of them that are the name of a model the install holds.
    named: BTreeSet<u32>,
    unheld: BTreeSet<u32>,
    unheld_named_as_a_model: usize,
    /// The control: the type files with an archetype, no interior, whose model no file holds.
    without_a_model: usize,
}

fn measure(game: &MountedGame) -> Figures {
    let mut catalogue = Catalogue::new();
    let mut type_files = Vec::new();
    game.for_each_entry(|facts| {
        catalogue.index(facts);
        if facts.leaf.to_ascii_lowercase().ends_with(".ytyp") {
            type_files.push(facts.logical.clone());
        }
    });
    let mut figures = Figures {
        type_files: type_files.len(),
        dictionaries_held: catalogue.count_of(Kind::TextureDict),
        ..Figures::default()
    };
    let holds_dictionary = |name: u32| catalogue.find(name, Kind::TextureDict).is_some();
    let holds_model = |name: u32| catalogue.find_any(name, &MODELS).is_some();
    for logical in &type_files {
        let read = (game.vfs.open(logical)).and_then(|bytes| CMapTypes::from_bytes(&bytes).ok());
        let Some(types) = read else {
            figures.unread += 1;
            continue;
        };
        let unheld = unheld_texture_dictionaries(&types, holds_dictionary);
        figures.firing += usize::from(!unheld.is_empty());
        figures.naming_unheld += unheld.len();
        figures.unheld.extend(unheld.iter().map(|name| name.hash));
        let mut modelless = false;
        for archetype in &types.archetypes {
            let textures = archetype.texture_dictionary.hash;
            if textures != 0 {
                figures.naming += 1;
                figures.named.insert(textures);
            }
            // As the check asks of a model: the asset, or else the archetype's own name, or the dictionary of drawables it is in.
            let interior = matches!(archetype, CBaseArchetypeDefAny::CMloArchetypeDef(_));
            let asset = match archetype.asset_name.hash {
                0 => archetype.name.hash,
                asset => asset,
            };
            let dictionary = archetype.drawable_dictionary.hash;
            modelless |=
                !interior && !holds_model(asset) && !(dictionary != 0 && holds_model(dictionary));
        }
        figures.without_a_model += usize::from(modelless);
    }
    figures.unheld_named_as_a_model = (figures.unheld.iter())
        .filter(|name| holds_model(**name))
        .count();
    figures
}

fn report(f: &Figures) {
    let read = f.type_files - f.unread;
    println!(
        "{} type files, {} of them unread; the install holds {} texture dictionaries by name",
        f.type_files, f.unread, f.dictionaries_held
    );
    println!(
        "{} of the {read} type files read have an archetype that names a texture dictionary no file of the install holds",
        f.firing
    );
    println!(
        "  {} of the {} archetypes that name a dictionary name one no file holds",
        f.naming_unheld, f.naming
    );
    println!(
        "  {} of the {} dictionaries named are held by no file; {} of those are the name of a model the install holds",
        f.unheld.len(),
        f.named.len(),
        f.unheld_named_as_a_model
    );
    println!(
        "the control: {} of the {read} have an archetype, no interior, whose model no file of the install holds",
        f.without_a_model
    );
}

#[test]
fn the_rule_for_a_texture_dictionary_nothing_holds_is_a_note_while_it_fires_on_a_tenth_of_the_games_own_type_files(
) {
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, format!("{GAME_ROOT_ENV} names no install"));
        return;
    };
    let game = mount(&root).expect("the install mounts");
    let figures = measure(&game);
    report(&figures);
    let f = &figures;
    let read = f.type_files - f.unread;

    // Enough was read for the figures to mean something, and the catalogue answers: it holds dictionaries and resolves the ones it holds.
    assert!(read >= 1_000 && f.unread == 0, "too little was read");
    assert!(f.dictionaries_held >= 10_000 && f.unheld.len() < f.named.len());
    // ⚠ A rule that fires on a tenth of the game's own type files says of a resource what the game says of itself: it is a note,
    // and a warning only once it fires on fewer.
    let share = f.firing as f64 / read as f64;
    let rule = rule_of("texture-dictionary-missing").expect("the rule is a row");
    match share >= A_TENTH {
        true => assert_eq!(
            rule.severity,
            Severity::Info,
            "the rule fires on {} of the {read} type files of the game and still warns",
            f.firing
        ),
        false => assert_eq!(
            rule.severity,
            Severity::Warning,
            "the rule fires on {} of the {read} type files of the game and no longer warns",
            f.firing
        ),
    }
}
