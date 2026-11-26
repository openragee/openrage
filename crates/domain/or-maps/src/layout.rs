//! Where the file of a map source lands inside a resource and inside a singleplayer DLC.
//!
//! A map file, a type file, a model and a texture dictionary cook to `stream/<name>.<extension>`, as every streamed file does: that is
//! the one place a FiveM resource has for them, and the manifest of the maps sits beside them.
//!
//! A singleplayer DLC keeps them as the packs of the game keep theirs, measured on two installs
//! (`how_the_packs_of_an_install_carry_their_maps`). The map files and their manifest go into one nested archive of the level's
//! folder, which the pack registers as map data: every archive of maps of both installs is under that folder and every one a pack
//! registers is stated so. A type file goes into an archive of the level's props folder named for it, with the models and the
//! texture dictionaries its archetypes name, and the pack registers that archive stating nothing and asks for the type file by an
//! `.ityp` name of the archive's folder, stating props: no pack asks for a type file that sits beside its maps, and each type file
//! a pack asks for is the one type file of an archive that holds models and no map.
//!
//! ⚠ What a build cannot state as the game's packs do keeps the cell of a singleplayer DLC locked for maps
//! (`or_project::cannot_ship`): no pack of either install turns an archive of maps on when it starts, which is the one way a pack
//! built here turns a file on. A type file that declares an interior is refused for a pack too: the packs never ask for one, and
//! state it in a list of the manifest a build does not write.

/// The folder FiveM streams from.
pub const STREAM: &str = "stream";

/// Where a cooked file called `file` (`harbour.ymap`) lands inside the resource.
pub fn stream_path(file: &str) -> String {
    format!("{STREAM}/{file}")
}

/// The `data_file` type a manifest registers a type file under, the one the retail packs register theirs under.
pub const TYPE_FILE: &str = "DLC_ITYP_REQUEST";

/// The line of a FiveM manifest that declares the resource a map, and its value.
pub const MAP_LINE: (&str, &str) = (or_project::resource::manifest::MAP_KEY, "yes");

/// The folder of a singleplayer DLC the archive of a content's maps is written in: the level's, which every archive of maps of the installs' packs is under.
pub const PACK_FOLDER: &str = or_project::LEVEL_FOLDER;

/// What a pack states an archive of map files holds, as the install's packs state it.
pub const MAP_DATA: &str = or_project::MAP_DATA;

/// The folder of a singleplayer DLC the archives of type files and models are written in, where the installs' packs keep the ones they ask a type file from.
pub fn props_folder() -> String {
    format!("{PACK_FOLDER}/props")
}

/// What a pack states of a type file it asks for, as the install's packs state it.
pub const PROPS: &str = "CONTENTS_PROPS";

/// The nested archive of a singleplayer DLC that holds the maps the content `module` ships and their manifest, by its path in the pack.
pub fn pack_archive(module: &str) -> String {
    format!("{PACK_FOLDER}/{module}.rpf")
}

/// Where a map file or the manifest called `file` lands inside a singleplayer DLC: in the archive of its content's maps.
pub fn pack_path(module: &str, file: &str) -> String {
    format!("{}/{file}", pack_archive(module))
}

/// The nested archive of a singleplayer DLC named `name` that holds a type file with its models and textures, by its path in the pack.
pub fn props_archive(name: &str) -> String {
    format!("{}/{name}.rpf", props_folder())
}

/// Where a type file, a model or a texture dictionary called `file` lands inside a singleplayer DLC: in the props archive `name`.
pub fn props_path(name: &str, file: &str) -> String {
    format!("{}/{file}", props_archive(name))
}

/// The name a pack asks for the type file `name` by: the `.ityp` of the folder its archive is in.
pub fn pack_type_request(name: &str) -> String {
    format!("{}/{name}.ityp", props_folder())
}
