//! The folders an asset is edited as and compiled back from: [`asset`] holds `asset.json`,
//! [`textures`] the source of a `.ytd`, [`models`] that of a `.ydr` or a `.ydd` (and [`drawable_values`] what its `drawable.json` states of the base), [`fragment`] that of a `.yft`, [`metadata`] that of a `.ymt`, `.ymap`
//! or `.ytyp`, [`bounds`] that of a `.ybn`, [`paths`] that of a `.ynd`, [`create`] the start of a source that is not a game file's, and [`cook`] the recook of one saved asset, [`optimise`] the format, mips and size of a textures source's pictures, [`game_import`] a game file's way into a project as a source and
//! [`git`] the ignore rules that keep game content out of a repository, and [`origin_check`] whether
//! the game's file a source was taken from has changed. Nothing here draws or watches a folder.

use std::fmt;

use serde::Serialize;

pub mod asset;
pub mod bounds;
pub mod budget;
pub mod cook;
pub mod create;
pub mod crop;
mod document;
pub mod drawable_values;
pub mod fragment;
pub mod game_import;
pub mod git;
mod header;
pub mod json;
pub mod maps;
pub mod metadata;
pub mod models;
pub mod optimise;
pub mod origin_check;
pub mod particles;
pub mod paths;
mod reencode;
pub mod sample;
pub mod schemas;
pub mod textures;

/// Why something in a source could not be read, cooked or written, as data a panel can list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    /// The file the problem is in, relative to the source folder with `/` between names.
    pub file: Option<String>,
    /// The texture it concerns.
    pub texture: Option<String>,
    pub message: String,
}

impl Problem {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            file: None,
            texture: None,
            message: message.into(),
        }
    }

    pub fn in_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    pub fn of_texture(mut self, texture: impl Into<String>) -> Self {
        self.texture = Some(texture.into());
        self
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(file) = &self.file {
            write!(f, "{file}: ")?;
        }
        if let Some(texture) = &self.texture {
            write!(f, "texture {texture}: ")?;
        }
        f.write_str(&self.message)
    }
}

impl std::error::Error for Problem {}
