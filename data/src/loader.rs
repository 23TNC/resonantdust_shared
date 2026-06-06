//! Content loader — parse a set of `.rd` sources into one ready-to-run [`Bundle`].
//!
//! This is the entry point both the gate (rlib) and the client (wasm) use to
//! turn the on-disk content into something executable: a [`SymbolTable`] for
//! resolution, a [`Catalog`] (assets / manifests / aspects, with their `@define`
//! records built), the global [`Functions`], and the card / recipe defs indexed
//! by id so a consumer can reach any card's `:data @init` or a recipe's
//! `@input`/`@output`. Content is read at *runtime* — no `include_str!`, so
//! editing a `.rd` never recompiles the crate.
//!
//! [`load`] also runs the corpus acceptance pass (validate + resolve every file
//! against the whole-corpus symbol table) and returns *all* problems on failure,
//! so a bad edit fails loading loudly rather than half-building.

use crate::parser::{parse, Header, Node, Stmt, Token};
use crate::resolve::{unresolved, SymbolTable};
use crate::validate::validate;
use crate::vm::{Catalog, Functions};
use std::collections::HashMap;

/// Card-type name → packed `card_type` nibble (the high 4 bits of a packed
/// definition `[type:u4 | def_id:u12]`). Mirrors `content/cards/types.json` —
/// the shared client/server type registry. Embedded here (rather than read from
/// JSON) because the set is tiny and stable; `type` is one of the registries
/// still deferred out of the DSL (cf. resolve.rs `DEFERRED_ROOTS`), so this is
/// the bridge until that migration (plan Phase 5). `id 3` is retired.
const TYPE_NIBBLES: &[(&str, u8)] = &[
  ("requisite", 0),
  ("blueprint", 1),
  ("revery", 2),
  ("discipline", 4),
  ("faculty", 5),
  ("soul", 6),
  ("tile", 7),
  ("mini_zone", 8),
  ("tile_decorator", 9),
  ("event", 10),
];

/// The `card_type` nibble for a type name, or `None` if unknown.
pub fn type_nibble(type_name: &str) -> Option<u8> {
  TYPE_NIBBLES.iter().find(|(n, _)| *n == type_name).map(|(_, id)| *id)
}

/// The **lineage** (version-stripped logical name) of a def name. A trailing
/// `.<digits>` is a version suffix — `apple.1` → `apple`, `apple.0` → `apple`;
/// a name without one is its own lineage — `corpus_dim` → `corpus_dim`.
///
/// Content versioning is append-only: `modify` ships a new version (a distinct
/// def with its own id + per-version schema) sharing the lineage. Recipes
/// reference the lineage (`$card::apple`), so they match **any** version of an
/// instance; a `create` emits the lineage **head** (see [`Bundle::card_head`]).
/// Old instances keep their version's def and drain naturally as recipes
/// consume them.
pub fn lineage(name: &str) -> &str {
  match name.rsplit_once('.') {
    Some((base, ver)) if !ver.is_empty() && ver.bytes().all(|b| b.is_ascii_digit()) => base,
    _ => name,
  }
}

/// The version number encoded in a def name's trailing `.<digits>` suffix, or
/// `0` for a bare (suffix-less) name. `apple.3` → `3`, `apple` → `0`.
pub fn version_of(name: &str) -> u32 {
  match name.rsplit_once('.') {
    Some((_, ver)) if !ver.is_empty() && ver.bytes().all(|b| b.is_ascii_digit()) => {
      ver.parse().unwrap_or(0)
    }
    _ => 0,
  }
}

/// A single load-time problem (parse error, or a validate/resolve diagnostic),
/// tagged with the file it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
  pub file: String,
  pub message: String,
}

/// Everything the runtime needs to execute content, built once at load.
#[derive(Default, Debug)]
pub struct Bundle {
  /// Whole-corpus symbol table (resolution + the aspect-member registry).
  pub table: SymbolTable,
  /// Asset / manifest / aspect records (their `@define` hooks run into cells).
  pub catalog: Catalog,
  /// Global `<functions:x>` bodies, callable via `$functions:x call`.
  pub functions: Functions,
  /// Card defs by name (the `::id` node — navigate its `:data`/`:visuals` facets).
  pub cards: HashMap<String, Node>,
  /// Recipe defs by name (the `::id` node — navigate its `@input`/`@output`).
  pub recipes: HashMap<String, Node>,
  /// Card names ordered by def_id: a card's packed `def_id` is `index + 1`
  /// (1-based, 0 = none). **First-appearance order** across the loaded sources
  /// (in the order given) — append-only, so adding a def never shifts an
  /// existing id (old stored instances stay valid). Gate + client agree because
  /// both load the same canonically-ordered `/content`. No `id.json`.
  pub card_ids: Vec<String>,
  /// Recipe names ordered by id (1-based), same scheme — for packing a card's
  /// bound `magnetic.recipe` and naming recipes over the wire.
  pub recipe_ids: Vec<String>,
  /// `<biome>` defs in **declaration order** (`(name, node)`). Order is
  /// load-bearing: biome selection walks them and takes the first whose climate
  /// envelope contains the cell (cf. `crate::worldgen::select_biome`), so this
  /// is a `Vec`, not a map.
  pub biomes: Vec<(String, Node)>,
  /// `<blueprint>` defs by name (the `::id` node — its `@define` sets `&card`
  /// the blueprint card spawned on request, and `&output` the card it builds).
  pub blueprints: HashMap<String, Node>,
  /// Blueprint names ordered by id (1-based), same sorted scheme as cards /
  /// recipes — the discovery-bit index and wire id. Retires `blueprints/id.json`.
  pub blueprint_ids: Vec<String>,
  /// Per-card identity precomputed once at load (`card_type` / per-type
  /// `def_id` / packed def). The worldgen + decode paths hit these per tile /
  /// per row, so they must be O(1) lookups, not per-call AST walks.
  card_meta: HashMap<String, CardMeta>,
  /// Reverse index `packed def → card name`, for O(1) [`Bundle::name_for_packed`].
  packed_to_name: HashMap<u16, String>,
}

/// Precomputed per-card identity, derived once at load from the card's
/// `:data @define`. All three are `None` for a card that declares no type.
#[derive(Default, Debug, Clone)]
struct CardMeta {
  /// The `&aspect.type` literal (e.g. `tile`).
  card_type: Option<String>,
  /// 1-based index within the card's type (sorted-by-name).
  type_def_id: Option<u16>,
  /// `[type:u4 | def_id:u12]`, present only when the type is in [`type_nibble`].
  packed: Option<u16>,
}

impl Bundle {
  pub fn card(&self, name: &str) -> Option<&Node> {
    self.cards.get(name)
  }
  /// The recipe def `name` (exact), else the lineage head — so `$recipe::eat`
  /// resolves the latest version when only `eat.0`, `eat.1` exist.
  pub fn recipe(&self, name: &str) -> Option<&Node> {
    self
      .recipes
      .get(name)
      .or_else(|| self.recipe_head(name).and_then(|h| self.recipes.get(h)))
  }
  /// The head (highest-version) recipe name in `lineage`, or `None`.
  pub fn recipe_head(&self, lineage_name: &str) -> Option<&str> {
    self
      .recipe_ids
      .iter()
      .filter(|n| lineage(n) == lineage_name)
      .max_by_key(|n| version_of(n))
      .map(String::as_str)
  }
  /// Packed `def_id` for a card name (1-based; `None` if unknown).
  pub fn card_def_id(&self, name: &str) -> Option<u16> {
    self.card_ids.iter().position(|n| n == name).map(|i| i as u16 + 1)
  }
  /// Card name for a packed `def_id` (`def_id == 0` is the none sentinel).
  pub fn card_name(&self, def_id: u16) -> Option<&str> {
    (def_id != 0).then(|| self.card_ids.get(def_id as usize - 1)).flatten().map(String::as_str)
  }
  pub fn recipe_def_id(&self, name: &str) -> Option<u16> {
    let resolved = if self.recipes.contains_key(name) {
      name
    } else {
      self.recipe_head(name)?
    };
    self.recipe_ids.iter().position(|n| n == resolved).map(|i| i as u16 + 1)
  }
  pub fn recipe_name(&self, id: u16) -> Option<&str> {
    (id != 0).then(|| self.recipe_ids.get(id as usize - 1)).flatten().map(String::as_str)
  }

  pub fn blueprint(&self, name: &str) -> Option<&Node> {
    self.blueprints.get(name)
  }
  /// Blueprint id for a name (1-based; `None` if unknown). This id is both the
  /// discovery-bit index (`1 << (id - 1)` on the soul's `blueprints_*` field)
  /// and the wire id the gate/client route on.
  pub fn blueprint_def_id(&self, name: &str) -> Option<u16> {
    self.blueprint_ids.iter().position(|n| n == name).map(|i| i as u16 + 1)
  }
  /// Blueprint name for an id (`id == 0` is the none sentinel).
  pub fn blueprint_name(&self, id: u16) -> Option<&str> {
    (id != 0).then(|| self.blueprint_ids.get(id as usize - 1)).flatten().map(String::as_str)
  }

  /// The card-ref a blueprint's `@define` writes to `slot`, `card::` prefix
  /// stripped — `"card"` for the blueprint card spawned on request, `"output"`
  /// for the card it builds. `None` if the blueprint or slot is absent.
  fn blueprint_ref(&self, name: &str, slot: &str) -> Option<String> {
    let define = self.blueprint(name)?.hook("define")?;
    let mut s = crate::vm::Store::default();
    let _ = crate::vm::run(
      &define.body,
      &mut s,
      &[],
      &crate::vm::Catalog::default(),
      &crate::vm::Functions::default(),
    );
    match s.read(slot)? {
      crate::vm::Cell::Sym(sym) => Some(sym.strip_prefix("card::").unwrap_or(&sym).to_string()),
      _ => None,
    }
  }
  /// The recipe id a magnetic-flagged card is locked to — read from its
  /// `:data @define` `$recipe::<x> &magnetic.recipe set`. A magnetic card may
  /// only be consumed by this recipe (`recipe_state::validate_bindings`). `None`
  /// if the packed def is unknown or declares no magnetic recipe.
  pub fn magnetic_recipe_id(&self, packed: u16) -> Option<u16> {
    let name = self.name_for_packed(packed)?;
    let define = self.card(name)?.facet("data")?.hook("define")?;
    let mut s = crate::vm::Store::default();
    let _ = crate::vm::run(&define.body, &mut s, &[], &self.catalog, &self.functions);
    match s.read("magnetic.recipe")? {
      crate::vm::Cell::Sym(sym) => {
        self.recipe_def_id(sym.strip_prefix("recipe::").unwrap_or(&sym))
      }
      _ => None,
    }
  }

  /// The blueprint *card* a blueprint spawns into the wrench panel on request.
  pub fn blueprint_card(&self, name: &str) -> Option<String> {
    self.blueprint_ref(name, "card")
  }
  /// The card a blueprint builds in-world when used.
  pub fn blueprint_output(&self, name: &str) -> Option<String> {
    self.blueprint_ref(name, "output")
  }

  /// The blueprint id whose **`card`** (the spawned-on-request card) is
  /// `card_name`. Recipes unlock a blueprint by referencing its card
  /// (`$card::blueprint_nd_furnace`), which is distinct from the `<blueprint>`
  /// registry key (`nd_furnace`); this resolves that reference. `None` if no
  /// blueprint declares that card.
  pub fn blueprint_id_for_card(&self, card_name: &str) -> Option<u16> {
    self
      .blueprint_ids
      .iter()
      .position(|n| self.blueprint_card(n).as_deref() == Some(card_name))
      .map(|i| i as u16 + 1)
  }

  /// The card's `type` aspect — the literal set by `<type> &aspect.type set` in
  /// its `:data @define` (e.g. `tile`). The DSL authority for a card's type
  /// (D1), replacing the legacy `id.json` per-type tables. `None` if the card
  /// is unknown or declares no type. Precomputed at load (O(1)).
  pub fn card_type(&self, name: &str) -> Option<String> {
    self.card_meta.get(name).and_then(|m| m.card_type.clone())
  }

  /// The card's `def_id` **within its own type** (1-based), the value packed
  /// into the low 12 bits of `[type:u4 | def_id:u12]` and into a Zone tile slot
  /// (where the type is carried once at the row level). Ids are sorted-by-name
  /// within the type, so they're stable + content-derived. `None` if unknown.
  /// Precomputed at load (O(1)).
  pub fn type_def_id(&self, name: &str) -> Option<u16> {
    self.card_meta.get(name).and_then(|m| m.type_def_id)
  }

  /// The packed definition `[type:u4 | def_id:u12]` for a card — the wire form
  /// the gate / client / modules store and route on. `None` if the card is
  /// unknown or its type isn't in [`type_nibble`].
  ///
  /// Exact-name lookups are O(1) (the common path: every stored def name +
  /// every `$card::<exact>` ref). A miss falls back to the lineage **head**, so
  /// a `create $card::apple` against a versioned lineage (`apple.0`, `apple.1`)
  /// resolves to the latest version — the only path that pays the scan, and
  /// only when the bare lineage isn't itself a def.
  pub fn packed_def(&self, name: &str) -> Option<u16> {
    self
      .card_meta
      .get(name)
      .and_then(|m| m.packed)
      .or_else(|| self.card_head(name).and_then(|h| self.card_meta.get(h)?.packed))
  }

  /// The head (highest-version) card name in `lineage`, or `None` if no card
  /// belongs to it. `create $card::<lineage>` resolves here so new instances
  /// are always the latest version while existing instances keep their own.
  pub fn card_head(&self, lineage_name: &str) -> Option<&str> {
    self
      .card_ids
      .iter()
      .filter(|n| lineage(n) == lineage_name)
      .max_by_key(|n| version_of(n))
      .map(String::as_str)
  }

  /// Inverse of [`packed_def`]: the card name for a packed `[type:u4 | def_id:u12]`.
  /// The modules' decode bridge (replacing legacy `decode_definition().key`).
  /// `None` if no card has that type + def_id. O(1) via the load-time reverse map.
  pub fn name_for_packed(&self, packed: u16) -> Option<&str> {
    self.packed_to_name.get(&packed).map(String::as_str)
  }

  /// Whether a card declares a lifecycle — its `:data @define` writes a
  /// `magnetic.*` slot (`$recipe::x &magnetic.recipe set`, see status.rd). The
  /// DSL equivalent of the legacy def-flag `magnetic` bit, which `cards::write_at`
  /// inherits onto spawned rows. The module maps this to its own flag bit.
  pub fn is_magnetic(&self, name: &str) -> bool {
    let Some(define) = self.card(name).and_then(|d| d.facet("data")).and_then(|f| f.hook("define"))
    else {
      return false;
    };
    define.body.iter().any(|stmt| {
      matches!(stmt, Stmt::Instr(toks)
        if toks.iter().any(|t| matches!(t, Token::Slot(s) if s.starts_with("magnetic"))))
    })
  }
}

/// Parse every `(name, source)` into a [`Bundle`]. Collects the symbol table and
/// catalog across *all* files first (so cross-file `$` refs resolve), then
/// validates + resolves each. Returns the bundle only if the whole corpus is
/// clean; otherwise every problem found.
pub fn load(sources: &[(String, String)]) -> Result<Bundle, Vec<LoadError>> {
  let mut errors = Vec::new();
  let mut parsed: Vec<(&str, Node)> = Vec::new();
  for (name, text) in sources {
    match parse(text) {
      Ok(node) => parsed.push((name, node)),
      Err(e) => errors.push(LoadError { file: name.clone(), message: format!("parse: {e}") }),
    }
  }

  // Build the corpus view from every successfully-parsed file. `index_defs`
  // records `*_ids` in **first-appearance order** across the sources (in the
  // order given), so a def's id is its position in that order — append-only.
  let mut b = Bundle::default();
  for (_, node) in &parsed {
    b.table.collect(node);
    b.catalog.add_assets(node);
    b.catalog.add_manifest(node);
    b.catalog.add_aspects(node);
    b.catalog.add_globals(node);
    b.functions.add(node);
    index_defs(
      node,
      &mut b.cards,
      &mut b.card_ids,
      &mut b.recipes,
      &mut b.recipe_ids,
      &mut b.biomes,
      &mut b.blueprints,
      &mut b.blueprint_ids,
    );
  }

  // Ids are content-derived + append-stable: the caller serves sources in a
  // canonical order (gate: base first, runtime adds appended), and gate + client
  // both load that same ordered `/content`, so they agree by construction. A new
  // def appends at the end and never renumbers an existing one — old stored
  // instances keep pointing at the same def. (No sort; no id.json.)

  // Precompute per-card identity once. Walking `card_ids` in order assigns each
  // type its 1-based, append-stable, content-derived `def_id`.
  // Replaces the old per-call AST walks (worldgen hit `type_def_id` per tile and
  // `name_for_packed` was O(n²)).
  let card_ids = b.card_ids.clone();
  let mut type_counters: HashMap<String, u16> = HashMap::new();
  for name in &card_ids {
    let card_type = b.cards.get(name).and_then(parse_card_type);
    let (type_def_id, packed) = match &card_type {
      Some(ty) => {
        let counter = type_counters.entry(ty.clone()).or_insert(0);
        *counter += 1;
        let packed = type_nibble(ty).map(|nib| crate::bits::pack_def(nib, *counter));
        (Some(*counter), packed)
      }
      None => (None, None),
    };
    if let Some(p) = packed {
      b.packed_to_name.insert(p, name.clone());
    }
    b.card_meta.insert(name.clone(), CardMeta { card_type, type_def_id, packed });
  }

  // Acceptance: validate (per-file) + resolve (whole-corpus) every file.
  for (name, node) in &parsed {
    for d in validate(node) {
      errors.push(LoadError { file: name.to_string(), message: format!("[{}] {}", d.path, d.message) });
    }
    for d in unresolved(node, &b.table) {
      errors.push(LoadError { file: name.to_string(), message: format!("[{}] {}", d.path, d.message) });
    }
  }

  if errors.is_empty() {
    Ok(b)
  } else {
    Err(errors)
  }
}

/// Walk a card def's `:data @define` for `<value> &aspect.type set` → the type
/// literal (e.g. `tile`). The DSL authority for a card's type (D1). `None` if it
/// declares none. Run once per card at load to populate [`CardMeta`].
fn parse_card_type(node: &Node) -> Option<String> {
  let define = node.facet("data")?.hook("define")?;
  for stmt in &define.body {
    let Stmt::Instr(toks) = stmt else { continue };
    // `<value> &aspect.type set` — value is the token before the slot.
    let Some(slot) = toks.iter().position(|t| matches!(t, Token::Slot(s) if s == "aspect.type"))
    else {
      continue;
    };
    if !matches!(toks.last(), Some(Token::Word(w)) if w == "set") || slot == 0 {
      continue;
    }
    return match &toks[slot - 1] {
      Token::Word(w) => Some(w.clone()),
      Token::Const(s) => Some(s.clone()),
      _ => None,
    };
  }
  None
}

/// Index `<card>` / `<recipe>` / `<blueprint>` defs by id (inline facet stripped:
/// `::a:visuals` → `a`), and collect `<biome>` defs in declaration order.
#[allow(clippy::too_many_arguments)]
fn index_defs(
  node: &Node,
  cards: &mut HashMap<String, Node>,
  card_ids: &mut Vec<String>,
  recipes: &mut HashMap<String, Node>,
  recipe_ids: &mut Vec<String>,
  biomes: &mut Vec<(String, Node)>,
  blueprints: &mut HashMap<String, Node>,
  blueprint_ids: &mut Vec<String>,
) {
  for bucket in &node.children {
    match &bucket.header {
      Header::Bucket(n) if n == "biome" => {
        for d in &bucket.children {
          if let Header::Def(id) = &d.header {
            biomes.push((id.clone(), d.clone()));
          }
        }
      }
      _ => {
        // Ids are assigned by **first-appearance order** (see [`Bundle::card_ids`])
        // — append-only, so a new def never shifts an existing id. The `order`
        // Vec records that order; push only on the first insert of a name.
        let (target, order) = match &bucket.header {
          Header::Bucket(n) if n == "card" => (&mut *cards, &mut *card_ids),
          Header::Bucket(n) if n == "recipe" => (&mut *recipes, &mut *recipe_ids),
          Header::Bucket(n) if n == "blueprint" => (&mut *blueprints, &mut *blueprint_ids),
          _ => continue,
        };
        for d in &bucket.children {
          if let Header::Def(id) = &d.header {
            let key = id.split(':').next().unwrap_or(id).to_string();
            if target.insert(key.clone(), d.clone()).is_none() {
              order.push(key);
            }
          }
        }
      }
    }
  }
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
  use super::*;

  fn src(name: &str, text: &str) -> (String, String) {
    (name.to_string(), text.to_string())
  }

  #[test]
  fn loads_a_clean_corpus() {
    let srcs = vec![
      src("aspects.rd", "<aspect>\n  ::type>\n    @define>\n      traits &section set\n"),
      src("cards.rd", "<card>\n  ::forest>\n    :data>\n      @define>\n        tile &aspect.type set\n    :visuals>\n      @update>\n        $functions:ring call drop\n"),
      src("recipes.rd", "<recipe>\n  ::r>\n    @output>\n      10 &sys.duration set\n"),
      src("fns.rd", "<functions:ring>\n  0 ret\n"),
    ];
    let b = load(&srcs).expect("clean load");
    // defs indexed + navigable
    assert!(b.card("forest").is_some());
    assert!(b.recipe("r").is_some());
    assert!(b.card("forest").unwrap().facet("data").unwrap().hook("define").is_some());
    assert!(b.card("forest").unwrap().facet("visuals").unwrap().hook("update").is_some());
    // symbol table + functions populated
    assert!(b.table.aspects.contains("type"));
    assert!(b.table.functions.contains("ring"));
  }

  #[test]
  fn lineage_head_resolves_across_spaces() {
    // Versioned recipes + aspects; a ref to the bare lineage resolves to the
    // head, and the lineage is registered so `$recipe::eat` / `$aspect::wood`
    // validate at load even though no bare `eat`/`wood` def exists.
    let srcs = vec![
      src(
        "aspects.rd",
        "<aspect>\n  ::type>\n    @define>\n      traits &section set\n  ::wood.0>\n    @define>\n      aspects &section set\n  ::wood.1>\n    @define>\n      aspects &section set\n",
      ),
      src(
        "recipes.rd",
        "<recipe>\n  ::eat.0>\n    @output>\n      5 &sys.duration set\n  ::eat.1>\n    @output>\n      9 &sys.duration set\n",
      ),
    ];
    let b = load(&srcs).expect("clean load");

    // recipe lineage: bare `eat` → head (eat.1), both node + id.
    assert_eq!(b.recipe_head("eat"), Some("eat.1"));
    assert_eq!(b.recipe_def_id("eat"), b.recipe_def_id("eat.1"));
    assert_eq!(
      b.recipe("eat").map(|n| n as *const _),
      b.recipe("eat.1").map(|n| n as *const _),
    );

    // aspect lineage via the catalog: bare `wood` → head (wood.1).
    assert!(b.catalog.aspect("wood.0").is_some());
    assert_eq!(
      b.catalog.aspect("wood").map(|c| c as *const _),
      b.catalog.aspect("wood.1").map(|c| c as *const _),
    );

    // lineages registered so bare refs validate at load.
    assert!(b.table.recipes.contains("eat"));
    assert!(b.table.aspects.contains("wood"));
  }

  #[test]
  fn derives_stable_def_ids_from_content() {
    let srcs = vec![
      src("aspects.rd", "<aspect>\n  ::type>\n    @define>\n      traits &section set\n"),
      src("cards.rd", "<card>\n  ::forest>\n    :data>\n      @define>\n        tile &aspect.type set\n  ::desert>\n    :data>\n      @define>\n        tile &aspect.type set\n"),
      src("recipes.rd", "<recipe>\n  ::cut>\n    @output>\n      10 &sys.duration set\n  ::burn>\n    @output>\n      10 &sys.duration set\n"),
    ];
    let b = load(&srcs).unwrap();
    // ids are 1-based, assigned by FIRST-APPEARANCE order (forest before desert
    // in the source), not by name — so appends never renumber existing defs.
    assert_eq!(b.card_def_id("forest"), Some(1));
    assert_eq!(b.card_def_id("desert"), Some(2));
    assert_eq!(b.card_name(1), Some("forest"));
    assert_eq!(b.card_name(2), Some("desert"));
    // round-trip + sentinels
    assert_eq!(b.card_name(b.card_def_id("forest").unwrap()), Some("forest"));
    assert_eq!(b.card_name(0), None); // 0 = none
    assert_eq!(b.card_def_id("ghost"), None);
    // recipes get their own id space, also first-appearance (cut before burn)
    assert_eq!(b.recipe_def_id("cut"), Some(1));
    assert_eq!(b.recipe_def_id("burn"), Some(2));
    assert_eq!(b.recipe_name(2), Some("burn"));
  }

  #[test]
  fn appending_a_def_does_not_renumber_existing_ids() {
    // The keystone for safe runtime add/modify: a new def appends at the end and
    // never shifts an existing id, so already-stored instances stay valid.
    let base = vec![
      src("a.rd", "<aspect>\n  ::type>\n    @define>\n      traits &section set\n"),
      src(
        "cards.rd",
        "<card>\n  ::forest>\n    :data>\n      @define>\n        tile &aspect.type set\n  ::desert>\n    :data>\n      @define>\n        tile &aspect.type set\n",
      ),
    ];
    let b0 = load(&base).unwrap();
    let forest_id = b0.packed_def("forest").unwrap();
    let desert_id = b0.packed_def("desert").unwrap();

    // Append a new card whose name sorts FIRST ("aaa"). Under the old
    // sort-by-name scheme it would steal id 1 and shift forest/desert; with
    // append-stable ids they keep their ids and the new def lands last.
    let mut added = base.clone();
    added.push(src(
      "added.rd",
      "<card>\n  ::aaa>\n    :data>\n      @define>\n        tile &aspect.type set\n",
    ));
    let b1 = load(&added).unwrap();

    assert_eq!(b1.packed_def("forest"), Some(forest_id), "forest id must not move");
    assert_eq!(b1.packed_def("desert"), Some(desert_id), "desert id must not move");
    assert_eq!(b1.type_def_id("aaa"), Some(3), "new def appends after existing");
  }

  #[test]
  fn name_for_packed_round_trips_and_detects_magnetic() {
    let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n";
    // forest/desert tiles (type nibble 7); `status` declares a lifecycle.
    let cards = "<card>\n\
      \x20 ::desert>\n    :data>\n      @define>\n        tile &aspect.type set\n\
      \x20 ::forest>\n    :data>\n      @define>\n        tile &aspect.type set\n\
      \x20 ::status>\n    :data>\n      @define>\n        faculty &aspect.type set\n        60000 &magnetic.duration set\n";
    let b = load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into())]).unwrap();

    // packed_def → name_for_packed round-trip (tile nibble 7)
    let p = b.packed_def("forest").unwrap();
    assert_eq!(crate::bits::unpack_def(p).0, 7);
    assert_eq!(b.name_for_packed(p), Some("forest"));
    assert_eq!(b.name_for_packed(b.packed_def("desert").unwrap()), Some("desert"));
    assert_eq!(b.name_for_packed(b.packed_def("status").unwrap()), Some("status"));
    // unknown packed → none
    assert_eq!(b.name_for_packed(crate::bits::pack_def(7, 999)), None);

    // lifecycle detection: only `status` writes a magnetic.* slot
    assert!(b.is_magnetic("status"));
    assert!(!b.is_magnetic("forest"));
  }

  #[test]
  fn indexes_blueprints_with_card_and_output_refs() {
    let srcs = vec![
      src("aspects.rd", "<aspect>\n  ::type>\n    @define>\n      traits &section set\n"),
      src(
        "cards.rd",
        "<card>\n\
         \x20 ::blueprint_furnace>\n    :data>\n      @define>\n        blueprint &aspect.type set\n\
         \x20 ::building_furnace>\n    :data>\n      @define>\n        tile &aspect.type set\n",
      ),
      src(
        "blueprints.rd",
        "<blueprint>\n  ::furnace>\n    @define>\n      $card::blueprint_furnace &card set\n      $card::building_furnace &output set\n",
      ),
    ];
    let b = load(&srcs).expect("clean load");
    assert!(b.blueprint("furnace").is_some());
    // 1-based id (single blueprint) + round-trip
    assert_eq!(b.blueprint_def_id("furnace"), Some(1));
    assert_eq!(b.blueprint_name(1), Some("furnace"));
    assert_eq!(b.blueprint_name(0), None);
    // card / output refs resolve to bare card names
    assert_eq!(b.blueprint_card("furnace").as_deref(), Some("blueprint_furnace"));
    assert_eq!(b.blueprint_output("furnace").as_deref(), Some("building_furnace"));
    // and those names pack to real defs
    assert!(b.packed_def("blueprint_furnace").is_some());
    assert!(b.packed_def("building_furnace").is_some());
  }

  #[test]
  fn surfaces_resolve_errors() {
    // `aspect.ghost` member is not in any <aspect> registry -> a load error
    let errs = load(&[src("bad.rd", "<functions:f>\n  2 &aspect.ghost set\n")]).unwrap_err();
    assert!(errs.iter().any(|e| e.file == "bad.rd" && e.message.contains("ghost")), "{errs:?}");
  }

  #[test]
  fn surfaces_parse_errors() {
    // an instruction at structural level is a parse error
    let errs = load(&[src("p.rd", "<card>\n  ::x>\n  10 &aspect.cost set\n")]).unwrap_err();
    assert!(errs.iter().any(|e| e.file == "p.rd" && e.message.starts_with("parse:")), "{errs:?}");
  }
}
