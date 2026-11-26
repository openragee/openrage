//! The particle inspector's edits as steps of the document's history, each holding the rule as it was.

use or_io::gta5::particle::{PtxEffectRule, PtxEmitterRule, PtxParticleRule};
use or_io::gta5::PtxFxList;
use or_particle::list;

use crate::state::particle_target::ParticleTarget;
use crate::state::{AppState, Change, DocPayload, DocumentId};

pub(crate) fn particles<T: ParticleTarget>(payload: &mut DocPayload) -> Option<&mut PtxFxList<T>> {
    T::list_mut(payload)
}

/// Effect rule `idx` of `doc` held `before` until the edit.
pub(crate) fn effect<T: ParticleTarget>(
    state: &mut AppState,
    doc: DocumentId,
    idx: usize,
    before: PtxEffectRule<T>,
) {
    let label = format!("Edit {}", before.name);
    let change = Change::part(format!("effect {idx}"), before, move |p| {
        list::effect_rules_mut(particles::<T>(p)?).nth(idx)
    });
    state.record(doc, label, change);
}

/// Emitter rule `idx` of `doc` held `before` until the edit.
pub(super) fn emitter<T: ParticleTarget>(
    state: &mut AppState,
    doc: DocumentId,
    idx: usize,
    before: PtxEmitterRule,
) {
    let label = format!("Edit {}", before.name);
    let change = Change::part(format!("emitter {idx}"), before, move |p| {
        list::emitter_rules_mut(particles::<T>(p)?).nth(idx)
    });
    state.record(doc, label, change);
}

/// The emitter rule of `doc` named as `before` is held it until the edit.
pub(super) fn named_emitter<T: ParticleTarget>(
    state: &mut AppState,
    doc: DocumentId,
    before: PtxEmitterRule,
) {
    let (label, name) = (format!("Edit {}", before.name), before.name.clone());
    let change = Change::part(format!("emitter {name}"), before, move |p| {
        list::emitter_rules_mut(particles::<T>(p)?).find(|rule| rule.name == name)
    });
    state.record(doc, label, change);
}

/// The particle rule of `doc` named as `before` is held it until the edit.
pub(crate) fn particle<T: ParticleTarget>(
    state: &mut AppState,
    doc: DocumentId,
    before: PtxParticleRule<T>,
) {
    let (label, name) = (format!("Edit {}", before.name), before.name.clone());
    let change = Change::part(format!("particle {name}"), before, move |p| {
        list::particle_rules_mut(particles::<T>(p)?).find(|rule| rule.name == name)
    });
    state.record(doc, label, change);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list_of<T: ParticleTarget>(state: &mut AppState) -> &mut PtxFxList<T> {
        let payload = &mut state.document_mut(1).expect("open").payload;
        particles::<T>(payload).expect("the list")
    }

    #[test]
    fn an_edit_of_a_rule_undoes_and_redoes_at_either_target() {
        undoes::<or_io::decl::Gen8>(or_io::Asset::Particles);
        undoes::<or_io::decl::Gen9>(or_io::Asset::ParticlesGen9);
    }

    fn undoes<T: ParticleTarget + std::fmt::Debug>(wrap: fn(PtxFxList<T>) -> or_io::Asset) {
        let mut fx = PtxFxList::<T>::default();
        list::add_effect(&mut fx, "fx");
        list::add_emitter_rule(&mut fx, "em");
        list::add_particle_rule(&mut fx, "pt");
        let mut state = AppState::new();
        state.documents.clear();
        let payload = DocPayload::Resource(wrap(fx));
        state.documents.push(crate::state::Document::new(
            1,
            Some("fx.ypt".into()),
            payload,
        ));
        state.active_document = Some(1);
        let shown = |state: &mut AppState| format!("{:?}", list_of::<T>(state));
        let opened = shown(&mut state);

        let before = list::effect_rules(list_of::<T>(&mut state)).next().cloned();
        list::effect_rules_mut(list_of::<T>(&mut state)).for_each(|e| e.duration_min += 1.0);
        effect(&mut state, 1, 0, before.expect("an effect"));

        let before = list::emitter_rules(list_of::<T>(&mut state))
            .next()
            .cloned();
        for rule in list::emitter_rules_mut(list_of::<T>(&mut state)) {
            rule.spawn_rate_over_time_kfp
                .keyframe
                .entries
                .push(Default::default());
        }
        emitter::<T>(&mut state, 1, 0, before.expect("an emitter rule"));

        let before = list::particle_rules(list_of::<T>(&mut state))
            .next()
            .cloned();
        list::particle_rules_mut(list_of::<T>(&mut state)).for_each(|r| r.tex_frame_id_min += 1);
        particle(&mut state, 1, before.expect("a particle rule"));

        let edited = shown(&mut state);
        assert_ne!(edited, opened, "the edits changed nothing");
        let mut undone = 0;
        while state.undo().is_some() {
            undone += 1;
        }
        assert_eq!(undone, 3, "not every edit is a step");
        assert_eq!(shown(&mut state), opened, "undone, it is not as it opened");
        while state.redo().is_some() {}
        assert_eq!(shown(&mut state), edited, "redone, an edit is lost");
    }
}
