//! Two departures from the hardware that no statement of naga 29 avoids, each named and
//! counted: a texel compared and swapped without the exchange, and a pixel program's ordered
//! section run in no order.

use naga::{BinaryOperator as Bo, Expression as E, Scalar, Statement, StorageFormat, VectorSize};

use super::{form, refused, vector, Cx, Done, Inst, TEXEL_SWAP};

/// What `image_atomic_cmpswap` of a 32-bit texel is counted as.
// Two invocations that update one texel at once may both read its old value, and the later
// store then undoes the other's update, where the hardware's exchange sends one round again.
pub const TEXEL_UNLOCKED: &str = "a texel compared and swapped as a load, a comparison and a \
    store";
/// What a pixel program that waits its turn at a pixel is counted as.
// Once at each place it turns the order on, where it stores, and at each read of the wave
// that last left: two primitives covering one pixel read and write its stores in either
// order, and may both read one old value.
pub const UNORDERED: &str = "a pixel program's ordered section run in no order";
/// The departures a renderer does not draw a stage with: it may be wrong wherever two invocations meet.
pub const NOT_DRAWN: [&str; 2] = [TEXEL_UNLOCKED, UNORDERED];

/// The first of `approximations` a renderer does not draw a stage with ([`NOT_DRAWN`]).
pub fn refused_for_drawing(
    approximations: &std::collections::BTreeMap<&'static str, u32>,
) -> Option<&'static str> {
    NOT_DRAWN
        .into_iter()
        .find(|name| approximations.get(name).is_some_and(|&count| count > 0))
}

impl Cx<'_> {
    /// `image_atomic_cmpswap` of a 32-bit texel: the texel loaded, the new dword stored where it
    /// held the compared one, and the dword loaded given back, in an active lane.
    pub(super) fn texel_swap(&mut self, pc: u32, inst: &Inst) -> Done {
        let Inst::Mimg {
            addrs,
            data,
            rsrc,
            dmask,
            dim,
            a16,
            glc,
            ..
        } = inst
        else {
            return Err(refused(pc, inst));
        };
        // Two registers are the new dword and the compared one; four would be a 64-bit texel.
        if *dmask != 3 {
            return Err(form(pc, inst, TEXEL_SWAP));
        }
        let stated = Some(StorageFormat::R32Uint);
        let (image, format, _) = self.stored_image(pc, inst, (*rsrc, *dim), stated)?;
        if format != StorageFormat::R32Uint {
            return Err(form(pc, inst, "an image updated as two formats"));
        }
        let (coordinate, array_index) = self.texel_place(pc, inst, addrs, (*dim, *a16))?;
        let (new, compared) = (self.vreg(pc, *data)?, self.vreg(pc, data.wrapping_add(1))?);
        let texel = self.b.emit(E::ImageLoad {
            image,
            coordinate,
            array_index,
            sample: None,
            level: None,
        });
        let old = self.b.index(texel, 0);
        let same = self.b.bin(Bo::Equal, old, compared);
        let ty = vector(&mut self.module, VectorSize::Quad, Scalar::U32);
        let zero = self.b.u32(0);
        let value = self.b.emit(E::Compose {
            ty,
            components: vec![new, zero, zero, zero],
        });
        self.when_active(|cx| {
            cx.when(same, |cx| {
                cx.b.push(Statement::ImageStore {
                    image,
                    coordinate,
                    array_index,
                    value,
                });
                Ok(())
            })
        })?;
        self.note(TEXEL_UNLOCKED);
        // The texel it found goes back to the data register where the program asked for it.
        if *glc {
            self.vput(*data, old);
        }
        Ok(())
    }
}
