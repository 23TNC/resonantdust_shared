//! Recipe binding **state** validation — the stack/world checks orthogonal to
//! recipe semantics: every bound card exists, is not dead, is not held by a
//! conflicting in-flight action, is owned by the caller (or world), appears only
//! once, and — if magnetic — is locked to this recipe.
//!
//! Ported from the legacy `resonantdust_content::recipe_validate::validate_bindings`
//! (the `@input` predicate matcher — `validate_input`/`verify_stmt`/`resolve_target`
//! — does NOT come along; the DSL vm replaces it, cf. `crate::vm` + the gate's
//! `dsl_recipe`). The two things the port took from the legacy `Recipe` object —
//! "does this recipe claim *exclusive* on a card" and "what recipe is a magnetic
//! def locked to" — are now closures the caller supplies from the DSL plan +
//! [`crate::loader::Bundle`], so this module has no recipe-registry dependency.
//!
//! Card reads go through the [`CardStore`] trait; the gate implements it over its
//! gathered snapshot. Flag reads delegate to [`crate::card_model`] (the single
//! owner of the `flags.json` bit layout) — this module keeps no layout of its own.

use crate::card_model::{drop_hold_count, hold_count, is_dead, is_magnetic, is_owned_by_player, HoldField};
use std::collections::BTreeSet;

/// The card fields recipe state-validation reads. The gate fills these from its
/// gathered snapshot; tests from a mock.
#[derive(Clone, Debug)]
pub struct CardView {
  pub card_id: u32,
  pub owner_id: u32,
  pub micro_location: u32,
  pub macro_zone: u64,
  pub packed_definition: u16,
  pub flags_state: u32,
  pub flags_bk: u32,
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

/// Walk a card's `owner_id` chain to the responsible player. A card carrying
/// `is_owned_by_player` names its player directly in `owner_id`; otherwise the
/// owner is another card and we recurse. A chain bottoming out at owner 0
/// resolves to [`WORLD_PLAYER_ID`]. `None` only on a missing row or a cycle past
/// the depth cap.
pub fn owning_player<S: CardStore>(store: &S, card_id: u32, now_ms: u64) -> Option<u32> {
  let mut cur = card_id;
  for _ in 0..OWNER_WALK_DEPTH_CAP {
    let row = store.card_at(cur, now_ms)?;
    if is_owned_by_player(row.flags_state) {
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
/// [`crate::plan::HoldKinds::slot_hold`]. When true, a card already *shared*-held
/// by another action is a conflict ("cannot claim"); an exclusive hold by another
/// action always conflicts regardless.
///
/// `magnetic_recipe(packed_def)` — the recipe id a magnetic-flagged def is locked
/// to (the gate reads it from the Bundle: a card's `&magnetic.recipe` ref). A
/// magnetic card may only be consumed by its own lifecycle recipe.
///
/// NB: over a gathered snapshot the hold-count gates are best-effort (a TOCTOU
/// window exists vs. concurrent gates); the per-shard dedup + lease reducers are
/// the exact race guard.
#[allow(clippy::too_many_arguments)]
pub fn validate_bindings<S: CardStore>(
  store: &S,
  recipe_id: u16,
  root: u32,
  bindings: &[Vec<u32>],
  caller_player_id: u32,
  now_ms: u64,
  wants_exclusive: impl Fn(u32) -> bool,
  magnetic_recipe: impl Fn(u16) -> Option<u16>,
) -> Result<(), String> {
  // Root: dead + hold-kind + touch + drop gates.
  if root != 0 {
    let card = store.card_at(root, now_ms).ok_or_else(|| format!("root card {root} not found"))?;
    if is_dead(card.flags_state) {
      return Err(format!("root card {root} is dead"));
    }
    if hold_count(card.flags_bk, HoldField::SlotHold) > 0 {
      return Err(format!("root card {root} is exclusively held by another in-flight action"));
    }
    if wants_exclusive(root) && hold_count(card.flags_bk, HoldField::SlotShare) > 0 {
      return Err(format!("root card {root} is shared-held by another in-flight action; cannot claim"));
    }
    if u32::from(hold_count(card.flags_bk, HoldField::Touch)) >= TOUCH_COUNT_CLIENT_CAP {
      return Err(format!(
        "root card {root} has too many concurrent in-flight actions (cap {TOUCH_COUNT_CLIENT_CAP})"
      ));
    }
    if drop_hold_count(card.flags_bk) > 0 {
      return Err(format!("root card {root} blocks stacking (drop_hold_count > 0)"));
    }
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
      if is_dead(card.flags_state) {
        return Err(format!("card {card_id} is dead"));
      }
      if hold_count(card.flags_bk, HoldField::SlotHold) > 0 {
        return Err(format!("card {card_id} is exclusively held by another in-flight action"));
      }
      if wants_exclusive(card_id) && hold_count(card.flags_bk, HoldField::SlotShare) > 0 {
        return Err(format!("card {card_id} is shared-held by another in-flight action; cannot claim"));
      }
      if u32::from(hold_count(card.flags_bk, HoldField::Touch)) >= TOUCH_COUNT_CLIENT_CAP {
        return Err(format!(
          "card {card_id} has too many concurrent in-flight actions (cap {TOUCH_COUNT_CLIENT_CAP})"
        ));
      }
      let owner_player = owning_player(store, card_id, now_ms).unwrap_or(WORLD_PLAYER_ID);
      if owner_player != caller_player_id && owner_player != WORLD_PLAYER_ID {
        return Err(format!(
          "card {card_id} is owned by player {owner_player}, not caller {caller_player_id}"
        ));
      }
      if is_magnetic(card.flags_state) {
        let expected = magnetic_recipe(card.packed_definition).ok_or_else(|| {
          format!("card {card_id} carries magnetic flag but def declares no magnetic recipe")
        })?;
        if expected != recipe_id {
          return Err(format!("card {card_id} is magnetic-locked to recipe {expected}, got {recipe_id}"));
        }
      }
    }
  }
  Ok(())
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
  use super::*;
  use crate::flags::{flag_bit, flag_field};
  use std::collections::HashMap;

  // Build flag values from the real `flags.json` layout (the same source
  // card_model reads), so the tests exercise the live bit positions.
  fn state_bit(name: &str) -> u32 {
    1u32 << flag_bit("cards_state", name).unwrap()
  }
  fn bk_one(field: &str) -> u32 {
    let f = flag_field("cards_bk", field).unwrap();
    (1u32 << f.shift) & f.mask()
  }

  struct Mock(HashMap<u32, CardView>);
  impl CardStore for Mock {
    fn card_at(&self, id: u32, _t: u64) -> Option<CardView> {
      self.0.get(&id).cloned()
    }
  }
  fn card(id: u32, owner: u32, flags_state: u32, flags_bk: u32) -> CardView {
    CardView { card_id: id, owner_id: owner, micro_location: 0, macro_zone: 0, packed_definition: 0, flags_state, flags_bk }
  }
  fn store(cards: &[CardView]) -> Mock {
    Mock(cards.iter().cloned().map(|c| (c.card_id, c)).collect())
  }
  // No recipe claims exclusive / nothing is magnetic, by default.
  fn no_excl(_: u32) -> bool {
    false
  }
  fn no_mag(_: u16) -> Option<u16> {
    None
  }

  #[test]
  fn world_owned_ok() {
    // owner_id 0 → WORLD_PLAYER_ID, allowed for any caller.
    let s = store(&[card(50, 0, 0, 0)]);
    validate_bindings(&s, 1, 0, &[vec![50]], 7, 0, no_excl, no_mag).expect("ok");
  }

  #[test]
  fn dup_rejected() {
    let s = store(&[card(50, 0, 0, 0)]);
    let err = validate_bindings(&s, 1, 0, &[vec![50, 50]], 7, 0, no_excl, no_mag).unwrap_err();
    assert!(err.contains("more than once"), "{err}");
  }

  #[test]
  fn dead_rejected() {
    let s = store(&[card(50, 0, state_bit("dead"), 0)]);
    let err = validate_bindings(&s, 1, 0, &[vec![50]], 7, 0, no_excl, no_mag).unwrap_err();
    assert!(err.contains("is dead"), "{err}");
  }

  #[test]
  fn foreign_owner_rejected() {
    // card owned by player 9 (is_owned_by_player set, owner_id = 9), caller 7.
    let s = store(&[card(50, 9, state_bit("is_owned_by_player"), 0)]);
    let err = validate_bindings(&s, 1, 0, &[vec![50]], 7, 0, no_excl, no_mag).unwrap_err();
    assert!(err.contains("owned by player 9"), "{err}");
  }

  #[test]
  fn exclusive_held_always_rejected() {
    // a card already exclusively held conflicts even if we don't want exclusive.
    let s = store(&[card(50, 0, 0, bk_one("slot_hold_count"))]);
    let err = validate_bindings(&s, 1, 0, &[vec![50]], 7, 0, no_excl, no_mag).unwrap_err();
    assert!(err.contains("exclusively held"), "{err}");
  }

  #[test]
  fn shared_held_rejected_only_when_claiming_exclusive() {
    let s = store(&[card(50, 0, 0, bk_one("slot_share_count"))]);
    // not claiming exclusive → shared hold is fine
    validate_bindings(&s, 1, 0, &[vec![50]], 7, 0, no_excl, no_mag).expect("share ok");
    // claiming exclusive on 50 → conflict
    let err = validate_bindings(&s, 1, 0, &[vec![50]], 7, 0, |id| id == 50, no_mag).unwrap_err();
    assert!(err.contains("shared-held"), "{err}");
  }

  #[test]
  fn magnetic_locked_to_other_recipe_rejected() {
    let s = store(&[card(50, 0, state_bit("magnetic"), 0)]);
    // def locked to recipe 9, proposal is recipe 1 → reject
    let err = validate_bindings(&s, 1, 0, &[vec![50]], 7, 0, no_excl, |_| Some(9)).unwrap_err();
    assert!(err.contains("magnetic-locked to recipe 9"), "{err}");
    // locked to recipe 1 (== proposal) → ok
    validate_bindings(&s, 1, 0, &[vec![50]], 7, 0, no_excl, |_| Some(1)).expect("matching magnetic ok");
  }
}
