//! Canonical stacking eligibility — the bundle-free core, shared by the client
//! (drag/feasibility), the gate, and the placement model (`state::stack`). The
//! `bundle → StackBits` lookup lives in the bundle-aware layer
//! (`rules::stacking::stack_bits`); everything here is pure bit math over
//! `StackBits`, so the lowest crate can host it (no circular `state → rules`).
//!
//! **Bit-fields are indexed by `stack_id`** (the value stored in a card's
//! `stack_state` nibble): bit `i` = stack `i`, where
//!   - `0` = loose (the root sentinel — never hosted or joined),
//!   - `1` = hex / under-root (where a tile mounts),
//!   - `2` = top, `3` = bottom.
//!
//!   - `hosts` — stacks this card SOURCES as a root (slots others attach to)
//!   - `joins` — stacks this card can OCCUPY as a member
//!
//! Drop `a` onto `b`: try `a` joins `b` (`b.hosts & a.joins`), else `b` joins
//! `a` (`a.hosts & b.joins`). Lowest stack wins (hex 1 preferred); the drop
//! direction breaks a top/bottom tie. The root is always the host side — which
//! is what makes a card-onto-tile drop push the *tile* into the card's stack 1.

use crate::packed::STACK_DIR_DOWN;

/// Stack ids (== the `stack_state` nibble value; 0 = loose).
pub const STACK_HEX: u8 = 1;
pub const STACK_TOP: u8 = 2;
pub const STACK_BOTTOM: u8 = 3;

/// A card's stacking bit-fields, indexed by `stack_id` (bit `i` = stack `i`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackBits {
    pub hosts: u8,
    pub joins: u8,
}

/// Regular card: hosts hex+top+bottom (`0b1110`), joins top+bottom (`0b1100`).
/// Bit 0 (loose) is never set — a card neither hosts nor joins the loose stack.
pub const DEFAULT_BITS: StackBits = StackBits { hosts: 0b1110, joins: 0b1100 };

/// Tile/event: hosts nothing, joins only the hex stack (`0b0010`).
pub const TILE_BITS: StackBits = StackBits { hosts: 0b0000, joins: 0b0010 };

#[inline]
fn bit(stack: u8) -> u8 {
    1 << stack
}

/// The `stack_id` a `joiner` occupies on a `host`, or `None` if none. Lowest
/// stack wins (hex first); `drop_dir` (`STACK_DIR_UP`/`DOWN`) breaks a top+bottom
/// tie. Returns a `stack_id` (`STACK_HEX`/`TOP`/`BOTTOM`).
pub fn match_stack(host: StackBits, joiner: StackBits, drop_dir: u8) -> Option<u8> {
    let m = host.hosts & joiner.joins;
    if m == 0 {
        return None;
    }
    if m & bit(STACK_HEX) != 0 {
        return Some(STACK_HEX);
    }
    let top = m & bit(STACK_TOP) != 0;
    let bottom = m & bit(STACK_BOTTOM) != 0;
    if top && bottom {
        return Some(if drop_dir == STACK_DIR_DOWN { STACK_BOTTOM } else { STACK_TOP });
    }
    if top {
        return Some(STACK_TOP);
    }
    if bottom {
        return Some(STACK_BOTTOM);
    }
    None
}

/// The outcome of a drop resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackResolution {
    /// `true` → the dragged card joins the target (target is root): the normal
    /// card-onto-card stack. `false` → the target joins the dragged card (dragged
    /// is root, target absorbed into its stack 1): the card-onto-tile case.
    pub dragged_is_member: bool,
    /// `stack_id` the member occupies on the root.
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
    use crate::packed::STACK_DIR_UP;

    #[test]
    fn regular_onto_regular_uses_drop_dir_for_top_bottom() {
        let up = resolve_stack_drop(DEFAULT_BITS, DEFAULT_BITS, STACK_DIR_UP).unwrap();
        assert_eq!(up, StackResolution { dragged_is_member: true, stack: STACK_TOP });
        let down = resolve_stack_drop(DEFAULT_BITS, DEFAULT_BITS, STACK_DIR_DOWN).unwrap();
        assert_eq!(down, StackResolution { dragged_is_member: true, stack: STACK_BOTTOM });
    }

    #[test]
    fn card_onto_tile_pushes_tile_into_card_hex() {
        // Forward fails (tile hosts nothing); reverse makes the card the root and
        // the tile joins its hex stack (1).
        let r = resolve_stack_drop(DEFAULT_BITS, TILE_BITS, STACK_DIR_UP).unwrap();
        assert_eq!(r, StackResolution { dragged_is_member: false, stack: STACK_HEX });
    }

    #[test]
    fn dragging_tile_onto_card_joins_hex_forward() {
        let r = resolve_stack_drop(TILE_BITS, DEFAULT_BITS, STACK_DIR_UP).unwrap();
        assert_eq!(r, StackResolution { dragged_is_member: true, stack: STACK_HEX });
    }

    #[test]
    fn two_tiles_cannot_stack() {
        assert_eq!(resolve_stack_drop(TILE_BITS, TILE_BITS, STACK_DIR_UP), None);
    }
}
