//! Generalized stacking eligibility — the Rust port of the client's
//! `stacking.ts`, now the single shared source so the client (via wasm) and the
//! gate agree by construction instead of the gate trusting a client-computed
//! direction.
//!
//! Two bit-fields over stack indices (bit `i` = stack `i`: 0 hex/under-root,
//! 1 top, 2 bottom):
//!   - `hosts` — stacks this card SOURCES as a root (slots others attach to)
//!   - `joins` — stacks this card can OCCUPY as a member
//!
//! Drop `a` onto `b`: try `a` joins `b` (`b.hosts & a.joins`), else `b` joins
//! `a` (`a.hosts & b.joins`). Leftmost stack wins (hex 0 preferred), the drop
//! direction breaks a top/bottom tie. The root is always the host side. This is
//! the mechanism that makes a card-onto-tile drop push the *tile* into the
//! card's stack 0 (the tile hosts nothing, joins only 0).
//!
//! Aspects live in content (`content/data/aspect/01.rd`, `traits`): tiles/events
//! set `stack_joins = {0}`; cards that declare neither aspect take the
//! regular-card default.

use resonantdust_codec::packed::{STACK_DIR_DOWN, STACK_DIR_HEX, STACK_DIR_UP};
use resonantdust_dsl::defs::aspect_value;
use resonantdust_dsl::loader::Bundle;

/// A card's stacking bit-fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackBits {
    pub hosts: u8,
    pub joins: u8,
}

/// Regular card: hosts the hex slot + top + bottom, joins top/bottom.
pub const DEFAULT_BITS: StackBits = StackBits { hosts: 0b111, joins: 0b110 };

#[inline]
fn bit(stack: u8) -> u8 {
    1 << stack
}

/// Read a card's stacking bit-fields from its definition. `stack_joins` present
/// marks an explicit config (tiles/events, `{0}`); absent → regular default.
/// `stack_hosts` defaults to 0 when only joins is set (a tile hosts nothing).
pub fn stack_bits(bundle: &Bundle, packed: u16) -> StackBits {
    match aspect_value(bundle, packed, "stack_joins") {
        None => DEFAULT_BITS,
        Some(joins) => StackBits {
            hosts: aspect_value(bundle, packed, "stack_hosts").unwrap_or(0) as u8,
            joins: joins as u8,
        },
    }
}

/// The stack a `joiner` occupies on a `host`, or `None` if none. Leftmost wins
/// (hex 0 first); `drop_dir` (UP/DOWN) breaks a top+bottom tie.
pub fn match_stack(host: StackBits, joiner: StackBits, drop_dir: u8) -> Option<u8> {
    let m = host.hosts & joiner.joins;
    if m == 0 {
        return None;
    }
    if m & bit(STACK_DIR_HEX) != 0 {
        return Some(STACK_DIR_HEX);
    }
    let up = m & bit(STACK_DIR_UP) != 0;
    let down = m & bit(STACK_DIR_DOWN) != 0;
    if up && down {
        return Some(if drop_dir == STACK_DIR_DOWN { STACK_DIR_DOWN } else { STACK_DIR_UP });
    }
    if up {
        return Some(STACK_DIR_UP);
    }
    if down {
        return Some(STACK_DIR_DOWN);
    }
    None
}

/// The outcome of a drop resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackResolution {
    /// `true` → the dragged card joins the target (target is root): the normal
    /// card-onto-card stack. `false` → the target joins the dragged card
    /// (dragged is root, target absorbed into its stack 0): the card-onto-tile
    /// case.
    pub dragged_is_member: bool,
    /// Stack index the member occupies on the root (`STACK_DIR_*`).
    pub stack: u8,
}

/// Bidirectional resolve for dropping `dragged` onto `target`. Forward first
/// (dragged joins target), then reverse (target joins dragged). `None` = the two
/// can't stack either way → caller rejects / falls back.
pub fn resolve_stack_drop(
    dragged: StackBits,
    target: StackBits,
    drop_dir: u8,
) -> Option<StackResolution> {
    if let Some(stack) = match_stack(target, dragged, drop_dir) {
        return Some(StackResolution { dragged_is_member: true, stack });
    }
    if let Some(stack) = match_stack(dragged, target, drop_dir) {
        return Some(StackResolution { dragged_is_member: false, stack });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const TILE: StackBits = StackBits { hosts: 0, joins: 0b001 }; // hosts nothing, joins stack 0

    #[test]
    fn regular_onto_regular_uses_drop_dir_for_top_bottom() {
        // dragged joins target; both up+down open → drop_dir picks.
        let up = resolve_stack_drop(DEFAULT_BITS, DEFAULT_BITS, STACK_DIR_UP).unwrap();
        assert_eq!(up, StackResolution { dragged_is_member: true, stack: STACK_DIR_UP });
        let down = resolve_stack_drop(DEFAULT_BITS, DEFAULT_BITS, STACK_DIR_DOWN).unwrap();
        assert_eq!(down, StackResolution { dragged_is_member: true, stack: STACK_DIR_DOWN });
    }

    #[test]
    fn card_onto_tile_pushes_tile_into_card_stack0() {
        // Forward fails (tile hosts nothing); reverse makes the card the root and
        // the tile joins its stack 0.
        let r = resolve_stack_drop(DEFAULT_BITS, TILE, STACK_DIR_UP).unwrap();
        assert_eq!(r, StackResolution { dragged_is_member: false, stack: STACK_DIR_HEX });
    }

    #[test]
    fn dragging_tile_onto_card_joins_stack0_forward() {
        // Tile dragged onto a card: forward (tile joins card's hex slot).
        let r = resolve_stack_drop(TILE, DEFAULT_BITS, STACK_DIR_UP).unwrap();
        assert_eq!(r, StackResolution { dragged_is_member: true, stack: STACK_DIR_HEX });
    }

    #[test]
    fn two_tiles_cannot_stack() {
        assert_eq!(resolve_stack_drop(TILE, TILE, STACK_DIR_UP), None);
    }
}
