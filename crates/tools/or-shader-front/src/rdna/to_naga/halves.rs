//! 16-bit floats by integer arithmetic on their patterns: a device's own packing rounds as it
//! likes and may lose a denormal, and a driver may fold a pack and an unpack into nothing.

use naga::{BinaryOperator as Bo, Expression as E, Handle, MathFunction as Mf};

use super::Builder;

/// How a 32-bit float is rounded to a 16-bit one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Rounding {
    /// To the nearest, a tie to the even one, and past the greatest to an infinity.
    Nearest,
    /// To the one no further from zero: never past the greatest finite one.
    TowardZero,
    /// Toward zero where the condition holds and to the nearest where it does not: the
    /// rounding a program sets as it runs.
    By(Handle<E>),
}

impl Builder {
    /// A 32-bit float's pattern as a 16-bit float's in the low half of a dword, the high half
    /// zero. A denormal result is kept; a NaN keeps its sign and the top of its payload, quiet.
    pub(super) fn half_of(&mut self, bits: Handle<E>, rounding: Rounding) -> Handle<E> {
        let (one, sixteen) = (self.u32(1), self.u32(16));
        let top = self.bin(Bo::ShiftRight, bits, sixteen);
        let sign_bit = self.u32(0x8000);
        let sign = self.bin(Bo::And, top, sign_bit);
        let (by, field) = (self.u32(23), self.u32(0xFF));
        let exponent = self.bin(Bo::ShiftRight, bits, by);
        let exponent = self.bin(Bo::And, exponent, field);
        let fraction = self.u32(0x7F_FFFF);
        let mantissa = self.bin(Bo::And, bits, fraction);
        let implied = self.u32(0x80_0000);
        let whole = self.bin(Bo::InclusiveOr, mantissa, implied);
        // A normal half drops 13 bits of the 24; a denormal one more for each step below the
        // least normal exponent, and past 25 nothing is left to round up.
        let (least, most, first) = (self.u32(13), self.u32(25), self.u32(126));
        let capped = self.math(Mf::Min, &[exponent, first]);
        let below = self.bin(Bo::Subtract, first, capped);
        let shift = self.math(Mf::Clamp, &[below, least, most]);
        let (normal, ten) = (self.u32(113), self.u32(10));
        let raised = self.math(Mf::Max, &[exponent, normal]);
        let steps = self.bin(Bo::Subtract, raised, normal);
        let steps = self.bin(Bo::ShiftLeft, steps, ten);
        let kept = self.bin(Bo::ShiftRight, whole, shift);
        // The implied bit lands on the exponent's lowest and counts as its first step.
        let mut half = self.bin(Bo::Add, steps, kept);
        if rounding != Rounding::TowardZero {
            let mask = self.bin(Bo::ShiftLeft, one, shift);
            let mask = self.bin(Bo::Subtract, mask, one);
            let dropped = self.bin(Bo::And, whole, mask);
            let before = self.bin(Bo::Subtract, shift, one);
            let tie = self.bin(Bo::ShiftLeft, one, before);
            // Past the tie, or on it where the half kept is odd. A carry out of the mantissa
            // rolls into the exponent, up to infinity.
            let odd = self.bin(Bo::And, half, one);
            let weighed = self.bin(Bo::Add, dropped, odd);
            let mut up = self.bin(Bo::Greater, weighed, tie);
            if let Rounding::By(toward_zero) = rounding {
                let nearest = self.lnot(toward_zero);
                up = self.bin(Bo::LogicalAnd, up, nearest);
            }
            let next = self.bin(Bo::Add, half, one);
            half = self.select(up, next, half);
        }
        let (past, infinity) = (self.u32(143), self.u32(0x7C00));
        let over = match rounding {
            Rounding::Nearest => infinity,
            Rounding::TowardZero => self.u32(0x7BFF),
            Rounding::By(toward_zero) => {
                let greatest = self.u32(0x7BFF);
                self.select(toward_zero, greatest, infinity)
            }
        };
        let too_big = self.bin(Bo::GreaterEqual, exponent, past);
        half = self.select(too_big, over, half);
        let (thirteen, quiet, zero) = (self.u32(13), self.u32(0x7E00), self.u32(0));
        let payload = self.bin(Bo::ShiftRight, mantissa, thirteen);
        let nan = self.bin(Bo::InclusiveOr, payload, quiet);
        let endless = self.bin(Bo::Equal, mantissa, zero);
        let special = self.select(endless, infinity, nan);
        let not_finite = self.bin(Bo::Equal, exponent, field);
        half = self.select(not_finite, special, half);
        self.bin(Bo::InclusiveOr, sign, half)
    }

    /// The 16-bit float in the low half of a dword as a 32-bit float's pattern, which holds it
    /// exactly: a denormal half is a normal float, and a NaN keeps its payload, quiet.
    pub(super) fn float_of_half(&mut self, half: Handle<E>) -> Handle<E> {
        let (sign_bit, sixteen) = (self.u32(0x8000), self.u32(16));
        let sign = self.bin(Bo::And, half, sign_bit);
        let sign = self.bin(Bo::ShiftLeft, sign, sixteen);
        let (ten, field, fraction) = (self.u32(10), self.u32(0x1F), self.u32(0x3FF));
        let exponent = self.bin(Bo::ShiftRight, half, ten);
        let exponent = self.bin(Bo::And, exponent, field);
        let mantissa = self.bin(Bo::And, half, fraction);
        let (by, thirteen, bias) = (self.u32(23), self.u32(13), self.u32(112));
        let placed = self.bin(Bo::ShiftLeft, mantissa, thirteen);
        let raised = self.bin(Bo::Add, exponent, bias);
        let raised = self.bin(Bo::ShiftLeft, raised, by);
        let normal = self.bin(Bo::InclusiveOr, raised, placed);
        // A denormal half's first set bit becomes the float's implied one.
        let (spare, least) = (self.u32(21), self.u32(113));
        let leading = self.math(Mf::CountLeadingZeros, &[mantissa]);
        let up = self.bin(Bo::Subtract, leading, spare);
        let moved = self.shift(Bo::ShiftLeft, false, mantissa, up);
        let moved = self.bin(Bo::And, moved, fraction);
        let moved = self.bin(Bo::ShiftLeft, moved, thirteen);
        let lowered = self.bin(Bo::Subtract, least, up);
        let lowered = self.bin(Bo::ShiftLeft, lowered, by);
        let denormal = self.bin(Bo::InclusiveOr, lowered, moved);
        let zero = self.u32(0);
        let none = self.bin(Bo::Equal, mantissa, zero);
        let small = self.select(none, zero, denormal);
        let (endless, quiet) = (self.u32(0x7F80_0000), self.u32(0x7FC0_0000));
        let nan = self.bin(Bo::InclusiveOr, placed, quiet);
        let special = self.select(none, endless, nan);
        let is_small = self.bin(Bo::Equal, exponent, zero);
        let not_finite = self.bin(Bo::Equal, exponent, field);
        let value = self.select(not_finite, special, normal);
        let value = self.select(is_small, small, value);
        self.bin(Bo::InclusiveOr, sign, value)
    }
}

/// A 64-bit number as its high and low dwords.
type Wide = [Handle<E>; 2];

impl Builder {
    /// `wide` shifted left by `by`, which is under 64.
    fn wide_shl(&mut self, [high, low]: Wide, by: Handle<E>) -> Wide {
        let (zero, one, most, word) = (self.u32(0), self.u32(1), self.u32(31), self.u32(32));
        let within = self.bin(Bo::And, by, most);
        let left = self.bin(Bo::Subtract, most, within);
        let carried = self.bin(Bo::ShiftRight, low, one);
        let carried = self.bin(Bo::ShiftRight, carried, left);
        let raised = self.bin(Bo::ShiftLeft, high, within);
        let near = self.bin(Bo::InclusiveOr, raised, carried);
        let moved = self.bin(Bo::ShiftLeft, low, within);
        let far = self.bin(Bo::GreaterEqual, by, word);
        [self.select(far, moved, near), self.select(far, zero, moved)]
    }

    /// `wide` shifted right by any `by`, and the bits that fell off its end, as a dword that
    /// is zero where none was set.
    fn wide_shr(&mut self, [high, low]: Wide, by: Handle<E>) -> (Wide, Handle<E>) {
        let (zero, one, most) = (self.u32(0), self.u32(1), self.u32(31));
        let (word, both) = (self.u32(32), self.u32(64));
        let within = self.bin(Bo::And, by, most);
        let left = self.bin(Bo::Subtract, most, within);
        let mask = self.bin(Bo::ShiftLeft, one, within);
        let mask = self.bin(Bo::Subtract, mask, one);
        let carried = self.bin(Bo::ShiftLeft, high, one);
        let carried = self.bin(Bo::ShiftLeft, carried, left);
        let near_low = self.bin(Bo::ShiftRight, low, within);
        let near_low = self.bin(Bo::InclusiveOr, near_low, carried);
        let near_high = self.bin(Bo::ShiftRight, high, within);
        let near_lost = self.bin(Bo::And, low, mask);
        let far_lost = self.bin(Bo::And, high, mask);
        let far_lost = self.bin(Bo::InclusiveOr, far_lost, low);
        let all = self.bin(Bo::InclusiveOr, high, low);
        let far = self.bin(Bo::GreaterEqual, by, word);
        let gone = self.bin(Bo::GreaterEqual, by, both);
        let top = self.select(far, zero, near_high);
        let bottom = self.select(far, near_high, near_low);
        let bottom = self.select(gone, zero, bottom);
        let lost = self.select(far, far_lost, near_lost);
        let lost = self.select(gone, all, lost);
        ([top, bottom], lost)
    }

    fn wide_add(&mut self, [ah, al]: Wide, [bh, bl]: Wide) -> Wide {
        let low = self.bin(Bo::Add, al, bl);
        let carry = self.bin(Bo::Less, low, al);
        let carry = self.flag(carry);
        let high = self.bin(Bo::Add, ah, bh);
        [self.bin(Bo::Add, high, carry), low]
    }

    fn wide_sub(&mut self, [ah, al]: Wide, [bh, bl]: Wide) -> Wide {
        let low = self.bin(Bo::Subtract, al, bl);
        let borrow = self.bin(Bo::Less, al, bl);
        let borrow = self.flag(borrow);
        let high = self.bin(Bo::Subtract, ah, bh);
        [self.bin(Bo::Subtract, high, borrow), low]
    }

    /// The zero bits above the first set one, 64 for zero.
    fn wide_clz(&mut self, [high, low]: Wide) -> Handle<E> {
        let (zero, word) = (self.u32(0), self.u32(32));
        let upper = self.math(Mf::CountLeadingZeros, &[high]);
        let lower = self.math(Mf::CountLeadingZeros, &[low]);
        let lower = self.bin(Bo::Add, lower, word);
        let empty = self.bin(Bo::Equal, high, zero);
        self.select(empty, lower, upper)
    }

    /// A finite float's pattern as its sign, its exponent field, a denormal's read as the
    /// least normal one, and the 24 bits of its magnitude.
    fn float_parts(&mut self, bits: Handle<E>) -> [Handle<E>; 3] {
        let (zero, one, top, by) = (self.u32(0), self.u32(1), self.u32(31), self.u32(23));
        let (field, fraction, implied) = (self.u32(0xFF), self.u32(0x7F_FFFF), self.u32(0x80_0000));
        let sign = self.bin(Bo::ShiftRight, bits, top);
        let exponent = self.bin(Bo::ShiftRight, bits, by);
        let exponent = self.bin(Bo::And, exponent, field);
        let mantissa = self.bin(Bo::And, bits, fraction);
        let normal = self.bin(Bo::NotEqual, exponent, zero);
        let whole = self.bin(Bo::InclusiveOr, mantissa, implied);
        [
            sign,
            self.math(Mf::Max, &[exponent, one]),
            self.select(normal, whole, mantissa),
        ]
    }

    /// `a * b + c` of three finite float patterns, `a` and `b` not zero, computed whole and
    /// rounded once, to the float no further from zero with its last bit set where anything
    /// was dropped: any rounding of that to a 16-bit float is the rounding of the whole
    /// sum, since it keeps 24 bits where a half keeps eleven. A sum past a float's range is
    /// the greatest float, and one under it the least normal one, which round as the sum.
    pub(super) fn fused_odd(&mut self, a: Handle<E>, b: Handle<E>, c: Handle<E>) -> Handle<E> {
        let (zero, one) = (self.u32(0), self.u32(1));
        let [sa, ea, ma] = self.float_parts(a);
        let [sb, eb, mb] = self.float_parts(b);
        let [sc, ec, mc] = self.float_parts(c);
        // The 48 bits of the product, by 12-bit limbs.
        let (limb, twelve, twenty) = (self.u32(0xFFF), self.u32(12), self.u32(20));
        let (eight, top) = (self.u32(8), self.u32(24));
        let (a0, a1) = (
            self.bin(Bo::And, ma, limb),
            self.bin(Bo::ShiftRight, ma, twelve),
        );
        let (b0, b1) = (
            self.bin(Bo::And, mb, limb),
            self.bin(Bo::ShiftRight, mb, twelve),
        );
        let ll = self.bin(Bo::Multiply, a0, b0);
        let (lh, hl) = (
            self.bin(Bo::Multiply, a0, b1),
            self.bin(Bo::Multiply, a1, b0),
        );
        let middle = self.bin(Bo::Add, lh, hl);
        let hh = self.bin(Bo::Multiply, a1, b1);
        let raised = self.bin(Bo::ShiftLeft, middle, twelve);
        let first = self.bin(Bo::Add, ll, raised);
        let carry = self.bin(Bo::Less, first, ll);
        let carry = self.flag(carry);
        let raised = self.bin(Bo::ShiftLeft, hh, top);
        let low = self.bin(Bo::Add, first, raised);
        let again = self.bin(Bo::Less, low, first);
        let again = self.flag(again);
        let high = self.bin(Bo::ShiftRight, hh, eight);
        let over = self.bin(Bo::ShiftRight, middle, twenty);
        let high = self.bin(Bo::Add, high, over);
        let high = self.bin(Bo::Add, high, carry);
        let high = self.bin(Bo::Add, high, again);
        // Both terms with their first set bit at bit 62, each with the power of two its
        // lowest bit is worth, counted from a common floor so that none is negative.
        let (most, floor) = (self.u32(63), self.u32(65));
        let zeros = self.wide_clz([high, low]);
        let by = self.bin(Bo::Subtract, zeros, one);
        let product = self.wide_shl([high, low], by);
        let power = self.bin(Bo::Add, ea, eb);
        let power = self.bin(Bo::Add, power, floor);
        let product_power = self.bin(Bo::Subtract, power, zeros);
        let zeros = self.wide_clz([zero, mc]);
        let by = self.bin(Bo::Subtract, zeros, one);
        let by = self.bin(Bo::And, by, most);
        let addend = self.wide_shl([zero, mc], by);
        let lift = self.u32(215);
        let power = self.bin(Bo::Add, ec, lift);
        let power = self.bin(Bo::Subtract, power, zeros);
        let none = self.bin(Bo::Equal, mc, zero);
        let addend_power = self.select(none, zero, power);
        // The greater in magnitude first, the lesser moved down to its scale.
        let above = self.bin(Bo::Greater, product_power, addend_power);
        let level = self.bin(Bo::Equal, product_power, addend_power);
        let high_less = self.bin(Bo::Less, product[0], addend[0]);
        let high_same = self.bin(Bo::Equal, product[0], addend[0]);
        let low_less = self.bin(Bo::Less, product[1], addend[1]);
        let less = self.bin(Bo::LogicalAnd, high_same, low_less);
        let less = self.bin(Bo::LogicalOr, high_less, less);
        let not_less = self.lnot(less);
        let tied = self.bin(Bo::LogicalAnd, level, not_less);
        let leads = self.bin(Bo::LogicalOr, above, tied);
        let greater = [0, 1].map(|k| self.select(leads, product[k], addend[k]));
        let lesser = [0, 1].map(|k| self.select(leads, addend[k], product[k]));
        let power = self.select(leads, product_power, addend_power);
        let lesser_power = self.select(leads, addend_power, product_power);
        let product_sign = self.bin(Bo::ExclusiveOr, sa, sb);
        let sign = self.select(leads, product_sign, sc);
        let apart = self.bin(Bo::Subtract, power, lesser_power);
        let (lesser, lost) = self.wide_shr(lesser, apart);
        let dropped = self.ne0(lost);
        let sum = self.wide_add(greater, lesser);
        // What fell off the lesser term is taken from the difference as a whole step, and
        // what is left of that step is dropped below.
        let step = self.flag(dropped);
        let difference = self.wide_sub(greater, lesser);
        let difference = self.wide_sub(difference, [zero, step]);
        let alike = self.bin(Bo::Equal, product_sign, sc);
        let whole = [0, 1].map(|k| self.select(alike, sum[k], difference[k]));
        let zeros = self.wide_clz(whole);
        let by = self.bin(Bo::And, zeros, most);
        let [top_word, bottom_word] = self.wide_shl(whole, by);
        let (byte, fraction) = (self.u32(0xFF), self.u32(0x7F_FFFF));
        let kept = self.bin(Bo::ShiftRight, top_word, eight);
        let rest = self.bin(Bo::And, top_word, byte);
        let rest = self.bin(Bo::InclusiveOr, rest, bottom_word);
        let rest = self.bin(Bo::InclusiveOr, rest, step);
        let odd = self.ne0(rest);
        let odd = self.flag(odd);
        let kept = self.bin(Bo::InclusiveOr, kept, odd);
        // The exponent field is this less 254.
        let (lift, bias, by) = (self.u32(80), self.u32(254), self.u32(23));
        let raised = self.bin(Bo::Add, power, lift);
        let raised = self.bin(Bo::Subtract, raised, zeros);
        let field = self.bin(Bo::Subtract, raised, bias);
        let field = self.bin(Bo::And, field, byte);
        let field = self.bin(Bo::ShiftLeft, field, by);
        let mantissa = self.bin(Bo::And, kept, fraction);
        let mut value = self.bin(Bo::InclusiveOr, field, mantissa);
        let (greatest, least, past) = (self.u32(0x7F7F_FFFF), self.u32(0x0080_0000), self.u32(509));
        let too_big = self.bin(Bo::GreaterEqual, raised, past);
        value = self.select(too_big, greatest, value);
        let too_small = self.bin(Bo::LessEqual, raised, bias);
        value = self.select(too_small, least, value);
        let thirty_one = self.u32(31);
        let sign = self.bin(Bo::ShiftLeft, sign, thirty_one);
        let value = self.bin(Bo::InclusiveOr, sign, value);
        let nothing = self.bin(Bo::InclusiveOr, whole[0], whole[1]);
        let nothing = self.bin(Bo::Equal, nothing, zero);
        self.select(nothing, zero, value)
    }

    /// `a * b + c` of three float patterns as a float pattern that any rounding to a 16-bit
    /// float rounds as it would the whole sum: [`Builder::fused_odd`], or the device's own
    /// where a term has no end or is no number, or a factor is zero and the sum is `c`.
    pub(super) fn fused(&mut self, a: Handle<E>, b: Handle<E>, c: Handle<E>) -> Handle<E> {
        let (field, magnitude, zero) = (self.u32(0x7F80_0000), self.u32(0x7FFF_FFFF), self.u32(0));
        let mut plain = None;
        for term in [a, b, c] {
            let exponent = self.bin(Bo::And, term, field);
            let endless = self.bin(Bo::Equal, exponent, field);
            plain = Some(match plain {
                Some(so_far) => self.bin(Bo::LogicalOr, so_far, endless),
                None => endless,
            });
        }
        let mut plain = plain.unwrap_or(zero);
        for factor in [a, b] {
            let size = self.bin(Bo::And, factor, magnitude);
            let nothing = self.bin(Bo::Equal, size, zero);
            plain = self.bin(Bo::LogicalOr, plain, nothing);
        }
        let whole = self.fused_odd(a, b, c);
        let (x, y, z) = (self.float(a), self.float(b), self.float(c));
        let device = self.math(Mf::Fma, &[x, y, z]);
        let device = self.bits(device);
        self.select(plain, device, whole)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use naga::{Literal, UnaryOperator as Uo};

    /// What an expression over dwords and conditions comes to, with the function's one argument `x`.
    fn eval(b: &Builder, at: Handle<E>, x: u32) -> u32 {
        eval_of(b, at, &[x], &mut Vec::new())
    }

    /// [`eval`] with several arguments; `known` holds what each expression came to, since a
    /// value used twice is one expression.
    fn eval_of(b: &Builder, at: Handle<E>, given: &[u32], known: &mut Vec<Option<u32>>) -> u32 {
        known.resize(b.fun.expressions.len(), None);
        if let Some(value) = known[at.index()] {
            return value;
        }
        let value = eval_one(b, at, given, known);
        known[at.index()] = Some(value);
        value
    }

    fn eval_one(b: &Builder, at: Handle<E>, given: &[u32], known: &mut Vec<Option<u32>>) -> u32 {
        let mut dword = |h| eval_of(b, h, given, known);
        match b.fun.expressions[at] {
            E::FunctionArgument(index) => given[index as usize],
            E::Literal(Literal::U32(word)) => word,
            E::Literal(Literal::Bool(flag)) => u32::from(flag),
            E::Binary { op, left, right } => {
                let (l, r) = (dword(left), dword(right));
                match op {
                    Bo::Add => l.wrapping_add(r),
                    Bo::Subtract => l.wrapping_sub(r),
                    Bo::Multiply => l.wrapping_mul(r),
                    Bo::And => l & r,
                    Bo::InclusiveOr => l | r,
                    Bo::ExclusiveOr => l ^ r,
                    Bo::NotEqual => u32::from(l != r),
                    Bo::Less => u32::from(l < r),
                    Bo::LessEqual => u32::from(l <= r),
                    Bo::LogicalOr => l | r,
                    Bo::ShiftLeft => l.checked_shl(r).expect("a shift a dword has no bit for"),
                    Bo::ShiftRight => l.checked_shr(r).expect("a shift a dword has no bit for"),
                    Bo::Equal => u32::from(l == r),
                    Bo::Greater => u32::from(l > r),
                    Bo::GreaterEqual => u32::from(l >= r),
                    Bo::LogicalAnd => l & r,
                    other => panic!("{other:?} is not evaluated"),
                }
            }
            E::Unary {
                op: Uo::BitwiseNot,
                expr,
            } => !dword(expr),
            E::Unary {
                op: Uo::LogicalNot,
                expr,
            } => dword(expr) ^ 1,
            E::Select {
                condition,
                accept,
                reject,
            } => {
                // Both sides are computed whichever is taken, as a device may.
                let (accept, reject) = (dword(accept), dword(reject));
                match dword(condition) {
                    0 => reject,
                    _ => accept,
                }
            }
            E::Math {
                fun,
                arg,
                arg1,
                arg2,
                ..
            } => match (fun, arg1, arg2) {
                (Mf::Min, Some(b), _) => {
                    let (a, b) = (dword(arg), dword(b));
                    a.min(b)
                }
                (Mf::Max, Some(b), _) => {
                    let (a, b) = (dword(arg), dword(b));
                    a.max(b)
                }
                (Mf::Clamp, Some(low), Some(high)) => {
                    let (x, low, high) = (dword(arg), dword(low), dword(high));
                    x.clamp(low, high)
                }
                (Mf::CountLeadingZeros, ..) => dword(arg).leading_zeros(),
                other => panic!("{other:?} is not evaluated"),
            },
            ref other => panic!("{other:?} is not evaluated"),
        }
    }

    /// What a 16-bit float's pattern is worth, by the format's definition; `None` for a NaN.
    fn worth(half: u16) -> Option<f64> {
        let (exponent, mantissa) = (i32::from(half >> 10 & 0x1F), f64::from(half & 0x3FF));
        let magnitude = match exponent {
            0 => mantissa * 2f64.powi(-24),
            31 if mantissa == 0.0 => f64::INFINITY,
            31 => return None,
            _ => (1024.0 + mantissa) * 2f64.powi(exponent - 25),
        };
        Some(if half & 0x8000 != 0 {
            -magnitude
        } else {
            magnitude
        })
    }

    /// The 16-bit float a 32-bit one rounds to, found among every 16-bit float there is.
    fn reference(bits: u32, rounding: Rounding) -> u16 {
        let sign = (bits >> 16 & 0x8000) as u16;
        let x = f64::from(f32::from_bits(bits & 0x7FFF_FFFF));
        if x.is_nan() {
            return sign | 0x7E00 | (bits >> 13 & 0x3FF) as u16;
        }
        // The greatest half that is no more than the magnitude: patterns order as their worths do.
        let (mut below, mut above) = (0u16, 0x7C01u16);
        while above - below > 1 {
            let middle = below + (above - below) / 2;
            match worth(middle).is_some_and(|w| w <= x) {
                true => below = middle,
                false => above = middle,
            }
        }
        let (low, high) = (worth(below).unwrap_or(0.0), worth(below + 1));
        let magnitude = match (rounding, high) {
            _ if low == x => below,
            (Rounding::TowardZero, _) => below,
            (Rounding::By(_), _) => panic!("a rounding the definition does not name"),
            // Past the greatest finite half the next step would be 65536, as an exponent with no end has it.
            (Rounding::Nearest, high) => {
                let high = high.filter(|h| h.is_finite()).unwrap_or(65536.0);
                let (down, up) = (x - low, high - x);
                match down < up || (down == up && below & 1 == 0) {
                    true => below,
                    false => below + 1,
                }
            }
        };
        sign | magnitude
    }

    fn patterns() -> Vec<u32> {
        let mut all = Vec::new();
        // Every half as a float, with the floats beside it and the ties between halves.
        for half in (0..=0x7C00u16)
            .step_by(7)
            .chain([1, 2, 0x3FF, 0x400, 0x7BFF, 0x7C00])
        {
            let Some(w) = worth(half) else { continue };
            let bits = (w as f32).to_bits();
            let next = worth(half + 1).filter(|n| n.is_finite());
            let tie = next.map_or(bits, |n| (((w + n) / 2.0) as f32).to_bits());
            for near in [bits, tie] {
                all.extend([near.wrapping_sub(1), near, near + 1]);
            }
        }
        // The least float, floats under the least half and under half of it, the edge of
        // infinity, the greatest float, infinities and NaNs of every payload end.
        all.extend([
            0,
            1,
            0x007F_FFFF,
            0x0080_0000,
            0x3280_0000,
            0x3300_0000,
            0x3300_0001,
            0x3380_0000,
            0x477F_E000,
            0x477F_EFFF,
            0x477F_F000,
            0x477F_F001,
            0x4780_0000,
            0x7F7F_FFFF,
            0x7F80_0000,
            0x7F80_0001,
            0x7F80_2000,
            0x7FC0_0000,
            0x7FFF_FFFF,
            0x7FA5_A000,
        ]);
        let mut seed = 0x9E37_79B9u32;
        for _ in 0..20_000 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            all.push(seed);
        }
        let negative: Vec<u32> = all.iter().map(|bits| bits | 0x8000_0000).collect();
        all.extend(negative);
        all
    }

    #[test]
    fn a_float_narrows_to_the_half_the_format_defines_in_either_rounding() {
        for rounding in [Rounding::Nearest, Rounding::TowardZero] {
            let mut b = Builder::new();
            let x = b.emit(E::FunctionArgument(0));
            let half = b.half_of(x, rounding);
            for bits in patterns() {
                let (got, want) = (eval(&b, half, bits), u32::from(reference(bits, rounding)));
                assert_eq!(got, want, "{bits:#010x} {rounding:?}");
            }
        }
    }

    #[test]
    fn a_rounding_set_as_the_program_runs_is_the_one_its_condition_names() {
        for (toward_zero, named) in [(true, Rounding::TowardZero), (false, Rounding::Nearest)] {
            let mut b = Builder::new();
            let x = b.emit(E::FunctionArgument(0));
            let flag = b.emit(E::Literal(Literal::Bool(toward_zero)));
            let half = b.half_of(x, Rounding::By(flag));
            for bits in patterns().into_iter().step_by(5) {
                let (got, want) = (eval(&b, half, bits), u32::from(reference(bits, named)));
                assert_eq!(got, want, "{bits:#010x} {named:?}");
            }
        }
    }

    /// The 16-bit float `a * b + c` rounds to, the sum taken whole: the product of two
    /// floats is exact in 53 bits, the sum's error is another double, and a sum rounded to
    /// the double with its last bit set wherever it is inexact rounds on as the whole one.
    fn fused_reference(a: f32, b: f32, c: f32, rounding: Rounding) -> u16 {
        let (p, c) = (f64::from(a) * f64::from(b), f64::from(c));
        let sum = p + c;
        let rest = sum - p;
        let error = (p - (sum - rest)) + (c - rest);
        let mut bits = sum.to_bits();
        if error != 0.0 && bits & 1 == 0 {
            // The neighbour on the error's side: further from zero where their signs agree.
            match (error > 0.0) == (sum > 0.0) {
                true => bits += 1,
                false => bits -= 1,
            }
        }
        let whole = f64::from_bits(bits);
        let sign = if whole.is_sign_negative() && whole != 0.0 {
            0x8000
        } else {
            0
        };
        let x = whole.abs();
        let (mut below, mut above) = (0u16, 0x7C01u16);
        while above - below > 1 {
            let middle = below + (above - below) / 2;
            match worth(middle).is_some_and(|w| w <= x) {
                true => below = middle,
                false => above = middle,
            }
        }
        let low = worth(below).unwrap_or(0.0);
        let high = worth(below + 1)
            .filter(|h| h.is_finite())
            .unwrap_or(65536.0);
        let (down, up) = (x - low, high - x);
        let nearer = down < up || (down == up && below & 1 == 0);
        let up_one = rounding == Rounding::Nearest && !nearer;
        sign | (below + u16::from(low != x && up_one))
    }

    #[test]
    fn a_fused_multiply_add_of_halves_is_the_whole_sum_rounded_once() {
        for rounding in [Rounding::Nearest, Rounding::TowardZero] {
            let mut b = Builder::new();
            let [x, y, z] = [0, 1, 2].map(|index| b.emit(E::FunctionArgument(index)));
            let whole = b.fused_odd(x, y, z);
            let half = b.half_of(whole, rounding);
            // What the lowering was: the device's 32-bit sum, rounded a second time.
            let twice = |a: f32, b: f32, c: f32| reference(a.mul_add(b, c).to_bits(), rounding);
            let mut seed = 0x2545_F491u32;
            let mut next = || {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                seed >> 8
            };
            let float = |half: u32| {
                let sign = if half & 0x8000 != 0 { -1.0 } else { 1.0 };
                (sign * worth((half & 0x7FFF) as u16).unwrap_or(0.0)) as f32
            };
            let mut known = Vec::new();
            for round in 0..20_000 {
                // Finite halves, the factors not zero; every fourth round the addend is near
                // the product's opposite, and every fifth the sources are whole floats.
                let pick = |word: u32| float((word % 0x7BFF + 1) | (word & 0x8000));
                let (mut a, mut b_, mut c) = (pick(next()), pick(next()), float(next() & 0xFBFF));
                if !c.is_finite() || round % 7 == 3 {
                    c = 0.0;
                }
                if round % 4 == 1 {
                    let near = reference((-a * b_).to_bits(), Rounding::TowardZero);
                    let near = float(u32::from(near)) * (1.0 + (next() % 5) as f32 / 1024.0);
                    c = if near.is_finite() { near } else { c };
                }
                if round % 5 == 2 {
                    let wide = |word: u32| f32::from_bits(word % 0x7F00_0000 + 0x0000_0100);
                    (a, b_, c) = (wide(next() << 8 | 1), wide(next() << 7), wide(next() << 6));
                }
                let given = [a.to_bits(), b_.to_bits(), c.to_bits()];
                known.clear();
                let got = eval_of(&b, half, &given, &mut known);
                let want = fused_reference(a, b_, c, rounding);
                assert_eq!(got, u32::from(want), "{a:e} * {b_:e} + {c:e}, {rounding:?}");
            }
            // What this is for, where rounding twice is not rounding once. 7/4 * 293/512 is
            // the middle of two halves and the least half under it is half a float's step:
            // the float rounds back to the middle, and the middle to the even half above. And
            // 4 less the least half is 4 as a float, which toward zero stays 4.
            let least = f32::from_bits(0x3380_0000);
            let (a, b_, c, want) = match rounding {
                Rounding::Nearest => (1.75f32, 293.0f32 / 512.0, -least, 0x3C01),
                _ => (2.0, 2.0, -least, 0x43FF),
            };
            known.clear();
            let given = [a.to_bits(), b_.to_bits(), c.to_bits()];
            assert_eq!(eval_of(&b, half, &given, &mut known), want, "{rounding:?}");
            assert_eq!(u32::from(fused_reference(a, b_, c, rounding)), want);
            assert_ne!(u32::from(twice(a, b_, c)), want, "{rounding:?}");
        }
    }

    #[test]
    fn the_two_roundings_differ_where_the_definition_says() {
        let mut b = Builder::new();
        let x = b.emit(E::FunctionArgument(0));
        let (nearest, zero) = (
            b.half_of(x, Rounding::Nearest),
            b.half_of(x, Rounding::TowardZero),
        );
        // 65520 is the tie between the greatest half and the step past it; 1 + 3/2048 a tie
        // between two halves whose lower one is odd.
        for (float, n, z) in [
            (65520.0f32, 0x7C00, 0x7BFF),
            (f32::MAX, 0x7C00, 0x7BFF),
            (1.0 + 3.0 / 2048.0, 0x3C02, 0x3C01),
            (-(1.0 + 3.0 / 2048.0), 0xBC02, 0xBC01),
            (2f32.powi(-25), 0, 0),
            (2f32.powi(-25) * 1.5, 1, 0),
        ] {
            assert_eq!(eval(&b, nearest, float.to_bits()), n, "{float}");
            assert_eq!(eval(&b, zero, float.to_bits()), z, "{float}");
        }
    }

    #[test]
    fn every_half_widens_to_the_float_of_its_worth_and_back() {
        let mut b = Builder::new();
        let x = b.emit(E::FunctionArgument(0));
        let wide = b.float_of_half(x);
        let back = b.half_of(wide, Rounding::TowardZero);
        for half in 0..=u16::MAX {
            // The high half of the dword is another value's: it changes nothing.
            let dword = u32::from(half) | 0xABCD_0000;
            let got = eval(&b, wide, dword);
            match worth(half) {
                Some(w) => assert_eq!(got, (w as f32).to_bits(), "{half:#06x}"),
                None => {
                    let payload = u32::from(half & 0x3FF) << 13;
                    let sign = u32::from(half & 0x8000) << 16;
                    assert_eq!(got, sign | 0x7FC0_0000 | payload, "{half:#06x}");
                }
            }
            let quiet = if worth(half).is_none() { 0x200 } else { 0 };
            assert_eq!(
                eval(&b, back, dword),
                u32::from(half | quiet),
                "{half:#06x}"
            );
        }
    }
}
