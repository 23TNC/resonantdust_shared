//! Terrain generation — the DSL worldgen surface.
//!
//! Turns a world hex `(q, r)` into the `(def_id, stock)` a Zone tile slot
//! stores, by the **simple biome model** (plan D5): sample climate
//! ([`crate::noise`]) → pick the first `<biome>` whose envelope contains the
//! cell → run that biome's default tile (`&tile.0`) `:data @define` + `@init`
//! with the `^biome` host → read the first two stock slots. Legacy
//! `world_gen.rs`'s rarity-weighted multi-candidate selection and cluster bias
//! are intentionally cut; `rarity` survives only as a `^biome` channel the tile
//! `@init` reads for its stock tiers.
//!
//! Pure over `(&Bundle, q, r, seed)` — no DB, no host I/O — so gate, client, and
//! the regions module all compute identical tiles from the shared crate.

use crate::loader::Bundle;
use crate::noise::{self, AXIS_COUNT};
use crate::vm::{run, Catalog, Cell, Functions, Store};

/// Build the `^biome` / `^seed` host table for a world cell — the deterministic
/// engine values the tile `@init` reads. `^biome` is a map of the five climate
/// axes as `0..=99` ints (D2); `^seed` is the cell's [`noise::cell_seed`].
pub fn biome_host(global_q: i32, global_r: i32, seed: u64) -> Vec<(String, Cell)> {
  let [elev, temp, humid, aeth, rar] = noise::climate_floats(global_q, global_r, seed);
  vec![
    (
      "biome".into(),
      Cell::Map(vec![
        ("rarity".into(), Cell::Float(rar)),
        ("elevation".into(), Cell::Float(elev)),
        ("temperature".into(), Cell::Float(temp)),
        ("humidity".into(), Cell::Float(humid)),
        ("aether".into(), Cell::Float(aeth)),
      ]),
    ),
    ("seed".into(), Cell::Int(noise::cell_seed(global_q, global_r, seed))),
  ]
}

/// Climate axis name → index into [`noise::climate_ints`]. Only the three axes a
/// `<biome>` envelope can constrain; aether/rarity aren't biome selectors.
const BIOME_AXES: [(&str, usize); 3] =
  [("elevation", 0), ("temperature", 1), ("humidity", 2)];

/// The default tile card name a biome's `@define` store declares (`&tile.0`),
/// `card::` prefix stripped. `None` if it set no tile.
fn tile_name(store: &Store) -> Option<String> {
  match store.read("tile") {
    Some(Cell::Arr(v)) => match v.first() {
      Some(Cell::Sym(sym)) => Some(sym.strip_prefix("card::").unwrap_or(sym).to_string()),
      _ => None,
    },
    _ => None,
  }
}

/// Select the biome for a cell and return its default tile card name. Walks
/// `bundle.biomes` in declaration order, running each `@define` to read its
/// climate-range envelope (`70 100 &elevation range`), and returns the first
/// biome whose every declared axis contains the cell's climate. An axis a biome
/// doesn't declare is unconstrained. Falls back to the last biome's tile if
/// none match (the broadest by convention — `plains` today). `None` only if
/// there are no biomes.
pub fn select_biome(bundle: &Bundle, climate: &[f64; AXIS_COUNT]) -> Option<String> {
  let mut fallback = None;
  for (_, node) in &bundle.biomes {
    let Some(define) = node.hook("define") else { continue };
    let mut s = Store::default();
    let _ = run(&define.body, &mut s, &[], &Catalog::default(), &Functions::default());

    let contained = BIOME_AXES.iter().all(|(axis, idx)| match s.read(axis) {
      Some(Cell::Ranged { min, max, .. }) => {
        climate[*idx] >= *min as f64 && climate[*idx] <= *max as f64
      }
      _ => true, // axis not declared → unconstrained
    });

    let tile = tile_name(&s);
    fallback = tile.clone(); // last iteration's tile wins as the fallback
    if contained {
      if let Some(t) = tile {
        return Some(t);
      }
    }
  }
  fallback
}

/// Sample a tile's first two stock slots for a world cell. Runs the tile's
/// `:data @define` (declares slots + statics) then `@init` (climate-driven
/// `range`/`normalize`) with the `^biome` host, and reads the first two
/// stock-schema aspects, clamped to the Zone's 2-bit budget (D4). Further
/// declared stocks are ignored for terrain — same as legacy `pick_stocks_for`.
fn tile_stock(bundle: &Bundle, tile: &str, global_q: i32, global_r: i32, seed: u64) -> [u8; 2] {
  let Some(data) = bundle.card(tile).and_then(|c| c.facet("data")) else {
    return [0, 0];
  };
  let mut s = Store::default();
  if let Some(define) = data.hook("define") {
    let _ = run(&define.body, &mut s, &[], &bundle.catalog, &bundle.functions);
  }
  if let Some(init) = data.hook("init") {
    let host = biome_host(global_q, global_r, seed);
    let _ = run(&init.body, &mut s, &host, &bundle.catalog, &bundle.functions);
  }

  /// Zone tile stock slots are 2 bits — values clamp to `0..=3`.
  const ZONE_STOCK_MAX: i64 = 3;
  let schema = crate::bridge::stock_schema(bundle, tile);
  let mut out = [0u8; 2];
  for (i, (aspect, _bits)) in schema.iter().take(2).enumerate() {
    let v = s.read(&format!("aspect.{aspect}")).map(Cell::as_int).unwrap_or(0);
    out[i] = v.clamp(0, ZONE_STOCK_MAX) as u8;
  }
  out
}

/// Generate the Zone tile slot value for a world hex: the chosen tile's `def_id`
/// (within the `tile` type — the Zone row carries the type once) and its two
/// 2-bit stock values. Returns `(0, [0, 0])` if no biome or tile resolves (the
/// renderer treats `def_id == 0` as empty).
pub fn generate_tile(bundle: &Bundle, global_q: i32, global_r: i32, seed: u64) -> (u16, [u8; 2]) {
  let climate = noise::climate_floats(global_q, global_r, seed);
  let Some(tile) = select_biome(bundle, &climate) else {
    return (0, [0, 0]);
  };
  let def_id = bundle.type_def_id(&tile).unwrap_or(0);
  let stock = tile_stock(bundle, &tile, global_q, global_r, seed);
  (def_id, stock)
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
  use super::*;
  use crate::loader::load;

  // A mini corpus: the aspects the tiles use, two biomes (forest envelope +
  // plains fallback), and two tile cards (forest stocks pine via @init; plains
  // is bare). Mirrors the shape of the real content/data without its bulk.
  fn bundle() -> Bundle {
    let aspects = "<aspect>\n\
      \x20 ::type>\n    @define>\n      traits &section set\n\
      \x20 ::cost>\n    @define>\n      traits &section set\n\
      \x20 ::pine>\n    @define>\n      aspects &section set\n\
      \x20 ::flora>\n    @define>\n      aspects &section set\n";
    let biomes = "<biome>\n\
      \x20 ::forest>\n    @define>\n      30 75 &elevation range\n      20 70 &temperature range\n      55 95 &humidity range\n      1 &tile array\n      $card::forest &tile.0 set\n\
      \x20 ::plains>\n    @define>\n      1 &tile array\n      $card::plains &tile.0 set\n";
    let cards = "<card>\n\
      \x20 ::forest>\n    :data>\n      @define>\n        2 &aspect.pine stock\n        2 &aspect.flora stock\n        tile &aspect.type set\n        30 &aspect.cost set\n      @init>\n        ^biome call &biome set\n        0 3 &aspect.pine range\n        &aspect.pine *biome.humidity normalize\n        0 2 &aspect.flora range\n        &aspect.flora *biome.humidity normalize\n\
      \x20 ::plains>\n    :data>\n      @define>\n        tile &aspect.type set\n        5 &aspect.cost set\n";
    load(&[
      ("a.rd".into(), aspects.into()),
      ("b.rd".into(), biomes.into()),
      ("c.rd".into(), cards.into()),
    ])
    .expect("clean load")
  }

  #[test]
  fn biomes_indexed_in_declaration_order() {
    let b = bundle();
    let names: Vec<&str> = b.biomes.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["forest", "plains"]);
  }

  #[test]
  fn select_biome_envelope_then_fallback() {
    let b = bundle();
    // inside the forest envelope (elev 58, temp 56, humid 75)
    assert_eq!(select_biome(&b, &[58.0, 56.0, 75.0, 15.0, 50.0]).as_deref(), Some("forest"));
    // outside it (humidity 10 < 55) → falls back to the last biome, plains
    assert_eq!(select_biome(&b, &[58.0, 56.0, 10.0, 15.0, 50.0]).as_deref(), Some("plains"));
  }

  #[test]
  fn type_def_id_is_per_type_one_based() {
    let b = bundle();
    // forest + plains are the only tiles, sorted by name
    assert_eq!(b.card_type("forest").as_deref(), Some("tile"));
    assert_eq!(b.type_def_id("forest"), Some(1));
    assert_eq!(b.type_def_id("plains"), Some(2));
    // packed = [type:u4 | def_id:u12], tile = nibble 7
    assert_eq!(b.packed_def("forest"), Some((7 << 12) | 1));
    assert_eq!(b.packed_def("plains"), Some((7 << 12) | 2));
  }

  #[test]
  fn generate_tile_at_origin_is_forest_with_climate_stock() {
    // WORLD_SEED origin (0,0) climate lands in the forest envelope (see
    // noise::tests::origin_lands_in_forest_envelope): humidity 75.
    let b = bundle();
    let (def_id, stock) = generate_tile(&b, 0, 0, 0x27);
    assert_eq!(def_id, b.type_def_id("forest").unwrap());
    // pine: range 0..3 normalized by humidity 75 → 0 + 75*3/100 = 2 (floored).
    // flora: range 0..2 normalized by humidity 75 → 75*2/100 = 1.
    assert_eq!(stock, [2, 1]);
    // both within the 2-bit Zone budget
    assert!(stock.iter().all(|&s| s <= 3));
  }

  #[test]
  fn generate_tile_is_deterministic() {
    let b = bundle();
    assert_eq!(generate_tile(&b, 5, -3, 0x27), generate_tile(&b, 5, -3, 0x27));
  }
}
