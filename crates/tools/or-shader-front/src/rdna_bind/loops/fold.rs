//! The bits of a value every device computes alike, folded by the value's own scalar kind.

use naga::{BinaryOperator as B, ScalarKind, UnaryOperator as U};

/// The bits of a scalar or a vector of 32-bit lanes, and whether a binding's words went into them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Word {
    pub lanes: [u32; 4],
    pub n: u8,
    /// Read from a binding at some step: a count to it is the binding's, not the program's.
    pub bound: bool,
}

impl Word {
    pub fn scalar(value: u32, bound: bool) -> Self {
        Self {
            lanes: [value; 4],
            n: 1,
            bound,
        }
    }

    pub fn lanes(&self) -> &[u32] {
        &self.lanes[..usize::from(self.n)]
    }

    /// The one value of a scalar, or of a vector whose lanes all hold it.
    pub fn one(&self) -> Option<u32> {
        let first = self.lanes[0];
        self.lanes()
            .iter()
            .all(|lane| *lane == first)
            .then_some(first)
    }

    /// Each lane of `self` and of `other` through `op`, a scalar taken for every lane.
    pub fn zip(self, other: Self, op: impl Fn(u32, u32) -> Option<u32>) -> Option<Self> {
        let n = match (self.n, other.n) {
            (a, b) if a == b => a,
            (1, b) => b,
            (a, 1) => a,
            _ => return None,
        };
        let mut lanes = [0; 4];
        for (k, lane) in lanes.iter_mut().enumerate().take(usize::from(n)) {
            let pick = |w: &Self| w.lanes[if w.n == 1 { 0 } else { k }];
            *lane = op(pick(&self), pick(&other))?;
        }
        Some(Self {
            lanes,
            n,
            bound: self.bound || other.bound,
        })
    }

    pub fn map(self, op: impl Fn(u32) -> Option<u32>) -> Option<Self> {
        let mut lanes = self.lanes;
        for lane in lanes.iter_mut().take(usize::from(self.n)) {
            *lane = op(*lane)?;
        }
        Some(Self { lanes, ..self })
    }
}

/// A float whose bits every device reads alike: nought, or a normal positive number. A
/// denormal a device may flush, a sign, an infinity or a NaN it may quiet are none.
pub(super) fn plain(bits: u32) -> bool {
    bits == 0 || (0x0080_0000..0x7F80_0000).contains(&bits)
}

/// `op` of two 32-bit lanes of kind `kind` where every device computes the same bits.
// No fold through what a device leaves undefined: a shift by 32 or more, a division by nought or
// of the least signed number by -1, float arithmetic past adding nought and multiplying by it.
pub(super) fn binary(op: B, kind: ScalarKind, l: u32, r: u32) -> Option<u32> {
    let truth = |b: bool| Some(u32::from(b));
    let (sl, sr) = (l as i32, r as i32);
    match kind {
        ScalarKind::Bool => match op {
            B::LogicalAnd | B::And => truth(l != 0 && r != 0),
            B::LogicalOr | B::InclusiveOr => truth(l != 0 || r != 0),
            B::ExclusiveOr | B::NotEqual => truth((l != 0) != (r != 0)),
            B::Equal => truth((l != 0) == (r != 0)),
            _ => None,
        },
        ScalarKind::Uint | ScalarKind::Sint => match op {
            B::Add => Some(l.wrapping_add(r)),
            B::Subtract => Some(l.wrapping_sub(r)),
            B::Multiply => Some(l.wrapping_mul(r)),
            B::Divide if kind == ScalarKind::Uint && r != 0 => Some(l / r),
            B::Modulo if kind == ScalarKind::Uint && r != 0 => Some(l % r),
            B::And => Some(l & r),
            B::InclusiveOr => Some(l | r),
            B::ExclusiveOr => Some(l ^ r),
            B::ShiftLeft if r < 32 => Some(l << r),
            B::ShiftRight if r < 32 => match kind {
                ScalarKind::Sint => Some((sl >> r) as u32),
                _ => Some(l >> r),
            },
            B::Equal => truth(l == r),
            B::NotEqual => truth(l != r),
            B::Less if kind == ScalarKind::Sint => truth(sl < sr),
            B::LessEqual if kind == ScalarKind::Sint => truth(sl <= sr),
            B::Greater if kind == ScalarKind::Sint => truth(sl > sr),
            B::GreaterEqual if kind == ScalarKind::Sint => truth(sl >= sr),
            B::Less => truth(l < r),
            B::LessEqual => truth(l <= r),
            B::Greater => truth(l > r),
            B::GreaterEqual => truth(l >= r),
            _ => None,
        },
        ScalarKind::Float if plain(l) && plain(r) => match op {
            // Plain floats order as the integers of their bits do.
            B::Equal => truth(l == r),
            B::NotEqual => truth(l != r),
            B::Less => truth(l < r),
            B::LessEqual => truth(l <= r),
            B::Greater => truth(l > r),
            B::GreaterEqual => truth(l >= r),
            B::Add | B::Subtract if r == 0 => Some(l),
            B::Add if l == 0 => Some(r),
            B::Multiply if l == 0 || r == 0 => Some(0),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn unary(op: U, kind: ScalarKind, v: u32) -> Option<u32> {
    match (op, kind) {
        (U::LogicalNot, ScalarKind::Bool) => Some(u32::from(v == 0)),
        (U::BitwiseNot, ScalarKind::Uint | ScalarKind::Sint) => Some(!v),
        (U::Negate, ScalarKind::Sint) => Some(v.wrapping_neg()),
        _ => None,
    }
}

/// A conversion of a lane from `from` to `to`: bits kept where `bitcast`, else nought to nought
/// and integers between the two 32-bit kinds.
pub(super) fn converted(from: ScalarKind, to: ScalarKind, bitcast: bool, v: u32) -> Option<u32> {
    use ScalarKind as K;
    match (from, to) {
        _ if bitcast => Some(v),
        (K::Uint | K::Sint, K::Uint | K::Sint) => Some(v),
        (K::Bool, K::Uint | K::Sint) => Some(u32::from(v != 0)),
        (K::Uint | K::Sint, K::Bool) => Some(u32::from(v != 0)),
        (K::Bool, K::Float) => Some(if v != 0 { 1f32.to_bits() } else { 0 }),
        _ if v == 0 => Some(0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_float_folds_only_where_a_flushing_and_a_quieting_device_agree() {
        let f = ScalarKind::Float;
        // -0.0, a denormal and a signalling NaN are not folded.
        assert_eq!(binary(B::Add, f, 0, 0x8000_0000), None);
        assert_eq!(binary(B::Equal, f, 1, 0), None);
        assert_eq!(binary(B::Add, f, 1, 0), None);
        assert_eq!(binary(B::Add, f, 0x7F80_0001, 0), None);
        assert_eq!(
            binary(B::Less, f, 1.0f32.to_bits(), 2.0f32.to_bits()),
            Some(1)
        );
        assert_eq!(
            binary(B::Add, f, 1.5f32.to_bits(), 0),
            Some(1.5f32.to_bits())
        );
        assert_eq!(binary(B::Multiply, f, 3.0f32.to_bits(), 0), Some(0));
        assert_eq!(binary(B::Add, f, 1.0f32.to_bits(), 1.0f32.to_bits()), None);
    }

    #[test]
    fn an_integer_folds_by_its_kind_and_not_where_a_device_leaves_it_undefined() {
        let (u, s) = (ScalarKind::Uint, ScalarKind::Sint);
        assert_eq!(binary(B::ShiftLeft, u, 1, 32), None);
        assert_eq!(binary(B::ShiftLeft, u, 1, 31), Some(0x8000_0000));
        assert_eq!(binary(B::Divide, u, 7, 0), None);
        assert_eq!(binary(B::Divide, s, 7, 2), None);
        assert_eq!(binary(B::Less, u, u32::MAX, 0), Some(0));
        assert_eq!(binary(B::Less, s, u32::MAX, 0), Some(1));
        assert_eq!(binary(B::ShiftRight, s, 0x8000_0000, 4), Some(0xF800_0000));
        assert_eq!(binary(B::Subtract, u, 0, 1), Some(u32::MAX));
        assert_eq!(
            converted(ScalarKind::Float, u, false, 1.0f32.to_bits()),
            None
        );
        assert_eq!(converted(ScalarKind::Float, u, false, 0), Some(0));
    }
}
