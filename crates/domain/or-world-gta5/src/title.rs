//! GTA V as a title of the world contract: one title for the Legacy and the Enhanced game, whose
//! options are the level of DLC the mount stops at and the map as story or as multiplayer.

use std::sync::Arc;

use or_io::container::rpf::crypto::{
    extract_keys, Gta5RetailCryptoProvider, KeysCache, NoOpCryptoProvider, RpfCryptoProvider,
};
use or_title::vfs::{build_mounted_game_with, overlay_root, DlcCutoff, MountOptions};
use or_title::{GtaInstall, MapMode};
use or_world::contract::{GameId, Keys, Opening, WorldError, WorldInstall, WorldSpec, WorldTitle};
use or_world::sentence;
use or_world::slots::{Choices, OptionId, OptionKind, OptionValue};

use crate::install::Gta5World;

/// What names the title in a kept file's head and in the kept file's name.
pub const TITLE: &str = "gta5";
/// The Legacy game, by the id its profile registers under.
pub const LEGACY: GameId = GameId("gta-v-legacy");
/// The Enhanced game, by the id its profile registers under.
pub const ENHANCED: GameId = GameId("gta-v-enhanced");
/// The pack of the DLC list the mount stops at, by the name the list gives it; unset, every pack is mounted.
pub const LEVEL: OptionId = OptionId(1);
/// The map as the story has it or as multiplayer has it; unset, every change set of every pack applies.
pub const MODE: OptionId = OptionId(2);
/// The choice of [`MODE`] for the story's map.
pub const STORY: u32 = 0;
/// The choice of [`MODE`] for multiplayer's map.
pub const MULTIPLAYER: u32 = 1;

const GAMES: [GameId; 2] = [LEGACY, ENHANCED];

/// GTA V's world, Legacy and Enhanced.
#[derive(Debug, Clone, Copy, Default)]
pub struct Gta5Title;

fn unread(what: impl Into<String>, why: impl std::fmt::Display) -> WorldError {
    WorldError::Unread {
        what: what.into(),
        why: why.to_string(),
    }
}

/// What opens the archives of an install under `keys`.
fn crypto_of(keys: Keys<'_>) -> Result<Arc<dyn RpfCryptoProvider>, WorldError> {
    let keys = match keys {
        Keys::None => return Ok(Arc::new(NoOpCryptoProvider::new())),
        Keys::Executable(exe) => extract_keys(exe, &|_| {})
            .map_err(|e| unread(format!("the keys of {}", exe.display()), e))?,
        Keys::Snapshot(file) => {
            let what = format!("the keys kept in {}", file.display());
            let cache = KeysCache::read_from(file).map_err(|e| unread(&what, e))?;
            let cache = cache.ok_or_else(|| unread(&what, "there is no such file"))?;
            cache.into_keys().map_err(|e| unread(&what, e))?
        }
    };
    Ok(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))))
}

/// Where the mount of `world` stops and how it is made, by the world's options and layers.
pub(crate) fn mount_of(world: &WorldSpec) -> Result<(DlcCutoff, MountOptions), WorldError> {
    let (mut cutoff, mut options) = (DlcCutoff::All, MountOptions::install_only());
    for (id, value) in &world.options {
        match (*id, value) {
            (LEVEL, OptionValue::Named(pack)) if !pack.trim().is_empty() => {
                cutoff = DlcCutoff::Level(pack.trim().to_owned());
            }
            (MODE, OptionValue::Choice(STORY)) => options.mode = Some(MapMode::Story),
            (MODE, OptionValue::Choice(MULTIPLAYER)) => options.mode = Some(MapMode::Multiplayer),
            _ => return Err(WorldError::Option(*id)),
        }
    }
    // ⚠ The mount reads one layer and no other: the install's own overlay folder, whose archives
    // stand in for the install's a copy at a time.
    let overlay = overlay_root(&world.install);
    match &world.layers[..] {
        [] => {}
        [layer] if layer.path == overlay => {
            options = MountOptions {
                mode: options.mode,
                ..MountOptions::default()
            }
        }
        _ => {
            return Err(unread(
                "the layers of the world",
                format_args!("a GTA V mount reads one layer, {}", overlay.display()),
            ))
        }
    }
    Ok((cutoff, options))
}

impl WorldTitle for Gta5Title {
    fn id(&self) -> &'static str {
        TITLE
    }

    fn games(&self) -> &'static [GameId] {
        &GAMES
    }

    fn options(&self) -> Vec<OptionKind> {
        vec![
            OptionKind {
                id: LEVEL,
                label: sentence!("DLC pack"),
                choices: Choices::Named,
            },
            OptionKind {
                id: MODE,
                label: sentence!("Mode"),
                choices: Choices::OneOf(vec![
                    (STORY, sentence!("Story")),
                    (MULTIPLAYER, sentence!("Multiplayer")),
                ]),
            },
        ]
    }

    fn open(
        &self,
        world: &WorldSpec,
        with: &Opening<'_>,
    ) -> Result<Arc<dyn WorldInstall>, WorldError> {
        if !GAMES.contains(&world.game) {
            return Err(WorldError::NotItsGame(world.game.0));
        }
        let (cutoff, options) = mount_of(world)?;
        let crypto = crypto_of(with.keys)?;
        let install = GtaInstall {
            root: world.install.clone(),
        };
        let game = build_mounted_game_with(&install, crypto, cutoff, options)
            .map_err(|e| unread(format!("the install at {}", world.install.display()), e))?;
        if with.cancel.is_set() {
            return Err(WorldError::Cancelled);
        }
        Ok(Arc::new(Gta5World::over(game, world.install.clone())))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use or_world::contract::Layer;

    use super::*;

    fn with(options: Vec<(OptionId, OptionValue)>) -> WorldSpec {
        WorldSpec {
            options,
            ..WorldSpec::of(LEGACY, Path::new("an install"))
        }
    }

    #[test]
    fn an_option_reaches_the_mount_as_the_cutoff_and_the_mode_it_names() {
        let stated = |options| mount_of(&with(options)).map_err(|e| e.to_string());
        assert_eq!(
            stated(Vec::new()),
            Ok((DlcCutoff::All, MountOptions::install_only()))
        );
        let level = (LEVEL, OptionValue::Named(" mpheist ".to_owned()));
        for (choice, mode) in [(STORY, MapMode::Story), (MULTIPLAYER, MapMode::Multiplayer)] {
            let options = vec![level.clone(), (MODE, OptionValue::Choice(choice))];
            let expected = MountOptions {
                mode: Some(mode),
                ..MountOptions::install_only()
            };
            assert_eq!(
                stated(options),
                Ok((DlcCutoff::Level("mpheist".to_owned()), expected))
            );
        }
        assert!(stated(vec![(LEVEL, OptionValue::Named("  ".to_owned()))]).is_err());
    }

    #[test]
    fn the_overlay_folder_as_a_layer_has_the_mount_read_the_overlay_first_in_the_mode_asked() {
        let mut world = with(vec![(MODE, OptionValue::Choice(MULTIPLAYER))]);
        world.layers = vec![Layer {
            path: overlay_root(&world.install),
        }];
        let (_, options) = mount_of(&world).expect("the one layer a mount reads");
        assert!(options.reads_the_overlay());
        assert_eq!(options.mode, Some(MapMode::Multiplayer));
        world.layers.clear();
        let (_, options) = mount_of(&world).expect("no layer");
        assert!(!options.reads_the_overlay());
    }
}
