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

#[cfg(test)]
mod tests {
    use super::*;
    use naga::{Literal, UnaryOperator as Uo};

    /// What an expression over dwords and conditions comes to, with the function's one argument `x`.
    fn eval(b: &Builder, at: Handle<E>, x: u32) -> u32 {
        let dword = |h| eval(b, h, x);
        match b.fun.expressions[at] {
            E::FunctionArgument(_) => x,
            E::Literal(Literal::U32(word)) => word,
            E::Literal(Literal::Bool(flag)) => u32::from(flag),
            E::Binary { op, left, right } => {
                let (l, r) = (dword(left), dword(right));
                match op {
                    Bo::Add => l.wrapping_add(r),
                    Bo::Subtract => l.wrapping_sub(r),
                    Bo::And => l & r,
                    Bo::InclusiveOr => l | r,
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
            } => match dword(condition) {
                0 => dword(reject),
                _ => dword(accept),
            },
            E::Math {
                fun,
                arg,
                arg1,
                arg2,
                ..
            } => match (fun, arg1, arg2) {
                (Mf::Min, Some(b), _) => dword(arg).min(dword(b)),
                (Mf::Max, Some(b), _) => dword(arg).max(dword(b)),
                (Mf::Clamp, Some(low), Some(high)) => dword(arg).clamp(dword(low), dword(high)),
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
