//! What the index keeps of an archetype's lights: one row a light, in the archetype's own space,
//! by the word its entities name it by.

use crate::gta6::map::light_ext::{self, Light, LightKind};
use crate::meta::binary::rbm::schema::RbmModel;
use bytemuck::{Pod, Zeroable};

/// One light an archetype holds, in its model's space.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct LightRow {
    /// The word an entity names the archetype by.
    pub archetype: u64,
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub colour: [f32; 3],
    pub intensity: f32,
    pub radius: f32,
    pub falloff_exponent: f32,
    pub hours: u32,
    /// The file's type code: 1 point, 2 spot, 4 capsule, 32 area.
    pub light_type: u32,
    pub cone_inner: f32,
    pub cone_outer: f32,
    pub length: f32,
    pub flags: u32,
    /// The corona's size, intensity and pull toward the eye.
    pub corona: [f32; 3],
    pub pad: u32,
}

fn type_code(kind: LightKind) -> u32 {
    match kind {
        LightKind::Point => 1,
        LightKind::Spot => 2,
        LightKind::Capsule => 4,
        LightKind::Area => 32,
    }
}

impl LightRow {
    fn of(archetype: u64, light: &Light) -> Self {
        Self {
            archetype,
            position: light.position,
            direction: light.direction,
            colour: light.colour,
            intensity: light.intensity,
            radius: light.radius,
            falloff_exponent: light.falloff_exponent,
            hours: light.hours,
            light_type: type_code(light.kind),
            cone_inner: light.cone_inner,
            cone_outer: light.cone_outer,
            length: light.length,
            flags: light.flags,
            corona: light.corona,
            pad: 0,
        }
    }

    /// The light the row keeps; `None` for a row of another type, which a kept index of another build may hold.
    pub fn light(&self) -> Option<Light> {
        let kind = LightKind::of(u8::try_from(self.light_type).ok()?)?;
        Some(Light {
            kind,
            position: self.position,
            direction: self.direction,
            colour: self.colour,
            intensity: self.intensity,
            radius: self.radius,
            falloff_exponent: self.falloff_exponent,
            hours: self.hours,
            flags: self.flags,
            cone_inner: self.cone_inner,
            cone_outer: self.cone_outer,
            length: self.length,
            corona: self.corona,
        })
    }
}

/// The rows of the lights each archetype of the map-types body `model` holds; none where its lights do not read.
pub fn rows_of(model: &RbmModel) -> Vec<LightRow> {
    match light_ext::archetype_lights(model) {
        Ok(found) => (found.iter())
            .flat_map(|(word, lights)| lights.iter().map(|l| LightRow::of(*word, l)))
            .collect(),
        Err(error) => {
            tracing::debug!(%error, "the lights of a map-types body are not read");
            Vec::new()
        }
    }
}

/// Put `rows` in the order [`of_archetype`] reads them in, with a light two bodies state alike kept once.
// ponytail: two bodies that state one archetype with other lights keep both sets; tag a row with its body if that is met.
pub fn settle(rows: &mut Vec<LightRow>) {
    rows.sort_unstable_by(|a, b| {
        (a.archetype.cmp(&b.archetype))
            .then_with(|| bytemuck::bytes_of(a).cmp(bytemuck::bytes_of(b)))
    });
    rows.dedup();
}

/// The rows of the archetype `word`, of rows [`settle`] ordered.
pub fn of_archetype(rows: &[LightRow], word: u64) -> &[LightRow] {
    let from = rows.partition_point(|r| r.archetype < word);
    let to = from + rows[from..].partition_point(|r| r.archetype == word);
    &rows[from..to]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(archetype: u64, intensity: f32) -> LightRow {
        LightRow {
            archetype,
            intensity,
            light_type: 2,
            ..LightRow::zeroed()
        }
    }

    #[test]
    fn an_archetype_s_rows_are_found_by_its_word_and_a_copy_is_kept_once() {
        let mut rows = vec![
            row(9, 1.0),
            row(3, 2.0),
            row(9, 3.0),
            row(3, 2.0),
            row(5, 4.0),
        ];
        settle(&mut rows);
        assert_eq!(rows.len(), 4);
        let nine: Vec<f32> = of_archetype(&rows, 9).iter().map(|r| r.intensity).collect();
        assert_eq!(nine.len(), 2);
        assert!(nine.contains(&1.0) && nine.contains(&3.0));
        assert_eq!(of_archetype(&rows, 3).len(), 1);
        assert!(of_archetype(&rows, 4).is_empty() && of_archetype(&rows, 10).is_empty());
    }

    #[test]
    fn a_row_gives_back_its_light_and_a_row_of_another_type_none() {
        let light = Light {
            kind: LightKind::Capsule,
            position: [1.0, 2.0, 3.0],
            direction: [0.0, 1.0, 0.0],
            colour: [1.0, 0.5, 0.25],
            intensity: 25.0,
            radius: 8.0,
            falloff_exponent: 6.0,
            hours: 0x00E0_007F,
            flags: 1 << 15,
            cone_inner: 10.0,
            cone_outer: 30.0,
            length: 4.0,
            corona: [0.5, 2.0, 0.1],
        };
        let kept = LightRow::of(1, &light);
        assert_eq!((kept.archetype, kept.light_type), (1, 4));
        assert_eq!(kept.light(), Some(light));
        assert_eq!(
            LightRow {
                light_type: 300,
                ..row(1, 5.0)
            }
            .light(),
            None
        );
        assert_eq!(std::mem::size_of::<LightRow>(), 96);
    }

    #[test]
    fn a_body_that_is_no_map_types_gives_no_row() {
        let model = super::super::lights_lod::tests::model(&[1.0], &[]);
        assert!(rows_of(&model).is_empty());
    }
}
