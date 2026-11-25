//! The merge of the archetypes of a type file, an archetype being the one of its name.

use super::*;

/// The members of an archetype that the comparison names, and whether `to` took what `from` states of `member`.
fn copy_of_archetype(
    member: &str,
    to: &mut CBaseArchetypeDefAny,
    from: &CBaseArchetypeDefAny,
) -> bool {
    match member {
        "lodDist" => to.lod_dist = from.lod_dist,
        "flags" => to.flags = from.flags,
        "specialAttribute" => to.special_attribute = from.special_attribute,
        "bbMin" => to.bb_min = from.bb_min,
        "bbMax" => to.bb_max = from.bb_max,
        "bsCentre" => to.bs_centre = from.bs_centre,
        "bsRadius" => to.bs_radius = from.bs_radius,
        "hdTextureDist" => to.hd_texture_dist = from.hd_texture_dist,
        "name" => to.name = from.name.clone(),
        "textureDictionary" => to.texture_dictionary = from.texture_dictionary.clone(),
        "clipDictionary" => to.clip_dictionary = from.clip_dictionary.clone(),
        "drawableDictionary" => to.drawable_dictionary = from.drawable_dictionary.clone(),
        "physicsDictionary" => to.physics_dictionary = from.physics_dictionary.clone(),
        "assetName" => to.asset_name = from.asset_name.clone(),
        "timeFlags" => match (to, from) {
            (
                CBaseArchetypeDefAny::CTimeArchetypeDef(to),
                CBaseArchetypeDefAny::CTimeArchetypeDef(from),
            ) => to.time_flags = from.time_flags,
            _ => return false,
        },
        "mloFlags" => match (to, from) {
            (
                CBaseArchetypeDefAny::CMloArchetypeDef(to),
                CBaseArchetypeDefAny::CMloArchetypeDef(from),
            ) => to.mlo_flags = from.mlo_flags,
            _ => return false,
        },
        _ => return false,
    }
    true
}

/// The words for what an interior holds besides its typed members.
const INSIDE: &str = "its entities and what no typed member names";

fn archetype_of(index: usize, archetype: &CBaseArchetypeDefAny) -> Ref {
    Ref::Archetype(ArchetypeRef {
        index,
        name: crate::changes::name_of(&archetype.name),
    })
}

/// What the merged type file holds at the place of an archetype of the base.
enum Held {
    Base,
    Whole(Side, usize),
    Merged(Box<CBaseArchetypeDefAny>),
    Gone,
}

/// The base, ours and theirs merged as one type file, an archetype being the one of its name, a conflict answered by `policy`; an error when the merged file would not be valid.
pub fn merge_types(
    base: &CMapTypes,
    ours: &CMapTypes,
    theirs: &CMapTypes,
    policy: &mut dyn FnMut(&Conflict) -> Take,
) -> Result<Merge<CMapTypes>, Error> {
    let changes = [types_changes(base, ours), types_changes(base, theirs)];
    let roots = [ours, theirs];
    let root = |side: Side| roots[side as usize];
    let named = |side: Side, index: usize| archetype_of(index, &root(side).archetypes[index]);
    let in_base = |index: usize| archetype_of(index, &base.archetypes[index]);
    let mut asked = Asked {
        policy,
        conflicts: Vec::new(),
        stopped: false,
    };
    let count = base.archetypes.len();

    let file_members = merge_file_members(
        &["name", "dependencies"],
        &changes[0].root,
        &changes[1].root,
        &mut asked,
    );
    let other = other_from(
        changes[0].other,
        changes[1].other,
        || !types_changes(ours, theirs).other,
        &mut asked,
    );

    let changed: [HashMap<usize, &ArchetypeKept>; 2] = [0, 1].map(|side| {
        (changes[side].changed.iter())
            .map(|kept| (kept.base.index, kept))
            .collect()
    });
    let deleted: [BTreeSet<usize>; 2] =
        [0, 1].map(|side| changes[side].deleted.iter().map(|a| a.index).collect());
    let mut held: Vec<Held> = (0..count).map(|_| Held::Base).collect();
    let mut decided = vec![false; count];
    let mut applied: Vec<(Applied, Option<usize>)> = Vec::new();
    // Each archetype a side added: the side, its index there, the index of the other side's that it is too, and whether a conflict put it here.
    let mut appended: Vec<(Side, usize, Option<usize>, bool)> = Vec::new();

    // A name one file declares more than once is told apart by nothing: every archetype of that name is one conflict.
    let twice: BTreeSet<u32> = (changes.iter())
        .flat_map(|c| c.ambiguous.iter().map(|group| group.name.hash))
        .collect();
    for name in twice {
        let of = |list: &[CBaseArchetypeDefAny]| -> Vec<usize> {
            (list.iter().enumerate())
                .filter(|(_, archetype)| archetype.name.hash == name)
                .map(|(index, _)| index)
                .collect()
        };
        let of_base = of(&base.archetypes);
        let of_side = [of(&ours.archetypes), of(&theirs.archetypes)];
        let taken = asked.ask(
            Disagreement::Ambiguous,
            of_base.iter().map(|&i| in_base(i)).collect(),
            (of_side[0].iter()).map(|&i| named(Side::Ours, i)).collect(),
            (of_side[1].iter())
                .map(|&i| named(Side::Theirs, i))
                .collect(),
        );
        for &index in &of_base {
            decided[index] = true;
            held[index] = Held::Gone;
        }
        for (at, &now) in of_side[taken as usize].iter().enumerate() {
            match of_base.get(at) {
                Some(&index) => held[index] = Held::Whole(taken, now),
                None => appended.push((taken, now, None, true)),
            }
        }
    }

    for index in 0..count {
        if decided[index] {
            continue;
        }
        let was = &base.archetypes[index];
        let of = |side: Side| changed[side as usize].get(&index).copied();
        let gone = |side: Side| deleted[side as usize].contains(&index);
        let said = |did: Did, by: By, members: Vec<MemberBy>| Applied {
            did,
            by,
            base: Some(in_base(index)),
            ours: of(Side::Ours).map(|k| named(Side::Ours, k.now.index)),
            theirs: of(Side::Theirs).map(|k| named(Side::Theirs, k.now.index)),
            merged: None,
            members,
            guid: None,
        };
        let members_of = |kept: &ArchetypeKept, by: By| -> Vec<MemberBy> {
            (kept.changes.iter())
                .map(|change| change.member.to_string())
                .chain((kept.entities.is_some() || kept.other).then(|| INSIDE.to_string()))
                .map(|member| MemberBy { member, by })
                .collect()
        };
        match (of(Side::Ours), of(Side::Theirs)) {
            (None, None) => match (gone(Side::Ours), gone(Side::Theirs)) {
                (false, false) => {}
                (ours, theirs) => {
                    held[index] = Held::Gone;
                    let by = match (ours, theirs) {
                        (true, true) => By::Both,
                        (true, false) => By::Ours,
                        _ => By::Theirs,
                    };
                    applied.push((said(Did::Deleted, by, Vec::new()), None));
                }
            },
            (Some(kept), None) | (None, Some(kept)) => {
                let changer = match of(Side::Ours) {
                    Some(_) => Side::Ours,
                    None => Side::Theirs,
                };
                if gone(changer.other()) {
                    let of_changer = vec![named(changer, kept.now.index)];
                    let (of_ours, of_theirs) = match changer {
                        Side::Ours => (of_changer, Vec::new()),
                        Side::Theirs => (Vec::new(), of_changer),
                    };
                    let changed = (members_of(kept, changer.into()).into_iter())
                        .map(|said| said.member)
                        .collect();
                    let taken = asked.ask(
                        Disagreement::ChangedAndDeleted {
                            deleted_by: changer.other(),
                            changed,
                        },
                        vec![in_base(index)],
                        of_ours,
                        of_theirs,
                    );
                    held[index] = match taken == changer {
                        true => Held::Whole(changer, kept.now.index),
                        false => Held::Gone,
                    };
                } else {
                    held[index] = Held::Whole(changer, kept.now.index);
                    let by = changer.into();
                    applied.push((said(Did::Changed, by, members_of(kept, by)), Some(index)));
                }
            }
            (Some(of_ours), Some(of_theirs)) => {
                let (a, b) = (
                    &ours.archetypes[of_ours.now.index],
                    &theirs.archetypes[of_theirs.now.index],
                );
                let refs = || {
                    (
                        vec![in_base(index)],
                        vec![named(Side::Ours, of_ours.now.index)],
                        vec![named(Side::Theirs, of_theirs.now.index)],
                    )
                };
                let of_type =
                    |kept: &ArchetypeKept| kept.changes.iter().any(|c| c.member == "type");
                if of_type(of_ours) || of_type(of_theirs) {
                    if a == b {
                        held[index] = Held::Whole(Side::Ours, of_ours.now.index);
                        let members = members_of(of_ours, By::Both);
                        applied.push((said(Did::Changed, By::Both, members), Some(index)));
                    } else {
                        let (base, ours, theirs) = refs();
                        let names = |kept: &ArchetypeKept| -> Vec<String> {
                            (members_of(kept, By::Both).into_iter())
                                .map(|said| said.member)
                                .collect()
                        };
                        let what = Disagreement::Record {
                            ours: names(of_ours),
                            theirs: names(of_theirs),
                        };
                        let taken = asked.ask(what, base, ours, theirs);
                        held[index] = Held::Whole(
                            taken,
                            match taken {
                                Side::Ours => of_ours.now.index,
                                Side::Theirs => of_theirs.now.index,
                            },
                        );
                    }
                    continue;
                }
                let inside = |kept: &ArchetypeKept| kept.entities.is_some() || kept.other;
                let mut members = Vec::new();
                let rest = three_way(
                    inside(of_ours),
                    inside(of_theirs),
                    || a == b,
                    || {
                        let (base, ours, theirs) = refs();
                        let what = Disagreement::Member {
                            member: INSIDE.to_string(),
                            ours: "changed".to_string(),
                            theirs: "changed".to_string(),
                        };
                        asked.ask(what, base, ours, theirs)
                    },
                );
                let record_of = |side: Option<Side>| match side {
                    None => was,
                    Some(Side::Ours) => a,
                    Some(Side::Theirs) => b,
                };
                let mut record = record_of(rest.map(|(side, _)| side)).clone();
                if let Some((_, Some(by))) = rest {
                    members.push(MemberBy {
                        member: INSIDE.to_string(),
                        by,
                    });
                }
                let stated_by = |kept: &ArchetypeKept, member: &str| -> Option<MemberChange> {
                    kept.changes.iter().find(|c| c.member == member).cloned()
                };
                let mut every: Vec<&'static str> = Vec::new();
                for kept in [of_ours, of_theirs] {
                    for change in &kept.changes {
                        if !every.contains(&change.member) {
                            every.push(change.member);
                        }
                    }
                }
                for member in every {
                    let (x, y) = (stated_by(of_ours, member), stated_by(of_theirs, member));
                    let from = three_way(
                        x.is_some(),
                        y.is_some(),
                        || x.as_ref().map(|c| &c.to) == y.as_ref().map(|c| &c.to),
                        || {
                            let (base, ours, theirs) = refs();
                            let said = |change: &Option<MemberChange>| {
                                change.as_ref().map_or(String::new(), |c| stated(&c.to))
                            };
                            let what = Disagreement::Member {
                                member: member.to_string(),
                                ours: said(&x),
                                theirs: said(&y),
                            };
                            asked.ask(what, base, ours, theirs)
                        },
                    );
                    let Some((side, by)) = from else {
                        continue;
                    };
                    if !copy_of_archetype(member, &mut record, record_of(Some(side))) {
                        return Err(Error::Invalid(format!(
                            "the member `{member}` of the archetype {index} of the base is one this merge cannot state"
                        )));
                    }
                    if let Some(by) = by {
                        members.push(MemberBy {
                            member: member.to_string(),
                            by,
                        });
                    }
                }
                // What a side that changed the inside states of the members neither changed is what the base states: nothing is left to put back.
                held[index] = Held::Merged(Box::new(record));
                if !members.is_empty() {
                    applied.push((said(Did::Changed, By::Both, members), Some(index)));
                }
            }
        }
    }

    // The archetypes a side added: one name added by both is a likely double.
    let mut added: BTreeMap<u32, [Vec<usize>; 2]> = BTreeMap::new();
    for side in [Side::Ours, Side::Theirs] {
        for archetype in &changes[side as usize].added {
            let name = root(side).archetypes[archetype.index].name.hash;
            added.entry(name).or_default()[side as usize].push(archetype.index);
        }
    }
    for [of_ours, of_theirs] in added.into_values() {
        if of_ours.is_empty() || of_theirs.is_empty() {
            appended.extend(of_ours.iter().map(|&i| (Side::Ours, i, None, false)));
            appended.extend(of_theirs.iter().map(|&i| (Side::Theirs, i, None, false)));
            continue;
        }
        let agree = of_ours.len() == of_theirs.len()
            && (of_ours.iter().zip(&of_theirs))
                .all(|(&i, &j)| ours.archetypes[i] == theirs.archetypes[j]);
        if agree {
            appended.extend(
                (of_ours.iter().zip(&of_theirs)).map(|(&i, &j)| (Side::Ours, i, Some(j), false)),
            );
            continue;
        }
        let mut differ: Vec<String> = Vec::new();
        if of_ours.len() != of_theirs.len() {
            differ.push("how many each side added of that name".to_string());
        }
        for (&i, &j) in of_ours.iter().zip(&of_theirs) {
            let between = archetype_changes(i, &ours.archetypes[i], j, &theirs.archetypes[j]);
            let inside = (between.entities.is_some() || between.other).then_some(INSIDE);
            for member in (between.changes.iter().map(|c| c.member)).chain(inside) {
                if !differ.iter().any(|said| said == member) {
                    differ.push(member.to_string());
                }
            }
        }
        let taken = asked.ask(
            Disagreement::Double { differ },
            Vec::new(),
            of_ours.iter().map(|&i| named(Side::Ours, i)).collect(),
            (of_theirs.iter())
                .map(|&i| named(Side::Theirs, i))
                .collect(),
        );
        let kept = match taken {
            Side::Ours => &of_ours,
            Side::Theirs => &of_theirs,
        };
        appended.extend(kept.iter().map(|&i| (taken, i, None, true)));
    }
    // By name, so the order does not depend on which side is called ours: a name both added is held once.
    appended.sort_by_key(|(side, index, ..)| {
        (
            root(*side).archetypes[*index].name.hash,
            *side as usize,
            *index,
        )
    });

    let exact = [Side::Ours, Side::Theirs].into_iter().find(|&side| {
        let untouched = match side {
            Side::Ours => theirs == base,
            Side::Theirs => ours == base,
        };
        untouched && !asked.stopped && asked.all_took(side)
    });
    let mut gone_before = vec![0usize; count + 1];
    for (index, held) in held.iter().enumerate() {
        gone_before[index + 1] = gone_before[index] + usize::from(matches!(held, Held::Gone));
    }
    // The index an archetype has in the merged file: its place less the deletions before it, or its index in the side that is
    // the result, where every archetype that is said to have changed is that side's.
    let merged_index = |place: usize| -> Option<usize> {
        match exact {
            None => Some(place - gone_before[place.min(count)]),
            Some(_) if place < count => match &held[place] {
                Held::Whole(_, now) => Some(*now),
                _ => None,
            },
            Some(_) => Some(appended[place - count].1),
        }
    };
    for (at, (side, index, also, decided)) in appended.iter().enumerate() {
        if *decided {
            continue;
        }
        let (near, far) = (
            Some(named(*side, *index)),
            also.map(|i| named(side.other(), i)),
        );
        let (of_ours, of_theirs) = match side {
            Side::Ours => (near, far),
            Side::Theirs => (far, near),
        };
        applied.push((
            Applied {
                did: Did::Added,
                by: match also {
                    Some(_) => By::Both,
                    None => (*side).into(),
                },
                base: None,
                ours: of_ours,
                theirs: of_theirs,
                merged: None,
                members: Vec::new(),
                guid: None,
            },
            Some(count + at),
        ));
    }
    let [changes_ours, changes_theirs] = changes;
    let mut report = Report {
        ours: SourceChanges::Types(changes_ours),
        theirs: SourceChanges::Types(changes_theirs),
        applied: (applied.into_iter())
            .map(|(applied, place)| Applied {
                merged: (place.filter(|_| !asked.stopped)).and_then(merged_index),
                ..applied
            })
            .collect(),
        file: file_said(&file_members, other),
        conflicts: asked.conflicts,
        merged: !asked.stopped,
        not_compared: not_compared(false),
        notes: Vec::new(),
        findings: Vec::new(),
    };
    if asked.stopped {
        return Ok(Merge {
            merged: None,
            report,
        });
    }

    let merged = match exact {
        Some(side) => {
            report.notes.push(format!(
                "{} states what the base states, so the merged type file is {} as it is",
                side.other().word(),
                side.word()
            ));
            root(side).clone()
        }
        None => {
            let root_of = |side: Option<Side>| match side {
                None => base,
                Some(side) => root(side),
            };
            let from = root_from(other, base, [ours, theirs])?;
            let mut types = root_of(from.map(|(side, _)| side)).clone();
            types.archetypes = base.archetypes.clone();
            for member in ["name", "dependencies"] {
                let side = (file_members.iter())
                    .find(|(taken, ..)| *taken == member)
                    .map(|(_, side, _)| *side);
                match member {
                    "name" => types.name = root_of(side).name.clone(),
                    _ => types.dependencies = root_of(side).dependencies.clone(),
                }
            }
            let source = String::new();
            let done = |types: &mut CMapTypes, operation: Operation| {
                apply_to_types(types, &operation).map_err(|why| {
                    Error::Invalid(format!("{} could not be done: {why}", operation.name()))
                })
            };
            let record_of = |archetype: &CBaseArchetypeDefAny| {
                Record::archetype(archetype)
                    .map_err(|why| Error::Invalid(format!("an archetype: {why}")))
            };
            for (index, held) in held.iter().enumerate() {
                let archetype = match held {
                    Held::Base | Held::Gone => continue,
                    Held::Whole(side, now) => &root(*side).archetypes[*now],
                    Held::Merged(record) => record,
                };
                let operation = Operation::ReplaceArchetype {
                    source: source.clone(),
                    expects: None,
                    archetype: index,
                    record: record_of(archetype)?,
                };
                done(&mut types, operation)?;
            }
            // The removals first: a name that left one place may be the name another archetype takes.
            for index in (0..count).rev() {
                if matches!(held[index], Held::Gone) {
                    let operation = Operation::RemoveArchetype {
                        source: source.clone(),
                        expects: None,
                        archetype: index,
                    };
                    done(&mut types, operation)?;
                }
            }
            for (side, index, ..) in &appended {
                let operation = Operation::InsertArchetype {
                    source: source.clone(),
                    expects: None,
                    archetype: types.archetypes.len(),
                    record: record_of(&root(*side).archetypes[*index])?,
                };
                done(&mut types, operation)?;
            }
            types
        }
    };
    let bytes = merged
        .to_bytes()
        .map_err(|why| Error::Invalid(why.to_string()))?;
    report.findings = judged(&bytes, CMapTypes::EXTENSION).map_err(|refusal| {
        refused_of_itself([("the base", base), ("ours", ours), ("theirs", theirs)])
            .unwrap_or(refusal)
    })?;
    Ok(Merge {
        merged: Some(merged),
        report,
    })
}
