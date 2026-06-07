//! Storage bridge — turn a stored card instance into the [`Cell`] view the VM
//! matches/renders against. The pure, host-agnostic half of migration step #2:
//! given a [`Card`] (its definition + per-instance stock) and a loaded
//! [`Bundle`], produce the operating-set cell a recipe reads as `slot.a.b`.
//!
//! Clean-room, not a port of the old packed layout: the typed [`Card`] here is
//! the *unpacked* row; bit-packing it into words is a separate encoding step.
//! The gate assembles many `card_view`s into one frame (placing each at its
//! `slot.a.b`), then runs [`crate::vm::match_recipe`] / `plan_recipe`.

use crate::loader::Bundle;
use crate::parser::{Stmt, Token};
use crate::vm::{run, Cell, Functions, Store};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// A card instance's persisted state — the bridge's *typed* row. Minimal for
/// recipe matching: which definition, and its per-instance stock values
/// (positional, matching the def's [`stock_schema`]). Flags / location / owner
/// join as later slices (matching them) need them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Card {
  pub def_id: u16,
  pub stock: Vec<i64>,
}

/// The ordered stock aspects a card declares — `<bits> &aspect.<name> stock` in
/// its `:data @define`, as `(name, bits)` in declaration order. A packed row's
/// stock slots map onto these positionally, and `card_view` overlays the row's
/// values here.
pub fn stock_schema(bundle: &Bundle, card: &str) -> Vec<(String, i64)> {
  let mut out = Vec::new();
  let Some(define) = bundle.card(card).and_then(|d| d.facet("data")).and_then(|f| f.hook("define")) else {
    return out;
  };
  for stmt in &define.body {
    let Stmt::Instr(toks) = stmt else { continue };
    if !matches!(toks.last(), Some(Token::Word(w)) if w == "stock") {
      continue;
    }
    let name = toks.iter().find_map(|t| match t {
      Token::Slot(s) => s.strip_prefix("aspect.").map(str::to_string),
      _ => None,
    });
    let bits = toks.iter().find_map(|t| match t {
      Token::Number(n) => Some(*n),
      _ => None,
    });
    if let Some(name) = name {
      out.push((name, bits.unwrap_or(0)));
    }
  }
  out
}

/// Build the per-card view the VM matches against: `def_id` (the `$card::name`
/// ref recipes compare), plus `aspect.*` = the card's static aspects (its
/// `:data @define`) with the row's stock values overlaid and rolled up the
/// `satisfies` hierarchy — so a recipe reading `aspect.wood` sees the sum of
/// pine/ash/etc. Pure over `(bundle, card)`.
pub fn card_view(bundle: &Bundle, card: &Card) -> Cell {
  let mut store = Store::default();
  let Some(name) = bundle.card_name(card.def_id).map(str::to_string) else {
    return Cell::Map(Vec::new());
  };
  // Match by LINEAGE, not exact version: the `def_id` symbol is the
  // version-stripped logical name, so a recipe's `$card::apple` matches any
  // version (apple.0, apple.1, …). The `@define` below still reads the
  // version-SPECIFIC def by `name`, so each instance keeps its own
  // aspects/stock schema even while several versions coexist.
  store.write("def_id", Cell::Sym(format!("card::{}", crate::loader::lineage(&name))));

  // static aspects (type/cost/…) and stock declarations from :data @define
  if let Some(define) = bundle.card(&name).and_then(|d| d.facet("data")).and_then(|f| f.hook("define")) {
    let _ = run(&define.body, &mut store, &[], &bundle.catalog, &Functions::default());
  }
  // overlay the per-instance stock values (positional, by the schema)
  for (i, (aspect, _bits)) in stock_schema(bundle, &name).iter().enumerate() {
    if let Some(v) = card.stock.get(i) {
      store.write(&format!("aspect.{aspect}"), Cell::Int(*v));
    }
  }
  fold_aspects(&mut store, bundle);
  store.into_root()
}

/// Roll every raw aspect value up its `satisfies` closure: a card with `pine: 2`
/// gains `wood += 2` (and `wood`'s own ancestors, transitively). Recipes then
/// read `aspect.wood` directly. Symbol-valued aspects (`type`) contribute 0.
fn fold_aspects(store: &mut Store, bundle: &Bundle) {
  let raw: Vec<(String, i64)> = match store.read("aspect") {
    Some(Cell::Map(m)) => m.iter().filter_map(|(k, v)| {
      let n = v.as_int();
      (n != 0).then(|| (k.clone(), n))
    }).collect(),
    _ => return,
  };
  for (name, val) in raw {
    for anc in satisfies_closure(bundle, &name) {
      let path = format!("aspect.{anc}");
      let cur = store.read(&path).map(Cell::as_int).unwrap_or(0);
      store.write(&path, Cell::Int(cur + val));
    }
  }
}

/// The transitive set of aspects `name` satisfies (its ancestors, excluding
/// itself), following each `<aspect>` record's `satisfies` list in the catalog.
fn satisfies_closure(bundle: &Bundle, name: &str) -> Vec<String> {
  let mut out = Vec::new();
  let mut seen = HashSet::new();
  let mut queue = vec![name.to_string()];
  while let Some(n) = queue.pop() {
    let Some(Cell::Map(m)) = bundle.catalog.aspect(&n) else { continue };
    let Some((_, Cell::Arr(sat))) = m.iter().find(|(k, _)| k == "satisfies") else { continue };
    for s in sat {
      if let Cell::Sym(sym) = s {
        let anc = sym.strip_prefix("aspect::").unwrap_or(sym).to_string();
        if seen.insert(anc.clone()) {
          out.push(anc.clone());
          queue.push(anc);
        }
      }
    }
  }
  out
}

/// The def-level default values of a card's first two stock slots (the Zone's
/// budget) — what a freshly-spawned tile seeds before any `@init` climate pass.
/// In the DSL the `stock` op initialises a slot to `0`, so this is `(0, 0)`
/// unless the `:data @define` explicitly sets a value after declaring the slot.
/// Replaces the legacy `decode_definition().stock[].default`.
pub fn stock_defaults(bundle: &Bundle, card: &str) -> (u8, u8) {
  let mut store = Store::default();
  if let Some(define) = bundle.card(card).and_then(|d| d.facet("data")).and_then(|f| f.hook("define")) {
    let _ = run(&define.body, &mut store, &[], &bundle.catalog, &Functions::default());
  }
  let schema = stock_schema(bundle, card);
  let read = |i: usize| -> u8 {
    schema
      .get(i)
      .and_then(|(a, _)| store.read(&format!("aspect.{a}")))
      .map(Cell::as_int)
      .unwrap_or(0)
      .clamp(0, 3) as u8
  };
  (read(0), read(1))
}

/// Whether `aspect` rolls up to `ancestor` (equal, or `ancestor` is in its
/// `satisfies` closure). The DSL equivalent of legacy `is_aspect_descendant`.
pub fn is_descendant(bundle: &Bundle, aspect: &str, ancestor: &str) -> bool {
  aspect == ancestor || satisfies_closure(bundle, aspect).iter().any(|a| a == ancestor)
}

/// The stock-slot index on `card` that a tile-stock op on `op_aspect` targets,
/// with sub-aspect widening: a slot declared for `pine` answers an op on
/// `aspect.wood` (pine satisfies wood). Mirrors the legacy `recipe_plan`'s
/// `def.stock.position(is_aspect_descendant(slot, aspect))`. `None` if the card
/// declares no stock slot rolling up to `op_aspect`.
pub fn stock_slot_for_aspect(bundle: &Bundle, card: &str, op_aspect: &str) -> Option<usize> {
  stock_schema(bundle, card).iter().position(|(aspect, _)| {
    aspect == op_aspect || satisfies_closure(bundle, aspect).iter().any(|a| a == op_aspect)
  })
}

/// Assemble an operating-set frame: write each card's [`card_view`] at its slot
/// path (`root`, `slot.1.0`, …). The gate computes the paths by walking the
/// chain; this is the pure assembly the matcher then reads as `slot.a.b`.
pub fn operating_set(bundle: &Bundle, placed: &[(&str, &Card)]) -> Store {
  let mut s = Store::default();
  for (path, card) in placed {
    s.write(path, card_view(bundle, card));
  }
  s
}

/// Pack a card's stock values into one `u32`, each in its schema-declared bit
/// width, laid out consecutively from bit 0. The schema is fixed per definition,
/// so the layout is stable. (Current content fits 32 bits — desert's 5×2 = 10;
/// wider sets would need a second word.)
pub fn pack_stock(schema: &[(String, i64)], values: &[i64]) -> u32 {
  let mut word = 0u32;
  let mut off = 0u32;
  for (i, (_, bits)) in schema.iter().enumerate() {
    let w = *bits as u32;
    let v = values.get(i).copied().unwrap_or(0) as u32;
    word = resonantdust_codec::bits::set_field(word, off, w, v);
    off += w;
  }
  word
}

/// Inverse of [`pack_stock`] — read each stock value back by its schema width.
pub fn unpack_stock(schema: &[(String, i64)], word: u32) -> Vec<i64> {
  let mut out = Vec::with_capacity(schema.len());
  let mut off = 0u32;
  for (_, bits) in schema {
    let w = *bits as u32;
    out.push(resonantdust_codec::bits::get_field(word, off, w) as i64);
    off += w;
  }
  out
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
  use super::*;
  use crate::loader::load;

  fn bundle() -> Bundle {
    let aspects = "<aspect>\n\
      \x20 ::material>\n    @define>\n      aspects &section set\n\
      \x20 ::wood>\n    @define>\n      aspects &section set\n      1 &satisfies array\n      $aspect::material &satisfies.0 set\n\
      \x20 ::pine>\n    @define>\n      aspects &section set\n      1 &satisfies array\n      $aspect::wood &satisfies.0 set\n\
      \x20 ::ash>\n    @define>\n      aspects &section set\n      1 &satisfies array\n      $aspect::wood &satisfies.0 set\n\
      \x20 ::type>\n    @define>\n      traits &section set\n\
      \x20 ::cost>\n    @define>\n      traits &section set\n";
    let cards = "<card>\n  ::grove>\n    :data>\n      @define>\n        2 &aspect.pine stock\n        2 &aspect.ash stock\n        tile &aspect.type set\n        30 &aspect.cost set\n";
    load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into())]).expect("load")
  }

  #[test]
  fn stock_schema_lists_stock_aspects_in_order() {
    let b = bundle();
    assert_eq!(stock_schema(&b, "grove"), vec![("pine".to_string(), 2), ("ash".to_string(), 2)]);
  }

  #[test]
  fn card_view_overlays_stock_and_folds_satisfies() {
    let b = bundle();
    // a grove instance: pine=3, ash=2 (positional, matching the schema)
    let card = Card { def_id: b.card_def_id("grove").unwrap(), stock: vec![3, 2] };
    let v = Store::with_root(card_view(&b, &card));

    assert_eq!(v.read("def_id"), Some(&Cell::Sym("card::grove".into())));
    // raw stock overlaid
    assert_eq!(v.read("aspect.pine"), Some(&Cell::Int(3)));
    assert_eq!(v.read("aspect.ash"), Some(&Cell::Int(2)));
    // folded: wood = pine + ash, material = wood (transitive)
    assert_eq!(v.read("aspect.wood"), Some(&Cell::Int(5)));
    assert_eq!(v.read("aspect.material"), Some(&Cell::Int(5)));
    // static aspects survive; symbol-valued type is untouched by the fold
    assert_eq!(v.read("aspect.cost"), Some(&Cell::Int(30)));
    assert_eq!(v.read("aspect.type"), Some(&Cell::Sym("tile".into())));
  }

  #[test]
  fn stock_defaults_and_descendant() {
    let b = bundle();
    // grove declares pine/ash stocks but sets no default → (0, 0)
    assert_eq!(stock_defaults(&b, "grove"), (0, 0));
    // satisfies roll-up
    assert!(is_descendant(&b, "pine", "wood"));
    assert!(is_descendant(&b, "pine", "material")); // transitive
    assert!(is_descendant(&b, "wood", "wood")); // reflexive
    assert!(!is_descendant(&b, "pine", "stone"));
  }

  #[test]
  fn stock_slot_widens_to_sub_aspect() {
    let b = bundle();
    // grove stocks pine (slot 0) + ash (slot 1); both satisfy wood.
    assert_eq!(stock_slot_for_aspect(&b, "grove", "pine"), Some(0));
    assert_eq!(stock_slot_for_aspect(&b, "grove", "ash"), Some(1));
    // an op on the rolled-up `wood` targets the first slot that satisfies it.
    assert_eq!(stock_slot_for_aspect(&b, "grove", "wood"), Some(0));
    // a slot the card doesn't stock (even transitively) → none.
    assert_eq!(stock_slot_for_aspect(&b, "grove", "stone"), None);
  }

  #[test]
  fn unknown_def_id_is_empty() {
    let b = bundle();
    assert_eq!(card_view(&b, &Card { def_id: 0, stock: vec![] }), Cell::Map(Vec::new()));
  }

  #[test]
  fn stock_round_trips_through_bits() {
    // desert-shaped: 5 stock slots, 2 bits each
    let schema: Vec<(String, i64)> = ["stone", "flora", "water", "food", "fuel"]
      .iter().map(|n| (n.to_string(), 2)).collect();
    let values = vec![2, 1, 2, 3, 0];
    let word = pack_stock(&schema, &values);
    assert_eq!(unpack_stock(&schema, word), values);
  }

  #[test]
  fn card_views_feed_match_recipe_end_to_end() {
    use crate::vm::{match_recipe, Hold};
    let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n";
    let cards = "<card>\n  ::corpus>\n    :data>\n      @define>\n        faculty &aspect.type set\n";
    let recipes = "<recipe>\n  ::use_corpus>\n    @input>\n      $card::corpus *slot.1.0.def_id eq if &slot.1.0 use\n    @output>\n      10 &sys.duration set\n      &slot.1.0 destroy\n";
    let b = load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into()), ("r.rd".into(), recipes.into())]).unwrap();

    // a stored corpus instance, placed at slot.1.0, drives the matcher
    let corpus = Card { def_id: b.card_def_id("corpus").unwrap(), stock: vec![] };
    let mut frame = operating_set(&b, &[("slot.1.0", &corpus)]);
    let input = &b.recipe("use_corpus").unwrap().hook("input").unwrap().body;
    let plan = match_recipe(input, &mut frame, &b.catalog, &b.functions).unwrap();

    assert!(plan.matched);
    assert_eq!(plan.holds, vec![("slot.1.0".to_string(), Hold::Use)]);
  }

  #[test]
  fn lineage_matches_across_versions_and_create_resolves_head() {
    use crate::loader::{lineage, version_of};
    use crate::vm::match_recipe;

    let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n  ::cost>\n    @define>\n      traits &section set\n";
    // Two versions of the `apple` lineage (distinct defs, distinct cost) plus an
    // unrelated `corpus`. A `modify` would have produced apple.1 from apple.0.
    let cards = "<card>\n\
      \x20 ::apple.0>\n    :data>\n      @define>\n        faculty &aspect.type set\n        1 &aspect.cost set\n\
      \x20 ::apple.1>\n    :data>\n      @define>\n        faculty &aspect.type set\n        2 &aspect.cost set\n\
      \x20 ::corpus>\n    :data>\n      @define>\n        faculty &aspect.type set\n";
    let recipes = "<recipe>\n  ::eat_apple>\n    @input>\n      $card::apple *slot.1.0.def_id eq if &slot.1.0 use\n    @output>\n      10 &sys.duration set\n      &slot.1.0 destroy\n";
    let b = load(&[
      ("a.rd".into(), aspects.into()),
      ("c.rd".into(), cards.into()),
      ("r.rd".into(), recipes.into()),
    ])
    .unwrap();

    // lineage / version helpers
    assert_eq!(lineage("apple.0"), "apple");
    assert_eq!(lineage("apple.1"), "apple");
    assert_eq!(lineage("corpus"), "corpus");
    assert_eq!(version_of("apple.1"), 1);
    assert_eq!(version_of("corpus"), 0);

    // both versions are distinct defs (distinct packed ids); existing instances
    // of either keep their own id, so nothing renumbers.
    let p0 = b.packed_def("apple.0").unwrap();
    let p1 = b.packed_def("apple.1").unwrap();
    assert_ne!(p0, p1);

    // `create $card::apple` resolves the lineage HEAD (apple.1).
    assert_eq!(b.card_head("apple"), Some("apple.1"));
    assert_eq!(b.packed_def("apple"), Some(p1));

    // card_view emits the LINEAGE symbol for either version, but reads the
    // version-SPECIFIC cost — so old + new instances coexist correctly.
    let a0 = Card { def_id: b.card_def_id("apple.0").unwrap(), stock: vec![] };
    let a1 = Card { def_id: b.card_def_id("apple.1").unwrap(), stock: vec![] };
    let v0 = Store::with_root(card_view(&b, &a0));
    let v1 = Store::with_root(card_view(&b, &a1));
    assert_eq!(v0.read("def_id"), Some(&Cell::Sym("card::apple".into())));
    assert_eq!(v1.read("def_id"), Some(&Cell::Sym("card::apple".into())));
    assert_eq!(v0.read("aspect.cost"), Some(&Cell::Int(1)));
    assert_eq!(v1.read("aspect.cost"), Some(&Cell::Int(2)));

    // a recipe consuming `$card::apple` matches EITHER version's instance.
    let input = &b.recipe("eat_apple").unwrap().hook("input").unwrap().body;
    for apple in [&a0, &a1] {
      let mut frame = operating_set(&b, &[("slot.1.0", apple)]);
      let plan = match_recipe(input, &mut frame, &b.catalog, &b.functions).unwrap();
      assert!(plan.matched, "apple version should match $card::apple");
    }

    // an unrelated lineage (corpus) does NOT match.
    let corpus = Card { def_id: b.card_def_id("corpus").unwrap(), stock: vec![] };
    let mut frame = operating_set(&b, &[("slot.1.0", &corpus)]);
    let plan = match_recipe(input, &mut frame, &b.catalog, &b.functions).unwrap();
    assert!(!plan.matched, "corpus must not match $card::apple");
  }
}
