//! The list of known titles, and lookup over it.
//!
//! ⚠ Detection order is load-bearing: a title with a looser install marker
//! asked before one with a more specific marker claims installs that are not
//! its own. An Enhanced install ships `GTA5.exe` beside `GTA5_Enhanced.exe`,
//! so Legacy asked first reads every Enhanced install as Legacy. Registered
//! titles are asked after every builtin. A GTA VI build's title id is the
//! most specific marker, so those are asked first.

use std::path::Path;
use std::sync::{OnceLock, RwLock};

use super::profile::GameProfile;
use super::profiles::{Gta6, Gta6Ps5, GtaVEnhanced, GtaVLegacy, Rdr2};

static BUILTIN: &[&dyn GameProfile] = &[&Gta6, &Gta6Ps5, &GtaVEnhanced, &GtaVLegacy, &Rdr2];

fn registered() -> &'static RwLock<Vec<&'static dyn GameProfile>> {
    static REGISTERED: OnceLock<RwLock<Vec<&'static dyn GameProfile>>> = OnceLock::new();
    REGISTERED.get_or_init(|| RwLock::new(Vec::new()))
}

pub fn register(profile: &'static dyn GameProfile) -> bool {
    if by_id(profile.id()).is_some() {
        return false;
    }
    let mut list = registered().write().expect("title registry poisoned");
    if list.iter().any(|p| p.id() == profile.id()) {
        return false;
    }
    list.push(profile);
    drop(list);
    publish_targets(profile);
    true
}

fn publish_builtin_targets() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        for profile in BUILTIN {
            publish_targets(*profile);
        }
    });
}

fn publish_targets(profile: &'static dyn GameProfile) {
    for descriptor in profile.target_labels() {
        or_io::generation::register(descriptor);
    }
}

pub fn all() -> Vec<&'static dyn GameProfile> {
    publish_builtin_targets();
    let mut out: Vec<&'static dyn GameProfile> = BUILTIN.to_vec();
    out.extend(registered().read().expect("title registry poisoned").iter());
    out
}

pub fn supported() -> impl Iterator<Item = &'static dyn GameProfile> {
    all().into_iter().filter(|p| p.is_supported())
}

pub fn by_id(id: &str) -> Option<&'static dyn GameProfile> {
    publish_builtin_targets();
    BUILTIN.iter().copied().find(|p| p.id() == id).or_else(|| {
        registered()
            .read()
            .expect("title registry poisoned")
            .iter()
            .copied()
            .find(|p| p.id() == id)
    })
}

pub fn detect(root: &Path) -> Option<&'static dyn GameProfile> {
    all().into_iter().find(|p| p.matches_install(root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::profile::ContainerFamily;
    use or_io::Generation;

    /// A folder removed when the first of the pair drops, so hold it as long as the second is used.
    fn scratch(name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let scratch = tempfile::Builder::new()
            .prefix(&format!("or-title-profile-{name}-"))
            .tempdir()
            .expect("scratch");
        let dir = scratch.path().to_path_buf();
        (scratch, dir)
    }

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"").expect("touch");
    }

    #[test]
    fn identifiers_are_unique() {
        let mut seen = Vec::new();
        for profile in all() {
            assert!(
                !seen.contains(&profile.id()),
                "duplicate profile id `{}`",
                profile.id()
            );
            seen.push(profile.id());
        }
    }

    #[test]
    fn an_enhanced_install_is_not_mistaken_for_legacy() {
        let (_scratch, dir) = scratch("enhanced");
        touch(&dir, "GTA5_Enhanced.exe");
        touch(&dir, "GTA5.exe");

        let detected = detect(&dir).expect("should detect a title");
        assert_eq!(detected.id(), "gta-v-enhanced");
    }

    #[test]
    fn a_legacy_install_detects_as_legacy() {
        let (_scratch, dir) = scratch("legacy");
        touch(&dir, "GTA5.exe");

        assert_eq!(detect(&dir).expect("detect").id(), "gta-v-legacy");
    }

    #[test]
    fn an_unrecognised_directory_detects_as_nothing() {
        let (_scratch, dir) = scratch("empty");
        assert!(detect(&dir).is_none());
    }

    #[test]
    fn both_gta_v_editions_share_a_container_and_a_layout() {
        let legacy = by_id("gta-v-legacy").expect("legacy");
        let enhanced = by_id("gta-v-enhanced").expect("enhanced");

        assert_eq!(legacy.container(), ContainerFamily::Rpf7);
        assert_eq!(enhanced.container(), ContainerFamily::Rpf7);
        assert_eq!(legacy.install_layout(), enhanced.install_layout());
    }

    #[test]
    fn enhanced_accepts_both_asset_targets() {
        let enhanced = by_id("gta-v-enhanced").expect("enhanced");
        assert!(enhanced.asset_targets().contains(&Generation::Gen8));
        assert!(enhanced.asset_targets().contains(&Generation::Gen9));
        assert_eq!(enhanced.native_target(), Generation::Gen9);
    }

    #[test]
    fn the_dlc_policy_reproduces_the_runtime_behaviour() {
        let legacy = by_id("gta-v-legacy").expect("legacy").dlc_policy();
        let enhanced = by_id("gta-v-enhanced").expect("enhanced").dlc_policy();

        for pack in ["mpg9ec", "patchdayg9ecng", "mpsum2_g9ec", "mp2025_01_G9EC"] {
            assert!(!legacy.accepts(pack), "Legacy must not mount {pack}");
            assert!(enhanced.accepts(pack), "Enhanced must mount {pack}");
        }

        for pack in ["mp2024_02", "mpsum2", "mpheist3", "patchday28ng"] {
            assert!(legacy.accepts(pack), "Legacy must mount {pack}");
            assert!(enhanced.accepts(pack), "Enhanced must mount {pack}");
        }
    }

    #[test]
    fn rdr2_is_browsable_but_extracts_no_gta_keys() {
        let rdr2 = by_id("rdr2").expect("rdr2");
        assert_eq!(rdr2.container(), ContainerFamily::Rpf8);
        assert!(rdr2.is_supported());
        assert!(!rdr2.supports_key_extraction());

        assert!(supported().any(|p| p.id() == "rdr2"));
        assert!(all().iter().any(|p| p.id() == "rdr2"));
    }

    #[test]
    fn every_key_extracting_title_names_a_distinct_cache() {
        let mut caches = Vec::new();
        for profile in all() {
            if profile.supports_key_extraction() {
                let cache = profile
                    .keys_cache_name()
                    .unwrap_or_else(|| panic!("{} extracts keys but names no cache", profile.id()));
                assert!(!caches.contains(&cache), "duplicate keys cache `{cache}`");
                caches.push(cache);
            }
        }
        assert_eq!(caches.len(), 2);
    }
}
