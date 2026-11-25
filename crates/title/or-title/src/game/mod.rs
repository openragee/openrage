pub mod overlay_loader;
pub mod paths;
pub mod profile;
pub mod profiles;
pub mod registry;

pub use overlay_loader::{check_overlay_loader, OverlayLoaderReport, OverlayLoaders};
pub use paths::GamePaths;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use or_io::Generation;

pub use profile::{
    Brand, ContainerFamily, DlcPolicy, GameId, GameProfile, InstallLayout, StoreHints,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Game(GameId);

#[allow(non_upper_case_globals)]
impl Game {
    pub const GtaVLegacy: Game = Game("gta-v-legacy");
    pub const GtaVEnhanced: Game = Game("gta-v-enhanced");
    pub const Rdr2: Game = Game("rdr2");
    pub const GtaVI: Game = Game("gta-vi");
}

impl Game {
    pub fn all() -> impl Iterator<Item = Game> {
        registry::all().into_iter().map(|p| Game(p.id()))
    }

    pub fn by_id(id: &str) -> Option<Game> {
        registry::by_id(id).map(|p| Game(p.id()))
    }

    pub fn profile(self) -> &'static dyn GameProfile {
        registry::by_id(self.0)
            .expect("a Game can only be built from a registered id; see game::registry")
    }

    pub fn label(self) -> &'static str {
        self.profile().label()
    }

    pub fn description(self) -> &'static str {
        self.profile().description()
    }

    pub fn executable_candidates(self) -> &'static [&'static str] {
        self.profile().executable_candidates()
    }

    pub fn executable_name(self) -> &'static str {
        self.profile().executable_name()
    }

    pub fn find_executable(self, install_root: &Path) -> Option<PathBuf> {
        self.profile().find_executable(install_root)
    }

    pub fn supports_key_extraction(self) -> bool {
        self.profile().supports_key_extraction()
    }

    pub fn asset_slug(self) -> GameId {
        self.0
    }

    pub fn detect_from_root(root: &Path) -> Self {
        registry::detect(root)
            .map(|p| Game(p.id()))
            .unwrap_or(Self::GtaVLegacy)
    }

    pub fn loads_gen9_content(self) -> bool {
        self.profile().native_target().is_gen9()
    }

    pub fn generation(self) -> Generation {
        self.profile().native_target()
    }

    pub fn accepts_dlc_pack(self, pack_name: &str) -> bool {
        self.profile().dlc_policy().accepts(pack_name)
    }
}

impl Serialize for Game {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for Game {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        let id = match raw.as_str() {
            "GtaVLegacy" => "gta-v-legacy",
            "GtaVEnhanced" => "gta-v-enhanced",
            "Rdr2" => "rdr2",
            other => other,
        };
        Game::by_id(id).ok_or_else(|| serde::de::Error::custom(format!("unknown title `{raw}`")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_edition_does_not_load_gen9() {
        assert!(!Game::GtaVLegacy.loads_gen9_content());
        assert!(Game::GtaVEnhanced.loads_gen9_content());
        assert!(!Game::Rdr2.loads_gen9_content());
    }

    #[test]
    fn edition_maps_to_generation() {
        assert_eq!(Game::GtaVLegacy.generation(), Generation::Gen8);
        assert_eq!(Game::GtaVEnhanced.generation(), Generation::Gen9);

        let rdr2 = Game::Rdr2.generation();
        assert_ne!(
            rdr2,
            Generation::Gen8,
            "RDR2 must not claim GTA V's Legacy layouts; that is the claim \
             section 9's step 6 exists to make expressible"
        );
        assert_ne!(rdr2, Generation::Gen9);
        assert_eq!(rdr2.slug(), "rdr2");
        assert_eq!(rdr2.edition_label(), "Red Dead Redemption 2");
    }

    #[test]
    fn every_variant_resolves_to_a_profile() {
        for game in Game::all() {
            assert_eq!(game.profile().id(), game.asset_slug());
        }
    }

    #[test]
    fn support_follows_the_container_family() {
        assert!(Game::GtaVLegacy.profile().is_supported());
        assert!(Game::GtaVEnhanced.profile().is_supported());
        assert!(
            Game::Rdr2.profile().is_supported(),
            "RPF8 archives open, so RDR2 is browsable"
        );
        assert!(
            !Game::GtaVI.profile().is_supported(),
            "RPF9 archives open individually; the install is browsed as a plain folder"
        );
    }

    #[test]
    fn an_unsupported_title_explains_itself() {
        for game in Game::all() {
            let profile = game.profile();
            if profile.is_supported() {
                continue;
            }
            let description = profile.description();
            let family = profile
                .container()
                .label()
                .split(" / ")
                .next()
                .expect("a label");
            assert!(
                description.len() > 20 && description.contains(family),
                "{}'s description should name the container that blocks it: {description:?}",
                game.label()
            );
        }
    }
}
