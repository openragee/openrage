#[cfg(test)]
mod a_b_compares_the_pack_with_its_previous_kept_version;
#[cfg(test)]
mod a_bones_numbers_can_be_edited;
#[cfg(test)]
mod a_build_keeps_what_it_checked;
#[cfg(test)]
mod a_change_on_disk_keeps_the_undo_steps_beside_it;
#[cfg(test)]
mod a_document_of_no_format_asks_for_no_image;
#[cfg(test)]
mod a_drag_carries_every_selected_row;
#[cfg(test)]
mod a_drag_copies_and_a_change_of_nothing_does_nothing;
#[cfg(test)]
mod a_field_that_shapes_the_data_is_not_edited_in_place;
#[cfg(test)]
mod a_garments_cloth_is_linked_and_its_shop_ids_are_kept;
#[cfg(test)]
mod a_gta_vi_animation_s_player_is_drawn_with_its_timeline;
#[cfg(test)]
mod a_gta_vi_archive_s_scan_is_kept;
#[cfg(test)]
mod a_gta_vi_dictionary_entry_shows_and_edits_its_own_bones;
#[cfg(test)]
mod a_gta_vi_drawable_is_browsed_as_a_gta_v_one;
#[cfg(test)]
mod a_member_taken_out_comes_back_and_a_source_folder_is_found_by_a_folder;
#[cfg(test)]
mod a_model_is_drawn_with_its_own_textures;
#[cfg(test)]
mod a_moved_or_deleted_archive_row_leaves_nothing_behind;
#[cfg(test)]
mod a_new_project;
#[cfg(test)]
mod a_pick_in_the_preview_opens_its_document_at_its_row;
#[cfg(test)]
mod a_pinned_inspector_shows_its_own_document;
#[cfg(test)]
mod a_plugin_asks_before_it_runs;
#[cfg(test)]
mod a_project_file_dropped_on_the_window_opens_as_a_project;
#[cfg(test)]
mod a_project_holds_only_sources;
#[cfg(test)]
mod a_recook_reloads_what_is_open;
#[cfg(test)]
mod a_right_click_or_a_drag_selects_its_row;
#[cfg(test)]
mod a_save_names_its_destination;
#[cfg(test)]
mod a_self_check_keeps_to_itself;
#[cfg(test)]
mod a_skeleton_is_seen_without_an_animation;
#[cfg(test)]
mod a_wardrobe_edit_is_recoverable;
#[cfg(test)]
mod adding_a_format;
#[cfg(test)]
mod adding_particle_files_is_an_import;
#[cfg(test)]
mod adding_weapons_is_an_import;
#[cfg(test)]
mod an_alt_click_in_the_viewport_picks_a_model;
#[cfg(test)]
mod an_awc_animation_is_written_back;
#[cfg(test)]
mod an_awc_stream_opens_its_animation;
#[cfg(test)]
mod an_edit_of_the_wardrobe_is_a_step_of_undo;
#[cfg(test)]
mod an_edit_stays_where_it_was_made;
#[cfg(test)]
mod an_error_names_what_failed;
#[cfg(test)]
mod an_export_asks_before_it_alters_the_bytes;
#[cfg(test)]
mod an_import_ends_with_a_toast_that_says_what_was_done_and_takes_it_back;
#[cfg(test)]
mod an_undo_finishes_or_says_why;
mod animation_session;
#[cfg(test)]
mod applying_a_preset;
#[cfg(test)]
mod authoring;
pub mod batch;
pub mod bone_numbers;
#[cfg(test)]
mod building_a_dlc_pack_from_the_app;
#[cfg(test)]
mod building_targets_from_the_app;
#[cfg(test)]
mod builds_are_edited_from_the_app;
pub mod capabilities;
#[cfg(test)]
mod checking_a_source_against_the_game;
mod clip_regions;
pub mod clipboard;
mod closing;
mod clothing;
pub mod compare;
#[cfg(test)]
mod comparing_every_resource_and_archive;
#[cfg(test)]
mod comparing_two_files;
#[cfg(test)]
mod content_is_linked_adopted_and_shared_from_the_app;
pub mod convert;
mod convert_words;
#[cfg(test)]
mod converting_from_the_app;
#[cfg(test)]
mod converting_in_a_modders_words;
#[cfg(test)]
mod copy_cut_and_paste;
pub mod dds_document;
pub mod defrag_job;
#[cfg(test)]
mod defragmenting_an_archive;
pub mod delete_prompt;
pub mod dependencies;
#[cfg(test)]
mod destructive_file_operations_ask;
pub mod dlc_pack;
mod document;
mod document_io;
pub mod drivers;
#[cfg(test)]
mod dropped_files;
#[cfg(test)]
mod dropping_and_importing_folders;
mod edit_history;
#[cfg(test)]
mod editing_a_clip;
#[cfg(test)]
mod enhanced_controls_say_so;
pub mod entry_preview;
#[cfg(test)]
mod every_add_lands_where_the_tree_shows_it;
#[cfg(test)]
mod every_edit_undoes;
#[cfg(test)]
mod every_view_follows_every_edit;
#[cfg(test)]
mod every_viewer_path_takes_both_targets;
#[cfg(test)]
mod every_write_asks_the_jobs_reading_it;
mod expander;
pub mod explorer;
pub mod extract_job;
pub mod extract_planning;
pub mod file_index;
mod file_io;
pub mod file_watcher;
pub mod files_view;
#[cfg(test)]
mod finding_a_scanned_body;
#[cfg(test)]
mod fix_all_safe_and_the_optimiser_in_the_app;
pub mod format_handlers;
pub mod frame_stats;
pub mod front_door;
pub mod game_discovery;
mod game_pack;
pub mod gltf;
#[cfg(test)]
mod gltf_in_and_out_from_the_app;
#[cfg(test)]
mod going_to_a_file;
mod gta6_anim_edit;
mod gta6_anim_in_audio_save;
pub(crate) mod gta6_anim_keys;
pub(crate) mod gta6_audio_links;
pub(crate) mod gta6_bcpack_calibration;
mod gta6_bone_edit;
pub(crate) mod gta6_bound;
mod gta6_bound_vertex;
pub(crate) mod gta6_clip_finding;
mod gta6_constant_types;
pub(crate) mod gta6_dialogue;
mod gta6_dictionary_index;
mod gta6_dictionary_write;
pub(crate) mod gta6_edit;
mod gta6_effect_facts;
mod gta6_effects;
mod gta6_embedded_edit;
pub(crate) mod gta6_entry_skeletons;
mod gta6_export;
pub(crate) mod gta6_face;
pub(crate) mod gta6_filter_edit;
mod gta6_first_entry;
mod gta6_fragment;
mod gta6_fragment_records;
mod gta6_gltf_import;
pub(crate) mod gta6_head;
mod gta6_install_harvest;
pub(crate) mod gta6_install_scan;
pub(crate) mod gta6_map;
mod gta6_map_edit;
mod gta6_map_extents;
pub(crate) mod gta6_material_declared;
pub(crate) mod gta6_material_layout;
mod gta6_meta_save;
mod gta6_morph;
mod gta6_name_pool;
pub(crate) mod gta6_names;
pub(crate) mod gta6_navmesh;
mod gta6_navmesh_delete;
mod gta6_navmesh_edit;
mod gta6_object_trees;
pub(crate) mod gta6_other_bodies;
mod gta6_other_bodies_save;
pub(crate) mod gta6_particle_models;
pub(crate) mod gta6_particles;
mod gta6_particles_save;
pub(crate) mod gta6_physics_edit;
pub(crate) mod gta6_resource;
mod gta6_resource_save;
pub(crate) mod gta6_selected;
pub(crate) mod gta6_set_finding;
pub(crate) mod gta6_shaders;
mod gta6_small;
mod gta6_summaries;
pub(crate) mod gta6_text;
mod gta6_texture_index;
mod gta6_texture_names_file;
mod gta6_texture_naming;
mod gta6_textures;
#[cfg(test)]
mod gta6_textures_resolve_and_save;
pub(crate) mod gta6_weapon_parts;
pub(crate) mod gta6_world;
pub(crate) mod gta_vi;
#[cfg(test)]
mod importing_game_files_from_the_app;
mod install_lifecycle;
pub mod jobs;
mod keys;
pub mod kinds;
mod load_error;
#[cfg(test)]
mod maps_and_a_budget_in_the_app;
mod materials_palette;
pub mod memory;
pub(crate) mod meta_edit;
pub mod mod_manager;
pub mod models;
mod move_network;
pub mod name_index;
pub mod navigation;
mod navmesh_edit;
#[cfg(test)]
mod nested_archives_survive_a_save;
#[cfg(test)]
mod no_save_replaces_a_file_a_job_reads;
#[cfg(test)]
mod nothing_draws_over_what_it_belongs_to;
#[cfg(test)]
mod nothing_extracts_into_an_install;
#[cfg(test)]
mod one_place_for_background_jobs;
#[cfg(test)]
mod opening_a_dds;
#[cfg(test)]
mod opening_a_gta_vi_awc;
#[cfg(test)]
mod opening_a_gta_vi_body;
#[cfg(test)]
mod opening_a_nametable;
#[cfg(test)]
mod opening_a_rel;
#[cfg(test)]
mod opening_a_ttpf;
#[cfg(test)]
mod opening_a_ydd;
#[cfg(test)]
mod opening_an_awc;
#[cfg(test)]
mod opening_an_rpf9_archive;
#[cfg(test)]
mod opening_an_unknown_file;
#[cfg(test)]
mod opening_metadata_without_a_schema;
#[cfg(test)]
mod opening_world_dat;
#[cfg(test)]
mod opus_on_a_worker;
pub mod os_drop;
pub mod overlay_copy;
pub mod overlay_loader;
pub mod overlay_refresh;
pub mod pack_edit;
pub(crate) mod particle_target;
mod particle_textures;
pub mod payload_handlers;
mod pending_prompts;
mod photo_documents;
pub mod plugins;
pub mod prefs;
pub mod presets;
pub mod preview_scene;
#[cfg(test)]
mod previewing_a_video;
mod project;
mod project_io;
mod project_shadow;
pub mod reference_names;
mod reload_open_file;
#[cfg(test)]
pub(crate) mod reopening_a_built_pack_from_the_app;
#[cfg(test)]
mod reordering_mods_from_the_app;
#[cfg(test)]
mod replacing_geometry_from_the_app;
pub mod retail;
mod rpf9_counted_names;
mod rpf9_kept;
mod rpf9_list;
pub mod rpf9_scan;
mod rpf_archive_open;
mod rpf_expansion;
mod rpf_moves;
mod rpf_node;
mod rpf_records;
mod rpf_session;
mod rpf_tree;
mod rpf_undo;
pub mod save_destination;
#[cfg(test)]
mod saving_twice_and_into_an_archive_not_yet_opened;
mod scaleform;
#[cfg(test)]
mod search_results_that_open;
pub mod search_session;
mod selection;
#[cfg(test)]
mod settings_recents_and_small_editor_fixes;
mod shader_defs;
#[cfg(test)]
mod shards;
#[cfg(test)]
mod sharing_a_project;
pub mod sim;
pub mod sinks;
mod sky_textures;
#[cfg(test)]
mod small_faults_in_the_default_views_menus_and_keys;
#[cfg(test)]
mod small_faults_in_the_editor_and_the_previews;
mod source_textures;
mod tabs;
#[cfg(test)]
mod tabs_that_behave_like_an_editors;
#[cfg(test)]
pub(crate) mod test_scratch;
mod texture_cache;
pub mod texture_edit;
pub mod texture_resolution;
mod texture_tiles;
pub mod textures;
#[cfg(test)]
mod the_addon_ped_in_the_app;
#[cfg(test)]
mod the_animation_preview_draws_what_its_sources_hold;
#[cfg(test)]
mod the_animation_preview_follows_its_sources;
#[cfg(test)]
mod the_default_views_in_a_modders_words;
#[cfg(test)]
mod the_dressed_ped;
#[cfg(test)]
mod the_dressed_ped_shows_sets_the_first_person_view_and_the_floor_of_heels;
#[cfg(test)]
mod the_dressed_ped_wears_props;
#[cfg(test)]
mod the_edit_mode_is_visible_and_honest;
#[cfg(test)]
mod the_files_view_in_the_centre;
#[cfg(test)]
mod the_front_door;
#[cfg(test)]
mod the_front_door_opens_and_closes;
#[cfg(test)]
mod the_gate_in_front_of_every_write;
#[cfg(test)]
mod the_grid_and_undo_survive_an_edit_of_the_contents;
#[cfg(test)]
mod the_inspector_leads_with_a_summary;
#[cfg(test)]
mod the_lock_a_window_holds;
#[cfg(test)]
mod the_mesh_doctor_in_the_app;
#[cfg(test)]
mod the_panel_counts_the_apps_cpu_state;
#[cfg(test)]
mod the_panels_around_the_viewer_take_both_targets;
#[cfg(test)]
mod the_project_history;
#[cfg(test)]
mod the_project_history_across_imports_and_projects;
#[cfg(test)]
mod the_project_polish;
#[cfg(test)]
mod the_project_screens;
#[cfg(test)]
mod the_project_workspace;
#[cfg(test)]
mod the_review_pictures;
#[cfg(test)]
mod the_rules_of_a_pack_are_set_and_its_problems_written_out_from_the_app;
#[cfg(test)]
mod the_shells_regions;
#[cfg(test)]
mod the_skeleton_files_own_model_can_be_hidden;
#[cfg(test)]
mod the_two_paths_of_the_workbench_end_to_end;
#[cfg(test)]
mod the_unsaved_marker_tells_the_truth;
#[cfg(test)]
mod the_viewport_draws_every_edit;
#[cfg(test)]
mod the_viewport_picks_every_line_it_draws;
#[cfg(test)]
mod the_viewport_picks_what_it_draws;
#[cfg(test)]
mod the_wardrobe_adds_a_folder_it_is_pointed_at;
#[cfg(test)]
mod the_wardrobe_edit_operations;
#[cfg(test)]
mod the_wardrobe_holds_texture_places;
#[cfg(test)]
mod the_wardrobe_import_breadth;
#[cfg(test)]
mod the_wardrobe_leaves_when_the_clothing_goes;
#[cfg(test)]
mod the_wardrobe_toast_undo;
#[cfg(test)]
mod the_wardrobe_undo_names_what_it_takes_back;
#[cfg(test)]
mod the_wardrobe_workspace;
pub mod toasts;
pub mod ttpf_document;
#[cfg(test)]
mod two_readers_of_one_archive_across_a_save;
mod undo;
#[cfg(test)]
mod undo_and_the_sliders_follow_the_pinned_inspector;
#[cfg(test)]
mod undo_from_files_acts_on_what_files_shows;
#[cfg(test)]
mod uses_and_used_by_in_the_app;
#[cfg(test)]
mod uses_over_a_retail_install;
#[cfg(test)]
mod uses_reaches_every_pack;
pub mod vfs_session;
#[cfg(test)]
mod viewing_never_edits;
#[cfg(test)]
mod viewport_basics_for_inspecting_assets;
mod viewport_camera;
#[cfg(test)]
mod what_a_comparison_still_missed;
#[cfg(test)]
mod what_a_write_still_does_not_ask_after_r60h;
#[cfg(test)]
mod what_archive_rows_still_get_wrong;
#[cfg(test)]
mod what_archive_rows_still_get_wrong_after_r20g;
#[cfg(test)]
mod what_archive_rows_still_get_wrong_after_r20h;
#[cfg(test)]
mod what_converting_still_gets_wrong_after_r55c;
#[cfg(test)]
mod what_converting_still_gets_wrong_after_r55d;
#[cfg(test)]
mod what_the_create_menu_offers;
pub mod workbench;

pub use clip_regions::*;
pub use closing::{AfterClose, CloseAnswer, PendingClose};
pub use clothing::{Clothing, ClothingVariant};
pub use document::*;
pub use document_io::{described_root, never_written};
pub use edit_history::{Change, ClipFieldSnapshot, EditDirection, EditHistory, Swap};
pub use expander::*;
pub use extract_job::{ExtractJobs, PendingExportFixes, TypeFilter};
pub use file_io::command_line_paths;
pub use frame_stats::FrameStats;
pub use game_pack::{GamePackController, GamePackStatus};
pub use keys::*;
pub use materials_palette::*;
pub use meta_edit::MetaEdit;
pub use models::{DocumentModel, ModelListing};
pub use or_title::{format_recovery_hint, KnownArchiveNames};
pub use pending_prompts::*;
pub use prefs::*;
pub use preview_scene::{AnimSession, MeshSource, PreviewScene, SkeletonSource};
pub use project::*;
pub use rpf_node::*;
pub use rpf_session::*;
pub use selection::*;
pub use shader_defs::*;
pub use texture_cache::*;
pub use texture_resolution::{AttachedDictionary, TextureResolutionReport};
pub use texture_tiles::Tile;
pub use textures::{DocumentTexture, TextureListing, TextureResolution, TextureSource};
pub use toasts::{ToastType, Toasts};

pub use search_session::SearchQuery;

use document_io::*;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use crate::ui::dock::{Workspace, WorkspaceDocks};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LoadingState {
    #[default]
    Idle,
    Loading,
    Saving,
    Error(String),
}

#[allow(clippy::large_enum_variant)]
pub enum LoadResult {
    Success {
        path: PathBuf,
        payload: DocPayload,
        binding: Option<RpfEntryBinding>,
        resource: Option<Arc<document::ResourceImage>>,
    },
    Error {
        path: PathBuf,
        message: String,
    },
}

pub enum SaveResult {
    Success {
        doc_id: DocumentId,
        path: PathBuf,
        /// The step of its history the document stood at when the save took it.
        at: u64,
    },
    Error {
        message: String,
    },
    /// Stopped before its write began, so the file is as it was.
    Stopped,
}

pub struct AppState {
    pub documents: Vec<Document>,
    pub active_document: Option<DocumentId>,
    pub active_workspace: Workspace,
    pub workspace_docks: WorkspaceDocks,
    pub selection: Selection,
    pub records_selected: presets::RecordSelection,
    pub presets: presets::PresetLibrary,
    pub texture_cache: TextureCache,
    pub pinned_texture_docs: std::collections::HashMap<DocumentId, Vec<DocumentId>>,
    pub auto_opened_texture_docs: std::collections::HashSet<DocumentId>,
    pub attached_texture_docs: Vec<AttachedDictionary>,
    pub clothing: Clothing,
    pub attempted_texture_siblings: std::collections::HashSet<DocumentId>,
    pub attempted_shared_dictionaries: std::collections::HashSet<(DocumentId, &'static str)>,
    pub attempted_texture_search_paths: std::collections::HashSet<DocumentId>,
    pub attempted_texture_project: std::collections::HashSet<DocumentId>,
    pub texture_report: TextureResolutionReport,
    pub closed_documents: Vec<DocumentId>,
    pub prefs: Prefs,
    pub project_path: Option<PathBuf>,
    pub project_carried: project::Carried,
    pub workbench: workbench::Workbench,
    pub front_door: front_door::FrontDoor,
    pub loading: LoadingState,
    /// The file being opened, and where its result arrives.
    load_receiver: Option<(PathBuf, Receiver<LoadResult>)>,
    pending_loads: VecDeque<PathBuf>,
    batch_loading: bool,
    save_receiver: Option<file_io::SaveJob>,
    pub show_error_dialog: bool,
    /// The operation that failed, as the error window's title names it, and the file when it was an open.
    pub error_dialog_title: String,
    /// What the reader said, which the window keeps for its details.
    pub error_dialog_message: String,
    /// One plain sentence in place of `error_dialog_message`, for an open that failed.
    pub error_dialog_plain: Option<String>,
    pub toasts: Toasts,
    pub keys: KeysController,
    pub materials_palette: MaterialsPaletteController,
    pub clip_regions: ClipRegionsController,
    pub name_index: name_index::NameIndex,
    pub interiors: reference_names::InstallInteriors,
    pub game_tables: reference_names::InstallTables,
    pub search: search_session::SearchSession,
    pub search_query: SearchQuery,
    pub deep_find: vfs_session::VfsSession,
    pub file_index: file_index::FileIndex,
    pub navigation: navigation::Navigation,
    pub tabs: tabs::Tabs,
    /// The game's shaders as a SPIR-V pack, which a helper program bakes.
    pub game_pack: GamePackController,
    pub shader_defs: ShaderDefsController,
    pub expander: ExpansionWorker,
    pub panels: crate::ui::panel_state::PanelStore,
    pub plugins: plugins::PluginSession,
    pub preview_scene: PreviewScene,
    pub cameras: viewport_camera::DocumentCameras,
    pub anim: AnimSession,
    pub anim_edits: or_anim::PoseEdits,
    pub anim_transport: or_transport::Transport,
    pub last_window_title: String,
    pub frame_stats: FrameStats,
    /// What the viewport holds on the GPU, as the render bridge last published it.
    pub gpu_usage: crate::render::gpu_budget::GpuUsage,
    /// The stages of the last viewport frame the device timed, each with its milliseconds; none where it cannot time them.
    pub gpu_stages: Vec<(&'static str, f32)>,
    next_doc_id: AtomicU64,
    pub known_archive_names: KnownArchiveNames,
    pub discovery: game_discovery::DiscoveryWorker,
    pub extract_jobs: ExtractJobs,
    pub defrag: defrag_job::DefragJobs,
    pub convert: convert::Conversions,
    pub dlc_pack: dlc_pack::DlcPacks,
    pub mod_manager: mod_manager::ModManager,
    pub gltf: gltf::Interchange,
    pub dependencies: dependencies::Dependencies,
    pub compare: compare::Comparisons,
    texture_naming: Vec<gta6_texture_naming::TextureNaming>,
    texture_packs: gta6_texture_index::PackIndex,
    set_finding: Vec<gta6_set_finding::SetFinding>,
    clip_finding: Option<gta6_clip_finding::ClipFinding>,
    particle_model_finding: Option<gta6_particle_models::ModelFinding>,
    map_search: Vec<gta6_map::ArchetypeSearch>,
    neighbour_search: Option<gta6_navmesh::Search>,
    pub jobs: jobs::JobLog,
    /// A delete in the archive tree waiting on its answer.
    pub pending_delete: Option<delete_prompt::PendingDelete>,
    pub collision_bvh: Option<std::sync::Arc<or_simulation::BvhCollider>>,
    pub retail_edit_session_allow: bool,
    pub pending_retail_edit: Option<PendingRetailEditPrompt>,
    /// A write the edit mode refused, offering to change the mode until answered.
    pub refused_write: Option<PathBuf>,
    /// An export waiting on whether to apply OpenRage's export fixes.
    pub pending_export_fixes: Option<PendingExportFixes>,
    /// The answer a drag out asked for, held for the next drag out.
    pub drag_out_fixes: Option<bool>,
    pub pending_enhanced_open: Option<PendingEnhancedOpen>,
    pub overlay_copy: overlay_copy::OverlayCopy,
    pub overlay_loader: overlay_loader::OverlayLoaderChecks,
    /// A save of an archive entry waiting on where it goes.
    pub save_destination: Option<save_destination::SaveDestinationChoice>,
    /// A close waiting on whether to save what it would drop.
    pub pending_close: Option<PendingClose>,
    /// Set once an exit's close has gone ahead; the editor loop exits on it.
    pub should_exit: bool,
    /// Files that changed on disk while they hold unsaved edits, waiting on whether to reload.
    pub reload_conflicts: Vec<PathBuf>,
    /// Counts the recooks that read an open tab again; what is drawn from another tab's file, as a model is from its dictionary, is built again when it moves.
    pub recooked: u64,
    pub sim: sim::SimSession,
    pub file_watcher: file_watcher::FileWatcher,
    pub drivers: drivers::Drivers,
    pub ptfx_sinks: sinks::EditorSinks,
    pub audio: crate::audio::AudioOut,
}

/// Reads the languages, with the `.po` files of the configuration's `languages` folder laid over the built-in ones.
fn read_the_languages() {
    crate::i18n::init(
        prefs::config_root()
            .map(|root| root.join("languages"))
            .as_deref(),
    );
}

impl AppState {
    pub fn new() -> Self {
        // ⚠ Before the preferences are read: the first text asked for fixes the list of languages for the process.
        read_the_languages();
        Self::from_prefs(load_prefs())
    }

    /// The state a launch reading `prefs` starts with.
    pub fn from_prefs(prefs: Prefs) -> Self {
        read_the_languages();
        crate::i18n::apply(prefs.language.as_deref());
        let mut keys = KeysController::default();
        keys.try_load_cached(&prefs);
        let mut materials_palette = MaterialsPaletteController::default();
        let mut clip_regions = ClipRegionsController::default();
        let game_pack = GamePackController::default();
        let mut shader_defs = ShaderDefsController::default();
        shader_defs.start_staged();
        if let Some(install_root) = gta_install_root_from_prefs(&prefs) {
            if keys.has_keys() {
                materials_palette.start_load(install_root.clone(), keys.provider());
                clip_regions.start_load(
                    install_root.clone(),
                    keys.provider(),
                    prefs.mount_options(),
                    None,
                );
            }
        }
        let known_game_paths: Vec<(Game, PathBuf)> = Game::all()
            .filter_map(|g| prefs.game_paths.get(g).cloned().map(|p| (g, p)))
            .collect();
        let workspace_docks = WorkspaceDocks::restored(&prefs.dock_layouts);
        // ⚠ No project is open at launch, and the Project workspace is offered only while one is.
        let active_workspace = Some(crate::ui::dock_layout::restored_workspace(
            &prefs.dock_layouts,
        ))
        .filter(|ws| !ws.needs_a_project())
        .unwrap_or_default();
        let drivers = drivers::Drivers::restored(&prefs.viewport);
        let preview_scene = PreviewScene::restored(&prefs);
        if let Some(game) = prefs.active_game {
            crate::diagnostics::crash::note_edition(game.generation().edition_label());
        }
        let mut state = Self {
            documents: Vec::new(),
            active_document: None,
            active_workspace,
            workspace_docks,
            selection: Selection::default(),
            records_selected: Default::default(),
            presets: Default::default(),
            texture_cache: TextureCache::new(prefs.texture_cache_budget()),
            pinned_texture_docs: Default::default(),
            auto_opened_texture_docs: Default::default(),
            attached_texture_docs: Vec::new(),
            clothing: Clothing::default(),
            attempted_texture_siblings: Default::default(),
            attempted_shared_dictionaries: Default::default(),
            attempted_texture_search_paths: Default::default(),
            attempted_texture_project: Default::default(),
            texture_report: TextureResolutionReport::default(),
            closed_documents: Vec::new(),
            prefs,
            project_path: None,
            project_carried: Default::default(),
            workbench: Default::default(),
            front_door: Default::default(),
            loading: LoadingState::Idle,
            load_receiver: None,
            pending_loads: VecDeque::new(),
            batch_loading: false,
            save_receiver: None,
            show_error_dialog: false,
            error_dialog_title: String::new(),
            error_dialog_message: String::new(),
            error_dialog_plain: None,
            toasts: Toasts::default(),
            keys,
            materials_palette,
            clip_regions,
            name_index: name_index::NameIndex::new(),
            interiors: reference_names::InstallInteriors::default(),
            game_tables: reference_names::InstallTables::default(),
            search: search_session::SearchSession::default(),
            search_query: SearchQuery::default(),
            deep_find: vfs_session::VfsSession::default(),
            file_index: Default::default(),
            navigation: navigation::Navigation::default(),
            tabs: tabs::Tabs::default(),
            game_pack,
            shader_defs,
            expander: ExpansionWorker::default(),
            panels: Default::default(),
            plugins: Default::default(),
            preview_scene,
            cameras: Default::default(),
            anim: AnimSession::default(),
            anim_edits: or_anim::PoseEdits::new(),
            anim_transport: or_transport::Transport::default(),
            last_window_title: String::new(),
            frame_stats: FrameStats::default(),
            gpu_usage: Default::default(),
            gpu_stages: Vec::new(),
            next_doc_id: AtomicU64::new(1),
            known_archive_names: KnownArchiveNames::with_stock(),
            discovery: game_discovery::DiscoveryWorker::spawn(known_game_paths),
            extract_jobs: ExtractJobs::default(),
            defrag: defrag_job::DefragJobs::default(),
            convert: Default::default(),
            dlc_pack: Default::default(),
            mod_manager: Default::default(),
            gltf: Default::default(),
            dependencies: Default::default(),
            compare: Default::default(),
            texture_naming: Vec::new(),
            texture_packs: Default::default(),
            set_finding: Vec::new(),
            clip_finding: None,
            particle_model_finding: None,
            map_search: Vec::new(),
            neighbour_search: None,
            jobs: jobs::JobLog::default(),
            pending_delete: None,
            retail_edit_session_allow: false,
            pending_retail_edit: None,
            refused_write: None,
            pending_export_fixes: None,
            drag_out_fixes: None,
            pending_enhanced_open: None,
            overlay_copy: overlay_copy::OverlayCopy::default(),
            overlay_loader: overlay_loader::OverlayLoaderChecks::default(),
            save_destination: None,
            pending_close: None,
            should_exit: false,
            reload_conflicts: Vec::new(),
            recooked: 0,
            sim: sim::SimSession::default(),
            file_watcher: file_watcher::FileWatcher::new(),
            drivers,
            collision_bvh: None,
            ptfx_sinks: sinks::EditorSinks::new(),
            audio: crate::audio::AudioOut::default(),
        };
        if let Some(root) = state
            .prefs
            .active_game_root()
            .cloned()
            .or_else(|| gta_install_root_from_prefs(&state.prefs))
        {
            let game = state.prefs.active_game.unwrap_or(Game::GtaVLegacy);
            if let Some(why) = gta_vi::refused_install(game, &root) {
                state.show_error_toast(why);
            } else if root.is_dir() {
                let install = state.open_game_root(root);
                state.set_active_document(install);
            }
        }
        state.maybe_start_name_scan();
        state.check_overlay_staleness();
        state.maybe_auto_extract_keys();
        state
    }

    /// Mount the install at `install_root` in place of any other, and return its document.
    pub fn open_game_root(&mut self, install_root: PathBuf) -> DocumentId {
        if let Some(existing) = self
            .documents
            .iter()
            .find(|d| d.kind == DocumentKind::GameRoot)
        {
            if existing.path.as_deref() == Some(install_root.as_path()) {
                return existing.id;
            }
        }
        self.documents.retain(|d| d.kind != DocumentKind::GameRoot);

        let doc = RpfDocument::new_game_root(install_root.clone(), self.keys.provider());
        let id = self.alloc_doc_id();
        let document = Document::new_game_root(id, install_root, doc);
        let root_node = match &document.payload {
            DocPayload::Rpf(rpf) => rpf.session.root_id(),
            _ => 0,
        };
        self.documents.insert(0, document);
        self.request_expand(id, root_node);
        id
    }

    pub fn open_folder(&mut self, folder: PathBuf) {
        if !folder.is_dir() {
            self.show_error_toast(format!(
                "Folder not found or not a directory: {}",
                folder.display()
            ));
            return;
        }
        if let Some(existing) = self
            .documents
            .iter()
            .find(|d| d.kind == DocumentKind::Folder && d.path.as_deref() == Some(folder.as_path()))
        {
            let id = existing.id;
            self.set_active_document(id);
            return;
        }

        let doc = RpfDocument::new_folder(folder.clone(), self.keys.provider());
        let id = self.alloc_doc_id();
        let document = Document::new_folder(id, folder.clone(), doc);
        let root_node = match &document.payload {
            DocPayload::Rpf(rpf) => rpf.session.root_id(),
            _ => 0,
        };
        let insert_at = self
            .documents
            .iter()
            .position(|d| d.kind != DocumentKind::GameRoot)
            .unwrap_or(self.documents.len());
        self.documents.insert(insert_at, document);
        self.set_active_document(id);
        self.request_expand(id, root_node);
        // ⚠ A game's folder is never made a project, and the banner reads no disk to tell: decided here, once.
        if or_title::game::registry::detect(&folder).is_some() {
            self.decline_adoption(&folder);
        }
        match gta_vi::plain_folder_note(&folder) {
            Some((note, true)) => self.show_warning_toast(note),
            Some((note, false)) => self.show_info_toast(note),
            None => self.show_toast("Folder opened"),
        }
        self.prefs.touch_recent_folder(folder);
        save_prefs(&self.prefs);
    }

    pub fn tick_frame_stats(&mut self) {
        self.frame_stats.tick();
    }

    /// Store the texture cache budget, save it, and evict down to it at once.
    pub fn set_texture_cache_budget_mib(&mut self, mib: u32) {
        self.prefs.texture_cache_mib = mib;
        self.texture_cache
            .set_budget(self.prefs.texture_cache_budget());
        save_prefs(&self.prefs);
    }

    /// Store the viewport's GPU texture budget and save it; the render bridge evicts down to it on its next frame.
    pub fn set_gpu_texture_budget_mib(&mut self, mib: u32) {
        self.prefs.gpu_texture_mib = mib;
        save_prefs(&self.prefs);
    }

    /// Store an appearance and save it; every window restyles on its next frame.
    pub fn set_appearance(&mut self, appearance: AppearancePrefs) {
        if self.prefs.appearance != appearance {
            self.prefs.appearance = appearance;
            save_prefs(&self.prefs);
        }
    }

    /// Store the interface language (none follows the system's own) and save it; the next frame draws in it.
    pub fn set_language(&mut self, code: Option<String>) {
        if self.prefs.language != code {
            self.prefs.language = code;
            save_prefs(&self.prefs);
        }
        crate::i18n::apply(self.prefs.language.as_deref());
        // The cards of the start page were said when they were read.
        self.front_door.cards.refresh();
        // The rows of the Project and Wardrobe say their kind in the language they were read in.
        self.workbench.want_scan();
    }

    /// Restyle at once without writing `prefs.json`, for a value still being dragged.
    pub fn preview_appearance(&mut self, appearance: AppearancePrefs) {
        self.prefs.appearance = appearance;
    }

    pub fn persist_appearance(&mut self) {
        save_prefs(&self.prefs);
    }

    pub fn is_busy(&self) -> bool {
        matches!(self.loading, LoadingState::Loading | LoadingState::Saving)
    }

    fn alloc_doc_id(&self) -> DocumentId {
        self.next_doc_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn active_document(&self) -> Option<&Document> {
        let id = self.active_document?;
        self.documents.iter().find(|d| d.id == id)
    }

    pub fn tick_previews(&mut self) {
        let changed = self.file_watcher.poll();
        for path in changed {
            self.reload_ypt_from_disk(&path, false);
        }

        match self.active_workspace {
            Workspace::Particle => self.tick_sim(),
            Workspace::Animation => self.tick_animation(),
            _ => {
                self.sim.last_built = None;
            }
        }
    }

    pub fn tick_sim(&mut self) {
        let Some(doc_id) = self.active_document else {
            self.sim.manager.clear_effect();
            self.sim.last_built = None;
            return;
        };
        let target = sim::SimSession::target_from_selection(self.selection.selected(doc_id));
        if !matches!(
            self.document(doc_id).map(|d| &d.payload),
            Some(DocPayload::Resource(or_io::Asset::Particles(_)))
        ) {
            self.sim.manager.clear_effect();
            self.sim.last_built = None;
            return;
        }
        if self.sim.needs_rebuild(doc_id, &target) {
            let asset_clone = match self.document(doc_id).map(|d| &d.payload) {
                Some(DocPayload::Resource(or_io::Asset::Particles(a))) => Some(a.clone()),
                _ => None,
            };
            if let Some(asset) = asset_clone {
                self.sim.rebuild(doc_id, &asset, target);
                self.give_gta6_domain_meshes(doc_id);
            }
        }
        let dt = self.sim.step_real_dt();
        let start = std::time::Instant::now();
        self.drivers.apply_to_eff_with_pinned(&mut self.sim.manager);
        self.sim.manager.env.wind_velocity = self.drivers.weather.wind_velocity();
        self.sim.manager.wind_field = Some(std::sync::Arc::new(
            crate::state::drivers::weather::WeatherDrivenWindField::from_driver(
                &self.drivers.weather,
            ),
        ));
        self.sim.manager.wall_time_seconds += dt;
        self.sim
            .manager
            .set_collision_bvh(self.collision_bvh.clone());
        self.ptfx_sinks.install(&mut self.sim.manager);
        self.sim.manager.tick(dt);
        self.sim.last_sim_ms = start.elapsed().as_secs_f32() * 1000.0;
    }
    pub fn document(&self, id: DocumentId) -> Option<&Document> {
        self.documents.iter().find(|d| d.id == id)
    }

    pub fn document_mut(&mut self, id: DocumentId) -> Option<&mut Document> {
        self.documents.iter_mut().find(|d| d.id == id)
    }

    /// Before `next` comes in front: the workspace the document in front is left in comes back with its tab, so a model left in Model is not met in Texture because a dictionary was looked at since.
    fn remember_where_the_front_document_is_seen(&mut self, next: DocumentId) {
        let leaving = self.active_workspace;
        if let Some(left) = self.active_document.filter(|left| *left != next) {
            if let Some(doc) = self.document_mut(left) {
                doc.seen_in = Some(leaving);
            }
        }
    }

    pub fn set_active_document(&mut self, id: DocumentId) {
        self.remember_where_the_front_document_is_seen(id);
        if let Some(doc) = self.document(id) {
            let back = doc.seen_in.filter(|ws| ws.handles_kind(doc.kind));
            if let Some(ws) = back {
                self.active_workspace = ws;
            } else if !self.active_workspace.handles_kind(doc.kind) {
                self.active_workspace = Workspace::for_kind(doc.kind);
            }
            self.active_document = Some(id);
            self.hide_start_page();
        }
    }

    /// Close a tab without asking; an open project keeps its loose file as a member.
    fn discard_document(&mut self, id: DocumentId) {
        if let (Some(_), Some(doc)) = (&self.project_path, self.document(id)) {
            // ⚠ An archive entry's path is a display path no load can reopen.
            if let (Some(path), None, false) = (
                doc.path.clone(),
                doc.rpf_entry_binding,
                workbench::is_a_place(doc.kind),
            ) {
                self.project_carried.member(&path, doc.kind);
            }
        }
        self.drop_document(id);
    }

    /// Close a document the app opened for itself, which joins no project.
    fn drop_document(&mut self, id: DocumentId) {
        let pos = match self.documents.iter().position(|d| d.id == id) {
            Some(p) => p,
            None => return,
        };
        let removed_binding = self.documents[pos].rpf_entry_binding;
        let cascaded_children: Vec<DocumentId> =
            if let DocPayload::Rpf(rpf) = &self.documents[pos].payload {
                rpf.entry_documents.values().copied().collect()
            } else {
                Vec::new()
            };

        self.documents.remove(pos);
        self.selection.forget(id);
        self.navigation.forget(id);
        self.texture_cache.forget_document(id);
        self.forget_texture_resolution(id);
        self.closed_documents.push(id);
        self.sim.forget_doc(id);

        if let Some(binding) = removed_binding {
            if let Some(parent) = self.document_mut(binding.parent_doc_id) {
                if let DocPayload::Rpf(rpf) = &mut parent.payload {
                    rpf.entry_documents.remove(&binding.entry_id);
                }
            }
        }

        if self.active_document == Some(id) {
            self.active_document = self
                .documents
                .iter()
                .filter(|d| self.has_tab(d))
                .nth(pos.saturating_sub(1))
                .or_else(|| self.documents.iter().find(|d| self.has_tab(d)))
                .map(|d| d.id);
        }

        for child_id in cascaded_children {
            self.drop_document(child_id);
        }
        self.forget_clothing(id);
    }

    fn insert_document(
        &mut self,
        path: PathBuf,
        payload: DocPayload,
        binding: Option<RpfEntryBinding>,
        resource: Option<Arc<document::ResourceImage>>,
    ) -> DocumentId {
        let id = self.alloc_doc_id();
        let mut doc = Document::new(id, Some(path.clone()), payload).with_resource(resource);
        doc.rpf_entry_binding = binding;
        if rpf9_scan::scanned_body(&path).is_some() {
            doc.read_only = Some(rpf9_scan::READ_ONLY_BODY);
        }
        let kind = doc.kind;
        self.name_index.absorb(&doc.payload);
        if let Some((names, keys)) =
            ttpf_document::texture_naming_work(&self.documents, &doc.payload)
        {
            (self.texture_naming).push(gta6_texture_naming::TextureNaming::start(names, keys));
        }
        if matches!(kind, DocumentKind::Ypt) {
            self.file_watcher.watch(path.clone());
        }
        // A dictionary opens on its first entry: the viewport draws an entry only while it is selected.
        let first_entry = crate::ui::panels::selection_ids::ydd_first_entry_hash(&doc.payload);
        self.documents.push(doc);
        gta6_head::trim(&self.documents);
        self.remember_where_the_front_document_is_seen(id);
        self.active_document = Some(id);
        self.hide_start_page();
        if let Some(hash) = first_entry {
            let entry = crate::ui::panels::selection_ids::dict_entry_id(hash);
            self.selection.select(id, entry);
        }
        if !self.active_workspace.handles_kind(kind) {
            self.active_workspace = Workspace::for_kind(kind);
        }
        if let Some(binding) = binding {
            if let Some(parent) = self.document_mut(binding.parent_doc_id) {
                if let DocPayload::Rpf(rpf) = &mut parent.payload {
                    rpf.entry_documents.insert(binding.entry_id, id);
                }
            }
        }
        self.remember_recent(id);
        id
    }

    pub fn bump_sim_revision(&mut self) {
        self.sim.pending_rebuild = true;
    }

    pub fn set_weather_state(&mut self, state: drivers::WeatherState) -> bool {
        if self.drivers.weather.set_state(state) {
            self.sim.pending_rebuild = true;
            self.sim.manager.snapshots.clear();
            true
        } else {
            false
        }
    }

    pub fn any_dirty(&self) -> bool {
        self.documents.iter().any(|d| d.dirty)
    }

    pub fn new_empty_document(&mut self, kind: DocumentKind) -> DocumentId {
        // ⚠ The blank can be *absent* (`.fxc` is read-only), and an empty
        // text tab says so rather than a panic or a wrong file.
        let payload = empty_payload_for_kind(kind).unwrap_or_else(|| {
            tracing::warn!(
                kind = kind.label(),
                "a blank file of this format cannot be authored"
            );
            DocPayload::Text(document::TextDocument::empty())
        });

        let id = self.alloc_doc_id();
        let mut doc = Document::new(id, None, payload);
        // Untitled docs are "dirty" from creation so save dialogs surface
        // even if the user never edits — otherwise an explicit New + Save
        // would silently no-op.
        doc.mark_edited();
        doc.name = format!("Untitled.{}", kind.extension());
        let kind = doc.kind;
        self.documents.push(doc);
        self.active_document = Some(id);
        if !self.active_workspace.handles_kind(kind) {
            self.active_workspace = Workspace::for_kind(kind);
        }
        id
    }

    pub fn rpf_add_empty_file(
        &mut self,
        doc_id: DocumentId,
        parent_folder: RpfNodeId,
        kind: DocumentKind,
    ) -> Result<RpfNodeId, String> {
        let target = self.affected_disk_path(doc_id, parent_folder);
        self.gate_tree_edit(
            target.as_deref(),
            PendingRetailEdit::AddEmptyFile {
                doc_id,
                parent_folder,
                kind,
            },
            "add new file to",
        )?;
        self.rpf_add_empty_file_inner(doc_id, parent_folder, kind)
    }

    fn rpf_add_empty_file_inner(
        &mut self,
        doc_id: DocumentId,
        parent_folder: RpfNodeId,
        kind: DocumentKind,
    ) -> Result<RpfNodeId, String> {
        let bytes = empty_file_bytes_for_kind(kind)?;
        self.open_archive_row(doc_id, parent_folder)?;
        let doc = self
            .document(doc_id)
            .ok_or_else(|| format!("Document {doc_id} not found"))?;
        let DocPayload::Rpf(rpf) = &doc.payload else {
            return Err("Document is not an RPF".to_string());
        };
        let name = unique_child_name(&rpf.session, parent_folder, "untitled", kind.extension());
        self.add_file_to_row(doc_id, parent_folder, &name, bytes)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

fn try_cache_icon_for(game: Game, install_root: &std::path::Path) {
    use crate::state::game_discovery::{cached_icon_path, extract_and_cache_icon};
    use or_title::install::discovery::{DiscoveredInstall, DiscoverySource};

    if cached_icon_path(game)
        .as_deref()
        .is_some_and(std::path::Path::is_file)
    {
        return;
    }
    let Some(exe) = game
        .profile()
        .icon_executable_candidates()
        .iter()
        .map(|n| install_root.join(n))
        .find(|p| p.is_file())
    else {
        return;
    };
    let install = DiscoveredInstall {
        game,
        install_dir: install_root.to_path_buf(),
        exe,
        source: DiscoverySource::UserConfigured,
    };
    if let Err(e) = extract_and_cache_icon(&install) {
        tracing::debug!(
            game = ?game,
            error = %e,
            "Manual-pick icon extraction failed",
        );
    }
}

#[cfg(test)]
mod tests;
