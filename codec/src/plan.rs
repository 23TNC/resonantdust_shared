//! The action-plan contract — what the gate's recipe evaluation produces and
//! its apply step consumes.
//!
//! Ported from the legacy `resonantdust_content::recipe_plan` (just the data
//! types — the gate builds these from the vm `Plan` via its own translation, so
//! the legacy `compute_holds` / tape-walking planner doesn't come along). Plan
//! `01_gate_authority_pivot`.

use std::collections::BTreeMap;

/// Per-card hold kinds a recipe claims at apply time. The gate maps each set bit
/// to an `acquire_lease(kind)` reducer call.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoldKinds {
  /// Exclusive claim — `slot_hold`.
  pub slot_hold: bool,
  /// Position pin — `position_hold`.
  pub position_hold: bool,
  /// Shared borrow — `slot_share`.
  pub slot_share: bool,
}

/// A completion-time effect from a recipe's `@output`. The gate maps each to a
/// reducer call future-stamped at `completion_ms`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
  /// Mark a card dead (`destroy_card`).
  Destroy { card_id: u32 },
  /// Spawn a card into an owner's inventory bucket (`create_card`).
  Create { def_key: String, surface: u8, macro_zone: u64, owner_id: u32 },
  /// Spawn a deferred stack member anchored to `host_card_id`.
  CreateDeferred { def_key: String, host_card_id: u32 },
  /// Mutate the synthetic tile's per-cell stock `slot` (`set_tile_stock`) — the
  /// zone-savable u4 path.
  ModifyTileStock { slot: u8, op: StockOp, delta: u8 },
  /// Set a bound CARD's full per-card `stock` u32 to a gate-computed value
  /// (current stock with one slot's bits replaced). The card holds the result —
  /// the upper 28 bits are card-only (transient unless the card persists), only
  /// the bottom u4 ever round-trips to a zone tile.
  SetCardStock { card_id: u32, stock: u32 },
  /// Set a blueprint's discovery bit on the target soul (`unlock_blueprint`).
  /// `blueprint_id` is the Bundle's blueprint id (the discovery-bit index),
  /// resolved from the recipe's `$blueprint::<key>` ref at plan-translation time.
  UnlockBlueprint { blueprint_id: u16, target_card_id: u32 },
}

/// Tile-stock arithmetic for [`Effect::ModifyTileStock`]. `code()` is the u8 the
/// gate passes to the regions `set_tile_stock` reducer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StockOp {
  Sub,
  Add,
  Set,
}

impl StockOp {
  pub fn code(self) -> u8 {
    match self {
      StockOp::Sub => 0,
      StockOp::Add => 1,
      StockOp::Set => 2,
    }
  }
}

/// The action plan the gate applies: duration + per-card holds + the synthetic
/// tile's holds + ordered effects + progress styles.
#[derive(Clone, Debug, Default)]
pub struct ActionPlan {
  /// `card_id → progress_style` for completion-row progress bars.
  pub styles: BTreeMap<u32, u8>,
  /// Action duration in seconds (`sys.duration`).
  pub duration: u32,
  /// Completion-time effects, in tape order.
  pub effects: Vec<Effect>,
  /// Per-card holds the action claims.
  pub holds: BTreeMap<u32, HoldKinds>,
  /// Holds for the action's synthetic tile (the sentinel-`0` slot), if any.
  pub tile_holds: Option<HoldKinds>,
}

impl ActionPlan {
  /// Milliseconds from start to completion (`duration * 1000`).
  pub fn duration_ms(&self) -> u64 {
    (self.duration as u64) * 1000
  }
}
