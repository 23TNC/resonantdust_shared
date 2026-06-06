//! Card flag-bit registry — the shared bit layout for `cards.flags_state`
//! (propagated state) and `cards.flags_bk` (bookkeeping + refcounts).
//!
//! The positions are the canonical layout in `content/cards/flags.json`; they're
//! transcribed here as `const` matches so the modules (which have no runtime
//! filesystem) and the gate share one source without embedding JSON. flags.json
//! stays as the human-facing spec — keep the two in sync (the layout is
//! append-only: never reuse a bit). Ported from the legacy
//! `resonantdust_content::flags_core` (plan `01_gate_authority_pivot`).

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

/// Single-bit flag position (0..=31) for `(field, name)`, or `None`. `field` is
/// `"cards_state"` or `"cards_bk"` (mirrors `flags.json`'s two sections).
pub fn flag_bit(field: &str, name: &str) -> Option<u8> {
  Some(match (field, name) {
    ("cards_state", "dead") => 0,
    ("cards_state", "pos_need") => 1,
    ("cards_state", "magnetic") => 2,
    ("cards_state", "surface_locked") => 3,
    ("cards_state", "is_owned_by_player") => 4,
    ("cards_state", "pos_want") => 12,
    ("cards_state", "zone_born") => 13,
    ("cards_bk", "position_dirty") => 0,
    ("cards_bk", "position_preserve") => 1,
    ("cards_bk", "data_dirty") => 2,
    ("cards_bk", "data_preserve") => 3,
    ("cards_bk", "micro_is_card") => 24,
    _ => return None,
  })
}

/// Multi-bit field for `(field, name)`, or `None`.
pub fn flag_field(field: &str, name: &str) -> Option<FlagField> {
  let (shift, width) = match (field, name) {
    ("cards_state", "progress_style") => (5, 3),
    ("cards_state", "portrait_id") => (8, 4),
    ("cards_bk", "position_hold_count") => (4, 3),
    ("cards_bk", "slot_share_count") => (7, 3),
    ("cards_bk", "drop_hold_count") => (10, 3),
    ("cards_bk", "slot_hold_count") => (13, 3),
    ("cards_bk", "touch_count") => (16, 2),
    ("cards_bk", "server_count") => (18, 2),
    ("cards_bk", "tile_stock_0") => (20, 2),
    ("cards_bk", "tile_stock_1") => (22, 2),
    ("cards_bk", "stack_state") => (25, 2),
    ("cards_bk", "stack_index") => (27, 4),
    _ => return None,
  };
  Some(FlagField { shift, width })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn known_bits_and_fields() {
    assert_eq!(flag_bit("cards_state", "dead"), Some(0));
    assert_eq!(flag_bit("cards_state", "is_owned_by_player"), Some(4));
    assert_eq!(flag_bit("cards_bk", "micro_is_card"), Some(24));
    assert_eq!(flag_bit("cards_bk", "ghost"), None);
    let f = flag_field("cards_bk", "slot_hold_count").unwrap();
    assert_eq!((f.shift, f.width), (13, 3));
    assert_eq!(f.mask(), 0b111 << 13);
    assert_eq!(f.pack(5), 5 << 13);
    let p = flag_field("cards_state", "progress_style").unwrap();
    assert_eq!((p.shift, p.width), (5, 3));
  }
}
