//! Shared card placement model — the `Micro` enum + the `flags_bk` stacking /
//! tile-stock bit helpers, lifted out of the `cards` module so both `cards`
//! (owner-sharded real cards) and `regions` (position-sharded tile-cards) build
//! their `Card` tables over ONE model. Pure bit math over raw `micro_location`
//! (u32) and `flags_bk` (u32) — no `ctx.db`, no module-specific `Card` type;
//! each module's table layer is a thin wrapper that reads/writes the two fields.
//!
//! The bit layout comes from `content/cards/flags.json` (via [`crate::flags_core`]),
//! the single source of truth both SpacetimeDB modules and the gateway already share.

use std::sync::OnceLock;

use crate::flags::{flag_bit, flag_field};
use crate::packed::{pack_micro_loose, unpack_micro_loose, STACK_STATE_DEFERRED};

/// A card's micro placement: a stack member of a root, or loose at a cell.
/// `micro_location` is interpreted via the `micro_is_card` bit in `flags_bk`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Micro {
    /// Stack member of `root` (a loose/snapped card) in `branch`
    /// (`STACK_DIR_HEX`/`UP`/`DOWN`, or `STACK_STATE_DEFERRED`) at slot `index`.
    Stacked { root: u32, branch: u8, index: u8 },
    /// Loose at cell `(local_q, local_r)` with within-cell offset `(x, y)` and
    /// `kind` (`LOOSE_HEX`/`LOOSE_RECT`/`SNAP_HEX`/`SNAP_RECT`).
    Loose {
        local_q: u8,
        local_r: u8,
        x: i16,
        y: i16,
        kind: u8,
    },
}

impl Micro {
    /// Loose with no within-cell offset (snapped to the cell center).
    pub fn snap(local_q: u8, local_r: u8, kind: u8) -> Self {
        Micro::Loose {
            local_q,
            local_r,
            x: 0,
            y: 0,
            kind,
        }
    }

    /// A deferred stack member anchored to `host` (resolved at mirror time).
    pub fn deferred(host: u32) -> Self {
        Micro::Stacked {
            root: host,
            branch: STACK_STATE_DEFERRED,
            index: 0,
        }
    }

    /// Compute the `(micro_location, flags_bk)` for this placement given the
    /// card's current `flags_bk` (all non-placement bits preserved). The caller
    /// writes both fields back onto its row.
    pub fn apply(self, flags_bk: u32) -> (u32, u32) {
        let bk = bk_layout();
        match self {
            Micro::Stacked { root, branch, index } => {
                let mut f = flags_bk | bk.micro_is_card;
                f = (f & !bk.stack_state_mask)
                    | (((branch as u32) << bk.stack_state_shift) & bk.stack_state_mask);
                f = (f & !bk.stack_index_mask)
                    | (((index as u32) << bk.stack_index_shift) & bk.stack_index_mask);
                (root, f)
            }
            Micro::Loose { local_q, local_r, x, y, kind } => {
                let ml = pack_micro_loose(local_q, local_r, x, y);
                let mut f = flags_bk & !bk.micro_is_card;
                f = (f & !bk.stack_state_mask)
                    | (((kind as u32) << bk.stack_state_shift) & bk.stack_state_mask);
                f &= !bk.stack_index_mask;
                (ml, f)
            }
        }
    }

    /// Decode a row's current micro placement from its `(micro_location, flags_bk)`.
    pub fn of(micro_location: u32, flags_bk: u32) -> Self {
        let bk = bk_layout();
        if flags_bk & bk.micro_is_card != 0 {
            Micro::Stacked {
                root: micro_location,
                branch: ((flags_bk & bk.stack_state_mask) >> bk.stack_state_shift) as u8,
                index: ((flags_bk & bk.stack_index_mask) >> bk.stack_index_shift) as u8,
            }
        } else {
            let (local_q, local_r, x, y) = unpack_micro_loose(micro_location);
            Micro::Loose {
                local_q,
                local_r,
                x,
                y,
                kind: ((flags_bk & bk.stack_state_mask) >> bk.stack_state_shift) as u8,
            }
        }
    }
}

/// True when `flags_bk` marks `micro_location` as a root card_id (a stack member).
pub fn micro_is_card(flags_bk: u32) -> bool {
    flags_bk & bk_layout().micro_is_card != 0
}

/// The `stack_state` branch/kind value (gated on [`micro_is_card`]).
pub fn stack_branch(flags_bk: u32) -> u8 {
    let bk = bk_layout();
    ((flags_bk & bk.stack_state_mask) >> bk.stack_state_shift) as u8
}

/// The `stack_index` slot value (only meaningful when [`micro_is_card`]).
pub fn stack_index(flags_bk: u32) -> u8 {
    let bk = bk_layout();
    ((flags_bk & bk.stack_index_mask) >> bk.stack_index_shift) as u8
}

/// Read tile-card per-row stock `slot` (0 or 1) from `flags_bk`.
pub fn tile_stock(flags_bk: u32, slot: usize) -> u8 {
    let bk = bk_layout();
    ((flags_bk & bk.tile_stock_mask[slot & 1]) >> bk.tile_stock_shift[slot & 1]) as u8
}

/// Write tile-card per-row stock `slot` (0 or 1) into `flags_bk`, returning the
/// new value (clamped to the 2-bit field width).
pub fn write_tile_stock(flags_bk: u32, slot: usize, value: u8) -> u32 {
    let bk = bk_layout();
    let i = slot & 1;
    (flags_bk & !bk.tile_stock_mask[i])
        | (((value as u32) << bk.tile_stock_shift[i]) & bk.tile_stock_mask[i])
}

/// True when any of the six `flags_bk` hold/refcount fields (`touch_count`,
/// `server_count`, `slot_hold_count`, `slot_share_count`, `drop_hold_count`,
/// `position_hold_count`) is nonzero — i.e. the card is actively held by at
/// least one party. A tile-card with active holds is mid-action and must NOT be
/// demoted. Checking the union mask is equivalent to testing each field > 0.
pub fn has_active_holds(flags_bk: u32) -> bool {
    flags_bk & bk_layout().hold_counts_mask != 0
}

/// True when `flags_state` carries something a bare zone tile slot can't
/// express, so the card must NOT be demoted back into its zone: any of `dead`,
/// `magnetic`, `pos_need`, `pos_want` set, or a nonzero `progress_style`.
/// Mirrors the monolith's `demote_blocking_state_bits` predicate.
pub fn state_blocks_demotion(flags_state: u32) -> bool {
    flags_state & state_layout().demote_blocking != 0
}

/// The `flags_bk` hold/refcount field a card carries for an in-flight action.
/// A promoted tile-card is held with these exactly like any bound card — the
/// recipe's slot verb (`use`/`claim`→`SlotHold`, `share`/`borrow`→`SlotShare`,
/// `position_hold` per the verb) picks which. The `u8` discriminants double as
/// the `kind` selector the gate passes to the regions tile-hold reducers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldField {
    Touch = 0,
    SlotHold = 1,
    SlotShare = 2,
    PositionHold = 3,
}

/// Read a hold/refcount `field` value out of `flags_bk`.
pub fn hold_count(flags_bk: u32, field: HoldField) -> u8 {
    count_field(field).read(flags_bk)
}

/// Read the `drop_hold_count` refcount — the stacking-block gate. Not a
/// [`HoldField`] (it's never an acquirable lease *kind*, only a readable
/// count), so it has its own reader. Used by recipe binding validation.
pub fn drop_hold_count(flags_bk: u32) -> u8 {
    bk_layout().drop_hold.read(flags_bk)
}

/// `cards_state.dead` set?
pub fn is_dead(flags_state: u32) -> bool {
    flags_state & state_layout().dead != 0
}
/// `cards_state.magnetic` set? (lifecycle-locked to a recipe).
pub fn is_magnetic(flags_state: u32) -> bool {
    flags_state & state_layout().magnetic != 0
}
/// `cards_state.is_owned_by_player` set? (the owner chain names a player here).
pub fn is_owned_by_player(flags_state: u32) -> bool {
    flags_state & state_layout().is_owned_by_player != 0
}

/// `flags_bk` with `field` incremented by one (saturating at the field's max).
pub fn increment_hold(flags_bk: u32, field: HoldField) -> u32 {
    let f = count_field(field);
    f.write(flags_bk, f.read(flags_bk) as u32 + 1)
}

/// `flags_bk` with `field` decremented by one (saturating at 0).
pub fn decrement_hold(flags_bk: u32, field: HoldField) -> u32 {
    let f = count_field(field);
    f.write(flags_bk, (f.read(flags_bk) as u32).saturating_sub(1))
}

fn count_field(field: HoldField) -> CountField {
    let bk = bk_layout();
    match field {
        HoldField::Touch => bk.touch,
        HoldField::SlotHold => bk.slot_hold,
        HoldField::SlotShare => bk.slot_share,
        HoldField::PositionHold => bk.position_hold,
    }
}

// `drop_hold` is read via `drop_hold_count`, not through `HoldField` — it is a
// stacking-block gate, never an acquirable lease kind. It still participates in
// `hold_counts_mask` so `has_active_holds` sees it.

// ---- bk layout (from content/cards/flags.json) -------------------------

/// A `flags_bk` refcount field's window: pre-built mask, low-bit shift, and max
/// value (`(1<<width)-1`), for saturating refcount arithmetic.
#[derive(Clone, Copy)]
struct CountField {
    mask: u32,
    shift: u8,
    max: u32,
}

impl CountField {
    fn read(self, flags_bk: u32) -> u8 {
        ((flags_bk & self.mask) >> self.shift) as u8
    }
    fn write(self, flags_bk: u32, value: u32) -> u32 {
        (flags_bk & !self.mask) | ((value.min(self.max) << self.shift) & self.mask)
    }
}

struct BkLayout {
    micro_is_card: u32,
    stack_state_mask: u32,
    stack_state_shift: u8,
    stack_index_mask: u32,
    stack_index_shift: u8,
    tile_stock_mask: [u32; 2],
    tile_stock_shift: [u8; 2],
    /// Union of the six hold/refcount field masks — nonzero iff any hold is held.
    hold_counts_mask: u32,
    touch: CountField,
    slot_hold: CountField,
    slot_share: CountField,
    position_hold: CountField,
    drop_hold: CountField,
}

fn bk_layout() -> &'static BkLayout {
    static L: OnceLock<BkLayout> = OnceLock::new();
    L.get_or_init(|| {
        let bit = |n: &str| {
            1u32 << flag_bit("cards_bk", n)
                .unwrap_or_else(|| panic!("cards/flags.json: missing bk bit {n:?}"))
        };
        let field = |n: &str| {
            flag_field("cards_bk", n)
                .unwrap_or_else(|| panic!("cards/flags.json: missing bk field {n:?}"))
        };
        let count = |n: &str| {
            let f = field(n);
            CountField {
                mask: f.mask(),
                shift: f.shift,
                max: (((1u64 << f.width) - 1) & 0xFFFF_FFFF) as u32,
            }
        };
        let ss = field("stack_state");
        let si = field("stack_index");
        let t0 = field("tile_stock_0");
        let t1 = field("tile_stock_1");
        let hold_counts_mask = field("touch_count").mask()
            | field("server_count").mask()
            | field("slot_hold_count").mask()
            | field("slot_share_count").mask()
            | field("drop_hold_count").mask()
            | field("position_hold_count").mask();
        BkLayout {
            micro_is_card: bit("micro_is_card"),
            stack_state_mask: ss.mask(),
            stack_state_shift: ss.shift,
            stack_index_mask: si.mask(),
            stack_index_shift: si.shift,
            tile_stock_mask: [t0.mask(), t1.mask()],
            tile_stock_shift: [t0.shift, t1.shift],
            hold_counts_mask,
            touch: count("touch_count"),
            slot_hold: count("slot_hold_count"),
            slot_share: count("slot_share_count"),
            position_hold: count("position_hold_count"),
            drop_hold: count("drop_hold_count"),
        }
    })
}

// ---- state layout (cards_state demotion-blocking bits) -----------------

struct StateLayout {
    /// Union of `dead | magnetic | pos_need | pos_want | progress_style` masks.
    demote_blocking: u32,
    /// Individual `cards_state` bits the recipe binding validator gates on.
    dead: u32,
    magnetic: u32,
    is_owned_by_player: u32,
}

fn state_layout() -> &'static StateLayout {
    static L: OnceLock<StateLayout> = OnceLock::new();
    L.get_or_init(|| {
        let bit = |n: &str| {
            1u32 << flag_bit("cards_state", n)
                .unwrap_or_else(|| panic!("cards/flags.json: missing state bit {n:?}"))
        };
        let field = |n: &str| {
            flag_field("cards_state", n)
                .unwrap_or_else(|| panic!("cards/flags.json: missing state field {n:?}"))
        };
        StateLayout {
            demote_blocking: bit("dead")
                | bit("magnetic")
                | bit("pos_need")
                | bit("pos_want")
                | field("progress_style").mask(),
            dead: bit("dead"),
            magnetic: bit("magnetic"),
            is_owned_by_player: bit("is_owned_by_player"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacked_roundtrip() {
        let m = Micro::Stacked { root: 1028, branch: 1, index: 3 };
        let (ml, bk) = m.apply(0);
        assert_eq!(ml, 1028);
        assert!(micro_is_card(bk));
        assert_eq!(stack_branch(bk), 1);
        assert_eq!(stack_index(bk), 3);
        assert_eq!(Micro::of(ml, bk), m);
    }

    #[test]
    fn loose_roundtrip() {
        let m = Micro::Loose { local_q: 2, local_r: 5, x: -3, y: 7, kind: 1 };
        let (ml, bk) = m.apply(0);
        assert!(!micro_is_card(bk));
        assert_eq!(Micro::of(ml, bk), m);
    }

    #[test]
    fn tile_stock_roundtrip() {
        let bk = write_tile_stock(write_tile_stock(0, 0, 3), 1, 2);
        assert_eq!(tile_stock(bk, 0), 3);
        assert_eq!(tile_stock(bk, 1), 2);
    }

    #[test]
    fn hold_count_roundtrip() {
        use HoldField::*;
        assert_eq!(hold_count(0, SlotHold), 0);
        // increment / decrement slot_hold
        let bk = increment_hold(increment_hold(0, SlotHold), SlotHold);
        assert_eq!(hold_count(bk, SlotHold), 2);
        let bk = decrement_hold(bk, SlotHold);
        assert_eq!(hold_count(bk, SlotHold), 1);
        // fields are independent
        let bk = increment_hold(bk, SlotShare);
        assert_eq!(hold_count(bk, SlotShare), 1);
        assert_eq!(hold_count(bk, SlotHold), 1);
        assert!(has_active_holds(bk));
        // decrement floors at zero
        assert_eq!(hold_count(decrement_hold(0, SlotHold), SlotHold), 0);
        // holds don't disturb tile stock and vice versa
        let bk = write_tile_stock(increment_hold(0, SlotHold), 0, 3);
        assert_eq!(tile_stock(bk, 0), 3);
        assert_eq!(hold_count(bk, SlotHold), 1);
        assert_eq!(hold_count(bk, Touch), 0);
    }

    #[test]
    fn demotion_predicates() {
        // A clean, unheld row is demotable.
        assert!(!has_active_holds(0));
        assert!(!state_blocks_demotion(0));
        // Any hold count field nonzero blocks demotion.
        let held = bk_layout().hold_counts_mask & (bk_layout().hold_counts_mask.wrapping_neg());
        assert!(has_active_holds(held));
        // Any demote-blocking state bit blocks demotion.
        let blocked =
            state_layout().demote_blocking & (state_layout().demote_blocking.wrapping_neg());
        assert!(state_blocks_demotion(blocked));
        // tile_stock bits live outside both predicates' masks (a stocked-but-idle
        // tile-card is still demotable).
        assert!(!has_active_holds(write_tile_stock(0, 0, 3)));
    }

    #[test]
    fn apply_preserves_other_bits() {
        // a hold count bit elsewhere in flags_bk survives a placement write.
        let other = 1u32 << 16; // touch_count bit
        let (_ml, bk) = Micro::snap(0, 0, 1).apply(other);
        assert_ne!(bk & other, 0);
    }
}
