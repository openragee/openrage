//! The arena's layout: byte offsets and bit fields of a hierarchy in the console's node format.
//!
//! A structure is a header, nodes of 64 or 128 bytes and a parent table, all dwords. The header's
//! fields beyond the four below, and a box node's bytes past its bounds, are not known: they are
//! written as nought, but in a top level's boxes, where OpenRage keeps [`BOX_INSTANCES`] there.

/// A structure starts on a multiple of this, so its address is carried as `address >> 8`.
pub const STRUCTURE_ALIGN: u32 = 256;
/// The byte of an arena a device's query is answered over that holds the top level's address: the first 256 bytes hold no structure.
pub const QUERY_TOP: u32 = 0;
/// The bytes a node pointer counts in: every node starts on a multiple of it.
pub const UNIT: u32 = 64;

/// The header's bytes; the first node is the unit after it.
pub const HEADER_BYTES: u32 = 128;
/// The root's node pointer; a walk does not start where it is not positive.
pub const HEADER_ROOT: u32 = 48;
/// The byte offset of the node the parent table's first entry belongs to.
pub const HEADER_FIRST_NODE: u32 = 52;
/// The byte offset of the parent table: a dword a unit, the node pointer of the unit's parent.
pub const HEADER_PARENTS: u32 = 56;
/// A bottom level's scale exponents and rotation number; written nought, which states neither.
pub const HEADER_SPACE: u32 = 60;

/// The pointer of no node: an absent child, a miss, the root's parent.
pub const INVALID: u32 = 0xFFFF_FFFF;
/// The type of a box node of 16-bit floats, which the builder never writes.
pub const TYPE_BOX16: u32 = 4;
/// The type of a box node of 32-bit floats.
pub const TYPE_BOX32: u32 = 5;
/// The type of an instance node.
pub const TYPE_INSTANCE: u32 = 6;

/// The pointer of the node at `unit` of its structure, of type `ty`: the low three bits.
pub const fn pointer(unit: u32, ty: u32) -> u32 {
    (unit << 3) | ty
}

/// The byte offset in its structure of the node `pointer` names.
pub const fn offset(pointer: u32) -> u32 {
    (pointer >> 3).wrapping_mul(UNIT)
}

/// Whether `pointer` names a box node of either width.
pub const fn is_box(pointer: u32) -> bool {
    pointer & 6 == 4
}

/// A box node's bytes.
pub const BOX_BYTES: u32 = 128;
/// Four child pointers, [`INVALID`] where a child is absent.
pub const BOX_CHILDREN: u32 = 0;
/// Child 0's bounds: the least corner then the greatest, three floats each.
pub const BOX_BOUNDS: u32 = 16;
/// The bytes from one child's bounds to the next.
pub const BOX_BOUNDS_STRIDE: u32 = 24;
/// In a top-level box, a dword a child: the instances under it, the number of the first in the
/// low 16 bits and of the last above them; [`INVALID`] for an absent child. An instance's number
/// is its node's place among the instance nodes as they were placed, a child before the next.
pub const BOX_INSTANCES: u32 = 112;

/// A triangle node's bytes: four vertices of three floats, then four dwords.
pub const TRIANGLE_BYTES: u32 = 64;
/// The bytes from one vertex to the next.
pub const TRIANGLE_VERTEX_STRIDE: u32 = 12;
/// Bit 0 [`TRIANGLE_OPAQUE`], bit 7 [`TRIANGLE_PAIR`], bits 8 to 31 the geometry's number.
pub const TRIANGLE_WORD: u32 = 48;
/// The primitive number of the node's first triangle; the second's is the dword after.
pub const TRIANGLE_PRIMITIVE: u32 = 52;
/// A byte a triangle: bits 1:0 the corner whose weight is returned first, bits 3:2 second.
pub const TRIANGLE_ID: u32 = 60;
/// The geometry is opaque.
pub const TRIANGLE_OPAQUE: u32 = 1;
/// The node holds a second triangle, reached as the pointer of type 1.
pub const TRIANGLE_PAIR: u32 = 0x80;
/// The vertices triangle `k` of a node uses, in its winding.
pub const CORNERS: [[u32; 3]; 4] = [[0, 1, 2], [1, 3, 2], [2, 3, 4], [2, 4, 0]];

/// An instance node's bytes.
pub const INSTANCE_BYTES: u32 = 128;
/// Object to world: three rows of four floats.
pub const INSTANCE_OBJECT_TO_WORLD: u32 = 0;
/// Bits 24 to 31 the mask a ray's is anded with, bits 0 to 23 the instance's id.
pub const INSTANCE_ID_MASK: u32 = 48;
/// Bits 24 to 31 the instance's flags, bits 0 to 23 its contribution to a geometry's number.
pub const INSTANCE_CONTRIBUTION_FLAGS: u32 = 52;
/// Bits 0 to 29 the bottom level's root pointer, bit 31 [`INSTANCE_IDENTITY`].
pub const INSTANCE_ROOT: u32 = 56;
/// The bottom level's address `>> 8`.
pub const INSTANCE_BOTTOM: u32 = 60;
/// World to object, its translation: three floats.
pub const INSTANCE_TRANSLATION: u32 = 64;
/// World to object, its rotation and scale: nine floats in rows.
pub const INSTANCE_ROTATION: u32 = 76;
/// The instance's place in the list its top level was built from, in the low 16 bits.
pub const INSTANCE_INDEX: u32 = 112;
/// The node pointer of the instance node's parent in the top level.
pub const INSTANCE_PARENT: u32 = 116;
/// The root field of an instance with no bottom level.
pub const INSTANCE_NO_ROOT: u32 = 0x3FFF_FFFF;
/// The rotation is the identity: only the translation turns a ray into the instance's space.
pub const INSTANCE_IDENTITY: u32 = 0x8000_0000;

/// Instance flag: no face of the instance is culled.
pub const FLAG_NO_CULL: u32 = 1;
/// Instance flag: the instance's front faces are the other winding.
pub const FLAG_FLIP_WINDING: u32 = 2;
/// Instance flag: every triangle of the instance is opaque.
pub const FLAG_FORCE_OPAQUE: u32 = 4;
/// Instance flag: no triangle of the instance is opaque.
pub const FLAG_FORCE_NON_OPAQUE: u32 = 8;
