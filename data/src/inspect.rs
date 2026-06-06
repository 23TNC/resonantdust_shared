//! Client-facing introspection over the flag + type registries.
//!
//! These are the model-stable lookups the pixijs client's `DefinitionManager`
//! needs that don't depend on a loaded [`crate::loader::Bundle`] — the flag bit
//! layout ([`crate::flags`]) and the card-type nibble table
//! ([`crate::loader::type_nibble`]). They replace the legacy
//! `resonantdust_content` free functions (`cardFlagBit`, `cardFlagFieldShape`,
//! `hasCardFlag`, `cardTypeId`, `isHexType`, …) one-for-one, so the client wraps
//! these via `resonantdust-wasm` instead of the deleted crate.

use crate::flags::{flag_bit, flag_field};
use crate::loader::type_nibble;

/// The two flag host integers, searched in this order when a lookup isn't
/// field-qualified (state bits take precedence — mirrors the legacy behaviour).
const FIELDS: [&str; 2] = ["cards_state", "cards_bk"];

/// Single-bit flag position (0..=31) by name, searching `cards_state` then
/// `cards_bk`. `None` for unknown names.
pub fn card_flag_bit(name: &str) -> Option<u8> {
  FIELDS.iter().find_map(|f| flag_bit(f, name))
}

/// Is the named single-bit flag set, routing to whichever host integer declares
/// it (`flags_state` checked against the `cards_state` layout, `flags_bk` against
/// `cards_bk`)? `false` for unknown names.
pub fn has_card_flag(flags_state: u32, flags_bk: u32, name: &str) -> bool {
  if let Some(bit) = flag_bit("cards_state", name) {
    return flags_state & (1 << bit) != 0;
  }
  if let Some(bit) = flag_bit("cards_bk", name) {
    return flags_bk & (1 << bit) != 0;
  }
  false
}

/// `(shift, width)` of a named multi-bit field within `field`, or `None`.
pub fn card_flag_field_shape(field: &str, name: &str) -> Option<(u8, u8)> {
  flag_field(field, name).map(|f| (f.shift, f.width))
}

/// Extract a named multi-bit field's value from `host` (the matching
/// `flags_state` / `flags_bk` column), within an explicit `field`. `None` if the
/// field isn't declared there.
pub fn card_flag_field_value_in(field: &str, host: u32, name: &str) -> Option<u32> {
  flag_field(field, name).map(|f| (host & f.mask()) >> f.shift)
}

/// Extract a named multi-bit field's value, routing to whichever host declares
/// it (`cards_state` → `flags_state`, `cards_bk` → `flags_bk`). `None` if
/// unknown.
pub fn card_flag_field_value_any(flags_state: u32, flags_bk: u32, name: &str) -> Option<u32> {
  card_flag_field_value_in("cards_state", flags_state, name)
    .or_else(|| card_flag_field_value_in("cards_bk", flags_bk, name))
}

/// The `card_type` nibble for a type name (e.g. `"tile"` → 7), or `None`.
pub fn card_type_id(name: &str) -> Option<u8> {
  type_nibble(name)
}

/// Whether a `card_type` nibble renders on the hex world grid (`tile`,
/// `mini_zone`, `tile_decorator`) vs. a rect card. Drives the client's
/// `shape()` ("hex" | "rect"). The hex set is the world-grid card types.
pub fn is_hex_type(type_id: u8) -> bool {
  matches!(
    type_id,
    7 /* tile */ | 8 /* mini_zone */ | 9 /* tile_decorator */
  )
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn flag_bit_searches_both_fields() {
    // `dead` lives in cards_state (bit 0); `micro_is_card` in cards_bk (bit 24).
    assert_eq!(card_flag_bit("dead"), Some(0));
    assert_eq!(card_flag_bit("micro_is_card"), Some(24));
    assert_eq!(card_flag_bit("nope"), None);
  }

  #[test]
  fn has_flag_routes_to_the_right_host() {
    let dead = 1u32 << 0; // cards_state.dead
    let mic = 1u32 << 24; // cards_bk.micro_is_card
    // dead is a state bit → read from flags_state, ignore flags_bk
    assert!(has_card_flag(dead, 0, "dead"));
    assert!(!has_card_flag(0, dead, "dead"));
    // micro_is_card is a bk bit → read from flags_bk
    assert!(has_card_flag(0, mic, "micro_is_card"));
    assert!(!has_card_flag(mic, 0, "micro_is_card"));
    assert!(!has_card_flag(0xFFFF_FFFF, 0xFFFF_FFFF, "nope"));
  }

  #[test]
  fn multi_bit_field_value_reads() {
    // slot_hold_count is cards_bk (shift 13, width 3); pack a 5.
    let host = 5u32 << 13;
    assert_eq!(card_flag_field_shape("cards_bk", "slot_hold_count"), Some((13, 3)));
    assert_eq!(card_flag_field_value_in("cards_bk", host, "slot_hold_count"), Some(5));
    // routed lookup finds it in cards_bk when passed as flags_bk
    assert_eq!(card_flag_field_value_any(0, host, "slot_hold_count"), Some(5));
    assert_eq!(card_flag_field_value_in("cards_bk", host, "nope"), None);
  }

  #[test]
  fn type_id_and_hex() {
    assert_eq!(card_type_id("tile"), Some(7));
    assert_eq!(card_type_id("requisite"), Some(0));
    assert_eq!(card_type_id("nope"), None);
    assert!(is_hex_type(7)); // tile
    assert!(!is_hex_type(0)); // requisite → rect
  }
}
