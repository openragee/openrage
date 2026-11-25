//! A `ptxd_Sprite`'s flip chances and a `ptxu_ZCull`'s mode written back in place, each only
//! where the edit changes it.

use super::*;

/// Where a sprite's two flip chances and a `ptxu_ZCull`'s mode are in the behaviour.
const FLIP_CHANCE: usize = 0x08;
const ZCULL_MODE: usize = 0x94;
/// The highest `ptxu_ZCull` mode measured.
const ZCULL_MODES: i32 = 5;

/// The words `edited` changes from `stored` at their section offsets; a value no file holds refused.
pub(super) fn patches(stored: &[Behaviour], edited: &[Behaviour]) -> Result<Vec<(usize, Vec<u8>)>> {
    let mut out = Vec::new();
    for (s, e) in stored.iter().zip(edited) {
        let at = s.at as usize;
        match (s.fields, e.fields) {
            (
                BehaviourFields::Sprite {
                    flip_chance: was, ..
                },
                BehaviourFields::Sprite { flip_chance, .. },
            ) if was.map(f32::to_bits) != flip_chance.map(f32::to_bits) => {
                need(
                    flip_chance.iter().all(|c| (0.0..=1.0).contains(c)),
                    "a ptxd_Sprite's flip chance is not between 0 and 1",
                )?;
                let bytes = flip_chance.iter().flat_map(|c| c.to_le_bytes()).collect();
                out.push((at + FLIP_CHANCE, bytes));
            }
            (BehaviourFields::ZCull { mode: was, .. }, BehaviourFields::ZCull { mode, .. })
                if was != mode =>
            {
                need(
                    (0..=ZCULL_MODES).contains(&mode),
                    "a ptxu_ZCull's mode is not one of the six measured, 0 to 5",
                )?;
                out.push((at + ZCULL_MODE, mode.to_le_bytes().to_vec()));
            }
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(at: u64, fields: BehaviourFields) -> Behaviour {
        Behaviour {
            at,
            class: None,
            class_bit: None,
            size: 0xA0,
            kfps: Vec::new(),
            fields,
        }
    }

    #[test]
    fn only_an_edited_flip_chance_or_mode_is_written_and_one_out_of_range_is_refused() {
        let sprite = BehaviourFields::Sprite {
            flip_chance: [0.5, 0.25],
            alignment: 2,
            bytes: [0; 16],
        };
        let zcull = BehaviourFields::ZCull {
            mode: 1,
            reference_space: 5,
        };
        let stored = [held(0x100, sprite), held(0x400, zcull)];
        let none = patches(&stored, &stored).expect("nothing changed");
        assert!(none.is_empty());

        let mut edited = stored.clone();
        if let BehaviourFields::Sprite { flip_chance, .. } = &mut edited[0].fields {
            flip_chance[1] = 0.75;
        }
        if let BehaviourFields::ZCull { mode, .. } = &mut edited[1].fields {
            *mode = 4;
        }
        let flips = [0.5f32.to_le_bytes(), 0.75f32.to_le_bytes()].concat();
        assert_eq!(
            patches(&stored, &edited).expect("both are written"),
            vec![(0x108, flips), (0x494, 4i32.to_le_bytes().to_vec())]
        );

        for (place, wrong) in [(0, 1.5f32), (0, f32::NAN), (1, 6.0)] {
            let mut out = stored.clone();
            match &mut out[place].fields {
                BehaviourFields::Sprite { flip_chance, .. } => flip_chance[0] = wrong,
                BehaviourFields::ZCull { mode, .. } => *mode = wrong as i32,
                _ => unreachable!("a sprite and a ptxu_ZCull"),
            }
            assert!(patches(&stored, &out).is_err(), "{wrong} refused");
        }
    }
}
