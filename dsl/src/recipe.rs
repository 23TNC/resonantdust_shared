//! Recipe binding layer — the bridge between the gate's positional `bindings`
//! and the DSL vm's slot-path frame.
//!
//! The gate (and client matcher) speak `bindings: Vec<Vec<u32>>` — a row of
//! card_ids per recipe *iterator*, offset within. The vm speaks slot-path
//! strings read from an operating-set frame. This module enumerates a recipe's
//! iterators **identically to the legacy `recipe_tape`** (source order of first
//! appearance, deduped by `(parent, branch)`, scanning `@input` then `@output`)
//! so `bindings[iterator_id][offset]` lines up with the client's rows — no
//! protocol change. [`Iter::path`] then gives the DSL path each binding cell
//! occupies, which the frame builder ([`crate::vm`] frame assembly, step 2b)
//! writes `card_view`s into.

use crate::bridge::{card_view, Card};
use crate::loader::Bundle;
use crate::parser::{Node, Stmt, Token};
use crate::vm::Store;

/// One sliding-window iterator over a stack branch — the DSL analog of the
/// legacy `recipe_tape::Iterator`. Identified by `(parent, branch)`; the gate's
/// `bindings` are indexed by iterator id, with the offset selecting the cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Iter {
  /// Resolved parent-path prefix as a DSL path string: `""` for a top-level
  /// iterator (parent is the action anchor), or e.g. `"slot.1.0.owner"` for an
  /// equipment-chain iterator. Two refs with the same parent + branch share an
  /// iterator (the offset distinguishes the cell).
  pub parent: String,
  /// Branch / stack selector = `stack_id` (`1` hex/tile, `2` top, `3` bottom).
  /// `0` is the chain root (`slot.0.0`), filled by the `root` param — never an
  /// iterator (skipped in [`iterators`]).
  pub branch: u8,
}

impl Iter {
  /// The full DSL slot path for this iterator at `offset` — what the frame
  /// builder writes the bound card's view to, and what the recipe's `@input`
  /// references. `""` parent → `slot.<branch>.<offset>`; a parent → joined with
  /// a `.` (`slot.1.0.owner` → `slot.1.0.owner.slot.<branch>.<offset>`).
  pub fn path(&self, offset: u32) -> String {
    if self.parent.is_empty() {
      format!("slot.{}.{}", self.branch, offset)
    } else {
      format!("{}.slot.{}.{}", self.parent, self.branch, offset)
    }
  }
}

/// Enumerate a recipe's iterators in legacy `recipe_tape` order: scan `@input`
/// then `@output`, walk each instruction's slot/value path token left-to-right,
/// and collapse every `slot.<branch>.<offset>` triple into an iterator (dedup by
/// parent + branch). Iterator `i`'s parent only ever references iterators
/// `0..i`, by construction.
pub fn iterators(recipe: &Node) -> Vec<Iter> {
  let mut iters: Vec<Iter> = Vec::new();
  for hook in ["input", "output"] {
    let Some(h) = recipe.hook(hook) else { continue };
    for stmt in &h.body {
      let Stmt::Instr(toks) = stmt else { continue };
      for tok in toks {
        // `&slot…` (write) and `*slot…` (read) both carry a dotted path.
        if let Token::Slot(p) | Token::Value(p) = tok {
          resolve_path(p, &mut iters);
        }
      }
    }
  }
  iters
}

/// Walk one dotted path, registering an iterator for each `slot.<branch>.<offset>`
/// triple. `prefix` accumulates the resolved path so far — the parent captured
/// when a nested slot is hit (mirrors `recipe_tape::resolve_slots`'s `out`).
fn resolve_path(path: &str, iters: &mut Vec<Iter>) {
  let segs: Vec<&str> = path.split('.').collect();
  let mut prefix = String::new();
  let mut i = 0;
  while i < segs.len() {
    // Pattern: `slot` <num branch> <num offset>.
    if segs[i] == "slot" && i + 2 < segs.len() {
      if let (Ok(branch), Ok(offset)) = (segs[i + 1].parse::<u16>(), segs[i + 2].parse::<u32>()) {
        if branch <= 255 {
          // Branch 0 (`slot.0.X`) is the chain root — a single card filled by the
          // `root` param, not a sliding-window iterator. Walk past it (so nested
          // `slot.0.0.…` paths still resolve) but register no iterator.
          if branch != 0 {
            find_or_create(iters, prefix.clone(), branch as u8);
          }
          append(&mut prefix, &format!("slot.{branch}.{offset}"));
          i += 3;
          continue;
        }
      }
    }
    append(&mut prefix, segs[i]);
    i += 1;
  }
}

fn append(prefix: &mut String, seg: &str) {
  if !prefix.is_empty() {
    prefix.push('.');
  }
  prefix.push_str(seg);
}

fn find_or_create(iters: &mut Vec<Iter>, parent: String, branch: u8) -> u32 {
  if let Some(i) = iters.iter().position(|it| it.parent == parent && it.branch == branch) {
    return i as u32;
  }
  iters.push(Iter { parent, branch });
  (iters.len() - 1) as u32
}

/// The card defs a magnet must pull onto itself to satisfy `recipe`: one
/// `(branch, offset, packed_def)` per top-level non-root input slot that pins a
/// def via `$card::X *slot.<branch>.<offset>.def_id eq`. Skips the root
/// (`branch 0` = the magnet), holds, and nested-owner slots — the V1 magnetic
/// gather selects candidates by def-id at the top level (despair → 1×dread,
/// strike → 3×corpus). `None`-resolving names are dropped.
pub fn required_input_defs(bundle: &Bundle, recipe: &Node) -> Vec<(u8, u32, u16)> {
  let mut out = Vec::new();
  let Some(input) = recipe.hook("input") else {
    return out;
  };
  for stmt in &input.body {
    let Stmt::Instr(toks) = stmt else { continue };
    let mut slot: Option<(u8, u32)> = None;
    let mut name: Option<&str> = None;
    for tok in toks {
      match tok {
        // `*slot.<branch>.<offset>.def_id` — top level only (4 segments).
        Token::Value(p) => {
          let segs: Vec<&str> = p.split('.').collect();
          if segs.len() == 4 && segs[0] == "slot" && segs[3] == "def_id" {
            if let (Ok(b), Ok(o)) = (segs[1].parse::<u8>(), segs[2].parse::<u32>()) {
              slot = Some((b, o));
            }
          }
        }
        // `$card::<name>`.
        Token::Const(c) => {
          if let Some(n) = c.strip_prefix("card::") {
            name = Some(n);
          }
        }
        _ => {}
      }
    }
    if let (Some((b, o)), Some(n)) = (slot, name) {
      if b != 0 {
        if let Some(def) = bundle.packed_def(n) {
          out.push((b, o, def));
        }
      }
    }
  }
  out
}

/// An assembled operating-set frame: the [`Store`] the vm matches/plans against,
/// plus the `slot-path → card_id` map for translating the resulting
/// slot-path-keyed [`crate::vm::Plan`] back to concrete cards.
pub struct Frame {
  /// The positioned frame — `card_view`s written at their slot paths, nested for
  /// owner re-anchors. Pass to `match_recipe` / `plan_recipe`.
  pub store: Store,
  /// Every directly-placed card's `(slot-path, card_id)`, plus `("slot.0.0", root)`.
  /// Back-translation looks a Plan's slot path up here; effect targets that walk
  /// past a placed card (`…owner.inventory`) resolve the longest placed prefix
  /// here and let the caller walk the snapshot remainder.
  pub paths: Vec<(String, u32)>,
}

impl Frame {
  /// The card_id placed at exactly `path`, if any.
  pub fn card_at(&self, path: &str) -> Option<u32> {
    self.paths.iter().find(|(p, _)| p == path).map(|(_, id)| *id)
  }

  /// The longest placed-card path that is a prefix of `path` (segment-aligned),
  /// and its card_id — for resolving an effect target like
  /// `slot.1.0.owner.inventory` to `(slot.1.0, axe_id, ".owner.inventory")`.
  pub fn longest_prefix<'s, 'p>(&'s self, path: &'p str) -> Option<(&'s str, u32, &'p str)> {
    self
      .paths
      .iter()
      .filter(|(p, _)| path == p || path.strip_prefix(p).is_some_and(|r| r.starts_with('.')))
      .max_by_key(|(p, _)| p.len())
      .map(|(p, id)| (p.as_str(), *id, &path[p.len()..]))
  }
}

/// Assemble the operating-set frame from positional `bindings`. For each
/// iterator (in legacy order) and offset, place the bound card's [`card_view`]
/// at its DSL slot path; the `slot.1.0` sentinel (`card_id == 0`, top-level
/// branch 1) takes the `synthetic` tile view instead; `root` is placed at
/// `slot.0.0`. `lookup` resolves a `card_id` to its typed [`Card`] (the gate bridges
/// stored rows → `Card` here, incl. the legacy-id → name decode during the
/// transition). Iterators are parent-before-child, so nested owner paths nest
/// cleanly into their already-written parent.
pub fn build_frame(
  bundle: &Bundle,
  recipe: &Node,
  root: u32,
  bindings: &[Vec<u32>],
  synthetic: Option<&Card>,
  lookup: &dyn Fn(u32) -> Option<Card>,
) -> Frame {
  let mut store = Store::default();
  let mut paths: Vec<(String, u32)> = Vec::new();

  for (iid, iter) in iterators(recipe).iter().enumerate() {
    let Some(row) = bindings.get(iid) else { continue };
    for (offset, &card_id) in row.iter().enumerate() {
      let path = iter.path(offset as u32);
      if card_id == 0 {
        // synthetic-tile sentinel: the tile is the hex member at `slot.1.0`
        // (stack 1), the top-level branch-1 / offset-0 slot.
        if iter.parent.is_empty() && iter.branch == 1 && offset == 0 {
          if let Some(tile) = synthetic {
            store.write(&path, card_view(bundle, tile));
          }
        }
        continue;
      }
      if let Some(card) = lookup(card_id) {
        store.write(&path, card_view(bundle, &card));
        paths.push((path, card_id));
      }
    }
  }

  if root != 0 {
    if let Some(card) = lookup(root) {
      store.write("slot.0.0", card_view(bundle, &card));
      paths.push(("slot.0.0".to_string(), root));
    }
  }

  Frame { store, paths }
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
  use super::*;
  use crate::parser::parse;

  fn iters_of(input: &str, output: &str) -> Vec<Iter> {
    let src = format!("<recipe>\n  ::r>\n    @input>\n{input}    @output>\n{output}");
    let root = parse(&src).unwrap();
    iterators(root.bucket("recipe").unwrap().def("r").unwrap())
  }

  fn it(parent: &str, branch: u8) -> Iter {
    Iter { parent: parent.to_string(), branch }
  }

  #[test]
  fn triple_corpus_single_branch_iterator() {
    // three refs into branch 1 share one iterator (cf. recipe_tape
    // dedupes_iterator_within_same_branch)
    let iters = iters_of(
      "      $card::corpus *slot.1.0.def_id eq if &slot.1.0 use\n\
       \x20     $card::corpus *slot.1.1.def_id eq if &slot.1.1 claim\n\
       \x20     $card::corpus *slot.1.2.def_id eq if &slot.1.2 claim\n",
      "      &slot.1.0 destroy\n",
    );
    assert_eq!(iters, vec![it("", 1)]);
  }

  #[test]
  fn distinct_branches_distinct_iterators() {
    let iters = iters_of(
      "      *slot.1.0.def_id eq if &slot.1.0 use\n      *slot.2.0.def_id eq if &slot.2.0 use\n",
      "",
    );
    assert_eq!(iters, vec![it("", 1), it("", 2)]);
  }

  #[test]
  fn cut_tree_nested_equipment_iterator() {
    // slot.1.0 (tile / hex member), slot.2.0 (actor, top), slot.2.0.owner.slot.2.0
    // (axe in the actor's owner's top stack) → 3 iterators, the nested one
    // parented on "slot.2.0.owner". Mirrors recipe_tape::nested_iterator_for_equipment_chain.
    let iters = iters_of(
      "      *slot.1.0.aspect.wood 1 ge if &slot.1.0 use\n\
       \x20     *slot.2.0.aspect.corpus_lit 1 ge if &slot.2.0 claim\n\
       \x20     $card::axe *slot.2.0.owner.slot.2.0.def_id eq if &slot.2.0.owner.slot.2.0 share\n",
      "",
    );
    assert_eq!(iters, vec![it("", 1), it("", 2), it("slot.2.0.owner", 2)]);
    // and the nested iterator's path round-trips to the source reference
    assert_eq!(iters[2].path(0), "slot.2.0.owner.slot.2.0");
    assert_eq!(iters[1].path(0), "slot.2.0");
  }

  #[test]
  fn parallel_nested_iterators_distinct_by_outer_offset() {
    // two outer offsets, each with its own equipment iterator — distinct
    // because the parent path bakes in the outer offset.
    let iters = iters_of(
      "      *slot.1.0.def_id eq if &slot.1.0 use\n\
       \x20     *slot.1.1.def_id eq if &slot.1.1 use\n\
       \x20     *slot.1.0.owner.slot.1.0.def_id eq if &slot.1.0.owner.slot.1.0 share\n\
       \x20     *slot.1.1.owner.slot.1.0.def_id eq if &slot.1.1.owner.slot.1.0 share\n",
      "",
    );
    assert_eq!(
      iters,
      vec![it("", 1), it("slot.1.0.owner", 1), it("slot.1.1.owner", 1)]
    );
  }

  #[test]
  fn root_only_recipe_has_no_iterators() {
    // The root is `slot.0.0` (branch 0) — filled by the `root` param, never an
    // iterator; a root-only recipe enumerates to nothing.
    let iters = iters_of(
      "      *slot.0.0.aspect.fleeting 1 ge if &slot.0.0 borrow\n",
      "      &slot.0.0 destroy\n",
    );
    assert_eq!(iters, Vec::<Iter>::new());
  }

  // ---- frame building ----

  use crate::loader::load;
  use crate::vm::{match_recipe, Hold};

  fn bundle() -> Bundle {
    let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n";
    let cards = "<card>\n\
      \x20 ::corpus>\n    :data>\n      @define>\n        faculty &aspect.type set\n\
      \x20 ::axe>\n    :data>\n      @define>\n        requisite &aspect.type set\n";
    load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into())]).expect("load")
  }

  fn card_of(b: &Bundle, name: &str) -> Card {
    Card { def_id: b.card_def_id(name).unwrap(), stock: vec![] }
  }

  fn recipe_node(src: &str) -> Node {
    parse(src).unwrap().bucket("recipe").unwrap().def("r").unwrap().clone()
  }

  #[test]
  fn frame_places_binding_and_matches_then_translates_back() {
    let b = bundle();
    let r = recipe_node(
      "<recipe>\n  ::r>\n    @input>\n      $card::corpus *slot.2.0.def_id eq if &slot.2.0 use\n    @output>\n      &slot.2.0 destroy\n",
    );
    // iterator 0 = branch 2 (top); bind card 101 (a corpus) at offset 0.
    let corpus = card_of(&b, "corpus");
    let lookup = |id: u32| (id == 101).then(|| corpus.clone());
    let frame = build_frame(&b, &r, 0, &[vec![101]], None, &lookup);

    // the placement resolves, and the recipe matches against it
    let mut store = frame.store;
    let plan =
      match_recipe(&r.hook("input").unwrap().body, &mut store, &b.catalog, &b.functions).unwrap();
    assert!(plan.matched);
    assert_eq!(plan.holds, vec![("slot.2.0".to_string(), Hold::Use)]);
    // and the hold's slot-path translates back to the bound card_id
    let frame2 = build_frame(&b, &r, 0, &[vec![101]], None, &lookup);
    assert_eq!(frame2.card_at("slot.2.0"), Some(101));
  }

  #[test]
  fn under_filled_recipe_does_not_match() {
    // A 2-corpus recipe (slot.2.0 + slot.2.1) must NOT match when only one
    // card is bound — the absent slot.2.1 must fail its `def_id eq`, not
    // spuriously pass (regression: `corpus_b_top` firing on a single corpus).
    let b = bundle();
    let r = recipe_node(
      "<recipe>\n  ::r>\n    @input>\n      $card::corpus *slot.2.0.def_id eq if &slot.2.0 use\n      $card::corpus *slot.2.1.def_id eq if &slot.2.1 claim\n    @output>\n      &slot.2.0 destroy\n",
    );
    let corpus = card_of(&b, "corpus");
    let one = |id: u32| (id == 101).then(|| corpus.clone());
    let mut store = build_frame(&b, &r, 0, &[vec![101]], None, &one).store;
    let mp = match_recipe(&r.hook("input").unwrap().body, &mut store, &b.catalog, &b.functions).unwrap();
    assert!(!mp.matched, "1 bound card must not satisfy a 2-slot recipe");

    // both bound → matches
    let two = |id: u32| (id == 101 || id == 102).then(|| corpus.clone());
    let mut store2 = build_frame(&b, &r, 0, &[vec![101, 102]], None, &two).store;
    let mp2 = match_recipe(&r.hook("input").unwrap().body, &mut store2, &b.catalog, &b.functions).unwrap();
    assert!(mp2.matched, "2 bound corpus must satisfy the 2-slot recipe");
  }

  #[test]
  fn frame_nests_owner_reanchor() {
    let b = bundle();
    let r = recipe_node(
      "<recipe>\n  ::r>\n    @input>\n      $card::axe *slot.2.0.owner.slot.2.0.def_id eq if &slot.2.0.owner.slot.2.0 share\n    @output>\n      $card::corpus &slot.2.0.owner.inventory create\n",
    );
    // iter 0 = branch 2 (actor, from the owner-path prefix); iter 1 = nested axe.
    let (actor, axe) = (card_of(&b, "corpus"), card_of(&b, "axe"));
    let lookup = |id: u32| match id {
      201 => Some(actor.clone()),
      301 => Some(axe.clone()),
      _ => None,
    };
    let frame = build_frame(&b, &r, 0, &[vec![201], vec![301]], None, &lookup);

    // the nested axe view resolves through the actor's baked-in `owner` key
    let mut store = build_frame(&b, &r, 0, &[vec![201], vec![301]], None, &lookup).store;
    let plan =
      match_recipe(&r.hook("input").unwrap().body, &mut store, &b.catalog, &b.functions).unwrap();
    assert!(plan.matched);
    assert_eq!(plan.holds, vec![("slot.2.0.owner.slot.2.0".to_string(), Hold::Share)]);

    // back-translation: exact placement + an effect target that walks past one
    assert_eq!(frame.card_at("slot.2.0.owner.slot.2.0"), Some(301));
    assert_eq!(
      frame.longest_prefix("slot.2.0.owner.inventory"),
      Some(("slot.2.0", 201, ".owner.inventory"))
    );
  }
}
