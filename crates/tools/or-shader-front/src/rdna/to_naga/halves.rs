//! 16-bit floats by integer arithmetic on their patterns: a device's own packing rounds as it
//! likes and may lose a denormal, and a driver may fold a pack and an unpack into nothing.
//! 64-bit floats likewise, as dword pairs: not every writer has the type.

use naga::{BinaryOperator as Bo, Expression as E, Handle, MathFunction as Mf, ScalarKind};

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

impl Builder {
    /// A 64-bit float's pattern, low dword and high, as the 32-bit float nearest to it, a tie
    /// to the even one: what `v_cvt_f32_f64` writes where the mode rounds to the nearest. A
    /// NaN keeps its sign and the top of its payload, quiet.
    pub(super) fn single_of_double(&mut self, low: Handle<E>, high: Handle<E>) -> Handle<E> {
        let (zero, one) = (self.u32(0), self.u32(1));
        let (sign_bit, top_bits, endless) = (
            self.u32(0x8000_0000),
            self.u32(0x000F_FFFF),
            self.u32(0x7F80_0000),
        );
        let sign = self.bin(Bo::And, high, sign_bit);
        let (twenty, field) = (self.u32(20), self.u32(0x7FF));
        let exponent = self.bin(Bo::ShiftRight, high, twenty);
        let exponent = self.bin(Bo::And, exponent, field);
        // The 23 bits a float keeps of the 52, and the 29 it drops.
        let (three, kept_by, dropped, tie) = (
            self.u32(3),
            self.u32(29),
            self.u32(0x1FFF_FFFF),
            self.u32(0x1000_0000),
        );
        let upper = self.bin(Bo::And, high, top_bits);
        let upper = self.bin(Bo::ShiftLeft, upper, three);
        let lower = self.bin(Bo::ShiftRight, low, kept_by);
        let kept = self.bin(Bo::InclusiveOr, upper, lower);
        let rest = self.bin(Bo::And, low, dropped);
        let odd = self.bin(Bo::And, kept, one);
        let above = self.bin(Bo::Greater, rest, tie);
        let above = self.flag(above);
        let even = self.bin(Bo::Equal, rest, tie);
        let even = self.flag(even);
        let even = self.bin(Bo::And, even, odd);
        let up = self.bin(Bo::InclusiveOr, above, even);
        // A float's exponent field is the double's less 896; a sum that carries out of the
        // 23 bits raises it, to an infinity from the greatest.
        let (least, most, bias, by) = (self.u32(897), self.u32(1150), self.u32(896), self.u32(23));
        let within = self.math(Mf::Clamp, &[exponent, least, most]);
        let within = self.bin(Bo::Subtract, within, bias);
        let within = self.bin(Bo::ShiftLeft, within, by);
        let normal = self.bin(Bo::InclusiveOr, within, kept);
        let normal = self.bin(Bo::Add, normal, up);
        // Under the least normal float: the 24 bits shifted down, what falls off rounded
        // with the 29 dropped bits behind it.
        let (implied, lowest) = (self.u32(0x80_0000), self.u32(873));
        let whole = self.bin(Bo::InclusiveOr, kept, implied);
        let held = self.math(Mf::Clamp, &[exponent, lowest, bias]);
        let down = self.bin(Bo::Subtract, least, held);
        let quotient = self.bin(Bo::ShiftRight, whole, down);
        let before = self.bin(Bo::Subtract, down, one);
        let middle = self.bin(Bo::ShiftLeft, one, before);
        let mask = self.bin(Bo::ShiftLeft, middle, one);
        let mask = self.bin(Bo::Subtract, mask, one);
        let fell = self.bin(Bo::And, whole, mask);
        let more = self.bin(Bo::Greater, fell, middle);
        let more = self.flag(more);
        let half = self.bin(Bo::Equal, fell, middle);
        let half = self.flag(half);
        let behind = self.bin(Bo::NotEqual, rest, zero);
        let behind = self.flag(behind);
        let last = self.bin(Bo::And, quotient, one);
        let breaks = self.bin(Bo::InclusiveOr, behind, last);
        let half = self.bin(Bo::And, half, breaks);
        let raised = self.bin(Bo::InclusiveOr, more, half);
        let small = self.bin(Bo::Add, quotient, raised);
        let nothing = self.bin(Bo::Less, exponent, lowest);
        let small = self.select(nothing, zero, small);
        // No number, or no end.
        let quiet = self.u32(0x7FC0_0000);
        let payload = self.bin(Bo::InclusiveOr, kept, rest);
        let number = self.bin(Bo::Equal, payload, zero);
        let no_number = self.bin(Bo::InclusiveOr, quiet, kept);
        let special = self.select(number, endless, no_number);
        let is_small = self.bin(Bo::Less, exponent, least);
        let is_great = self.bin(Bo::Greater, exponent, most);
        let is_special = self.bin(Bo::Equal, exponent, field);
        let magnitude = self.select(is_small, small, normal);
        let magnitude = self.select(is_great, endless, magnitude);
        let magnitude = self.select(is_special, special, magnitude);
        self.bin(Bo::InclusiveOr, sign, magnitude)
    }

    /// A dword as another lane of this lane's quad holds it: `steps` is, for each axis some
    /// lane of the pattern moves along, how many lanes this one moves, as a float. Each 16-bit
    /// half goes by its fine derivative, a difference of two whole numbers under 65,536 and
    /// so exact where a float's own derivative rounds; the second axis takes the derivative of
    /// what the first made, which in the lane across is that lane's neighbour.
    pub(super) fn quad_neighbour(
        &mut self,
        bits: Handle<E>,
        steps: [Option<Handle<E>>; 2],
    ) -> Handle<E> {
        let (low, sixteen) = (self.u32(0xFFFF), self.u32(16));
        let halves = [
            self.bin(Bo::And, bits, low),
            self.bin(Bo::ShiftRight, bits, sixteen),
        ];
        let mut halves = halves.map(|half| self.convert(half, ScalarKind::Float));
        let axes = [naga::DerivativeAxis::X, naga::DerivativeAxis::Y];
        for (axis, steps) in axes.into_iter().zip(steps) {
            let Some(steps) = steps else { continue };
            for half in &mut halves {
                let ctrl = naga::DerivativeControl::Fine;
                let slope = self.emit(E::Derivative {
                    axis,
                    ctrl,
                    expr: *half,
                });
                let change = self.bin(Bo::Multiply, steps, slope);
                *half = self.bin(Bo::Add, *half, change);
            }
        }
        let [low, high] = halves.map(|half| self.convert(half, ScalarKind::Uint));
        let high = self.bin(Bo::ShiftLeft, high, sixteen);
        self.bin(Bo::InclusiveOr, low, high)
    }

    /// A whole number as the 64-bit float that holds it exactly: its low dword and its high.
    pub(super) fn double_of_count(&mut self, count: Handle<E>) -> [Handle<E>; 2] {
        let (zero, one, most) = (self.u32(0), self.u32(1), self.u32(31));
        let (twelve, twenty, bias) = (self.u32(12), self.u32(20), self.u32(1023 + 31));
        let leading = self.math(Mf::CountLeadingZeros, &[count]);
        let leading = self.math(Mf::Min, &[leading, most]);
        // The first set bit leaves at the top: the 31 bits after it are the fraction's first.
        let fraction = self.bin(Bo::ShiftLeft, count, leading);
        let fraction = self.bin(Bo::ShiftLeft, fraction, one);
        let exponent = self.bin(Bo::Subtract, bias, leading);
        let exponent = self.bin(Bo::ShiftLeft, exponent, twenty);
        let upper = self.bin(Bo::ShiftRight, fraction, twelve);
        let high = self.bin(Bo::InclusiveOr, exponent, upper);
        let low = self.bin(Bo::ShiftLeft, fraction, twenty);
        let none = self.bin(Bo::Equal, count, zero);
        [self.select(none, zero, low), self.select(none, zero, high)]
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

    #[test]
    fn a_double_narrows_to_the_float_nearest_to_it() {
        let mut b = Builder::new();
        let [low, high] = [0, 1].map(|index| b.emit(E::FunctionArgument(index)));
        let single = b.single_of_double(low, high);
        let check = |bits: u64| {
            let given = [bits as u32, (bits >> 32) as u32];
            let got = eval_of(&b, single, &given, &mut Vec::new());
            let want = f64::from_bits(bits) as f32;
            match want.is_nan() {
                true => assert!(f32::from_bits(got).is_nan(), "{bits:#018x}"),
                false => assert_eq!(got, want.to_bits(), "{bits:#018x}"),
            }
        };
        // The doubles about a float and the tie above it, at each exponent a float's range
        // ends or changes at and a sample of the others.
        let exponents =
            (0..=0x7FFu64).filter(|e| e % 97 == 0 || (840..=900).contains(e) || *e >= 1140);
        for exponent in exponents {
            for dropped in [0u64, 1, 0x1000_0000, 0x0FFF_FFFF, 0x1000_0001, 0x3000_0000] {
                for kept in [0u64, 1, 2, 0x7F_FFFF, 0x7F_FFFE, 0x40_0000, 0x3F_FFFF] {
                    for sign in [0u64, 1] {
                        check(sign << 63 | exponent << 52 | kept << 29 | dropped);
                    }
                }
            }
        }
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..100_000 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            check(seed);
            // The exponents a float has, and those just under them.
            check(seed & !(0x7FF << 52) | (850 + (seed >> 40) % 310) << 52);
        }
    }

    /// What an expression of [`Builder::quad_neighbour`] comes to in lane `lane` of a quad
    /// whose lanes hold `held`, as a float: argument 0 is the lane's dword, and arguments 1
    /// and 2 its steps along the row and the column. A fine derivative is the lane to the
    /// right or below less the one to the left or above.
    fn in_quad(b: &Builder, at: Handle<E>, lane: usize, held: &[u32; 4], to: &[usize; 4]) -> f64 {
        let value = |at| in_quad(b, at, lane, held, to);
        match b.fun.expressions[at] {
            E::FunctionArgument(0) => f64::from(held[lane]),
            E::FunctionArgument(axis) => {
                let shift = axis as usize - 1;
                ((to[lane] >> shift & 1) as f64) - ((lane >> shift & 1) as f64)
            }
            E::Literal(Literal::U32(word)) => f64::from(word),
            E::Binary { op, left, right } => {
                let (l, r) = (value(left), value(right));
                match op {
                    Bo::And => f64::from(l as u32 & r as u32),
                    Bo::InclusiveOr => f64::from(l as u32 | r as u32),
                    Bo::ShiftRight => f64::from(l as u32 >> r as u32),
                    Bo::ShiftLeft => f64::from((l as u32) << r as u32),
                    // The sums and products here are a float's: rounded as one.
                    Bo::Add => f64::from(l as f32 + r as f32),
                    Bo::Multiply => f64::from(l as f32 * r as f32),
                    other => panic!("{other:?} is not evaluated"),
                }
            }
            // To a float, the nearest; to a dword, a whole number as it is.
            E::As {
                expr,
                kind: ScalarKind::Float,
                ..
            } => f64::from(value(expr) as f32),
            E::As { expr, .. } => {
                let whole = value(expr);
                assert_eq!(whole, whole.trunc());
                whole
            }
            E::Derivative { axis, expr, .. } => {
                let bit = if axis == naga::DerivativeAxis::X {
                    1
                } else {
                    2
                };
                let (near, far) = (lane & !bit, lane | bit);
                let far = in_quad(b, expr, far, held, to) as f32;
                f64::from(far - in_quad(b, expr, near, held, to) as f32)
            }
            ref other => panic!("{other:?} is not evaluated"),
        }
    }

    #[test]
    fn a_lane_of_a_quad_reads_any_other_lane_s_dword_whole() {
        let mut b = Builder::new();
        let [bits, across, down] = [0, 1, 2].map(|index| b.emit(E::FunctionArgument(index)));
        // The lane in the row, in the column, across both, each lane to one lane, and itself.
        let patterns = [
            [1, 0, 3, 2],
            [2, 3, 0, 1],
            [3, 2, 1, 0],
            [0, 0, 0, 0],
            [0, 1, 2, 3],
        ];
        let mut seed = 0x2545_F491u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed
        };
        for to in patterns {
            let moves = |shift: usize| (0..4).any(|lane| (to[lane] ^ lane) >> shift & 1 != 0);
            let steps = [moves(0).then_some(across), moves(1).then_some(down)];
            let read = b.quad_neighbour(bits, steps);
            for _ in 0..2_000 {
                // Floats near one another, as a quad's are, and any four dwords.
                let near = 0x3F80_0000 | next() >> 9;
                let held = match next() & 1 {
                    0 => [near, near ^ 1, near.wrapping_add(next() & 0xFFFF), next()],
                    _ => [next(), next(), next(), next()],
                };
                for lane in 0..4 {
                    let got = in_quad(&b, read, lane, &held, &to) as u32;
                    assert_eq!(got, held[to[lane]], "lane {lane} of {held:x?} to {to:?}");
                }
            }
        }
    }

    #[test]
    fn a_count_is_the_double_that_holds_it() {
        let mut b = Builder::new();
        let x = b.emit(E::FunctionArgument(0));
        let [low, high] = b.double_of_count(x);
        let more = [u32::MAX, 0x8000_0000, 0x8000_0001, 0x00FF_FFFF, 0x1234_5678];
        for count in (0..=4096u32).chain(more) {
            let bits = f64::from(count).to_bits();
            assert_eq!(eval(&b, low, count), bits as u32, "{count}");
            assert_eq!(eval(&b, high, count), (bits >> 32) as u32, "{count}");
        }
    }
}
