//! Fixed-width bit-field primitives over a `u32` word.
//!
//! The real packed layouts (definition id + flags + indices, etc.) will
//! be expressed in terms of these two functions rather than ad-hoc shifts
//! scattered across call sites — one place to get the masking right.

/// Mask covering the low `width` bits. `width == 0` -> 0, `width >= 32`
/// -> all ones (avoids the `1 << 32` overflow trap).
#[inline]
fn mask(width: u32) -> u32 {
  if width >= 32 {
    u32::MAX
  } else {
    (1u32 << width) - 1
  }
}

/// Extract the `width`-bit field starting at `offset` from `word`.
///
/// `offset + width` must be `<= 32`.
#[inline]
pub fn get_field(word: u32, offset: u32, width: u32) -> u32 {
  debug_assert!(offset + width <= 32, "field {offset}+{width} exceeds 32 bits");
  (word >> offset) & mask(width)
}

/// Return `word` with the `width`-bit field at `offset` replaced by
/// `value` (masked to `width` bits, so over-wide values can't bleed into
/// neighbouring fields).
///
/// `offset + width` must be `<= 32`.
#[inline]
pub fn set_field(word: u32, offset: u32, width: u32, value: u32) -> u32 {
  debug_assert!(offset + width <= 32, "field {offset}+{width} exceeds 32 bits");
  let m = mask(width) << offset;
  (word & !m) | ((value << offset) & m)
}

// ---- packed definitions & tile slots ---------------------------------
//
// The two `u16` wire layouts the gate / client / modules share (mirrors
// `content/cards/packed.rs`; ported here so the DSL runtime owns the codec):
//
//   packed_def   = [ card_type:u4 (bits 12-15) | def_id:u12 (bits 0-11) ]
//   tile slot    = [ stock1:u2 (bits 14-15) | stock0:u2 (bits 12-13) | def_id:u12 ]
//
// The 64-tile-per-zone packing (tile slots laid into 16 `u64`s) stays a
// regions-module concern; these are the single-value primitives it builds on.

/// Mask isolating a packed definition's `def_id` (low 12 bits).
pub const DEF_ID_MASK: u16 = 0x0FFF;

/// Pack `[card_type:u4 | def_id:u12]`. Over-wide inputs are masked to width.
pub fn pack_def(card_type: u8, def_id: u16) -> u16 {
  (((card_type & 0xF) as u16) << 12) | (def_id & DEF_ID_MASK)
}

/// Split a packed definition into `(card_type, def_id)`.
pub fn unpack_def(packed: u16) -> (u8, u16) {
  (((packed >> 12) & 0xF) as u8, packed & DEF_ID_MASK)
}

/// Pack one Zone tile slot `[def_id:u12 | stock0:u2 | stock1:u2]`. Stock values
/// clamp to their 2-bit field; `def_id` to 12 bits.
pub fn pack_tile_slot(def_id: u16, stock0: u8, stock1: u8) -> u16 {
  (def_id & DEF_ID_MASK)
    | (((stock0 as u16) & 0x3) << 12)
    | (((stock1 as u16) & 0x3) << 14)
}

/// Split a Zone tile slot into `(def_id, stock0, stock1)`.
pub fn unpack_tile_slot(slot: u16) -> (u16, u8, u8) {
  (slot & DEF_ID_MASK, ((slot >> 12) & 0x3) as u8, ((slot >> 14) & 0x3) as u8)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn packed_def_round_trips() {
    // tile (type 7), def_id 42
    let p = pack_def(7, 42);
    assert_eq!(unpack_def(p), (7, 42));
    // over-wide def_id is masked to 12 bits, not bleeding into the type nibble
    assert_eq!(unpack_def(pack_def(7, 0x1FFF)), (7, 0x0FFF));
  }

  #[test]
  fn tile_slot_round_trips() {
    let s = pack_tile_slot(42, 2, 3);
    assert_eq!(unpack_tile_slot(s), (42, 2, 3));
    // stocks clamp to 2 bits, def_id stays intact
    assert_eq!(unpack_tile_slot(pack_tile_slot(0x0FFF, 0xFF, 0xFF)), (0x0FFF, 3, 3));
  }

  #[test]
  fn round_trips() {
    let w = set_field(0, 8, 4, 0b1011);
    assert_eq!(get_field(w, 8, 4), 0b1011);
  }

  #[test]
  fn over_wide_value_is_masked() {
    // 0xFF into a 4-bit field keeps only the low 4 bits...
    let w = set_field(0, 0, 4, 0xFF);
    assert_eq!(get_field(w, 0, 4), 0xF);
    // ...and does not corrupt the adjacent field above it.
    assert_eq!(get_field(w, 4, 4), 0);
  }

  #[test]
  fn fields_are_independent() {
    let w = set_field(set_field(0, 0, 8, 0xAB), 8, 8, 0xCD);
    assert_eq!(get_field(w, 0, 8), 0xAB);
    assert_eq!(get_field(w, 8, 8), 0xCD);
  }

  #[test]
  fn full_width_mask() {
    let w = set_field(0, 0, 32, 0xDEAD_BEEF);
    assert_eq!(get_field(w, 0, 32), 0xDEAD_BEEF);
  }
}
