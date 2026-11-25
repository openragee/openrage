#[path = "timeline_curves.rs"]
mod curves;
use curves::behaviour_kpd_curves;

use crate::state::particle_target::{on_particle_target, ParticleTarget};
use crate::state::{AppState, DocumentId};
use crate::ui::theme;
use crate::ui::timeline::{
    self, ChannelRow, CurveChannel, CurveDomain, CurveHandle, RowEvents, RowId, SpanEvent,
    TimelineEdit, TimelineSource,
};
use or_transport::TimeRange;

pub type TimelineUiState = timeline::TimelineState;

#[derive(Default)]
pub struct ParticleTimelineState(pub TimelineUiState);

impl crate::ui::panel_state::PanelState for ParticleTimelineState {
    const KEY: &'static str = "timeline";
}

const KIND_EMITTER: u8 = 0;

/// Why an Enhanced effect's transport and drivers are greyed: `tick_sim` builds a simulation from a Legacy list only.
pub(crate) const NOT_SIMULATED: &str =
    tr_key!("Nothing plays: Enhanced effects are not simulated yet.");

/// Whether the open document is a particle list that nothing simulates, so a control only a simulation reads has nothing to act on.
pub(crate) fn holds_unsimulated_effects(state: &AppState) -> bool {
    state
        .active_document()
        .is_some_and(|doc| on_particle_target!(&doc.payload, T => !T::SIMULATED, else false))
}

const CHANNEL_COLOURS: [egui::Color32; 4] = [
    egui::Color32::from_rgb(220, 70, 70),
    egui::Color32::from_rgb(70, 200, 90),
    egui::Color32::from_rgb(80, 130, 220),
    egui::Color32::from_rgb(220, 200, 70),
];

pub struct YptTimelineSource {
    rows: Vec<ChannelRow>,
    spans: Vec<TimeRange>,
    curves: Vec<timeline::CurveSeries>,
    curve_sources: Vec<Option<KfpCurveSource>>,
    duration: f32,
    summary: String,
    no_playback: Option<&'static str>,
    read_only: Option<&'static str>,
}

impl YptTimelineSource {
    fn new(
        track_rows: Vec<TrackRow>,
        curve_series: Vec<CurveSeries>,
        duration: f32,
        summary: String,
    ) -> Self {
        let mut rows = Vec::with_capacity(track_rows.len());
        let mut spans = Vec::with_capacity(track_rows.len());
        for (index, track) in track_rows.iter().enumerate() {
            let start = track.start_ratio * duration;
            let end = track.end_ratio * duration;
            rows.push(
                ChannelRow::group(
                    RowId::new(KIND_EMITTER, index as u32, 0),
                    track.label.clone(),
                )
                .with_colour(theme::ACCENT.get())
                .animated(true),
            );
            spans.push(TimeRange::new(start.min(end), end.max(start)));
        }

        let mut curves = Vec::with_capacity(curve_series.len());
        let mut curve_sources = Vec::with_capacity(curve_series.len());
        for series in curve_series {
            curves.push(convert_curve(&series));
            curve_sources.push(series.source);
        }

        Self {
            rows,
            spans,
            curves,
            curve_sources,
            duration,
            summary,
            no_playback: None,
            read_only: None,
        }
    }

    fn kfp_edit(&self, edit: &TimelineEdit) -> Option<KfpEdit> {
        let TimelineEdit {
            series,
            channel,
            index,
            time,
            value,
        } = edit;
        let source = self.curve_sources.get(*series)?.clone()?;
        Some(KfpEdit {
            source,
            entry_idx: *index,
            channel: *channel,
            new_time: time.clamp(0.0, 1.0),
            new_value: *value,
        })
    }
}

impl TimelineSource for YptTimelineSource {
    fn bounds(&self) -> TimeRange {
        TimeRange::from_duration(self.duration.max(0.1))
    }

    fn rows(&self) -> &[ChannelRow] {
        &self.rows
    }

    fn row_events(&self, row: usize, view: TimeRange, out: &mut RowEvents) {
        out.clear();
        let Some(span) = self.spans.get(row) else {
            return;
        };
        if span.end < view.start || span.start > view.end {
            return;
        }
        out.spans.push(SpanEvent {
            range: *span,
            colour: theme::ACCENT.get(),
            label: self.rows.get(row).map(|r| r.label.clone()),
        });
    }

    fn curves(&self) -> &[timeline::CurveSeries] {
        &self.curves
    }

    fn editable(&self) -> bool {
        self.read_only.is_none()
    }

    fn read_only_reason(&self) -> Option<&str> {
        self.read_only.map(crate::i18n::tr)
    }

    fn playback_unavailable(&self) -> Option<&str> {
        self.no_playback
    }

    fn summary(&self) -> String {
        self.summary.clone()
    }
}

fn convert_curve(series: &CurveSeries) -> timeline::CurveSeries {
    let mut channels = Vec::with_capacity(series.channel_count);
    for channel in 0..series.channel_count {
        channels.push(CurveChannel {
            colour: CHANNEL_COLOURS[channel % 4],
            samples: series
                .samples
                .iter()
                .map(|(ratio, values)| (*ratio, values[channel]))
                .collect(),
        });
    }

    let mut handles = Vec::new();
    if series.source.is_some() {
        for (entry_idx, entry) in series.raw_entries.iter().enumerate() {
            for channel in 0..series.channel_count {
                handles.push(CurveHandle {
                    channel,
                    index: entry_idx,
                    time: entry.time[channel].clamp(0.0, 1.0),
                    value: entry.value[channel],
                });
            }
        }
    }

    timeline::CurveSeries {
        label: series.label.clone(),
        channels,
        handles,
        domain: CurveDomain::Normalized,
        domain_label: tr!("particle lifetime").to_string(),
    }
}

pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    let active_selection = state
        .active_document
        .and_then(|id| state.selection.selected(id));
    let shown = state.active_document().and_then(|doc| {
        on_particle_target!(&doc.payload, T => T::list(&doc.payload).map(|asset| (
            collect_track_rows(asset, active_selection),
            collect_curve_series_for_selection(asset, active_selection),
            authored_seconds(asset, active_selection),
            T::SIMULATED,
        )), else None)
    });
    let Some((track_rows, curve_series, authored, simulates)) = shown else {
        timeline::draw_placeholder(
            ui,
            tr!("Timeline available when a particle effect (.ypt) document is active"),
        );
        return;
    };
    // A list nothing simulates (an Enhanced one) takes its axis from the effects it authors.
    let simulated = state.sim.manager.duration();
    let duration = if simulated > 0.0 { simulated } else { authored }.max(0.1);
    let summary = tr!(
        "{name} · {live} live · {ms} ms",
        name = state
            .sim
            .manager
            .effect
            .as_ref()
            .map(|e| e.rule.name.clone())
            .unwrap_or_else(|| tr!("(no effect)").to_string()),
        live = state.sim.manager.live_count(),
        ms = format!("{:.2}", state.sim.last_sim_ms)
    );

    let mut source = YptTimelineSource::new(track_rows, curve_series, duration, summary);
    source.no_playback = (!simulates).then_some(crate::i18n::tr(NOT_SIMULATED));
    source.read_only = state
        .active_document()
        .filter(|doc| doc.is_from_rpf9_table())
        .map(|_| crate::state::rpf9::entry::READ_ONLY);

    ui.vertical(|ui| {
        draw_drivers_rail(ui, state, simulates);
        draw_pin_rail(ui, state, duration, simulates);

        if !source.curves().is_empty() {
            ui.horizontal(|ui| {
                let mut show = state.panels.get::<ParticleTimelineState>().0.show_curves;
                if ui
                    .checkbox(&mut show, tr!("Curves"))
                    .on_hover_text(tr!("Show the behaviour keyframe editor"))
                    .changed()
                {
                    state.panels.get::<ParticleTimelineState>().0.show_curves = show;
                }
                ui.label(
                    egui::RichText::new(tr!("{count} curves", count = source.curves().len()))
                        .small()
                        .color(theme::TEXT_SECONDARY),
                );
            });
        }
        ui.separator();

        let mut tl = state.panels.take::<ParticleTimelineState>();
        let mut transport = state.sim.manager.clock;
        let output = timeline::draw(ui, &source, &mut transport, &mut tl.0);
        state.panels.put(tl);

        state.sim.manager.clock = transport;
        if let Some(time) = output.seek_to {
            if output.evaluate {
                state.sim.seek(time);
            } else {
                state.sim.manager.clock.time = time;
                state.sim.manager.clock.seek_pending = None;
            }
        }

        let kfp_edits: Vec<KfpEdit> = output
            .edits
            .iter()
            .filter_map(|edit| source.kfp_edit(edit))
            .collect();
        if !kfp_edits.is_empty() {
            apply_kfp_edits(state, &kfp_edits);
        }
    });
}

fn draw_pin_rail(ui: &mut egui::Ui, state: &mut AppState, duration: f32, simulated: bool) {
    if state.sim.pinned_effects.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        if !simulated {
            ui.disable();
        }
        ui.label(
            egui::RichText::new(tr!("Pinned:"))
                .small()
                .color(theme::TEXT_SECONDARY),
        );
        let pins: Vec<crate::state::sim::SimBuildKey> = state.sim.pinned_effects.clone();
        for key in pins {
            let label = pin_label(state, &key);
            ui.group(|ui| {
                let resp = ui
                    .small_button(format!("{label} {}", egui_phosphor::regular::X))
                    .on_hover_text(tr!("Click to unpin"))
                    .on_disabled_hover_text(crate::i18n::tr(NOT_SIMULATED));
                if resp.clicked() {
                    state.sim.toggle_pin(key.clone());
                    return;
                }
                let mut off = state.sim.pin_offset(&key);
                let resp = ui.add(
                    egui::DragValue::new(&mut off)
                        .speed(0.05)
                        .range(0.0..=duration as f64)
                        .suffix("s")
                        .prefix("+"),
                );
                if resp.changed() {
                    state.sim.set_pin_offset(key.clone(), off);
                }
                resp.on_hover_text(tr!("Start offset for this pinned effect"))
                    .on_disabled_hover_text(crate::i18n::tr(NOT_SIMULATED));
            });
        }
    });
}

struct TrackRow {
    label: String,
    start_ratio: f32,
    end_ratio: f32,
}

struct CurveSeries {
    label: String,
    samples: Vec<(f32, [f32; 4])>,
    channel_count: usize,
    source: Option<KfpCurveSource>,
    raw_entries: Vec<or_io::gta5::particle::RawKeyframeEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct KfpCurveSource {
    particle_name: String,
    behaviour_index: usize,
    slot: ParticleKfpSlot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParticleKfpSlot {
    AccelMin,
    AccelMax,
    ColourRgbaMin,
    ColourRgbaMax,
    ColourEmissive,
    SizeWhdMin,
    SizeWhdMax,
    SizeTblrScalar,
    SizeTblrVelScalar,
    RotInitAngleMin,
    RotInitAngleMax,
    RotAngleMin,
    RotAngleMax,
    NoisePosMin,
    NoisePosMax,
    NoiseVelMin,
    NoiseVelMax,
    DampMin,
    DampMax,
    WindInfluence,
    AttractorStrength,
    AnimateTextureRate,
    CollisionBounciness,
    CollisionBounceDirVar,
    MatrixWeight,
    ZCullingHeight,
    ZCullingFadeDist,
    DecalDimensions,
    DecalAlpha,
}

#[derive(Debug, Clone)]
pub(crate) struct KfpEdit {
    source: KfpCurveSource,
    entry_idx: usize,
    channel: usize,
    new_time: f32,
    new_value: f32,
}

fn collect_curve_series_for_selection<T: ParticleTarget>(
    asset: &or_io::gta5::PtxFxList<T>,
    selection: Option<u64>,
) -> Vec<CurveSeries> {
    use crate::ui::panels::selection_ids::{
        ypt_tag_of, ypt_unpack_emitter, ypt_unpack_particle, ypt_unpack_particle_lib, YPT_EMITTER,
        YPT_PARTICLE, YPT_PARTICLE_LIB,
    };
    use or_particle::list;
    let particle_rule_name = match selection.map(ypt_tag_of) {
        Some(YPT_EMITTER) => {
            let (eff, ev) = ypt_unpack_emitter(selection.unwrap());
            list::effect_rules(asset)
                .nth(eff)
                .and_then(|e| list::events(e).nth(ev))
                .map(|evt| evt.particle_rule_name.clone())
        }
        Some(YPT_PARTICLE) => {
            let (eff, ev) = ypt_unpack_particle(selection.unwrap());
            list::effect_rules(asset)
                .nth(eff)
                .and_then(|e| list::events(e).nth(ev))
                .map(|evt| evt.particle_rule_name.clone())
        }
        Some(YPT_PARTICLE_LIB) => list::particle_rules(asset)
            .nth(ypt_unpack_particle_lib(selection.unwrap()))
            .map(|p| p.name.clone()),
        _ => None,
    };
    let particle = particle_rule_name
        .as_deref()
        .and_then(|n| list::particle_rule(asset, n));
    let Some(particle) = particle else {
        return Vec::new();
    };

    let mut series: Vec<CurveSeries> = Vec::new();
    let behaviours = list::behaviours(particle);
    for (b_idx, b) in behaviours.iter().enumerate() {
        for (label, slot, kpd) in behaviour_kpd_curves(b_idx, b) {
            let Some(kpd) = kpd else { continue };
            if kpd.entries.is_empty() {
                continue;
            }
            let channel_count = detect_channel_count(&kpd.entries);
            let samples = sample_curve(&kpd.entries, 64);
            series.push(CurveSeries {
                label,
                samples,
                channel_count,
                source: Some(KfpCurveSource {
                    particle_name: particle.name.clone(),
                    behaviour_index: b_idx,
                    slot,
                }),
                raw_entries: kpd.entries.clone(),
            });
        }
    }
    series
}

fn detect_channel_count(entries: &[or_io::gta5::particle::RawKeyframeEntry]) -> usize {
    let mut active = 1usize;
    for e in entries {
        if e.value[3].abs() > 1e-4 {
            return 4;
        }
        if e.value[2].abs() > 1e-4 {
            active = active.max(3);
        } else if e.value[1].abs() > 1e-4 {
            active = active.max(2);
        }
    }
    active
}

fn sample_curve(
    entries: &[or_io::gta5::particle::RawKeyframeEntry],
    n: usize,
) -> Vec<(f32, [f32; 4])> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / (n.saturating_sub(1).max(1) as f32);
        let v = or_simulation::kfp::query_raw_entries(entries, t);
        out.push((t, [v.x, v.y, v.z, v.w]));
    }
    out
}

/// How long the effect `selection` is in lasts, else the longest of `asset`: the ratios of an event are of its effect's duration.
fn authored_seconds<T: ParticleTarget>(
    asset: &or_io::gta5::PtxFxList<T>,
    selection: Option<u64>,
) -> f32 {
    use crate::ui::panels::selection_ids::{
        ypt_tag_of, ypt_unpack_domain, ypt_unpack_effect, ypt_unpack_emitter, ypt_unpack_particle,
        YPT_DOMAIN, YPT_EFFECT, YPT_EMITTER, YPT_PARTICLE,
    };
    let effect = selection.and_then(|id| match ypt_tag_of(id) {
        YPT_EFFECT => Some(ypt_unpack_effect(id)),
        YPT_EMITTER => Some(ypt_unpack_emitter(id).0),
        YPT_PARTICLE => Some(ypt_unpack_particle(id).0),
        YPT_DOMAIN => Some(ypt_unpack_domain(id).0),
        _ => None,
    });
    let all: Vec<f32> = (or_particle::list::effect_rules(asset))
        .map(|effect| effect.duration_max)
        .filter(|seconds| seconds.is_finite())
        .collect();
    let own = effect.and_then(|at| or_particle::list::effect_rules(asset).nth(at));
    match own
        .map(|effect| effect.duration_max)
        .filter(|s| s.is_finite())
    {
        Some(seconds) => seconds,
        None => all.into_iter().fold(0.0, f32::max),
    }
}

fn collect_track_rows<T: ParticleTarget>(
    asset: &or_io::gta5::PtxFxList<T>,
    selection: Option<u64>,
) -> Vec<TrackRow> {
    use crate::ui::panels::selection_ids::{
        ypt_tag_of, ypt_unpack_behaviour, ypt_unpack_domain, ypt_unpack_effect, ypt_unpack_emitter,
        ypt_unpack_emitter_lib, ypt_unpack_particle, ypt_unpack_particle_lib, YPT_BEHAVIOUR,
        YPT_DOMAIN, YPT_EFFECT, YPT_EMITTER, YPT_EMITTER_LIB, YPT_PARTICLE, YPT_PARTICLE_LIB,
    };
    use or_particle::list;

    let tag = selection.map(ypt_tag_of);
    let emitter_lib_name = match (tag, selection) {
        (Some(YPT_EMITTER_LIB), Some(id)) => list::emitter_rules(asset)
            .nth(ypt_unpack_emitter_lib(id))
            .map(|r| r.name.clone()),
        _ => None,
    };
    let particle_lib_name = match (tag, selection) {
        (Some(YPT_PARTICLE_LIB), Some(id)) => list::particle_rules(asset)
            .nth(ypt_unpack_particle_lib(id))
            .map(|r| r.name.clone()),
        (Some(YPT_BEHAVIOUR), Some(id)) => {
            let (lib_idx, _) = ypt_unpack_behaviour(id);
            list::particle_rules(asset)
                .nth(lib_idx)
                .map(|r| r.name.clone())
        }
        _ => None,
    };
    let narrowed = matches!(
        tag,
        Some(YPT_EFFECT)
            | Some(YPT_EMITTER)
            | Some(YPT_PARTICLE)
            | Some(YPT_DOMAIN)
            | Some(YPT_EMITTER_LIB)
            | Some(YPT_PARTICLE_LIB)
            | Some(YPT_BEHAVIOUR)
    );

    let mut rows = Vec::new();
    for (eff_idx, eff) in list::effect_rules(asset).enumerate() {
        for (i, evt) in list::events(eff).enumerate() {
            let include = match (tag, selection) {
                (Some(YPT_EFFECT), Some(s)) => ypt_unpack_effect(s) == eff_idx,
                (Some(YPT_EMITTER), Some(s)) => ypt_unpack_emitter(s) == (eff_idx, i),
                (Some(YPT_PARTICLE), Some(s)) => ypt_unpack_particle(s) == (eff_idx, i),
                (Some(YPT_DOMAIN), Some(s)) => {
                    let (e, em, _k) = ypt_unpack_domain(s);
                    e == eff_idx && em == i
                }
                (Some(YPT_EMITTER_LIB), _) => emitter_lib_name
                    .as_deref()
                    .map(|n| !n.is_empty() && evt.emitter_rule_name == n)
                    .unwrap_or(false),
                (Some(YPT_PARTICLE_LIB), _) | (Some(YPT_BEHAVIOUR), _) => particle_lib_name
                    .as_deref()
                    .map(|n| !n.is_empty() && evt.particle_rule_name == n)
                    .unwrap_or(false),
                _ => true,
            };
            if !include {
                continue;
            }
            let label = if !evt.emitter_rule_name.is_empty() {
                format!("{}.{}", eff.name, evt.emitter_rule_name)
            } else {
                format!("{}.event[{i}]", eff.name)
            };
            rows.push(TrackRow {
                label,
                start_ratio: evt.base.start_ratio.clamp(0.0, 1.0),
                end_ratio: evt.base.end_ratio.clamp(0.0, 1.0),
            });
        }
        if !narrowed && eff_idx > 6 {
            break;
        }
    }
    rows
}

/// Apply `edits` to the active document's particle rules, each rule's edits one step of its history.
fn apply_kfp_edits(state: &mut AppState, edits: &[KfpEdit]) -> bool {
    let Some(doc_id) = state.active_document else {
        return false;
    };
    let Some(doc) = state.document(doc_id) else {
        return false;
    };
    on_particle_target!(&doc.payload, T => apply_kfp_edits_to::<T>(state, doc_id, edits),
        else false)
}

fn apply_kfp_edits_to<T: ParticleTarget>(
    state: &mut AppState,
    doc_id: DocumentId,
    edits: &[KfpEdit],
) -> bool {
    if state
        .document(doc_id)
        .is_some_and(crate::state::Document::is_from_rpf9_table)
    {
        return false;
    }
    let mut before = Vec::new();
    if let Some(asset) = (state.document_mut(doc_id)).and_then(|d| T::list_mut(&mut d.payload)) {
        for edit in edits {
            let Some(particle) = or_particle::list::particle_rules_mut(asset)
                .find(|p| p.name == edit.source.particle_name)
            else {
                continue;
            };
            let was = particle.clone();
            let mut behaviours = or_particle::list::behaviours(particle);
            let Some(behaviour) = behaviours.get_mut(edit.source.behaviour_index) else {
                continue;
            };
            if let Some(kpd) = kpd_for_slot_mut(behaviour, edit.source.slot) {
                if apply_single_edit(kpd, edit) {
                    or_particle::list::set_behaviours(particle, &behaviours);
                    before.push(was);
                }
            }
        }
    }
    let applied = !before.is_empty();
    for rule in before {
        crate::ui::panels::inspector::ypt_steps::particle(state, doc_id, rule);
    }
    applied
}

fn kpd_for_slot_mut(
    b: &mut or_particle::behaviour::Behaviour,
    slot: ParticleKfpSlot,
) -> Option<&mut or_particle::keyframe::KeyframePropData> {
    use or_particle::behaviour::Behaviour;
    use ParticleKfpSlot::*;
    match (b, slot) {
        (Behaviour::Acceleration(d), AccelMin) => d.xyz_min_keyframes.as_mut(),
        (Behaviour::Acceleration(d), AccelMax) => d.xyz_max_keyframes.as_mut(),
        (Behaviour::Colour(d), ColourRgbaMin) => d.rgba_min_keyframes.as_mut(),
        (Behaviour::Colour(d), ColourRgbaMax) => d.rgba_max_keyframes.as_mut(),
        (Behaviour::Colour(d), ColourEmissive) => d.emissive_intensity_keyframes.as_mut(),
        (Behaviour::Size(d), SizeWhdMin) => d.whd_min_keyframes.as_mut(),
        (Behaviour::Size(d), SizeWhdMax) => d.whd_max_keyframes.as_mut(),
        (Behaviour::Size(d), SizeTblrScalar) => d.tblr_scalar_keyframes.as_mut(),
        (Behaviour::Size(d), SizeTblrVelScalar) => d.tblr_vel_scalar_keyframes.as_mut(),
        (Behaviour::Rotation(d), RotInitAngleMin) => d.initial_angle_min_keyframes.as_mut(),
        (Behaviour::Rotation(d), RotInitAngleMax) => d.initial_angle_max_keyframes.as_mut(),
        (Behaviour::Rotation(d), RotAngleMin) => d.angle_min_keyframes.as_mut(),
        (Behaviour::Rotation(d), RotAngleMax) => d.angle_max_keyframes.as_mut(),
        (Behaviour::Noise(d), NoisePosMin) => d.pos_noise_min_keyframes.as_mut(),
        (Behaviour::Noise(d), NoisePosMax) => d.pos_noise_max_keyframes.as_mut(),
        (Behaviour::Noise(d), NoiseVelMin) => d.vel_noise_min_keyframes.as_mut(),
        (Behaviour::Noise(d), NoiseVelMax) => d.vel_noise_max_keyframes.as_mut(),
        (Behaviour::Dampening(d), DampMin) => d.xyz_min_keyframes.as_mut(),
        (Behaviour::Dampening(d), DampMax) => d.xyz_max_keyframes.as_mut(),
        (Behaviour::Wind(d), WindInfluence) => d.influence_keyframes.as_mut(),
        (Behaviour::Attractor(d), AttractorStrength) => d.strength_keyframes.as_mut(),
        (Behaviour::AnimateTexture(d), AnimateTextureRate) => d.anim_rate_keyframes.as_mut(),
        (Behaviour::Collision(d), CollisionBounciness) => d.bounciness_keyframes.as_mut(),
        (Behaviour::Collision(d), CollisionBounceDirVar) => d.bounce_dir_var_keyframes.as_mut(),
        (Behaviour::MatrixWeight(d), MatrixWeight) => d.weight_keyframes.as_mut(),
        (Behaviour::ZCulling(d), ZCullingHeight) => d.height_keyframes.as_mut(),
        (Behaviour::ZCulling(d), ZCullingFadeDist) => d.fade_dist_keyframes.as_mut(),
        (Behaviour::Decal(d), DecalDimensions) => d.dimensions_keyframes.as_mut(),
        (Behaviour::Decal(d), DecalAlpha) => d.alpha_keyframes.as_mut(),
        _ => None,
    }
}

pub(crate) fn apply_single_edit(
    kpd: &mut or_particle::keyframe::KeyframePropData,
    edit: &KfpEdit,
) -> bool {
    if edit.entry_idx >= kpd.entries.len() {
        return false;
    }
    if edit.channel >= 4 {
        return false;
    }
    let (lo, hi) = {
        let prev = if edit.entry_idx > 0 {
            kpd.entries[edit.entry_idx - 1].time[edit.channel]
        } else {
            0.0
        };
        let next = if edit.entry_idx + 1 < kpd.entries.len() {
            kpd.entries[edit.entry_idx + 1].time[edit.channel]
        } else {
            1.0
        };
        (prev.max(0.0), next.min(1.0))
    };
    let new_time = edit.new_time.clamp(lo, hi);
    let entry = &mut kpd.entries[edit.entry_idx];
    let changed = (entry.time[edit.channel] - new_time).abs() > 1e-6
        || (entry.value[edit.channel] - edit.new_value).abs() > 1e-6;
    entry.time[edit.channel] = new_time;
    entry.value[edit.channel] = edit.new_value;
    changed
}

fn draw_drivers_rail(ui: &mut egui::Ui, state: &mut AppState, simulated: bool) {
    use crate::state::drivers::WeatherState;

    ui.horizontal(|ui| {
        if !simulated {
            ui.disable();
        }
        ui.label(
            egui::RichText::new(tr!("Drivers:"))
                .small()
                .color(theme::TEXT_SECONDARY),
        );

        ui.label(tr!("ToD:"));
        let mut hour = state.drivers.time_of_day.hour;
        let resp = ui.add(
            egui::DragValue::new(&mut hour)
                .speed(0.05)
                .range(0.0..=24.0)
                .fixed_decimals(2)
                .suffix("h"),
        );
        let resp = crate::ui::accessible_name::named(resp, tr!("Time of day"))
            .on_disabled_hover_text(crate::i18n::tr(NOT_SIMULATED));
        if resp.changed() {
            state.drivers.time_of_day.set_hour(hour);
        }

        ui.separator();

        ui.label(tr!("Weather:"));
        let current = state.drivers.weather.state;
        let resp = egui::ComboBox::from_id_salt("ws5_weather_dropdown")
            .selected_text(current.label())
            .show_ui(ui, |ui| {
                let mut chosen = current;
                for s in WeatherState::ALL {
                    if ui.selectable_label(s == current, s.label()).clicked() {
                        chosen = s;
                    }
                }
                chosen
            });
        crate::ui::accessible_name::named(resp.response.clone(), tr!("Weather"))
            .on_disabled_hover_text(crate::i18n::tr(NOT_SIMULATED));
        if let Some(chosen) = resp.inner {
            state.set_weather_state(chosen);
        }

        ui.separator();

        ui.label(tr!("Exposure:"));
        let mut exp = state.drivers.exposure.value;
        let resp = ui.add(egui::DragValue::new(&mut exp).speed(0.005).range(0.0..=1.0));
        let resp = crate::ui::accessible_name::named(resp, tr!("Exposure"))
            .on_disabled_hover_text(crate::i18n::tr(NOT_SIMULATED));
        if resp.changed() {
            state.drivers.exposure.set_value(exp);
        }
    });
}

pub(crate) fn pin_label(state: &AppState, key: &crate::state::sim::SimBuildKey) -> String {
    let crate::state::sim::SerialisableTarget::Effect(idx) = key.target else {
        return "?".into();
    };
    let name = state.document(key.doc_id).and_then(|doc| {
        on_particle_target!(&doc.payload, T => T::list(&doc.payload).and_then(|asset| {
            or_particle::list::effect_rules(asset).nth(idx).map(|e| e.name.clone())
        }), else None)
    });
    name.filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("effect[{idx}]"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DocPayload;
    use or_io::gta5::particle::RawKeyframeEntry;
    use or_particle::behaviour::{AnimateTextureBehaviour, Behaviour};
    use or_particle::keyframe::KeyframePropData;

    fn build_state_with_anim_kfp() -> (AppState, crate::state::DocumentId) {
        build_state_with_anim_kfp_at::<or_io::decl::Gen8>(or_io::Asset::Particles)
    }

    fn build_state_with_anim_kfp_at<T: ParticleTarget>(
        wrap: fn(or_io::gta5::PtxFxList<T>) -> or_io::Asset,
    ) -> (AppState, crate::state::DocumentId) {
        let mut state = AppState::new();
        state.documents.clear();
        state.active_document = None;
        let mut asset = or_io::gta5::PtxFxList::<T>::default();
        or_particle::list::add_particle_rule(&mut asset, "p");
        let behaviours = vec![Behaviour::AnimateTexture(AnimateTextureBehaviour {
            anim_rate_min: 1.0,
            anim_rate_max: 1.0,
            anim_rate_keyframes: Some(KeyframePropData {
                entries: vec![
                    RawKeyframeEntry {
                        time: [0.0; 4],
                        value: [1.0, 0.0, 0.0, 0.0],
                    },
                    RawKeyframeEntry {
                        time: [1.0; 4],
                        value: [2.0, 0.0, 0.0, 0.0],
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        })];
        if let Some(particle) = or_particle::list::particle_rules_mut(&mut asset).next() {
            or_particle::list::set_behaviours(particle, &behaviours);
        }
        let id: crate::state::DocumentId = 1;
        let doc = crate::state::Document::new(
            id,
            Some(std::path::PathBuf::from("kfp.ypt")),
            DocPayload::Resource(wrap(asset)),
        );
        state.documents.push(doc);
        state.active_document = Some(id);
        (state, id)
    }

    #[test]
    fn a_table_particle_timeline_refuses_curve_edits_and_keeps_playback() {
        let _serial = crate::ui::theme::serial();
        let (mut state, id) = build_state_with_anim_kfp();
        let reason = crate::state::rpf9::entry::READ_ONLY;
        state.document_mut(id).unwrap().read_only = Some(reason);
        let shown = |state: &AppState| {
            let asset = or_io::decl::Gen8::list(&state.document(id).unwrap().payload).unwrap();
            format!("{:?}", or_particle::list::particle_rules(asset).next())
        };
        let before = shown(&state);
        let edit = KfpEdit {
            source: KfpCurveSource {
                particle_name: "p".into(),
                behaviour_index: 0,
                slot: ParticleKfpSlot::AnimateTextureRate,
            },
            entry_idx: 0,
            channel: 0,
            new_time: 0.25,
            new_value: 5.5,
        };
        assert!(!apply_kfp_edits(&mut state, &[edit]));
        assert_eq!(shown(&state), before);
        assert!(!state.document(id).unwrap().history.can_undo());
        let mut source = source_with_one_of_each();
        source.read_only = Some(reason);
        assert!(!source.editable());
        assert_eq!(source.read_only_reason(), Some(crate::i18n::tr(reason)));
        assert!(source.playback_unavailable().is_none());
    }

    #[test]
    fn a_list_nothing_simulates_is_scaled_to_the_effect_it_selects_else_the_longest() {
        use crate::ui::panels::selection_ids::{ypt_effect_id, ypt_emitter_id};
        use or_particle::list;

        // The seconds taken with nothing selected, with the short effect and with an event of the long one.
        fn authored<T: ParticleTarget>() -> [f32; 3] {
            let mut fx = or_io::gta5::PtxFxList::<T>::default();
            for (name, seconds) in [("short", 1.0), ("long", 3.5)] {
                list::add_effect(&mut fx, name);
                list::effect_rules_mut(&mut fx)
                    .filter(|effect| effect.name == name)
                    .for_each(|effect| effect.duration_max = seconds);
            }
            let at = |name: &str| list::effect_rules(&fx).position(|e| e.name == name);
            let (short, long) = (at("short").expect("short"), at("long").expect("long"));
            [
                authored_seconds(&fx, None),
                authored_seconds(&fx, Some(ypt_effect_id(short))),
                authored_seconds(&fx, Some(ypt_emitter_id(long, 0))),
            ]
        }
        assert_eq!(authored::<or_io::decl::Gen8>(), [3.5, 1.0, 3.5]);
        assert_eq!(authored::<or_io::decl::Gen9>(), [3.5, 1.0, 3.5]);
    }

    #[test]
    fn a_dragged_keyframe_undoes_to_the_curve_the_file_opened_with() {
        a_drag_undoes::<or_io::decl::Gen8>(or_io::Asset::Particles);
        a_drag_undoes::<or_io::decl::Gen9>(or_io::Asset::ParticlesGen9);
    }

    fn a_drag_undoes<T: ParticleTarget + std::fmt::Debug>(
        wrap: fn(or_io::gta5::PtxFxList<T>) -> or_io::Asset,
    ) {
        let (mut state, id) = build_state_with_anim_kfp_at::<T>(wrap);
        let rule = |state: &AppState| {
            let payload = &state.document(id).expect("open").payload;
            let asset = T::list(payload).expect("not a YPT doc");
            format!("{:?}", or_particle::list::particle_rules(asset).next())
        };
        let opened = rule(&state);
        let drag = |time: f32| KfpEdit {
            source: KfpCurveSource {
                particle_name: "p".into(),
                behaviour_index: 0,
                slot: ParticleKfpSlot::AnimateTextureRate,
            },
            entry_idx: 0,
            channel: 0,
            new_time: time,
            new_value: 5.5,
        };
        for time in [0.1, 0.2, 0.25] {
            assert!(apply_kfp_edits(&mut state, &[drag(time)]));
        }
        let dragged = rule(&state);
        assert_ne!(dragged, opened, "the drag moved nothing");

        assert!(state.undo().is_some(), "the drag is not a step");
        assert_eq!(rule(&state), opened, "one undo does not take back one drag");
        assert!(!state.document(id).expect("open").dirty);
        state.redo();
        assert_eq!(rule(&state), dragged);
    }

    fn source_with_one_of_each() -> YptTimelineSource {
        let track = TrackRow {
            label: "emitter".to_string(),
            start_ratio: 0.25,
            end_ratio: 0.75,
        };
        let curve = CurveSeries {
            label: "rate".to_string(),
            samples: vec![(0.0, [1.0, 0.0, 0.0, 0.0]), (1.0, [2.0, 0.0, 0.0, 0.0])],
            channel_count: 1,
            source: Some(KfpCurveSource {
                particle_name: "p".to_string(),
                behaviour_index: 0,
                slot: ParticleKfpSlot::AnimateTextureRate,
            }),
            raw_entries: vec![
                RawKeyframeEntry {
                    time: [0.0; 4],
                    value: [1.0, 0.0, 0.0, 0.0],
                },
                RawKeyframeEntry {
                    time: [1.0; 4],
                    value: [2.0, 0.0, 0.0, 0.0],
                },
            ],
        };
        YptTimelineSource::new(vec![track], vec![curve], 4.0, "summary".to_string())
    }

    #[test]
    fn emitter_ratios_become_seconds_on_the_timeline() {
        let source = source_with_one_of_each();
        assert_eq!(source.rows().len(), 1);
        assert_eq!(source.bounds(), TimeRange::new(0.0, 4.0));

        let mut events = RowEvents::default();
        source.row_events(0, source.bounds(), &mut events);
        let span = &events.spans[0];
        assert!((span.range.start - 1.0).abs() < 1e-6);
        assert!((span.range.end - 3.0).abs() < 1e-6);
    }

    #[test]
    fn events_outside_the_view_are_not_emitted() {
        let source = source_with_one_of_each();
        let mut events = RowEvents::default();
        source.row_events(0, TimeRange::new(3.5, 4.0), &mut events);
        assert!(events.keys.is_empty() && events.spans.is_empty());
        source.row_events(99, source.bounds(), &mut events);
        assert!(events.keys.is_empty() && events.spans.is_empty());
    }

    #[test]
    fn curves_declare_a_normalised_domain() {
        let source = source_with_one_of_each();
        let curve = &source.curves()[0];
        assert_eq!(curve.domain, CurveDomain::Normalized);
        assert!(!curve.domain_label.is_empty());
        assert_eq!(curve.channels.len(), 1);
        assert_eq!(curve.channels[0].samples.len(), 2);
    }

    #[test]
    fn one_handle_is_emitted_per_entry_and_channel() {
        let source = source_with_one_of_each();
        assert_eq!(source.curves()[0].handles.len(), 2);
        assert_eq!(source.curves()[0].handles[0].index, 0);
        assert_eq!(source.curves()[0].handles[1].index, 1);
        assert!(source.curves()[0].handles.iter().all(|h| h.channel == 0));
    }

    #[test]
    fn a_sourceless_curve_gets_no_handles() {
        let curve = CurveSeries {
            label: "read only".to_string(),
            samples: vec![(0.0, [1.0; 4])],
            channel_count: 1,
            source: None,
            raw_entries: vec![RawKeyframeEntry {
                time: [0.0; 4],
                value: [1.0; 4],
            }],
        };
        let source = YptTimelineSource::new(Vec::new(), vec![curve], 1.0, String::new());
        assert!(source.curves()[0].handles.is_empty());
    }

    #[test]
    fn a_handle_drag_routes_back_to_the_right_kfp_entry() {
        let source = source_with_one_of_each();
        let edit = source
            .kfp_edit(&TimelineEdit {
                series: 0,
                channel: 0,
                index: 1,
                time: 0.6,
                value: 9.5,
            })
            .expect("routed");

        assert_eq!(edit.entry_idx, 1);
        assert_eq!(edit.channel, 0);
        assert_eq!(edit.source.particle_name, "p");
        assert_eq!(edit.source.slot, ParticleKfpSlot::AnimateTextureRate);
        assert!((edit.new_time - 0.6).abs() < 1e-6);
        assert!((edit.new_value - 9.5).abs() < 1e-6);
    }

    #[test]
    fn a_handle_drag_clamps_time_into_the_lifetime_ratio() {
        let source = source_with_one_of_each();
        for (dragged, expected) in [(-3.0f32, 0.0f32), (7.0, 1.0)] {
            let edit = source
                .kfp_edit(&TimelineEdit {
                    series: 0,
                    channel: 0,
                    index: 0,
                    time: dragged,
                    value: 0.0,
                })
                .expect("routed");
            assert!((edit.new_time - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn an_unknown_series_routes_to_nothing() {
        let source = source_with_one_of_each();
        assert!(source
            .kfp_edit(&TimelineEdit {
                series: 99,
                channel: 0,
                index: 0,
                time: 0.5,
                value: 0.0,
            })
            .is_none());
    }

    #[test]
    fn an_empty_asset_still_yields_a_usable_bounds() {
        let source = YptTimelineSource::new(Vec::new(), Vec::new(), 0.0, String::new());
        assert!(source.bounds().span() > 0.0);
        assert!(source.rows().is_empty());
        assert!(source.curves().is_empty());
    }

    #[test]
    fn kfp_curve_drag_mutates_entry_and_marks_dirty() {
        let (mut state, _id) = build_state_with_anim_kfp();
        state.sim.pending_rebuild = false;

        let edits = vec![KfpEdit {
            source: KfpCurveSource {
                particle_name: "p".into(),
                behaviour_index: 0,
                slot: ParticleKfpSlot::AnimateTextureRate,
            },
            entry_idx: 0,
            channel: 0,
            new_time: 0.25,
            new_value: 5.5,
        }];
        let applied = apply_kfp_edits(&mut state, &edits);
        assert!(applied, "edit must land");

        let Some(doc) = state.active_document() else {
            panic!("no active doc")
        };
        let DocPayload::Resource(or_io::Asset::Particles(asset)) = &doc.payload else {
            panic!("not a YPT doc")
        };
        let particle = or_particle::list::particle_rules(asset)
            .next()
            .expect("the rule is there");
        let behaviours = or_particle::list::behaviours(particle);
        let Behaviour::AnimateTexture(at) = &behaviours[0] else {
            panic!("not an animate-texture behaviour")
        };
        let kpd = at.anim_rate_keyframes.as_ref().expect("kfp present");
        assert!((kpd.entries[0].time[0] - 0.25).abs() < 1e-6);
        assert!((kpd.entries[0].value[0] - 5.5).abs() < 1e-6);
        assert!(state.sim.pending_rebuild, "drag must arm sim rebuild");
    }

    #[test]
    fn kfp_drag_clamps_time_to_neighbours() {
        let mut kpd = KeyframePropData {
            entries: vec![
                RawKeyframeEntry {
                    time: [0.0; 4],
                    value: [0.0; 4],
                },
                RawKeyframeEntry {
                    time: [0.5; 4],
                    value: [0.0; 4],
                },
                RawKeyframeEntry {
                    time: [1.0; 4],
                    value: [0.0; 4],
                },
            ],
            ..Default::default()
        };
        let edit = KfpEdit {
            source: KfpCurveSource {
                particle_name: String::new(),
                behaviour_index: 0,
                slot: ParticleKfpSlot::AnimateTextureRate,
            },
            entry_idx: 1,
            channel: 0,
            new_time: 5.0,
            new_value: 99.0,
        };
        assert!(apply_single_edit(&mut kpd, &edit));
        assert_eq!(kpd.entries[1].time[0], 1.0);
        assert_eq!(kpd.entries[1].value[0], 99.0);
    }
}
