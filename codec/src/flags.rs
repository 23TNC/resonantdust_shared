//! Card flag-bit registry — the shared bit layout for the three flag columns on
//! the cards table:
//!
//!   - `flags`    (u32) — the **propagating** word: gameplay state bits,
//!                        placement (`stack`/`index`), and the refcount holds.
//!                        Everything here carries forward through a card's
//!                        history (state bits via bit-diff, refcounts via delta
//!                        arithmetic, placement per-row).
//!   - `flags_bk` (u8)  — **non-propagating** bookkeeping: the dirty / preserve
//!                        markers, recomputed on every write, never carried
//!                        forward.
//!   - `stock`    (u8)  — tile-card per-row stock slots (u2 each).
//!
//! This module is the single source of truth for the bit positions; the modules
//! (which have no runtime filesystem) and the gate share it directly. The layout
//! is append-only within a column: never reuse a bit.
//!
//! Stack semantics: `stack` (bits 0-3 of `flags`) is `0` for a loose card
//! (then `micro_location` is packed coords and `index` is the loose `kind`) and
//! nonzero for a stack member (then `micro_location` is the root card_id,
//! `stack` is `branch + 1`, and `index` is the slot within the branch). The
//! `stack == 0` sentinel replaces the former `micro_is_card` discriminator bit.

/// A multi-bit field within a host integer — described by its lowest bit
/// (`shift`) and bit count (`width`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlagField {
  pub shift: u8,
  pub width: u8,
}

impl FlagField {
  /// Bitmask covering this field's window. `& !mask()` clears the field.
  pub fn mask(self) -> u32 {
    self.value_mask() << self.shift
  }
  /// Pack a value into the field's bit positions (truncated to width).
  pub fn pack(self, value: u32) -> u32 {
    (value & self.value_mask()) << self.shift
  }
  fn value_mask(self) -> u32 {
    (((1u64 << self.width) - 1) & 0xFFFF_FFFF) as u32
  }
}

/// Single-bit flag position for `(field, name)`, or `None`. `field` is one of
/// `"flags"` (the propagating word), `"flags_bk"` (bookkeeping), or `"stock"`.
pub fn flag_bit(field: &str, name: &str) -> Option<u8> {
  Some(match (field, name) {
    // --- flags (propagating) — single-bit state ---
    ("flags", "player_owned") => 24,
    ("flags", "surface_locked") => 25,
    ("flags", "dead") => 26,
    ("flags", "pos_need") => 27,
    ("flags", "pos_want") => 28,
    ("flags", "zone_born") => 29,
    // --- flags_bk (non-propagating bookkeeping) ---
    ("flags_bk", "position_dirty") => 0,
    ("flags_bk", "position_preserve") => 1,
    ("flags_bk", "data_dirty") => 2,
    ("flags_bk", "data_preserve") => 3,
    _ => return None,
  })
}

/// Multi-bit field for `(field, name)`, or `None`.
pub fn flag_field(field: &str, name: &str) -> Option<FlagField> {
  let (shift, width) = match (field, name) {
    // --- flags (propagating) — placement ---
    ("flags", "stack") => (0, 4),
    ("flags", "index") => (4, 4),
    // --- flags (propagating) — refcount holds ---
    ("flags", "slot_claim_count") => (8, 3),
    ("flags", "slot_borrow_count") => (11, 3),
    ("flags", "position_hold_count") => (14, 3),
    ("flags", "drop_hold_count") => (17, 3),
    ("flags", "touch_count") => (20, 2),
    ("flags", "server_count") => (22, 2),
    // --- stock (tile-card per-row stock slots) ---
    ("stock", "stock_0") => (0, 2),
    ("stock", "stock_1") => (2, 2),
    _ => return None,
  };
  Some(FlagField { shift, width })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn known_bits_and_fields() {
    assert_eq!(flag_bit("flags", "dead"), Some(26));
    assert_eq!(flag_bit("flags", "player_owned"), Some(24));
    assert_eq!(flag_bit("flags_bk", "position_dirty"), Some(0));
    assert_eq!(flag_bit("flags", "ghost"), None);
    let f = flag_field("flags", "slot_claim_count").unwrap();
    assert_eq!((f.shift, f.width), (8, 3));
    assert_eq!(f.mask(), 0b111 << 8);
    assert_eq!(f.pack(5), 5 << 8);
    let s = flag_field("flags", "stack").unwrap();
    assert_eq!((s.shift, s.width), (0, 4));
    let st = flag_field("stock", "stock_1").unwrap();
    assert_eq!((st.shift, st.width), (2, 2));
  }

  #[test]
  fn flags_word_fits_u32() {
    // The propagating word's highest declared bit must be < 32.
    for name in ["player_owned", "surface_locked", "dead", "pos_need", "pos_want", "zone_born"] {
      assert!(flag_bit("flags", name).unwrap() < 32);
    }
    for name in [
      "stack", "index", "slot_claim_count", "slot_borrow_count",
      "position_hold_count", "drop_hold_count", "touch_count", "server_count",
    ] {
      let f = flag_field("flags", name).unwrap();
      assert!(f.shift + f.width <= 32, "{name} overflows u32");
    }
  }
}
