//! Shared card placement + flag model — the `Micro` enum plus the bit helpers
//! over the cards table's three flag columns, lifted out of the module layer so
//! both the shard module and the gateway build over ONE model. Pure bit math; no
//! `ctx.db`, no module-specific `Card` type.
//!
//! The bit layout comes from [`crate::flags`], the single source of truth.
//!
//! # Columns
//!
//! - `flags` (u32) — **propagating**: state bits, placement (`stack`/`index`),
//!   and the refcount holds. All helpers here that take a `flags: u32` read this
//!   word.
//! - `flags_bk` (u8) — non-propagating dirty/preserve markers (owned by the
//!   module's `write_at`; no helpers here).
//! - `stock` (u8) — tile-card stock slots, via [`stock`] / [`write_stock`].
//!
//! # Stack semantics
//!
//! `stack` (bits 0-3 of `flags`) is the discriminator:
//!   - `stack == 0` → **loose**: `micro_location` is packed cell coords + offset;
//!     `index` (bits 4-7) is unused. Every surface is a uniform hex cell — there
//!     is no loose "kind"; snapped-vs-free is a render concern (zero offset).
//!   - `stack != 0` → **stack member**: `micro_location` is the root card_id,
//!     `stack` is `branch + 1` (so the loose sentinel stays distinct), and
//!     `index` is the slot within the branch (0..15).

use std::sync::OnceLock;

use crate::flags::{flag_bit, flag_field};
use crate::packed::{pack_micro_loose, unpack_micro_loose, STACK_STATE_DEFERRED};

/// A card's micro placement: a stack member of a root, or loose at a cell.
/// Interpreted via the `stack` field in `flags` (0 = loose).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Micro {
    /// Stack member of `root` (a loose/snapped card) in `branch`
    /// (`STACK_DIR_HEX`/`UP`/`DOWN`, or `STACK_STATE_DEFERRED`) at slot `index`.
    Stacked { root: u32, branch: u8, index: u8 },
    /// Loose at cell `(local_q, local_r)` with a within-cell offset `(x, y)`.
    /// Every surface is a uniform hex cell; "snapped vs free" is render-only (a
    /// snapped card carries a zero offset), so there is no `kind`.
    Loose { local_q: u8, local_r: u8, x: i16, y: i16 },
}

impl Micro {
    /// Loose with no within-cell offset (snapped to the cell center).
    pub fn snap(local_q: u8, local_r: u8) -> Self {
        Micro::Loose { local_q, local_r, x: 0, y: 0 }
    }

    /// A deferred stack member anchored to `host` (resolved at mirror time).
    pub fn deferred(host: u32) -> Self {
        Micro::Stacked {
            root: host,
            branch: STACK_STATE_DEFERRED,
            index: 0,
        }
    }

    /// Compute the `(micro_location, flags)` for this placement given the card's
    /// current `flags` (all non-placement bits — state + refcounts — preserved).
    /// The caller writes both fields back onto its row.
    pub fn apply(self, flags: u32) -> (u32, u32) {
        let l = layout();
        let cleared = flags & !(l.stack.mask | l.index.mask);
        match self {
            Micro::Stacked { root, branch, index } => {
                // Store `branch + 1` so the `stack == 0` loose sentinel is distinct.
                let stack = (branch as u32).saturating_add(1);
                let f = cleared | l.stack.pack(stack) | l.index.pack(index as u32);
                (root, f)
            }
            Micro::Loose { local_q, local_r, x, y } => {
                // stack stays 0 (the loose sentinel); `index` is left clear.
                (pack_micro_loose(local_q, local_r, x, y), cleared)
            }
        }
    }

    /// Decode a row's current micro placement from its `(micro_location, flags)`.
    pub fn of(micro_location: u32, flags: u32) -> Self {
        let l = layout();
        let stack = l.stack.read(flags);
        if stack != 0 {
            Micro::Stacked {
                root: micro_location,
                branch: (stack - 1) as u8,
                index: l.index.read(flags) as u8,
            }
        } else {
            let (local_q, local_r, x, y) = unpack_micro_loose(micro_location);
            Micro::Loose { local_q, local_r, x, y }
        }
    }
}

/// True when `flags` marks `micro_location` as a root card_id (a stack member) —
/// i.e. the `stack` field is nonzero.
pub fn micro_is_card(flags: u32) -> bool {
    layout().stack.read(flags) != 0
}

/// The stack `branch` value (gated on [`micro_is_card`]; meaningless when loose).
pub fn stack_branch(flags: u32) -> u8 {
    let s = layout().stack.read(flags);
    if s == 0 { 0 } else { (s - 1) as u8 }
}

/// The `index` slot value within a branch (only meaningful when [`micro_is_card`]).
pub fn stack_index(flags: u32) -> u8 {
    layout().index.read(flags) as u8
}

/// Bitmask of the placement fields (`stack` + `index`) within `flags`. A change
/// inside this mask is a position change (drives `position_dirty`).
pub fn placement_mask() -> u32 {
    let l = layout();
    l.stack.mask | l.index.mask
}

/// Bitmask of the bit-diff-propagated state bits within `flags` (`dead`,
/// `pos_need`, `pos_want`, `surface_locked`, `player_owned`, `zone_born`). These
/// are the only bits the module's forward bit-diff propagator carries; refcounts
/// (delta-propagated) and placement (per-row) are deliberately excluded. Also the
/// set the `data_dirty` diff keys on.
pub fn state_mask() -> u32 {
    layout().state_mask
}

/// The `stock` column is a full `u32` of per-card variable data — pack it however
/// you like (16 u2s, 4 u8s, a u8 progress counter + flags, …). Nothing is special
/// about any bits EXCEPT this: only the **bottom 4 bits** (`STOCK_ZONE_SAVE_BITS`,
/// the two legacy u2 slots) can be persisted back into a zone tile slot — a zone
/// tile only stores a u4. The upper 28 bits are card-only (transient unless the
/// card itself persists). [`stock`] / [`write_stock`] access the two u2 zone-saved
/// slots; read/write the upper bits with your own masks.
pub fn stock(stock: u32, slot: usize) -> u8 {
    let f = stock_field(slot);
    ((stock & f.mask) >> f.shift) as u8
}

/// Write zone-savable stock `slot` (0 or 1, the bottom u4) into the `stock` word,
/// returning the new word (value clamped to the 2-bit field width; all other bits
/// preserved).
pub fn write_stock(stock: u32, slot: usize, value: u8) -> u32 {
    let f = stock_field(slot);
    (stock & !f.mask) | ((value as u32).min(f.max) << f.shift)
}

/// Mask of the stock bits that can be saved back to a zone tile slot (the bottom
/// u4 — a zone tile only persists a u4). The upper 28 bits are card-only.
pub const STOCK_ZONE_SAVE_MASK: u32 = 0x0000_000F;

/// True when any refcount hold field (`touch_count`, `server_count`,
/// `slot_claim_count`, `slot_borrow_count`, `drop_hold_count`,
/// `position_hold_count`) in `flags` is nonzero — i.e. the card is actively held
/// by at least one party. A tile-card with active holds is mid-action and must
/// NOT be demoted.
pub fn has_active_holds(flags: u32) -> bool {
    flags & layout().hold_counts_mask != 0
}

/// True when `flags` carries something a bare zone tile slot can't express, so
/// the card must NOT be demoted back into its zone: any of `dead`, `pos_need`,
/// `pos_want`.
pub fn state_blocks_demotion(flags: u32) -> bool {
    flags & layout().demote_blocking != 0
}

/// A refcount count field in `flags`. The lease-acquiring recipe verbs map to
/// `SlotClaim` (`use`/`claim`), `SlotBorrow` (`share`/`borrow`), `PositionHold`,
/// and `Touch` (rides along on any held card). `DropHold` (stacking-block) and
/// `Server` (parallel-reducer safety) are server-internal counts that are still
/// incremented/decremented through the same machinery. The `u8` discriminants
/// double as the `kind` selector the gate passes to the tile-hold reducers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldField {
    Touch = 0,
    SlotClaim = 1,
    SlotBorrow = 2,
    PositionHold = 3,
    DropHold = 4,
    Server = 5,
}

/// Read a hold/refcount `field` value out of `flags`.
pub fn hold_count(flags: u32, field: HoldField) -> u8 {
    count_field(field).read(flags) as u8
}

/// Read the `drop_hold_count` refcount — the stacking-block gate. Not a
/// [`HoldField`] (it's never an acquirable lease *kind*, only a readable
/// count), so it has its own reader. Used by recipe binding validation.
pub fn drop_hold_count(flags: u32) -> u8 {
    layout().drop_hold.read(flags) as u8
}

/// `flags.dead` set?
pub fn is_dead(flags: u32) -> bool {
    flags & layout().dead != 0
}

/// `flags.pos_need` — the server REQUIRES this card's position (a spawn or an
/// authoritative relocation). A client with a pending local move must adopt it.
pub fn pos_need(flags: u32) -> bool {
    flags & layout().pos_need != 0
}

/// `flags.pos_want` — the server's position is advisory (set on recipe outputs).
/// A client holding a pending local move (dirty position) keeps its own.
pub fn pos_want(flags: u32) -> bool {
    flags & layout().pos_want != 0
}

/// Whether a card is ineligible to be bound into ANY new action, verb-independent:
/// it's `dead` (destroyed) or exclusively `slot_claim`-held by an in-flight action.
/// The shared baseline both sides apply — the gate's `check_card` gates on it
/// before its verb-specific checks (borrow / touch cap), and the client matcher
/// skips such cards so it never proposes what the gate would reject. (A borrow
/// hold only blocks *exclusive* claims, so it's verb-dependent and lives in
/// `check_card`, not here.)
pub fn bind_blocked(flags: u32) -> bool {
    is_dead(flags) || hold_count(flags, HoldField::SlotClaim) > 0
}
// `player_owned` flag RETIRED (bit 24 reclaimed): the player_soul is identified
// by its DEFINITION now (`packed::is_player_soul`, the reserved 0xFFF0..=0xFFFF
// range), not a flag — so the owner-walk terminus needs no propagating bit.

/// `flags` with `field` incremented by one (saturating at the field's max).
pub fn increment_hold(flags: u32, field: HoldField) -> u32 {
    count_field(field).increment(flags)
}

/// `flags` with `field` decremented by one (saturating at 0).
pub fn decrement_hold(flags: u32, field: HoldField) -> u32 {
    count_field(field).decrement(flags)
}

fn count_field(field: HoldField) -> CountField {
    let l = layout();
    match field {
        HoldField::Touch => l.touch,
        HoldField::SlotClaim => l.slot_claim,
        HoldField::SlotBorrow => l.slot_borrow,
        HoldField::PositionHold => l.position_hold,
        HoldField::DropHold => l.drop_hold,
        HoldField::Server => l.server,
    }
}

// ---- layout (from crate::flags) ----------------------------------------

/// A refcount / placement field's window: pre-built mask, low-bit shift, and max
/// value (`(1<<width)-1`), for saturating arithmetic.
#[derive(Clone, Copy)]
struct CountField {
    mask: u32,
    shift: u8,
    max: u32,
}

impl CountField {
    fn read(self, flags: u32) -> u32 {
        (flags & self.mask) >> self.shift
    }
    fn pack(self, value: u32) -> u32 {
        (value.min(self.max) << self.shift) & self.mask
    }
    fn write(self, flags: u32, value: u32) -> u32 {
        (flags & !self.mask) | self.pack(value)
    }
    fn increment(self, flags: u32) -> u32 {
        let next = self.read(flags).saturating_add(1).min(self.max);
        self.write(flags, next)
    }
    fn decrement(self, flags: u32) -> u32 {
        let next = self.read(flags).saturating_sub(1);
        self.write(flags, next)
    }
}

struct FlagsLayout {
    stack: CountField,
    index: CountField,
    // refcount holds
    touch: CountField,
    slot_claim: CountField,
    slot_borrow: CountField,
    position_hold: CountField,
    drop_hold: CountField,
    server: CountField,
    /// Union of all six refcount field masks — nonzero iff any hold is held.
    hold_counts_mask: u32,
    // single-bit state
    dead: u32,
    /// `flags.pos_need` — the server REQUIRES this position (authoritative).
    pos_need: u32,
    /// `flags.pos_want` — the server's position is advisory (a recipe output);
    /// a client holding a pending local move keeps its own.
    pos_want: u32,
    /// Union of the bit-diff-propagated state bits.
    state_mask: u32,
    /// `dead | pos_need | pos_want` — blocks demotion back to a tile slot.
    demote_blocking: u32,
}

fn layout() -> &'static FlagsLayout {
    static L: OnceLock<FlagsLayout> = OnceLock::new();
    L.get_or_init(|| {
        let bit = |n: &str| {
            1u32 << flag_bit("flags", n).unwrap_or_else(|| panic!("flags: missing bit {n:?}"))
        };
        let field = |n: &str| {
            let f =
                flag_field("flags", n).unwrap_or_else(|| panic!("flags: missing field {n:?}"));
            CountField {
                mask: f.mask(),
                shift: f.shift,
                max: (((1u64 << f.width) - 1) & 0xFFFF_FFFF) as u32,
            }
        };
        let touch = field("touch_count");
        let slot_claim = field("slot_claim_count");
        let slot_borrow = field("slot_borrow_count");
        let position_hold = field("position_hold_count");
        let drop_hold = field("drop_hold_count");
        let server = field("server_count");
        let hold_counts_mask = touch.mask
            | slot_claim.mask
            | slot_borrow.mask
            | position_hold.mask
            | drop_hold.mask
            | server.mask;
        let dead = bit("dead");
        let pos_need = bit("pos_need");
        let pos_want = bit("pos_want");
        let surface_locked = bit("surface_locked");
        let zone_born = bit("zone_born");
        FlagsLayout {
            stack: field("stack"),
            index: field("index"),
            touch,
            slot_claim,
            slot_borrow,
            position_hold,
            drop_hold,
            server,
            hold_counts_mask,
            dead,
            pos_need,
            pos_want,
            state_mask: dead | pos_need | pos_want | surface_locked | zone_born,
            demote_blocking: dead | pos_need | pos_want,
        }
    })
}

fn stock_field(slot: usize) -> CountField {
    let name = if slot & 1 == 0 { "stock_0" } else { "stock_1" };
    let f = flag_field("stock", name).unwrap_or_else(|| panic!("stock: missing field {name:?}"));
    CountField {
        mask: f.mask(),
        shift: f.shift,
        max: (((1u64 << f.width) - 1) & 0xFFFF_FFFF) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacked_roundtrip() {
        let m = Micro::Stacked { root: 1028, branch: 1, index: 3 };
        let (ml, f) = m.apply(0);
        assert_eq!(ml, 1028);
        assert!(micro_is_card(f));
        assert_eq!(stack_branch(f), 1);
        assert_eq!(stack_index(f), 3);
        assert_eq!(Micro::of(ml, f), m);
    }

    #[test]
    fn loose_roundtrip() {
        let m = Micro::Loose { local_q: 2, local_r: 5, x: -3, y: 7 };
        let (ml, f) = m.apply(0);
        assert!(!micro_is_card(f));
        assert_eq!(stack_index(f), 0); // index unused when loose
        assert_eq!(Micro::of(ml, f), m);
    }

    #[test]
    fn deferred_branch_roundtrips() {
        let m = Micro::deferred(2048);
        let (ml, f) = m.apply(0);
        assert_eq!(ml, 2048);
        assert!(micro_is_card(f));
        assert_eq!(stack_branch(f), STACK_STATE_DEFERRED);
        assert_eq!(Micro::of(ml, f), m);
    }

    #[test]
    fn apply_preserves_state_and_holds() {
        // A state bit and a hold count survive a placement write.
        let base = layout().dead | increment_hold(0, HoldField::Touch);
        let (_ml, f) = Micro::snap(0, 0).apply(base);
        assert!(is_dead(f));
        assert_eq!(hold_count(f, HoldField::Touch), 1);
    }

    #[test]
    fn stock_roundtrip() {
        let s = write_stock(write_stock(0, 0, 3), 1, 2);
        assert_eq!(stock(s, 0), 3);
        assert_eq!(stock(s, 1), 2);
    }

    #[test]
    fn hold_count_roundtrip() {
        use HoldField::*;
        assert_eq!(hold_count(0, SlotClaim), 0);
        let f = increment_hold(increment_hold(0, SlotClaim), SlotClaim);
        assert_eq!(hold_count(f, SlotClaim), 2);
        let f = decrement_hold(f, SlotClaim);
        assert_eq!(hold_count(f, SlotClaim), 1);
        // fields are independent
        let f = increment_hold(f, SlotBorrow);
        assert_eq!(hold_count(f, SlotBorrow), 1);
        assert_eq!(hold_count(f, SlotClaim), 1);
        assert!(has_active_holds(f));
        // decrement floors at zero
        assert_eq!(hold_count(decrement_hold(0, SlotClaim), SlotClaim), 0);
    }

    #[test]
    fn demotion_predicates() {
        assert!(!has_active_holds(0));
        assert!(!state_blocks_demotion(0));
        let held = increment_hold(0, HoldField::Touch);
        assert!(has_active_holds(held));
        assert!(state_blocks_demotion(layout().dead));
        // holds and placement live outside the demote-blocking mask.
        let (_ml, placed) = Micro::snap(0, 0).apply(0);
        assert!(!state_blocks_demotion(placed));
    }

    #[test]
    fn placement_and_state_masks_disjoint() {
        // Placement (bits 0-7) and state bits (24+) must not overlap.
        assert_eq!(placement_mask() & state_mask(), 0);
        // Refcounts (8-23) must not overlap either.
        assert_eq!(placement_mask() & layout().hold_counts_mask, 0);
        assert_eq!(state_mask() & layout().hold_counts_mask, 0);
    }
}
