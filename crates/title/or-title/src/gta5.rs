//! A GTA V install as one value: found, mounted with keys from its own
//! executable, searched by file name, and never written into.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};

use or_io::container::rpf::crypto::{
    extract_keys, Gta5RetailCryptoProvider, KeyOrigin, NoOpCryptoProvider, RpfCryptoProvider,
    WITHOUT_A_SNAPSHOT,
};
use or_io::decl::{kind_name, File};
use or_io::gta5::formats;

use crate::game::Game;
use crate::install::discovery::discover_installs;
use crate::install::materials::GtaInstall;
use crate::vfs::{build_mounted_game, DlcCutoff, MountedGame};

/// Names install roots, `;`-separated; [`Gta5::find`] takes the first before asking any store.
pub const GAME_ROOT_ENV: &str = "OPENRAGE_GAME_ROOT";

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum InstallError {
    #[error(
        "no GTA V install found: Steam, the Rockstar launcher and Epic name none; \
         point {GAME_ROOT_ENV} at one"
    )]
    NotFound,

    #[error("{} is not a GTA V install: it holds no common.rpf", root.display())]
    NotAnInstall { root: PathBuf },

    #[error("{} is {title}, not GTA V", root.display())]
    NotGta5 { root: PathBuf, title: &'static str },

    #[error("the install at {} could not be mounted: {reason}", root.display())]
    Unmounted { root: PathBuf, reason: String },

    #[error("the install holds no file named {name}")]
    NotInInstall { name: String },

    #[error("{name} is in the install but could not be read{reason}")]
    Unreadable { name: String, reason: String },

    #[error("{} is inside the install at {}; save a mod outside it", path.display(), root.display())]
    InsideInstall { path: PathBuf, root: PathBuf },

    #[error(transparent)]
    File(#[from] or_io::Error),
}

/// A mounted GTA V install, Legacy or Enhanced.
pub struct Gta5 {
    root: PathBuf,
    mount: Result<MountedGame, String>,
    /// What the keys lack, said once for every read they fail.
    missing_keys: Option<String>,
    /// Lowercase file name to the logical path its last copy in load order has.
    index: OnceLock<HashMap<String, String>>,
}

impl Gta5 {
    /// The install [`GAME_ROOT_ENV`] names, else the first Steam, the Rockstar launcher or Epic names.
    pub fn find() -> Result<Gta5, InstallError> {
        if let Some(root) = std::env::var_os(GAME_ROOT_ENV).and_then(|raw| {
            raw.to_string_lossy()
                .split(';')
                .map(str::trim)
                .find(|s| !s.is_empty())
                .map(PathBuf::from)
        }) {
            return Self::at(root);
        }
        let install = discover_installs()
            .into_iter()
            .find(|i| is_gta5(i.game))
            .ok_or(InstallError::NotFound)?;
        Self::at(install.install_dir)
    }

    /// The install at `root`, mounted with every DLC pack.
    ///
    /// ⚠ Succeeds without keys: an archive they cannot open fails each read
    /// from it, naming what is missing.
    pub fn at(root: impl AsRef<Path>) -> Result<Gta5, InstallError> {
        let root = root.as_ref().to_path_buf();
        let install = GtaInstall { root: root.clone() };
        if !install.common_rpf().is_file() {
            return Err(InstallError::NotAnInstall { root });
        }
        // A folder that holds `common.rpf` and no profile's marker is an install without its executable.
        let game = Game::detect_from_root(&root).unwrap_or(Game::GtaVLegacy);
        if !is_gta5(game) {
            return Err(InstallError::NotGta5 {
                root,
                title: game.label(),
            });
        }
        let (crypto, missing_keys) = crypto_for(&root, game);
        let mount = build_mounted_game(&install, crypto, DlcCutoff::All)
            .map_err(|e| with_note(e.to_string(), missing_keys.as_deref()));
        Ok(Gta5 {
            root,
            mount,
            missing_keys,
            index: OnceLock::new(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The file `name` resolves to, opened as `T`: base archives, then DLC in load order, the last copy winning.
    pub fn open<T: File>(&self, name: &str) -> Result<T, InstallError> {
        check_kind(name, T::EXTENSION)?;
        Ok(T::from_bytes(&self.bytes(name)?)?)
    }

    /// The bytes of the file `name` resolves to, a resource's with its RSC7 header.
    pub fn bytes(&self, name: &str) -> Result<Vec<u8>, InstallError> {
        let logical = self
            .index()?
            .get(&name.to_ascii_lowercase())
            .ok_or_else(|| InstallError::NotInInstall { name: name.into() })?;
        self.mounted()?
            .vfs
            .open(logical)
            .ok_or_else(|| InstallError::Unreadable {
                name: name.into(),
                reason: with_note(String::new(), self.missing_keys.as_deref()),
            })
    }

    /// Every file name the install resolves, lowercase and sorted.
    pub fn names(&self) -> Result<Vec<&str>, InstallError> {
        let mut names: Vec<&str> = self.index()?.keys().map(String::as_str).collect();
        names.sort_unstable();
        Ok(names)
    }

    /// Saves `value` to `path`, making its folder; refused for any path inside the install.
    pub fn save_to_mod<T: File>(
        &self,
        value: &T,
        path: impl AsRef<Path>,
    ) -> Result<(), InstallError> {
        let path = path.as_ref();
        let write_failed = |source| or_io::Error::FileWriteFailed {
            path: path.to_path_buf(),
            source,
        };
        let root = std::fs::canonicalize(&self.root).map_err(write_failed)?;
        if resolved(path).map_err(write_failed)?.starts_with(&root) {
            return Err(InstallError::InsideInstall {
                path: path.to_path_buf(),
                root: self.root.clone(),
            });
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(write_failed)?;
        }
        Ok(value.save_as(path)?)
    }

    fn mounted(&self) -> Result<&MountedGame, InstallError> {
        self.mount
            .as_ref()
            .map_err(|reason| InstallError::Unmounted {
                root: self.root.clone(),
                reason: reason.clone(),
            })
    }

    fn index(&self) -> Result<&HashMap<String, String>, InstallError> {
        let mount = self.mounted()?;
        Ok(self.index.get_or_init(|| {
            let mut index = HashMap::new();
            mount.for_each_entry(|facts| {
                index.insert(facts.leaf.to_ascii_lowercase(), facts.logical.clone());
            });
            index
        }))
    }
}

fn is_gta5(game: Game) -> bool {
    game == Game::GtaVLegacy || game == Game::GtaVEnhanced
}

/// The provider the install's own executable yields, and what it cannot open.
fn crypto_for(root: &Path, game: Game) -> (Arc<dyn RpfCryptoProvider>, Option<String>) {
    let Some(exe) = game.find_executable(root) else {
        return (
            Arc::new(NoOpCryptoProvider::new()),
            Some(format!(
                "no {} to extract keys from, so only unencrypted archives open",
                game.executable_name()
            )),
        );
    };
    match extract_keys(&exe, &|_| {}) {
        Ok(keys) => {
            let missing = (keys.provenance.ng_keys == KeyOrigin::Absent)
                .then(|| WITHOUT_A_SNAPSHOT.to_string());
            (
                Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))),
                missing,
            )
        }
        Err(e) => (
            Arc::new(NoOpCryptoProvider::new()),
            Some(format!(
                "keys could not be extracted from {}: {e}",
                exe.display()
            )),
        ),
    }
}

fn with_note(reason: String, note: Option<&str>) -> String {
    match note {
        Some(note) if reason.contains(note) => reason,
        Some(note) if reason.is_empty() => format!(": {note}"),
        Some(note) => format!("{reason} ({note})"),
        None => reason,
    }
}

/// `T::open`'s refusal for a name whose extension is another declared format.
fn check_kind(name: &str, ext: &'static str) -> Result<(), or_io::Error> {
    let found = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match found {
        Some(f) if f != ext && formats::format(&f).is_some() => Err(or_io::Error::WrongKind {
            found: kind_name(&f),
            wanted: kind_name(ext),
        }),
        _ => Ok(()),
    }
}

/// `path` made absolute with `.` and `..` removed, the part that exists canonicalized.
fn resolved(path: &Path) -> std::io::Result<PathBuf> {
    let mut lexical = PathBuf::new();
    for component in std::path::absolute(path)?.components() {
        match component {
            Component::ParentDir => {
                lexical.pop();
            }
            Component::CurDir => {}
            other => lexical.push(other),
        }
    }
    let mut existing = lexical.as_path();
    let mut below = Vec::new();
    while !existing.exists() {
        let (Some(parent), Some(leaf)) = (existing.parent(), existing.file_name()) else {
            break;
        };
        below.push(leaf);
        existing = parent;
    }
    let mut out = std::fs::canonicalize(existing)?;
    out.extend(below.into_iter().rev());
    Ok(out)
}
