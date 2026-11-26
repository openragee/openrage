//! The units that build a document of each kind and draw it.

use super::{draw_unit, Painted, Report};
use crate::state::AppState;
use crate::ui::dock::{Tab, Workspace};

/// Every kind's unit, in the order the report lists them.
pub(super) fn kind_units(
    state: &mut AppState,
    mut bridge: Option<&mut crate::render::RenderBridge>,
    frames: usize,
    blank: &Painted,
    report: &mut Report,
) {
    use crate::state::{DatTable, DocPayload};

    explorer_units(state, bridge.as_deref_mut(), frames, blank, report);
    script_unit(state, bridge.as_deref_mut(), frames, report);
    super::gta6_script::units(state, bridge.as_deref_mut(), frames, report);
    name_table_unit(state, bridge.as_deref_mut(), frames, report);
    let inspected = [
        (
            "inspector: a .bik's preview",
            Ok(DocPayload::Bik(crate::video::BikDocument::new(&[]))),
        ),
        (
            "inspector: an .awc's streams",
            crate::audio::AwcDocument::open(&one_stream_awc()).map(DocPayload::Awc),
        ),
        (
            "inspector: a .rel's typed items",
            one_curve_rel().map(DocPayload::Rel),
        ),
        (
            "inspector: a .gfx's tags",
            or_io::gfx::GfxMovie::open(&one_frame_gfx())
                .map(DocPayload::Gfx)
                .map_err(|e| e.to_string()),
        ),
        (
            "inspector: a .mrf's state machines",
            Ok(DocPayload::MoveNetwork(two_state_network())),
        ),
        (
            "inspector: a heightmap.dat's grid",
            Ok(DocPayload::HeightMap(two_by_three_height_map())),
        ),
        (
            "inspector: a distantlights.dat's groups",
            Ok(DocPayload::DistantLights(one_group_distant_lights())),
        ),
        (
            "inspector: a waterheight.dat's bodies",
            Ok(DocPayload::WaterMap(one_lake_water_map())),
        ),
        (
            "inspector: a worldblanket.dat's grid",
            Ok(DocPayload::WorldBlanket(two_by_two_world_blanket())),
        ),
        (
            "inspector: a navprecalc.dat's cells",
            Ok(DocPayload::NavPrecalc(four_cell_nav_precalc())),
        ),
        (
            "inspector: a .dds's mip chain",
            crate::state::dds_document::open("self_check", &four_mip_dds()).map(DocPayload::Dds),
        ),
        (
            "inspector: an unread file's bytes",
            Ok(DocPayload::Hex(crate::state::HexDocument {
                bytes: b"RSC7\x01\x02\x03\x04".repeat(64),
                banner: "OpenRage has no reader for probe.bin, so it opens as its bytes"
                    .to_string(),
            })),
        ),
        (
            "inspector: a cache file's stores",
            Ok(DocPayload::DatTable(DatTable::CacheFile(
                one_record_per_store_cache_file(),
            ))),
        ),
        (
            "inspector: a metadata.dat's header",
            Ok(DocPayload::DatTable(DatTable::SocialClubMetadata(
                or_io::social_club_metadata::SocialClubMetadata {
                    format_word: 1,
                    built: "2024-02-01T15:54:51".into(),
                    body: vec![0; 16],
                },
            ))),
        ),
        (
            "inspector: an audio sector grid",
            Ok(DocPayload::DatTable(DatTable::AudioWorldSectors(
                Default::default(),
            ))),
        ),
        (
            "inspector: a comparison's two sides",
            Ok(DocPayload::Compare(one_change_comparison())),
        ),
    ];
    for (what, payload) in inspected {
        inspector_unit(
            state,
            bridge.as_deref_mut(),
            frames,
            blank,
            report,
            what,
            payload,
        );
    }
    let compared = DocPayload::Compare(one_change_comparison());
    let centre = crate::ui::panels::registry::ids::DOCUMENT;
    let outcome = kind_panel(
        state,
        bridge.as_deref_mut(),
        frames,
        centre,
        compared,
        |_| true,
    )
    .and_then(|painted| match painted.vertices > blank.vertices {
        true => Ok(painted),
        false => Err("the comparison's centre painted nothing".to_string()),
    });
    report.record("document: a comparison's differences", outcome);
    texture_preview_units(state, bridge, frames, report);
}

/// Two metadata files that differ in one field of one record.
fn one_change_comparison() -> crate::state::compare::Comparison {
    use crate::state::compare::{By, Change, Comparison, FieldChange, Found};
    Comparison {
        headings: ["a.ymt".into(), "b.ymt".into()],
        places: ["C:\\mods\\a.ymt".into(), "C:\\mods\\b.ymt".into()],
        found: Found::Fields {
            by: By::RecordAndField,
            changes: vec![FieldChange {
                record: "Items[0]".into(),
                named: "CItem  (Name=a)".into(),
                field: "Weight".into(),
                change: Change::Changed,
                left: "1".into(),
                right: "2".into(),
            }],
            unread: None,
        },
    }
}

/// The tree in Files, the Files view's Details and Icons and the preview pane
/// over an archive that holds something, since with nothing open they draw
/// only their empty states.
fn explorer_units(
    state: &mut AppState,
    mut bridge: Option<&mut crate::render::RenderBridge>,
    frames: usize,
    blank: &Painted,
    report: &mut Report,
) {
    use crate::state::explorer::{ExplorerPrefs, GroupKey, ViewMode};
    use crate::state::{DocPayload, DocumentKind};
    use crate::ui::panels::registry::ids;

    let saved = state.prefs.explorer.clone();
    let doc = state.new_empty_document(DocumentKind::Rpf);
    let mut selected = None;
    if let Some(DocPayload::Rpf(rpf)) = state.document_mut(doc).map(|d| &mut d.payload) {
        let root = rpf.session.root_id();
        if let Ok(props) = rpf.session.add_folder(root, "props") {
            let _ = rpf.session.add_file_copy(props, "chair.ydr", vec![0; 64]);
        }
        for (name, size) in [("big.ytd", 40 * 1024), ("a.ydr", 12), ("empty.ybn", 0)] {
            let _ = rpf.session.add_file_copy(root, name, vec![0; size]);
        }
        selected = rpf
            .session
            .add_file_copy(
                root,
                "notes.meta",
                b"<CMapTypes>\n  <name>props</name>\n</CMapTypes>\n".to_vec(),
            )
            .ok();
    }
    state.set_active_document(doc);
    if let Some(node) = selected {
        state.selection.select(doc, node);
    }

    let tree = draw_unit(state, bridge.as_deref_mut(), frames, |state| {
        state.active_workspace = Workspace::Particle;
        *state.workspace_docks.get_mut(Workspace::Particle) =
            egui_dock::DockState::new(vec![Tab(ids::FILES)]);
    });
    report.record("explorer: an archive in the tree in Files", tree);

    // ⚠ Each view after the first is judged against the first as well as the
    // blank: a view that fell back to another would paint as much as it.
    let mut files_view_in = |state: &mut AppState, view: ViewMode, group: GroupKey| {
        draw_unit(state, bridge.as_deref_mut(), frames, move |state| {
            state.prefs.explorer = ExplorerPrefs {
                view,
                group,
                ..ExplorerPrefs::default()
            };
            state.active_workspace = Workspace::Browse;
            *state.workspace_docks.get_mut(Workspace::Browse) =
                egui_dock::DockState::new(vec![Tab(ids::DOCUMENT)]);
        })
    };
    let details = files_view_in(state, ViewMode::Details, GroupKey::None).and_then(|painted| {
        if painted.vertices > blank.vertices {
            Ok(painted)
        } else {
            Err("the Files view in Details painted nothing".to_string())
        }
    });
    report.record("explorer: the Files view in Details", details.clone());
    if let Ok(details) = details {
        for (view, group) in [
            (ViewMode::Details, GroupKey::Size),
            (ViewMode::Icons, GroupKey::Type),
        ] {
            let outcome = files_view_in(state, view, group).and_then(|painted| {
                if painted.vertices != details.vertices && painted.vertices > blank.vertices {
                    Ok(painted)
                } else {
                    Err(format!(
                        "painted {} vertices against Details' {}: this view drew the \
                         same thing as Details, or nothing",
                        painted.vertices, details.vertices
                    ))
                }
            });
            report.record(
                format!(
                    "explorer: the Files view in {}, {}",
                    view.label(),
                    group.label()
                ),
                outcome,
            );
        }
    }

    let preview = draw_unit(state, bridge, frames, |state| {
        state.active_workspace = Workspace::Particle;
        *state.workspace_docks.get_mut(Workspace::Particle) =
            egui_dock::DockState::new(vec![Tab(ids::FILE_PREVIEW)]);
    })
    .and_then(|painted| {
        if painted.vertices > blank.vertices {
            Ok(painted)
        } else {
            Err("the preview of a selected text entry painted nothing".to_string())
        }
    });
    report.record("explorer: the preview pane on a selected entry", preview);

    super::discard(state, doc);
    state.prefs.explorer = saved;
}

/// The inspector over a script's listing and the text editor over its source,
/// each judged against the same panel over a script with neither: the chrome
/// alone would pass the blank check.
fn script_unit(
    state: &mut AppState,
    mut bridge: Option<&mut crate::render::RenderBridge>,
    frames: usize,
    report: &mut Report,
) {
    use crate::state::{DocumentKind, ScriptDocument};
    use crate::ui::panels::registry::ids;
    use or_io::container::resource::{ResourceType, YSC_FILE_VERSION};
    use or_io::gta5::script::{opcode, ScrProgram};

    let code = vec![
        opcode::ENTER,
        0,
        2,
        0,
        0,
        opcode::NATIVE,
        1 << 2,
        0,
        0,
        opcode::LEAVE,
        0,
        0,
    ];
    let program = ScrProgram {
        code_length: code.len() as u32,
        natives: vec![0x4EDE_34FB_ADD9_67A6],
        code,
        name: "self_check".to_string(),
        resource_version: YSC_FILE_VERSION,
        ..ScrProgram::default()
    };
    let doc = state.new_empty_document(DocumentKind::Registered(ResourceType::Ysc));
    let mut inspector = |state: &mut AppState| {
        draw_unit(state, bridge.as_deref_mut(), frames, |state| {
            state.active_workspace = Workspace::Data;
            *state.workspace_docks.get_mut(Workspace::Data) =
                egui_dock::DockState::new(vec![Tab(ids::DETAILS)]);
        })
    };
    let outcome = inspector(state).and_then(|without| {
        let script = ScriptDocument::of(&program)?;
        if let Some(d) = state.document_mut(doc) {
            d.payload = script.into_payload();
        }
        inspector(state).and_then(|painted| {
            if painted.vertices > without.vertices {
                Ok(painted)
            } else {
                Err(format!(
                    "painted {} vertices against {} with no listing: the listing drew nothing",
                    painted.vertices, without.vertices
                ))
            }
        })
    });
    report.record("inspector: a script's disassembly listing", outcome);

    let mut editor = |state: &mut AppState, script: ScriptDocument| {
        if let Some(d) = state.document_mut(doc) {
            d.payload = script.into_payload();
        }
        draw_unit(state, bridge.as_deref_mut(), frames, |state| {
            state.set_active_document(doc);
            state.active_workspace = Workspace::Data;
            *state.workspace_docks.get_mut(Workspace::Data) =
                egui_dock::DockState::new(vec![Tab(ids::TEXT_EDITOR)]);
        })
    };
    let outcome = editor(state, ScriptDocument::default()).and_then(|without| {
        let script = ScriptDocument::of(&program)?;
        editor(state, script).and_then(|painted| {
            if painted.vertices > without.vertices {
                Ok(painted)
            } else {
                Err(format!(
                    "painted {} vertices against {} with no source: the source drew nothing",
                    painted.vertices, without.vertices
                ))
            }
        })
    });
    report.record("text editor: a script's decompiled source", outcome);
    super::discard(state, doc);
}

/// The text editor over a `.nametable`'s names, judged against the same editor
/// over a name table with none: the panel's chrome alone would pass the blank check.
fn name_table_unit(
    state: &mut AppState,
    mut bridge: Option<&mut crate::render::RenderBridge>,
    frames: usize,
    report: &mut Report,
) {
    use crate::state::{name_table_text, DocPayload, DocumentKind};
    use crate::ui::panels::registry::ids;
    use or_io::gta5::data::nametable;

    let doc = state.new_empty_document(DocumentKind::Text);
    let mut editor = |state: &mut AppState, names: &[&str]| {
        let table = name_table_text(&nametable::write(names))?;
        if let Some(d) = state.document_mut(doc) {
            d.payload = DocPayload::NameTable(table);
            d.kind = DocumentKind::NameTable;
        }
        draw_unit(state, bridge.as_deref_mut(), frames, |state| {
            state.set_active_document(doc);
            state.active_workspace = Workspace::Data;
            *state.workspace_docks.get_mut(Workspace::Data) =
                egui_dock::DockState::new(vec![Tab(ids::TEXT_EDITOR)]);
        })
    };
    let outcome = editor(state, &[]).and_then(|without| {
        editor(state, &["BIFTA", "KALAHARI"]).and_then(|painted| {
            if painted.vertices > without.vertices {
                Ok(painted)
            } else {
                Err(format!(
                    "painted {} vertices against {} with no names: the names drew nothing",
                    painted.vertices, without.vertices
                ))
            }
        })
    });
    report.record("text editor: a .nametable's names", outcome);
    super::discard(state, doc);
}

/// One PCM16 stream of four samples: a 20-byte `format` chunk at 36, an 8-byte `data` chunk at 56.
fn one_stream_awc() -> Vec<u8> {
    let mut bytes = vec![0u8; 64];
    bytes[0..4].copy_from_slice(b"ADAT");
    bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
    bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&36u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&(0x0123_4567u32 | (2 << 29)).to_le_bytes());
    for (at, (kind, offset, size)) in [(0xFAu64, 36u64, 20u64), (0x55, 56, 8)].iter().enumerate() {
        let raw = offset | (size << 28) | (kind << 56);
        bytes[20 + at * 8..28 + at * 8].copy_from_slice(&raw.to_le_bytes());
    }
    bytes[36..40].copy_from_slice(&4u32.to_le_bytes());
    bytes[44..46].copy_from_slice(&32000u16.to_le_bytes());
    bytes
}

/// A `rel_type` 16 file of one Constant curve, selected, so its fields draw too.
fn one_curve_rel() -> Result<crate::state::RelDocument, String> {
    use or_io::rel::{RelFile, RelIndex, RelIndexHash};

    let mut data = vec![1, 0, 0, 0, 0, 0, 0, 0];
    for v in [0.0f32, 1.0, 0.5] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    let file = RelFile {
        rel_type: 16,
        index: RelIndex::Hashed(vec![RelIndexHash {
            hash: 0x1234_5678,
            offset: 0,
            length: data.len() as u32,
        }]),
        data,
        names: Vec::new(),
        hash_offsets: Vec::new(),
        pack_offsets: Vec::new(),
    };
    let bytes = file.save().map_err(|e| e.to_string())?;
    let mut rel =
        crate::state::RelDocument::open(std::path::Path::new("curves.dat16.rel"), &bytes)?;
    rel.selected = Some(0);
    Ok(rel)
}

/// One frame: a background colour, an external image named by file, and the end.
fn one_frame_gfx() -> Vec<u8> {
    fn tag(out: &mut Vec<u8>, code: u16, body: &[u8]) {
        out.extend_from_slice(&((code << 6) | body.len() as u16).to_le_bytes());
        out.extend_from_slice(body);
    }
    let mut body = vec![0x00, 0x00, 0x1E, 0x01, 0x00];
    tag(&mut body, 9, &[0x20, 0x40, 0x60]);
    let mut image = 1u32.to_le_bytes().to_vec();
    image.extend_from_slice(&[0; 6]);
    for name in ["logo", "logo.dds"] {
        image.push(name.len() as u8);
        image.extend_from_slice(name.as_bytes());
    }
    tag(&mut body, 1009, &image);
    tag(&mut body, 0, &[]);
    let mut out = b"GFX\x08".to_vec();
    out.extend_from_slice(&((8 + body.len()) as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// A root machine of two states, the first with a transition to the second on a request.
fn two_state_network() -> or_io::mrf::MvNetworkDef {
    use or_io::hash::string_hash_str as hash;
    use or_io::mrf::*;
    let state = |id: &str, transitions: Vec<MvTransition>| MvNodeDef {
        node_type: NodeType::State,
        index: 0,
        id: hash(id),
        body: NodeBody::State(StateDef {
            base: StateBaseDef {
                transitions_offset: 0x40 - 0x1c,
                ..StateBaseDef::default()
            },
            child_count: 0,
            param_inputs_offset: 0,
            events_offset: 0,
            param_outputs_offset: 0,
            operations_offset: 0,
            transitions,
            param_inputs: Vec::new(),
            events: Vec::new(),
            param_outputs: Vec::new(),
            operations: Vec::new(),
        }),
    };
    let go = MvTransition {
        flags: 0,
        tag: 0,
        duration: 0.25,
        signal: 0,
        transition_weight_signal: 0,
        target: 0,
        conditions: vec![MvCondition {
            set: 0,
            test: Test::OnRequest {
                request: 0,
                not_set: 0,
            },
        }],
        filter: None,
    };
    let mut network = MvNetworkDef {
        version: VERSION,
        xrefs: Vec::new(),
        requests: vec![KeyIndex {
            key: hash("Go"),
            index: 0,
        }],
        flags: Vec::new(),
        nodes: vec![
            MvNodeDef {
                node_type: NodeType::StateMachine,
                index: 0,
                id: hash("Root"),
                body: NodeBody::StateMachine(StateMachineDef {
                    base: StateBaseDef::default(),
                    states: vec![
                        StateNode {
                            id: hash("Idle"),
                            node: 0,
                        },
                        StateNode {
                            id: hash("Moving"),
                            node: 0,
                        },
                    ],
                    transitions: Vec::new(),
                }),
            },
            state("Idle", vec![go]),
            state("Moving", Vec::new()),
        ],
        string_table: Vec::new(),
    };
    // Offsets are relative to the field holding them, so they follow from where each node lands.
    let at = network.node_offsets();
    let rel = |field: usize, node: usize| at[node] as i32 - field as i32;
    if let NodeBody::StateMachine(sm) = &mut network.nodes[0].body {
        sm.base.initial = rel(at[0] + 8, 1);
        sm.states[0].node = rel(at[0] + 0x24, 1);
        sm.states[1].node = rel(at[0] + 0x2c, 2);
    }
    if let NodeBody::State(idle) = &mut network.nodes[1].body {
        idle.transitions[0].target = rel(at[1] + 0x40 + 0x14, 2);
    }
    network
}

/// A grid of three by two cells, two of them with a height.
fn two_by_three_height_map() -> or_io::height_map::HeightMap {
    use or_io::height_map::HeightMap;

    HeightMap {
        version: 1,
        flags: HeightMap::RLE_DATA,
        num_cols: 3,
        num_rows: 2,
        bounds_min: [-300.0, -200.0, 0.0],
        bounds_max: [300.0, 200.0, 800.0],
        max: vec![0, 40, 90, 0, 0, 0],
        min: vec![0, 10, 20, 0, 0, 0],
        water_mask: Vec::new(),
    }
}

/// One archive date and one record in each of the three stores.
fn one_record_per_store_cache_file() -> or_io::cache_loader::CacheFile {
    use or_io::cache_loader::{CacheFile, CacheFileDate};

    CacheFile {
        file_dates: vec![CacheFileDate {
            name_hash: 2740459947,
            file_time: 130680580712018938,
            third_column: Some(8944),
        }],
        map_data_store: vec![Default::default()],
        interior_proxies: vec![Default::default()],
        bounds_store: vec![Default::default()],
        ..CacheFile::default()
    }
}

/// The 16-block grid with one coastal group of four points in its first block.
fn one_group_distant_lights() -> or_io::distant_lights::DistantLights {
    use or_io::distant_lights::{DistantLightGroup, DistantLightPoint, DistantLights, BLOCKS};

    let cells = BLOCKS * BLOCKS;
    let mut coastal = vec![0; cells];
    coastal[0] = 1;
    DistantLights {
        block_first_group: vec![0; cells],
        block_num_coastal_groups: coastal,
        block_num_inland_groups: vec![0; cells],
        points: vec![DistantLightPoint { x: 1, y: 2, z: 3 }; 4],
        groups: vec![DistantLightGroup {
            centre_x: -500,
            centre_y: 900,
            centre_z: 30,
            radius: 120,
            point_count: 4,
            display_properties: 0x1211,
            ..DistantLightGroup::default()
        }],
    }
}

/// Two tiles by two, the north-east one over a lake of one box.
fn one_lake_water_map() -> or_io::water_map::WaterMap {
    use or_io::water_map::{WaterMap, WaterMapBody, WaterMapRefId, WaterMapTileRow};

    WaterMap {
        offset_x: -50.0,
        offset_y: 50.0,
        tile_size_x: 50.0,
        tile_size_y: 50.0,
        num_tile_cols: 2,
        tile_rows: vec![
            WaterMapTileRow {
                first: 1,
                count: 1,
                start: 0,
            },
            WaterMapTileRow::default(),
        ],
        tiles: vec![0],
        ref_ids: vec![WaterMapRefId(0x8000 | 0x4000)],
        lake_boxes: vec![[25.0, 25.0, 25.0, 25.0]],
        lakes: vec![WaterMapBody {
            count: 1,
            ..WaterMapBody::default()
        }],
        lake_colours: vec![[0, 0xff, 0xff, 0xff]],
        ..WaterMap::default()
    }
}

fn two_by_two_world_blanket() -> or_io::world_blanket::WorldBlanket {
    or_io::world_blanket::WorldBlanket {
        cells: vec![0, 1, 1, 0],
    }
}

/// Four cells, two of them with entries.
fn four_cell_nav_precalc() -> or_io::nav_precalc::NavPrecalc {
    use or_io::nav_precalc::{NavPrecalc, NavPrecalcCell};

    let cell = |n: usize| NavPrecalcCell {
        values: [1, 3],
        groups: [vec![[0; 4]; n], vec![], vec![], vec![]],
    };
    NavPrecalc {
        header: [3, 1],
        cells: vec![cell(0), cell(3), cell(1), cell(0)],
        bits: vec![0; 4],
    }
}

/// The inspector over a document of a kind with no blank to author.
fn inspector_unit(
    state: &mut AppState,
    bridge: Option<&mut crate::render::RenderBridge>,
    frames: usize,
    blank: &Painted,
    report: &mut Report,
    what: &str,
    payload: Result<crate::state::DocPayload, String>,
) {
    use crate::ui::panels::registry::ids;

    let outcome = payload.and_then(|payload| {
        kind_panel(state, bridge, frames, ids::DETAILS, payload, |_| true).and_then(|painted| {
            if painted.vertices > blank.vertices {
                Ok(painted)
            } else {
                Err(format!(
                    "painted {} vertices, no more than the {} an empty panel costs",
                    painted.vertices, blank.vertices
                ))
            }
        })
    });
    report.record(what, outcome);
}

/// The texture preview over each binary world `.dat`'s picture and a `.dds`'s mips.
///
/// ⚠ Judged by the decoded bytes the texture cache took, not by vertices: the
/// name, the controls and a "cannot preview" hint paint without any picture.
fn texture_preview_units(
    state: &mut AppState,
    mut bridge: Option<&mut crate::render::RenderBridge>,
    frames: usize,
    report: &mut Report,
) {
    use crate::state::DocPayload;
    use crate::ui::panels::registry::ids;

    let previewed = [
        (
            "texture preview: a heightmap.dat's picture",
            Ok(DocPayload::HeightMap(two_by_three_height_map())),
        ),
        (
            "texture preview: a waterheight.dat's picture",
            Ok(DocPayload::WaterMap(one_lake_water_map())),
        ),
        (
            "texture preview: a worldblanket.dat's picture",
            Ok(DocPayload::WorldBlanket(two_by_two_world_blanket())),
        ),
        (
            "texture preview: a navprecalc.dat's picture",
            Ok(DocPayload::NavPrecalc(four_cell_nav_precalc())),
        ),
        (
            "texture preview: a .dds and its mips",
            crate::state::dds_document::open("self_check", &four_mip_dds()).map(DocPayload::Dds),
        ),
    ];
    for (what, payload) in previewed {
        let outcome = payload.and_then(|payload| {
            let before = state.texture_cache.bytes();
            kind_panel(
                state,
                bridge.as_deref_mut(),
                frames,
                ids::TEXTURE_PREVIEW,
                payload,
                |state| state.texture_cache.bytes() > before,
            )
        });
        report.record(what, outcome);
    }
}

/// `panel` alone in the kind's workspace over `payload`, held by a Text blank
/// re-kinded since most of these kinds have no blank to author; `drew` judges
/// the state before the document closes.
fn kind_panel(
    state: &mut AppState,
    bridge: Option<&mut crate::render::RenderBridge>,
    frames: usize,
    panel: crate::ui::panel_registry::PanelId,
    payload: crate::state::DocPayload,
    drew: impl FnOnce(&AppState) -> bool,
) -> super::Outcome {
    use crate::state::DocumentKind;

    let kind = payload.kind();
    let doc = state.new_empty_document(DocumentKind::Text);
    if let Some(d) = state.document_mut(doc) {
        d.payload = payload;
        d.kind = kind;
    }
    let outcome = draw_unit(state, bridge, frames, |state| {
        state.set_active_document(doc);
        state.active_workspace = Workspace::for_kind(kind);
        *state.workspace_docks.get_mut(state.active_workspace) =
            egui_dock::DockState::new(vec![Tab(panel)]);
    })
    .and_then(|painted| {
        if drew(state) {
            Ok(painted)
        } else {
            Err(format!(
                "`{panel}` drew {} vertices but not the document's picture",
                painted.vertices
            ))
        }
    });
    super::discard(state, doc);
    outcome
}

/// An 8x8 DXT1 with its four mips.
fn four_mip_dds() -> or_io::dds::Dds {
    use or_io::dds::{pixel_format_flags, Dds, DdsHeader};

    let mut header = DdsHeader {
        width: 8,
        height: 8,
        mip_map_count: 4,
        ..DdsHeader::default()
    };
    header.pixel_format.flags = pixel_format_flags::FOURCC;
    header.pixel_format.four_cc = u32::from_le_bytes(*b"DXT1");
    Dds {
        header,
        dxt10: None,
        data: (0..56).map(|i| (i * 37) as u8).collect(),
    }
}
