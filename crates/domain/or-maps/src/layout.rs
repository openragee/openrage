//! Where the file of a map source lands inside a resource and inside a singleplayer DLC.
//!
//! A map file, a type file, a model and a texture dictionary cook to `stream/<name>.<extension>`, as every streamed file does: that is
//! the one place a FiveM resource has for them, and the manifest of the maps sits beside them.
//!
//! A singleplayer DLC keeps them inside one nested archive of the pack, which the pack registers as map data, with each type file asked
//! for by an `.ityp` name of the archive's folder: the install keeps the map files of its packs in nested archives that `content.xml`
//! registers as `RPF_FILE` with `CONTENTS_DLC_MAP_DATA`, registers no `.ymap` itself, and registers a type file as the `.ityp` of a
//! `.ytyp` inside a nested archive of the folder named (`where_the_install_keeps_its_map_files`).
//!
//! ⚠ Three things about that archive are not measured, and the cell of a singleplayer DLC stays locked for maps until they are
//! (`or_project::cannot_ship`): the folder of a pack the game's packs keep it in ([`PACK_FOLDER`] is a choice), what a pack states of
//! an archive that holds type files, and the change set that turns map data on.

/// The folder FiveM streams from.
pub const STREAM: &str = "stream";

/// Where a cooked file called `file` (`harbour.ymap`) lands inside the resource.
pub fn stream_path(file: &str) -> String {
    format!("{STREAM}/{file}")
}

/// The `data_file` type a manifest registers a type file under, the one the retail packs register theirs under.
pub const TYPE_FILE: &str = "DLC_ITYP_REQUEST";

/// The line of a FiveM manifest that declares the resource a map, and its value.
pub const MAP_LINE: (&str, &str) = ("this_is_a_map", "yes");

/// The folder of a singleplayer DLC the archive of a content's maps is written in: a choice, since nobody measured where the game's packs keep theirs.
pub const PACK_FOLDER: &str = "x64/levels/gta5";

/// What a pack states an archive of map files holds, as the install's packs state it.
pub const MAP_DATA: &str = "CONTENTS_DLC_MAP_DATA";

/// The nested archive of a singleplayer DLC that holds what the content `module` ships, by its path in the pack.
pub fn pack_archive(module: &str) -> String {
    format!("{PACK_FOLDER}/{module}.rpf")
}

/// Where a cooked file called `file` lands inside a singleplayer DLC: in the archive of its content.
pub fn pack_path(module: &str, file: &str) -> String {
    format!("{}/{file}", pack_archive(module))
}

/// The name a pack asks for the type file `name` by: the `.ityp` of the folder its archive is in.
pub fn pack_type_request(name: &str) -> String {
    format!("{PACK_FOLDER}/{name}.ityp")
}
