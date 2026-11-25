//! Mutating a YPT from the outliner, deferred.
//!
//! A row is drawn while the asset is borrowed, so nothing mutates in place:
//! [`YptDeferredOp`] records the intent and [`apply_ypt_ops`] runs the whole
//! queue once the frame's rows are done. [`super::menus`] is what fills it.

use super::*;
use crate::state::{Change, Swap};
use crate::ui::panels::inspector::ypt_steps::particles;
use or_io::gta5::particle::{PtxEmitterRule, PtxParticleRule};

pub(super) enum YptDeferredOp {
    AddEffect,
    DeleteEffect(usize),
    DuplicateEffect(usize),
    AddEmitterRule,
    DeleteEmitterRule(usize, bool),
    AddParticleRule,
    DeleteParticleRule(usize, bool),
    DuplicateEmitterRule(usize),
    DuplicateParticleRule(usize),
    /// An effect's index and the index of its event to copy to its timeline's end.
    DuplicateEvent(usize, usize),
    /// A particle rule's index, its behaviour's, and the rule given a copy of that behaviour.
    CopyBehaviour(usize, usize, usize),
    BindEmitterRule {
        effect_idx: usize,
        emitter_idx: usize,
        rule_name: String,
    },
    BindParticleRule {
        effect_idx: usize,
        emitter_idx: usize,
        rule_name: String,
    },
    SelectAndReveal(u64),
    ToastUsages(String),
    /// A command a row's menu offers, run once the list is back in its document; boxed, a command being several times the size of every other intent.
    Run(Box<crate::ui::commands::Command>),
}

/// Whether Delete takes the outline row `id` selects: an effect or a rule.
pub(crate) fn deletes(id: u64) -> bool {
    matches!(
        ypt_tag_of(id),
        YPT_EFFECT | YPT_EMITTER_LIB | YPT_PARTICLE_LIB
    )
}

/// Delete the effect or rule `id` selects as one step, and a rule from the emitters naming it.
pub(crate) fn delete_row(state: &mut AppState, doc_id: DocumentId, id: u64) {
    use crate::ui::panels::selection_ids::{ypt_unpack_emitter_lib, ypt_unpack_particle_lib};
    let op = match ypt_tag_of(id) {
        YPT_EFFECT => YptDeferredOp::DeleteEffect(ypt_unpack_effect(id)),
        YPT_EMITTER_LIB => YptDeferredOp::DeleteEmitterRule(ypt_unpack_emitter_lib(id), true),
        YPT_PARTICLE_LIB => YptDeferredOp::DeleteParticleRule(ypt_unpack_particle_lib(id), true),
        _ => return,
    };
    let Some(doc) = state.document(doc_id) else {
        return;
    };
    on_particle_target!(&doc.payload, T => mutate::<T>(state, doc_id, op), else ())
}

pub(super) fn apply_ypt_ops<T: ParticleTarget>(
    state: &mut AppState,
    doc_id: DocumentId,
    ops: Vec<YptDeferredOp>,
    ctx: &egui::Context,
) {
    for op in ops {
        match op {
            YptDeferredOp::SelectAndReveal(sel) => {
                state.selection.select(doc_id, sel);
                super::super::ui_state(state).scroll_to = Some((doc_id, sel));
            }
            YptDeferredOp::ToastUsages(msg) => {
                state.show_info_toast(msg);
            }
            YptDeferredOp::Run(command) => crate::ui::commands::run(state, ctx, &command),
            other => mutate::<T>(state, doc_id, other),
        }
    }
}

fn mutate<T: ParticleTarget>(state: &mut AppState, doc_id: DocumentId, op: YptDeferredOp) {
    let asset = ypt_asset_mut::<T>(state, doc_id);
    if let Some((label, change)) = asset.and_then(|a| apply_one_mutation(a, op)) {
        state.record(doc_id, label, change);
    }
}

fn ypt_asset_mut<T: ParticleTarget>(
    state: &mut AppState,
    doc_id: DocumentId,
) -> Option<&mut PtxFxList<T>> {
    let document = state.document_mut(doc_id)?;
    if document.is_from_rpf9_table() {
        return None;
    }
    T::list_mut(&mut document.payload)
}

/// A dictionary's codes and the rules they key.
type Run<'a, R> = (&'a mut Vec<u32>, &'a mut Vec<Option<Box<R>>>);

/// A rule of one of the list's dictionaries, held while it is out of the dictionary.
struct RuleEntry<R, T: ParticleTarget> {
    dictionary: fn(&mut PtxFxList<T>) -> Run<'_, R>,
    name: String,
    other: Option<R>,
}

impl<R, T: ParticleTarget> Swap for RuleEntry<R, T> {
    fn swap(&mut self, payload: &mut DocPayload) -> bool {
        let Some(fx) = particles::<T>(payload) else {
            return false;
        };
        let (codes, entries) = (self.dictionary)(fx);
        let code = list::code(&self.name);
        match self.other.take() {
            Some(rule) => {
                let at = codes.partition_point(|c| *c < code);
                codes.insert(at, code);
                entries.insert(at.min(entries.len()), Some(Box::new(rule)));
                true
            }
            None => match codes.iter().position(|c| *c == code) {
                Some(at) if at < entries.len() => {
                    codes.remove(at);
                    self.other = entries.remove(at).map(|rule| *rule);
                    self.other.is_some()
                }
                _ => false,
            },
        }
    }
}

fn effects<T: ParticleTarget>(fx: &mut PtxFxList<T>) -> Run<'_, PtxEffectRule<T>> {
    let d = fx.effect_rule_dict.get_or_insert_with(Default::default);
    (&mut d.codes, &mut d.entries)
}

fn emitters<T: ParticleTarget>(fx: &mut PtxFxList<T>) -> Run<'_, PtxEmitterRule> {
    let d = fx.emitter_rule_dict.get_or_insert_with(Default::default);
    (&mut d.codes, &mut d.entries)
}

fn particle_rules<T: ParticleTarget>(fx: &mut PtxFxList<T>) -> Run<'_, PtxParticleRule<T>> {
    let d = fx.particle_rule_dict.get_or_insert_with(Default::default);
    (&mut d.codes, &mut d.entries)
}

/// The rule `name` of `dictionary`, which the edit added when `other` is `None` and took out as `other` otherwise.
fn entry<R: 'static, T: ParticleTarget>(
    dictionary: fn(&mut PtxFxList<T>) -> Run<'_, R>,
    name: &str,
    other: Option<R>,
) -> Box<dyn Swap> {
    let name = name.to_string();
    Box::new(RuleEntry {
        dictionary,
        name,
        other,
    })
}

/// The spawners that named a rule, each `(effect, event, name)` holding the name it does not hold now.
struct SpawnerNames<T> {
    emitter: bool,
    names: Vec<(usize, usize, String)>,
    target: std::marker::PhantomData<fn() -> T>,
}

impl<T: ParticleTarget> Swap for SpawnerNames<T> {
    fn swap(&mut self, payload: &mut DocPayload) -> bool {
        let Some(fx) = particles::<T>(payload) else {
            return false;
        };
        // ⚠ Every place is looked for before any name moves: a part that says false has changed nothing
        if self
            .names
            .iter()
            .any(|(effect, event, _)| event_mut(fx, *effect, *event).is_none())
        {
            return false;
        }
        for (effect, event, name) in &mut self.names {
            let Some(slot) = event_mut(fx, *effect, *event) else {
                return false;
            };
            let held = match self.emitter {
                true => &mut slot.emitter_rule_name,
                false => &mut slot.particle_rule_name,
            };
            std::mem::swap(held, name);
        }
        true
    }
}

/// Clear every spawner's name of `name`, as the parts that put them back.
fn unbind<T: ParticleTarget>(
    asset: &mut PtxFxList<T>,
    name: &str,
    emitter: bool,
) -> SpawnerNames<T> {
    let mut names = Vec::new();
    for (e, effect) in list::effect_rules_mut(asset).enumerate() {
        for (i, event) in list::events_mut(effect).enumerate() {
            let held = match emitter {
                true => &mut event.emitter_rule_name,
                false => &mut event.particle_rule_name,
            };
            if held == name {
                names.push((e, i, std::mem::take(held)));
            }
        }
    }
    SpawnerNames {
        emitter,
        names,
        target: std::marker::PhantomData,
    }
}

/// What `op` did, as the step that undoes it.
fn apply_one_mutation<T: ParticleTarget>(
    asset: &mut PtxFxList<T>,
    op: YptDeferredOp,
) -> Option<(String, Change)> {
    let effect_name = |asset: &PtxFxList<T>, idx: usize| {
        list::effect_rules(asset).nth(idx).map(|e| e.name.clone())
    };
    let parts = |parts: Vec<Box<dyn Swap>>| Change::Part { key: None, parts };
    match op {
        YptDeferredOp::AddEffect => {
            let name = next_unique_name(
                "new_effect",
                list::effect_rules(asset).map(|e| e.name.as_str()),
            );
            list::add_effect(asset, &name).then(|| {
                let added = vec![entry(effects::<T>, &name, None)];
                (format!("Add effect {name}"), parts(added))
            })
        }
        YptDeferredOp::DeleteEffect(idx) => {
            let rule = list::effect_rules(asset).nth(idx)?.clone();
            let name = rule.name.clone();
            list::remove_effect(asset, &name).then(|| {
                let gone = vec![entry(effects::<T>, &name, Some(rule))];
                (format!("Delete effect {name}"), parts(gone))
            })
        }
        YptDeferredOp::DuplicateEffect(idx) => {
            let from = effect_name(asset, idx)?;
            let name = next_unique_name(
                &format!("{from}_copy"),
                list::effect_rules(asset).map(|e| e.name.as_str()),
            );
            list::duplicate_effect(asset, &from, &name).then(|| {
                let added = vec![entry(effects::<T>, &name, None)];
                (format!("Duplicate {from}"), parts(added))
            })
        }
        YptDeferredOp::AddEmitterRule => {
            let name = next_unique_name(
                "new_emitter",
                list::emitter_rules(asset).map(|r| r.name.as_str()),
            );
            list::add_emitter_rule(asset, &name).then(|| {
                let added = vec![entry(emitters::<T>, &name, None)];
                (format!("Add emitter {name}"), parts(added))
            })
        }
        YptDeferredOp::DeleteEmitterRule(idx, clear_refs) => {
            let rule = list::emitter_rules(asset).nth(idx)?.clone();
            let name = rule.name.clone();
            if !list::remove_emitter_rule(asset, &name) {
                return None;
            }
            let mut gone = vec![entry(emitters::<T>, &name, Some(rule))];
            if clear_refs {
                gone.push(Box::new(unbind(asset, &name, true)));
            }
            Some((format!("Delete emitter {name}"), parts(gone)))
        }
        YptDeferredOp::AddParticleRule => {
            let name = next_unique_name(
                "new_particle",
                list::particle_rules(asset).map(|r| r.name.as_str()),
            );
            list::add_particle_rule(asset, &name).then(|| {
                let added = vec![entry(particle_rules::<T>, &name, None)];
                (format!("Add particle {name}"), parts(added))
            })
        }
        YptDeferredOp::DeleteParticleRule(idx, clear_refs) => {
            let rule = list::particle_rules(asset).nth(idx)?.clone();
            let name = rule.name.clone();
            if !list::remove_particle_rule(asset, &name) {
                return None;
            }
            let mut gone = vec![entry(particle_rules::<T>, &name, Some(rule))];
            if clear_refs {
                gone.push(Box::new(unbind(asset, &name, false)));
            }
            Some((format!("Delete particle {name}"), parts(gone)))
        }
        YptDeferredOp::DuplicateEmitterRule(idx) => {
            let from = list::emitter_rules(asset).nth(idx)?.name.clone();
            let name = next_unique_name(
                &format!("{from}_copy"),
                list::emitter_rules(asset).map(|r| r.name.as_str()),
            );
            list::duplicate_emitter_rule(asset, &from, &name).then(|| {
                let added = vec![entry(emitters::<T>, &name, None)];
                (format!("Duplicate {from}"), parts(added))
            })
        }
        YptDeferredOp::DuplicateParticleRule(idx) => {
            let from = list::particle_rules(asset).nth(idx)?.name.clone();
            let name = next_unique_name(
                &format!("{from}_copy"),
                list::particle_rules(asset).map(|r| r.name.as_str()),
            );
            list::duplicate_particle_rule(asset, &from, &name).then(|| {
                let added = vec![entry(particle_rules::<T>, &name, None)];
                (format!("Duplicate {from}"), parts(added))
            })
        }
        YptDeferredOp::DuplicateEvent(effect_idx, event_idx) => {
            let effect = list::effect_rules_mut(asset).nth(effect_idx)?;
            let mut copy = list::events(effect).nth(event_idx)?.clone();
            let before = effect.timeline.events.clone();
            copy.base.index = i32::try_from(before.len()).ok()?;
            effect.timeline.events.push(Some(Box::new(copy)));
            let key = format!("events of {effect_idx} at {}", before.len());
            let change = Change::part(key, before, move |p| {
                let effect = list::effect_rules_mut(particles::<T>(p)?).nth(effect_idx)?;
                Some(&mut effect.timeline.events)
            });
            Some(("Duplicate an event".to_string(), change))
        }
        YptDeferredOp::CopyBehaviour(rule, behaviour, into) => {
            let from = list::particle_rules(asset).nth(rule)?;
            let owned = from.all_behaviours.iter();
            let copy = (owned.filter(|b| matches!(b, or_io::decl::Ref::Own(_))))
                .nth(behaviour)?
                .clone();
            let to = list::particle_rules_mut(asset).nth(into)?;
            let before = to.all_behaviours.clone();
            to.all_behaviours.push(copy);
            let key = format!("behaviours of {into} at {}", before.len());
            let change = Change::part(key, before, move |p| {
                let to = list::particle_rules_mut(particles::<T>(p)?).nth(into)?;
                Some(&mut to.all_behaviours)
            });
            Some(("Copy a behaviour to another rule".to_string(), change))
        }
        YptDeferredOp::BindEmitterRule {
            effect_idx,
            emitter_idx,
            rule_name,
        } => {
            let slot = event_mut(asset, effect_idx, emitter_idx)?;
            let before = std::mem::replace(&mut slot.emitter_rule_name, rule_name);
            let key = format!("emitter of {effect_idx} {emitter_idx}");
            let change = Change::part(key, before, move |p| {
                let slot = event_mut(particles::<T>(p)?, effect_idx, emitter_idx)?;
                Some(&mut slot.emitter_rule_name)
            });
            Some(("Bind an emitter rule".to_string(), change))
        }
        YptDeferredOp::BindParticleRule {
            effect_idx,
            emitter_idx,
            rule_name,
        } => {
            let slot = event_mut(asset, effect_idx, emitter_idx)?;
            let before = std::mem::replace(&mut slot.particle_rule_name, rule_name);
            let key = format!("particle of {effect_idx} {emitter_idx}");
            let change = Change::part(key, before, move |p| {
                let slot = event_mut(particles::<T>(p)?, effect_idx, emitter_idx)?;
                Some(&mut slot.particle_rule_name)
            });
            Some(("Bind a particle rule".to_string(), change))
        }
        YptDeferredOp::SelectAndReveal(_)
        | YptDeferredOp::ToastUsages(_)
        | YptDeferredOp::Run(_) => None,
    }
}

fn event_mut<T: ParticleTarget>(
    asset: &mut PtxFxList<T>,
    effect_idx: usize,
    event_idx: usize,
) -> Option<&mut PtxEventEmitter<T>> {
    let effect = or_particle::list::effect_rules_mut(asset).nth(effect_idx)?;
    or_particle::list::events_mut(effect).nth(event_idx)
}

fn next_unique_name<'a>(base: &str, existing: impl Iterator<Item = &'a str>) -> String {
    let names: std::collections::HashSet<&str> = existing.collect();
    if !names.contains(base) {
        return base.to_string();
    }
    for i in 2..u32::MAX {
        let candidate = format!("{}_{}", base, i);
        if !names.contains(candidate.as_str()) {
            return candidate;
        }
    }
    base.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta5::particle::PtxEventEmitter;

    #[test]
    fn a_table_particle_outline_refuses_copies_and_deletions_but_keeps_selection() {
        use or_io::decl::Gen8;
        let _serial = crate::ui::theme::serial();
        let mut asset = PtxFxList::<Gen8>::default();
        list::add_effect(&mut asset, "table_effect");
        let mut state = AppState::new();
        let path = "particles.rpf/.rpf9-table/0/hash_particles.ypt".into();
        state.documents.push(crate::state::Document::new(
            1,
            Some(path),
            DocPayload::Resource(or_io::Asset::Particles(asset)),
        ));
        let selected = ypt_effect_id(0);
        let ops = vec![
            YptDeferredOp::DuplicateEffect(0),
            YptDeferredOp::SelectAndReveal(selected),
        ];
        apply_ypt_ops::<Gen8>(&mut state, 1, ops, &egui::Context::default());
        delete_row(&mut state, 1, selected);
        let document = state.document(1).unwrap();
        let asset = Gen8::list(&document.payload).unwrap();
        assert_eq!(
            list::effect_rules(asset)
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["table_effect"]
        );
        assert_eq!(state.selection.selected(1), Some(selected));
        assert!(!document.history.can_undo());
    }

    #[test]
    fn the_outlines_adds_deletes_and_binds_undo_to_the_list_it_opened_as() {
        undo_to_the_list_it_opened_as::<or_io::decl::Gen8>(or_io::Asset::Particles);
        undo_to_the_list_it_opened_as::<or_io::decl::Gen9>(or_io::Asset::ParticlesGen9);
    }

    fn undo_to_the_list_it_opened_as<T: ParticleTarget + std::fmt::Debug>(
        wrap: fn(PtxFxList<T>) -> or_io::Asset,
    ) {
        let mut fx = PtxFxList::<T>::default();
        list::add_effect(&mut fx, "fx");
        list::add_emitter_rule(&mut fx, "em");
        list::add_particle_rule(&mut fx, "pt");
        if let Some(effect) = list::effect_rules_mut(&mut fx).next() {
            effect.timeline.events.push(Some(Box::new(PtxEventEmitter {
                emitter_rule_name: "em".into(),
                particle_rule_name: "pt".into(),
                ..Default::default()
            })));
        }
        let mut state = AppState::new();
        state.documents.clear();
        let payload = DocPayload::Resource(wrap(fx));
        let doc = crate::state::Document::new(1, Some("fx.ypt".into()), payload);
        state.documents.push(doc);
        state.active_document = Some(1);
        let shown = |state: &mut AppState| format!("{:?}", ypt_asset_mut::<T>(state, 1));
        let opened = shown(&mut state);

        // ⚠ A rule's index is its place in hash order, so it is asked for after each add.
        let effect = |state: &mut AppState| {
            let fx = ypt_asset_mut::<T>(state, 1).expect("the list");
            list::effect_rules(fx)
                .position(|e| e.name == "fx")
                .expect("fx")
        };
        let at = effect(&mut state);
        let ops = vec![YptDeferredOp::AddEffect, YptDeferredOp::DuplicateEffect(at)];
        apply_ypt_ops::<T>(&mut state, 1, ops, &egui::Context::default());
        let at = effect(&mut state);
        let ops = vec![
            YptDeferredOp::DeleteEmitterRule(0, true),
            YptDeferredOp::DeleteParticleRule(0, false),
            YptDeferredOp::BindParticleRule {
                effect_idx: at,
                emitter_idx: 0,
                rule_name: "elsewhere".into(),
            },
        ];
        apply_ypt_ops::<T>(&mut state, 1, ops, &egui::Context::default());
        let edited = shown(&mut state);
        assert_ne!(edited, opened, "the outline changed nothing");

        let mut undone = Vec::new();
        while let Some(label) = state.undo() {
            undone.push(label);
        }
        assert_eq!(undone.len(), 5, "not every edit is a step: {undone:?}");
        assert_eq!(shown(&mut state), opened, "undone, it is not as it opened");
        assert!(!state.document(1).expect("open").dirty);
        while state.redo().is_some() {}
        assert_eq!(shown(&mut state), edited, "redone, an edit is lost");
    }

    #[test]
    fn spawner_names_that_cannot_all_go_back_change_none_of_them() {
        spawner_names_that_cannot_all_go_back::<or_io::decl::Gen8>(or_io::Asset::Particles);
        spawner_names_that_cannot_all_go_back::<or_io::decl::Gen9>(or_io::Asset::ParticlesGen9);
    }

    fn spawner_names_that_cannot_all_go_back<T: ParticleTarget>(
        wrap: fn(PtxFxList<T>) -> or_io::Asset,
    ) {
        let mut fx = PtxFxList::<T>::default();
        list::add_effect(&mut fx, "fx");
        list::add_emitter_rule(&mut fx, "em");
        let effect = list::effect_rules_mut(&mut fx).next().expect("the effect");
        for _ in 0..2 {
            effect.timeline.events.push(Some(Box::new(PtxEventEmitter {
                emitter_rule_name: "em".into(),
                ..Default::default()
            })));
        }
        let mut step = unbind(&mut fx, "em", true);
        assert_eq!(step.names.len(), 2, "both spawners named the rule");
        // The second spawner is gone, as an edit made since can leave it
        let effect = list::effect_rules_mut(&mut fx).next().expect("the effect");
        effect.timeline.events.pop();
        let mut payload = DocPayload::Resource(wrap(fx));
        assert!(!step.swap(&mut payload), "one place is gone");
        let fx = particles::<T>(&mut payload).expect("the list");
        let first = list::events_mut(list::effect_rules_mut(fx).next().expect("the effect"))
            .next()
            .expect("the first spawner");
        assert_eq!(
            first.emitter_rule_name, "",
            "a swap that said false put a name back"
        );
    }

    #[test]
    fn the_outlines_copies_are_steps_that_undo_to_the_list_it_opened_as() {
        use or_io::decl::Gen8;
        let mut fx = PtxFxList::<Gen8>::default();
        list::add_effect(&mut fx, "fx");
        list::add_emitter_rule(&mut fx, "em");
        list::add_particle_rule(&mut fx, "pt");
        list::add_particle_rule(&mut fx, "pu");
        if let Some(effect) = list::effect_rules_mut(&mut fx).next() {
            effect.timeline.events.push(Some(Box::new(PtxEventEmitter {
                emitter_rule_name: "em".into(),
                particle_rule_name: "pt".into(),
                ..Default::default()
            })));
        }
        let size = or_io::gta5::particle::PtxBehaviourAny::PtxuSize(Default::default());
        if let Some(rule) = list::particle_rules_mut(&mut fx).next() {
            rule.all_behaviours
                .push(or_io::decl::Ref::Own(Box::new(size)));
        }
        let mut state = AppState::new();
        state.documents.clear();
        let payload = DocPayload::Resource(or_io::Asset::Particles(fx));
        let doc = crate::state::Document::new(1, Some("fx.ypt".into()), payload);
        state.documents.push(doc);
        state.active_document = Some(1);
        let shown = |state: &mut AppState| format!("{:?}", ypt_asset_mut::<Gen8>(state, 1));
        let opened = shown(&mut state);

        // The behaviour is copied first, while rule 0 is still the one holding it.
        let ops = vec![
            YptDeferredOp::CopyBehaviour(0, 0, 1),
            YptDeferredOp::DuplicateEmitterRule(0),
            YptDeferredOp::DuplicateParticleRule(0),
            YptDeferredOp::DuplicateEvent(0, 0),
            YptDeferredOp::DuplicateEvent(0, 0),
        ];
        apply_ypt_ops::<Gen8>(&mut state, 1, ops, &egui::Context::default());
        let fx = ypt_asset_mut::<Gen8>(&mut state, 1).expect("the list");
        assert_eq!(list::emitter_rules(fx).count(), 2);
        assert_eq!(list::particle_rules(fx).count(), 3);
        let effect = list::effect_rules(fx).next().expect("the effect");
        assert_eq!(list::events(effect).count(), 3);
        let edited = shown(&mut state);

        let mut undone = 0;
        while state.undo().is_some() {
            undone += 1;
        }
        assert_eq!(undone, 5, "each copy is a step of its own");
        assert_eq!(shown(&mut state), opened, "undone, it is not as it opened");
        while state.redo().is_some() {}
        assert_eq!(shown(&mut state), edited, "redone, a copy is lost");
    }
}
