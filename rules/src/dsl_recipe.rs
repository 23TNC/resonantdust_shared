//! DSL recipe evaluation — run a proposal's recipe on the shared VM and
//! translate the result into the [`ActionPlan`] a consumer applies.
//!
//! Generic over [`CardStore`]: cards are read through the trait, so the gate
//! (over its gathered snapshot) and the client (over its world model) share this
//! code by construction. The shared crate provides the recipe runtime; this
//! module bridges a card store to it:
//!   - **rows → `Card`**: a stored row's `packed_definition` is decoded to a card
//!     **name** via the [`Bundle`] and re-id'd to the bound `Card` the vm reads.
//!   - **frame**: [`build_frame`] places the bound cards at their slot paths.
//!   - **match + plan**: the vm matches `@input` and runs `@output`.
//!   - **`vm::Plan` → `ActionPlan`**: holds, effects, styles, duration mapped to
//!     the shape consumers apply. Owner-walk for `create` / unlock targets
//!     resolves against the store here.
//!
//! State validation (ownership / not-dead / holds / dedup) lives in
//! [`resonantdust_state::recipe_state::validate_bindings`]; this module is recipe
//! *semantics* only.

use std::collections::BTreeMap;

use resonantdust_codec::card_model;
use resonantdust_codec::packed::{pack_macro_zone_full, INVENTORY_LAYER};
use resonantdust_codec::plan::{ActionPlan, Effect, HoldKinds, StockOp};
use resonantdust_dsl::bridge::{stock_slot_bits, stock_slot_for_aspect, stock_to_vec, Card};
use resonantdust_dsl::loader::Bundle;
use resonantdust_dsl::recipe::{build_frame, Frame};
use resonantdust_dsl::vm::{match_recipe, plan_recipe, Effect as VmEffect, Hold};
use resonantdust_state::recipe_state::CardStore;

/// Evaluate `recipe_name`'s DSL recipe against the card `store` and bound cards,
/// returning the [`ActionPlan`] to apply. Errors if the recipe is unknown or its
/// `@input` predicates don't hold against the bindings (the match step replaces
/// the legacy `validate_input`). `now_ms` is the read time for `store.card_at`.
pub fn run<S: CardStore>(
    bundle: &Bundle,
    store: &S,
    recipe_name: &str,
    root: u32,
    bindings: &[Vec<u32>],
    synthetic: Option<(u16, (u8, u8))>,
    now_ms: u64,
) -> Result<ActionPlan, String> {
    let recipe = bundle
        .recipe(recipe_name)
        .ok_or_else(|| format!("DSL recipe {recipe_name:?} not found"))?;

    // The synthetic tile as a typed Card (def re-id'd, its two zone stocks
    // overlaid positionally onto the def's schema by `card_view`).
    let synth_card = match synthetic {
        Some((packed, (s0, s1))) => {
            let name = bundle
                .name_for_packed(packed)
                .ok_or_else(|| format!("tile packed {packed:#06x} not in DSL bundle"))?;
            let def_id = bundle
                .card_def_id(name)
                .ok_or_else(|| format!("tile {name:?} not in DSL bundle"))?;
            Some(Card { def_id, stock: vec![s0 as i64, s1 as i64] })
        }
        None => None,
    };

    // Bridge a bound card_id → typed Card (decode packed → name → bound Card).
    // Per-instance `stock` u32 is decoded per the def's stock schema so a card's
    // live stock aspects (build progress, etc.) read in matching — not just its
    // static `@define` defaults.
    let lookup = |id: u32| -> Option<Card> {
        let c = store.card_at(id, now_ms)?;
        let name = bundle.name_for_packed(c.packed_definition)?;
        let def_id = bundle.card_def_id(name)?;
        Some(Card { def_id, stock: stock_to_vec(bundle, name, c.stock) })
    };

    let mut frame = build_frame(bundle, recipe, root, bindings, synth_card.as_ref(), &lookup);

    // Match @input (the conjunction verdict) then run @output.
    let input = recipe.hook("input").map(|h| h.body.as_slice()).unwrap_or(&[]);
    let mp = match_recipe(input, &mut frame.store, &bundle.catalog, &bundle.functions)?;
    if !mp.matched {
        return Err(format!("recipe {recipe_name:?} input not satisfied by bindings"));
    }
    let output = recipe.hook("output").map(|h| h.body.as_slice()).unwrap_or(&[]);
    let pp = plan_recipe(output, &mut frame.store, &bundle.catalog, &bundle.functions)?;

    translate(bundle, store, &frame, &mp.holds, &pp.styles, &pp.effects, pp.duration, &synth_card, now_ms)
}

/// `vm::Plan` parts → [`ActionPlan`]. Holds split into per-card and the
/// synthetic tile's (`tile_holds`); effects map 1:1 with owner-walk target
/// resolution; styles + duration carry through.
#[allow(clippy::too_many_arguments)]
fn translate<S: CardStore>(
    bundle: &Bundle,
    store: &S,
    frame: &Frame,
    holds: &[(String, Hold)],
    styles: &[(String, String)],
    effects: &[VmEffect],
    duration: i64,
    synth: &Option<Card>,
    now_ms: u64,
) -> Result<ActionPlan, String> {
    let mut ap = ActionPlan {
        styles: BTreeMap::new(),
        duration: duration.max(0) as u32,
        effects: Vec::new(),
        holds: BTreeMap::new(),
        tile_holds: None,
    };

    // Holds: a path with a placed card → per-card; an unplaced path is the
    // synthetic tile (addressed positionally by apply) → tile_holds.
    for (path, hold) in holds {
        let k = kinds(hold);
        match frame.card_at(path) {
            Some(cid) => merge(ap.holds.entry(cid).or_insert(ZERO), &k),
            None => {
                let mut t = ap.tile_holds.take().unwrap_or(ZERO);
                merge(&mut t, &k);
                ap.tile_holds = Some(t);
            }
        }
    }

    for (path, style) in styles {
        if let Some(cid) = frame.card_at(path) {
            ap.styles.insert(cid, style_code(style));
        }
    }

    for eff in effects {
        match eff {
            VmEffect::Destroy { slot } => {
                let cid = frame
                    .card_at(slot)
                    .ok_or_else(|| format!("destroy: {slot:?} is not a bound card"))?;
                ap.effects.push(Effect::Destroy { card_id: cid });
            }
            VmEffect::Create { def, target } => {
                let (owner_card, container) = resolve_target(frame, store, target, now_ms)?;
                if container.as_deref() != Some("inventory") {
                    return Err(format!("create target must end in .inventory; got {target:?}"));
                }
                ap.effects.push(Effect::Create {
                    def_key: def_key(def),
                    surface: INVENTORY_LAYER,
                    macro_zone: pack_macro_zone_full(owner_card, INVENTORY_LAYER, 0, 0),
                    owner_id: owner_card,
                });
            }
            VmEffect::Stock { slot, aspect, delta, abs } => match frame.card_at(slot) {
                // A bound CARD → write its per-card `stock` u32: compute the new
                // value here (current stock with this slot's bits replaced) and
                // emit an absolute SetCardStock. The card holds it; only the
                // bottom u4 can later save to a zone.
                Some(card_id) => {
                    let c = store
                        .card_at(card_id, now_ms)
                        .ok_or_else(|| format!("stock op: bound card {card_id} not found"))?;
                    let name = bundle
                        .name_for_packed(c.packed_definition)
                        .ok_or_else(|| format!("stock op: card {card_id} packed not in bundle"))?;
                    let (shift, width) = stock_slot_bits(bundle, name, aspect).ok_or_else(|| {
                        format!("card {name:?} declares no stock slot for aspect {aspect:?}")
                    })?;
                    let cap: u32 = if width >= 32 { u32::MAX } else { (1 << width) - 1 };
                    let mask: u32 = cap << shift;
                    let cur = (c.stock & mask) >> shift;
                    let new_val = if *abs {
                        (*delta).clamp(0, cap as i64) as u32
                    } else {
                        (cur as i64 + *delta).clamp(0, cap as i64) as u32
                    };
                    let new_stock = (c.stock & !mask) | (new_val << shift);
                    ap.effects.push(Effect::SetCardStock { card_id, stock: new_stock });
                }
                // Unplaced → the synthetic tile (the zone-savable u4 path),
                // addressed positionally by apply via the proposal cell.
                None => {
                    let tile = synth
                        .as_ref()
                        .ok_or_else(|| "tile stock op but no synthetic tile".to_string())?;
                    let tile_name = bundle
                        .card_name(tile.def_id)
                        .ok_or_else(|| "tile def has no name".to_string())?;
                    let idx = stock_slot_for_aspect(bundle, tile_name, aspect).ok_or_else(|| {
                        format!("tile {tile_name:?} declares no stock slot for aspect {aspect:?}")
                    })?;
                    let (op, mag) = if *abs {
                        (StockOp::Set, *delta)
                    } else if *delta < 0 {
                        (StockOp::Sub, -*delta)
                    } else {
                        (StockOp::Add, *delta)
                    };
                    ap.effects.push(Effect::ModifyTileStock {
                        slot: idx as u8,
                        op,
                        delta: mag.clamp(0, 255) as u8,
                    });
                }
            },
            VmEffect::Blueprint { def, target } => {
                let (target_card, _) = resolve_target(frame, store, target, now_ms)?;
                // The recipe may reference the blueprint by its registry key
                // (`nd_furnace`) OR by its spawned card (`blueprint_nd_furnace`).
                let key = def_key(def);
                let blueprint_id = bundle
                    .blueprint_def_id(&key)
                    .or_else(|| bundle.blueprint_id_for_card(&key))
                    .ok_or_else(|| {
                        format!("recipe unlocks unknown blueprint {key:?} (no <blueprint> def)")
                    })?;
                ap.effects.push(Effect::UnlockBlueprint {
                    blueprint_id,
                    target_card_id: target_card,
                });
            }
        }
    }

    Ok(ap)
}

/// All-false [`HoldKinds`] — the merge base.
const ZERO: HoldKinds = HoldKinds { slot_hold: false, position_hold: false, slot_share: false };

/// DSL [`Hold`] → [`HoldKinds`] (`slot_hold` true → exclusive; false →
/// `slot_share`; `position_hold` from the verb's pin).
fn kinds(h: &Hold) -> HoldKinds {
    match h {
        Hold::Use => HoldKinds { slot_hold: true, slot_share: false, position_hold: false },
        Hold::Claim => HoldKinds { slot_hold: true, slot_share: false, position_hold: true },
        Hold::Share => HoldKinds { slot_hold: false, slot_share: true, position_hold: true },
        Hold::Borrow => HoldKinds { slot_hold: false, slot_share: true, position_hold: false },
    }
}

fn merge(into: &mut HoldKinds, k: &HoldKinds) {
    into.slot_hold |= k.slot_hold;
    into.slot_share |= k.slot_share;
    into.position_hold |= k.position_hold;
}

/// Map the DSL style constant to the progress-style code (none=0, ltr=1, rtl=2).
fn style_code(style: &str) -> u8 {
    match style {
        "ltr" => 1,
        "rtl" => 2,
        _ => 0,
    }
}

/// `card::corpus_dim` / `blueprint::nd_furnace` → the bare key (`corpus_dim`).
fn def_key(def: &str) -> String {
    def.rsplit("::").next().unwrap_or(def).to_string()
}

/// Resolve a slot-path target to a concrete card_id, walking `.owner` / `.parent`
/// past the longest placed-card prefix via the store. Returns the resolved card
/// and any trailing container word (`inventory` / `blueprint`).
fn resolve_target<S: CardStore>(
    frame: &Frame,
    store: &S,
    path: &str,
    now_ms: u64,
) -> Result<(u32, Option<String>), String> {
    if let Some(id) = frame.card_at(path) {
        return Ok((id, None));
    }
    let (_, mut cid, remainder) = frame
        .longest_prefix(path)
        .ok_or_else(|| format!("unresolved target path {path:?}"))?;
    let mut container = None;
    for seg in remainder.split('.').filter(|s| !s.is_empty()) {
        match seg {
            "owner" => {
                cid = store
                    .card_at(cid, now_ms)
                    .map(|c| c.owner_id)
                    .filter(|&o| o != 0)
                    .ok_or_else(|| format!("owner step: card {cid} has no owner"))?;
            }
            "parent" => {
                let c = store
                    .card_at(cid, now_ms)
                    .ok_or_else(|| format!("parent step: card {cid} not in store"))?;
                if !card_model::micro_is_card(c.flags) {
                    return Err(format!("parent step: card {cid} is not stacked"));
                }
                cid = c.micro_location;
            }
            "inventory" | "blueprint" => container = Some(seg.to_string()),
            other => return Err(format!("unsupported target step {other:?} in {path:?}")),
        }
    }
    Ok((cid, container))
}
