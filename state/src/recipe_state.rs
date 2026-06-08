//! Recipe binding **state** validation — the stack/world checks orthogonal to
//! recipe semantics: every bound card exists, is not dead, is not held by a
//! conflicting in-flight action, and appears only once.
//!
//! Ownership is **not** gated here: any valid recipe may bind any cards
//! regardless of who owns them — a future Permissions system will decide which
//! souls may act on which cards. (Ownership is still enforced for *moving /
//! stacking* cards, in [`crate::stack`]'s `plan_place` / the `place_card` path.)
//!
//! Card reads go through the [`CardStore`] trait; the gate implements it over its
//! gathered snapshot. Flag reads delegate to [`resonantdust_codec::card_model`] (the single
//! owner of the bit layout) — this module keeps no layout of its own.

use resonantdust_codec::card_model::{bind_blocked, drop_hold_count, hold_count, is_dead, HoldField};
use resonantdust_codec::packed::is_player_soul;
use std::collections::BTreeSet;

/// The card fields recipe state-validation reads. The gate fills these from its
/// gathered snapshot; tests from a mock. `flags` is the propagating word (state
/// bits + refcount holds).
#[derive(Clone, Debug)]
pub struct CardView {
  pub card_id: u32,
  pub owner_id: u32,
  pub micro_location: u32,
  pub macro_zone: u64,
  pub packed_definition: u16,
  pub flags: u32,
  /// Per-card variable data (the `stock` u32) — decoded per the def's stock
  /// schema into recipe-readable aspects.
  pub stock: u32,
}

/// Point-in-time card reads. The gate implements this over its snapshot.
pub trait CardStore {
  /// The card's state as of `time_ms` (latest version ≤ time), or `None`.
  fn card_at(&self, card_id: u32, time_ms: u64) -> Option<CardView>;
}

/// World-anonymous player id (cards whose owner chain bottoms out at 0).
pub const WORLD_PLAYER_ID: u32 = 0;
const OWNER_WALK_DEPTH_CAP: u32 = 32;
const TOUCH_COUNT_CLIENT_CAP: u32 = 3;

/// Walk a card's `owner_id` chain to the responsible player. A **player_soul**
/// card (identified by definition — `is_player_soul`, the reserved 0xFFF0..=0xFFFF
/// range) names its player directly in `owner_id`; otherwise the owner is another
/// card and we recurse. A chain bottoming out at owner 0 resolves to
/// [`WORLD_PLAYER_ID`]. `None` only on a missing row or a cycle past the depth
/// cap.
pub fn owning_player<S: CardStore>(store: &S, card_id: u32, now_ms: u64) -> Option<u32> {
  let mut cur = card_id;
  for _ in 0..OWNER_WALK_DEPTH_CAP {
    let row = store.card_at(cur, now_ms)?;
    if is_player_soul(row.packed_definition) {
      return Some(row.owner_id);
    }
    if row.owner_id == 0 {
      return Some(WORLD_PLAYER_ID);
    }
    cur = row.owner_id;
  }
  None
}

/// Stack-vs-world validation over the gathered snapshot. `Ok(())` means every
/// bound card passes; `Err` names the failing card + reason.
///
/// `wants_exclusive(card_id)` — does *this* recipe claim an exclusive (`use` /
/// `claim`) hold on the card? The gate derives it from the DSL plan's per-card
/// [`resonantdust_codec::plan::HoldKinds::slot_hold`]. When true, a card already *borrow*-held
/// by another action is a conflict ("cannot claim"); an exclusive (claim) hold by
/// another action always conflicts regardless.
///
/// NB: over a gathered snapshot the hold-count gates are best-effort (a TOCTOU
/// window exists vs. concurrent gates); the per-shard dedup + lease reducers are
/// the exact race guard.
pub fn validate_bindings<S: CardStore>(
  store: &S,
  _recipe_id: u16,
  root: u32,
  bindings: &[Vec<u32>],
  _caller_player_id: u32,
  now_ms: u64,
  wants_exclusive: impl Fn(u32) -> bool,
) -> Result<(), String> {
  // Root: dead + hold-kind + touch + drop gates.
  if root != 0 {
    let card = store.card_at(root, now_ms).ok_or_else(|| format!("root card {root} not found"))?;
    check_card(&card, root, wants_exclusive(root))?;
  }

  let mut seen: BTreeSet<u32> = BTreeSet::new();
  for binding_row in bindings.iter() {
    for &card_id in binding_row.iter() {
      if card_id == 0 {
        continue;
      }
      if !seen.insert(card_id) {
        return Err(format!("card {card_id} appears more than once in bindings"));
      }
      let card = store.card_at(card_id, now_ms).ok_or_else(|| format!("card {card_id} not found"))?;
      check_card(&card, card_id, wants_exclusive(card_id))?;
      // Ownership intentionally NOT checked — Permissions will gate this. See the
      // module docs; `_caller_player_id` is kept for that future use.
    }
  }
  Ok(())
}

/// The per-card dead / hold-conflict / touch / drop gate, shared by the root and
/// binding-row checks.
fn check_card(card: &CardView, card_id: u32, wants_exclusive: bool) -> Result<(), String> {
  // Verb-independent baseline (dead or exclusively claimed) — the SAME predicate
  // the client matcher applies (`bind_blocked`), so the matcher never proposes a
  // binding the gate would reject here.
  if bind_blocked(card.flags) {
    return Err(format!(
      "card {card_id} unavailable: {}",
      if is_dead(card.flags) { "dead" } else { "exclusively held by another in-flight action" }
    ));
  }
  if wants_exclusive && hold_count(card.flags, HoldField::SlotBorrow) > 0 {
    return Err(format!("card {card_id} is borrow-held by another in-flight action; cannot claim"));
  }
  if u32::from(hold_count(card.flags, HoldField::Touch)) >= TOUCH_COUNT_CLIENT_CAP {
    return Err(format!(
      "card {card_id} has too many concurrent in-flight actions (cap {TOUCH_COUNT_CLIENT_CAP})"
    ));
  }
  if drop_hold_count(card.flags) > 0 {
    return Err(format!("card {card_id} blocks stacking (drop_hold_count > 0)"));
  }
  Ok(())
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
  use super::*;
  use resonantdust_codec::flags::{flag_bit, flag_field};
  use std::collections::HashMap;

  // Build flag values from the real layout (the same source card_model reads),
  // so the tests exercise the live bit positions.
  fn state_bit(name: &str) -> u32 {
    1u32 << flag_bit("flags", name).unwrap()
  }
  fn hold_one(field: &str) -> u32 {
    let f = flag_field("flags", field).unwrap();
    (1u32 << f.shift) & f.mask()
  }

  struct Mock(HashMap<u32, CardView>);
  impl CardStore for Mock {
    fn card_at(&self, id: u32, _t: u64) -> Option<CardView> {
      self.0.get(&id).cloned()
    }
  }
  fn card(id: u32, owner: u32, flags: u32) -> CardView {
    CardView { card_id: id, owner_id: owner, micro_location: 0, macro_zone: 0, packed_definition: 0, flags, stock: 0 }
  }
  fn store(cards: &[CardView]) -> Mock {
    Mock(cards.iter().cloned().map(|c| (c.card_id, c)).collect())
  }
  fn no_excl(_: u32) -> bool {
    false
  }

  #[test]
  fn world_owned_ok() {
    let s = store(&[card(50, 0, 0)]);
    validate_bindings(&s, 1, 0, &[vec![50]], 7, 0, no_excl).expect("ok");
  }

  #[test]
  fn dup_rejected() {
    let s = store(&[card(50, 0, 0)]);
    let err = validate_bindings(&s, 1, 0, &[vec![50, 50]], 7, 0, no_excl).unwrap_err();
    assert!(err.contains("more than once"), "{err}");
  }

  #[test]
  fn dead_rejected() {
    let s = store(&[card(50, 0, state_bit("dead"))]);
    let err = validate_bindings(&s, 1, 0, &[vec![50]], 7, 0, no_excl).unwrap_err();
    assert!(err.contains("dead"), "{err}");
  }

  #[test]
  fn foreign_owner_allowed() {
    // Ownership is no longer gated here (Permissions will): a card owned by
    // player 9 binds fine for caller 7, as long as its state is otherwise valid.
    let s = store(&[card(50, 9, 0)]);
    validate_bindings(&s, 1, 0, &[vec![50]], 7, 0, no_excl).expect("foreign owner allowed");
  }

  #[test]
  fn exclusive_held_always_rejected() {
    let s = store(&[card(50, 0, hold_one("slot_claim_count"))]);
    let err = validate_bindings(&s, 1, 0, &[vec![50]], 7, 0, no_excl).unwrap_err();
    assert!(err.contains("exclusively held"), "{err}");
  }

  #[test]
  fn borrow_held_rejected_only_when_claiming_exclusive() {
    let s = store(&[card(50, 0, hold_one("slot_borrow_count"))]);
    // not claiming exclusive → borrow hold is fine
    validate_bindings(&s, 1, 0, &[vec![50]], 7, 0, no_excl).expect("borrow ok");
    // claiming exclusive on 50 → conflict
    let err = validate_bindings(&s, 1, 0, &[vec![50]], 7, 0, |id| id == 50).unwrap_err();
    assert!(err.contains("borrow-held"), "{err}");
  }
}
