//! The least and the greatest of two floats where an operand is NaN: the hardware gives the
//! other operand, and of two NaNs the second, where naga's `min` and `max` leave it open.
//!
//! ⚠ That is the rule with the IEEE bit of the mode register clear. With it set, a signalling
//! NaN is quieted and returned instead. No program of the build writes that bit and its
//! package states none, so the bit is the launch's; the rule is exact for a quiet NaN either way.

use naga::{
    BinaryOperator as Bo, Expression as E, Handle, Literal, MathFunction as Mf, ScalarKind,
};

use super::Builder;

/// How many expressions deep an operand is followed to show it is no NaN.
const LOOK: u8 = 6;

impl Builder {
    /// `v_min_f32` (`fun` `Min`) or `v_max_f32` (`Max`) of two patterns, the NaN rule tested
    /// only for an operand that may be one.
    pub(super) fn fminmax(&mut self, fun: Mf, a: Handle<E>, b: Handle<E>) -> Handle<E> {
        let (fa, fb) = (self.float(a), self.float(b));
        let mut value = self.math(fun, &[fa, fb]);
        if self.may_be_nan(b, LOOK) {
            let nan = self.nan_pattern(b);
            value = self.select(nan, fa, value);
        }
        // Tested last, so that of two NaNs the second is given.
        if self.may_be_nan(a, LOOK) {
            let nan = self.nan_pattern(a);
            value = self.select(nan, fb, value);
        }
        self.bits(value)
    }

    /// Whether a pattern is a NaN's: an exponent of all ones and a mantissa that is not nought.
    fn nan_pattern(&mut self, bits: Handle<E>) -> Handle<E> {
        let (magnitude, infinity) = (self.u32(0x7FFF_FFFF), self.u32(0x7F80_0000));
        let held = self.bin(Bo::And, bits, magnitude);
        self.bin(Bo::Greater, held, infinity)
    }

    /// Whether the pattern `bits` may be a NaN's, as far as `look` expressions show.
    fn may_be_nan(&self, bits: Handle<E>, look: u8) -> bool {
        match self.fun.expressions[bits] {
            E::Literal(Literal::U32(word)) => word & 0x7FFF_FFFF > 0x7F80_0000,
            E::Literal(Literal::I32(word)) => word as u32 & 0x7FFF_FFFF > 0x7F80_0000,
            E::As {
                expr,
                kind: ScalarKind::Uint,
                convert: None,
            } if look > 0 => self.float_may_be_nan(expr, look - 1),
            _ => true,
        }
    }

    /// Whether the float `value` may be a NaN, as far as `look` expressions show.
    fn float_may_be_nan(&self, value: Handle<E>, look: u8) -> bool {
        match self.fun.expressions[value] {
            E::Literal(Literal::F32(float)) => float.is_nan(),
            // ⚠ Every conversion to a float this lowering writes is of a 32-bit integer.
            E::As {
                kind: ScalarKind::Float,
                convert: Some(_),
                ..
            } => false,
            E::As {
                expr,
                kind: ScalarKind::Float,
                convert: None,
            } if look > 0 => self.may_be_nan(expr, look - 1),
            // naga's own leaves a NaN operand's result open.
            E::Math {
                fun: Mf::Min | Mf::Max,
                arg,
                arg1: Some(other),
                ..
            } if look > 0 => {
                self.float_may_be_nan(arg, look - 1) || self.float_may_be_nan(other, look - 1)
            }
            // This rule's: a NaN only where both operands are.
            E::Select { .. } if look > 0 && self.guarded(value).is_some() => {
                let (arg, other) = self.guarded(value).unwrap_or((value, value));
                self.float_may_be_nan(arg, look - 1) && self.float_may_be_nan(other, look - 1)
            }
            E::Select { accept, reject, .. } if look > 0 => {
                self.float_may_be_nan(accept, look - 1) || self.float_may_be_nan(reject, look - 1)
            }
            _ => true,
        }
    }

    /// The two operands of a least or a greatest [`Builder::fminmax`] wrote: selects on a
    /// NaN test, one or two, around naga's own.
    fn guarded(&self, mut value: Handle<E>) -> Option<(Handle<E>, Handle<E>)> {
        loop {
            match self.fun.expressions[value] {
                E::Select {
                    condition, reject, ..
                } if self.tests_nan(condition) => value = reject,
                E::Math {
                    fun: Mf::Min | Mf::Max,
                    arg,
                    arg1: Some(other),
                    ..
                } => return Some((arg, other)),
                _ => return None,
            }
        }
    }

    /// Whether `condition` is [`Builder::is_nan`]'s test.
    fn tests_nan(&self, condition: Handle<E>) -> bool {
        let literal = |at: Handle<E>, word: u32| matches!(self.fun.expressions[at], E::Literal(Literal::U32(held)) if held == word);
        let E::Binary {
            op: Bo::Greater,
            left,
            right,
        } = self.fun.expressions[condition]
        else {
            return false;
        };
        let masked = matches!(
            self.fun.expressions[left],
            E::Binary { op: Bo::And, right: mask, .. } if literal(mask, 0x7FFF_FFFF)
        );
        masked && literal(right, 0x7F80_0000)
    }
}
