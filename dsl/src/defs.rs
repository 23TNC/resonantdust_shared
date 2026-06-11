//! Client render/UI definition surface — the DSL-native replacement for the
//! legacy `resonantdust_content` `decodeDefinition` / `aspectInfo` / `aspectValue`.
//!
//! These read a card's `:data` + `:visuals` facets (and the `<aspect>` registry)
//! out of a loaded [`Bundle`] and return render-ready shapes the pixijs client
//! consumes. The shapes are **DSL-native** (the client reshapes to them): visuals
//! are `shape` + `color.{bg,title,text}` + `objects[]` + `texture`; aspects are
//! name-keyed with a multi-parent `satisfies` LUT (not numeric id + single
//! parent). Pure over `&Bundle`.

use crate::loader::Bundle;
use crate::vm::{run, Cell, Store};
use serde::Serialize;

/// One row-mutable aspect slot — `<bits> &aspect.<name> stock` in `:data`.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StockSlot {
  pub aspect: String,
  /// Cap (the declared `<bits>` magnitude; the zone stores it in 2 bits).
  pub max: i64,
  /// Seed value the `:data @define` set the aspect to.
  pub default: i64,
  /// Display visibility — `:visuals` override or the aspect's registry default.
  pub visibility: u8,
}

/// A card's render/UI definition, derived from its `.rd` facets.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct CardRenderDef {
  pub card_type: u8,
  pub def_id: u16,
  pub key: String,
  /// The card's type *name* (`&aspect.type`, e.g. `"requisite"`) — the locale
  /// namespace (`cards.<type>.<key>.label`).
  pub type_name: String,
  /// `"hex"` | `"rect"` (from `&shape`, falling back to the type's hex-ness).
  pub shape: String,
  /// `0xRRGGBB` (the DSL `#hex` colors parse to integers, like the legacy shape).
  pub color_bg: i64,
  pub color_title: i64,
  pub color_text: i64,
  /// Card-body texture asset name (from `&texture`), or `None`.
  pub texture: Option<String>,
  /// Card-art asset names (from `&objects`), in order.
  pub objects: Vec<String>,
  /// Static `(aspect, value, visibility)` triples from `:data @define` (excludes
  /// the `type` symbol; includes numeric aspects like `cost`, `wood`, …). The
  /// visibility is the card's `:visuals` override or the aspect's registry default.
  pub aspects: Vec<(String, i64, u8)>,
  /// Row-mutable stock slots (positional, mapping to the zone's per-tile slots).
  pub stock: Vec<StockSlot>,
  /// Magnetic lifecycle recipe name (`&magnetic.recipe`), or `None`.
  pub lifecycle_recipe: Option<String>,
  /// Magnetic phase duration in ms (`&magnetic.duration`), or `None`.
  pub lifecycle_duration_ms: Option<u64>,
}

/// An `<aspect>` registry record — DSL-native (multi-parent `satisfies`).
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct AspectInfo {
  pub name: String,
  /// Display glyph (inherited from the first satisfied aspect if omitted).
  pub icon: String,
  /// `0xRRGGBB` (inherited if omitted), or `0` if none in the chain.
  pub color: i64,
  /// Default display visibility (`&visibility`): `0` hidden · `1` aspect slot ·
  /// `2` function slot. A card's `:visuals` can override per aspect. Absent = `1`.
  pub visibility: u8,
  /// Aspects this one IS-A (`&satisfies` names), for matcher widening + display.
  pub satisfies: Vec<String>,
  /// Render sprite asset name (`&art`), or `None`.
  pub art: Option<String>,
}

/// A 2-component float vector — anchor pivot `0..1`, or primitive coords in
/// card-space `0..100` (`vec2` → `{x, y}`).
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Vec2f {
  pub x: f64,
  pub y: f64,
}

/// A min/max scale envelope, `0..1` (the DSL stores hundredths).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ScaleF {
  pub min: f64,
  pub max: f64,
}

/// An `<asset>` pack's render metadata — the client's texture registry entry.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TextureDef {
  pub id: u32,
  pub name: String,
  pub size: i64,
  pub scale: ScaleF,
  pub anchor: Vec2f,
}

/// Every `<asset>` pack as a [`TextureDef`] (id = sorted index + 1). `scale` /
/// `anchor` are converted from the DSL's hundredths to `0..1`; defaults are
/// scale `1.0` and anchor `(0.5, 0.5)`.
pub fn all_textures(bundle: &Bundle) -> Vec<TextureDef> {
  bundle
    .catalog
    .asset_names()
    .into_iter()
    .enumerate()
    .map(|(i, name)| {
      let cell = bundle.catalog.asset(&name);
      let field = |k: &str| -> Option<&Cell> {
        match cell {
          Some(Cell::Map(m)) => m.iter().find(|(kk, _)| kk == k).map(|(_, v)| v),
          _ => None,
        }
      };
      let size = field("size").map(Cell::as_int).unwrap_or(0);
      let scale = match field("scale") {
        Some(Cell::Ranged { min, max, .. }) => ScaleF { min: *min as f64 / 100.0, max: *max as f64 / 100.0 },
        _ => ScaleF { min: 1.0, max: 1.0 },
      };
      let anchor = match field("anchor") {
        Some(Cell::Map(m)) => {
          let g = |k: &str| m.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.as_int()).unwrap_or(50);
          Vec2f { x: g("x") as f64 / 100.0, y: g("y") as f64 / 100.0 }
        }
        _ => Vec2f { x: 0.5, y: 0.5 },
      };
      TextureDef { id: i as u32 + 1, name, size, scale, anchor }
    })
    .collect()
}

/// One recipe iterator for the client's discovery matcher: the resolved
/// parent-path prefix (`""` for top-level), its branch selector, and the
/// aggregated hold policy of the verbs that bind it (for predicted holds).
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IterMeta {
  pub parent: String,
  pub branch: u8,
  pub slot_hold: bool,
  pub position_hold: bool,
}

/// Everything the client needs about a recipe WITHOUT the legacy path-statement
/// IR (predicate eval runs on the VM via `match_recipe`): iterators (in
/// `bindings` order), the anchor set (for priority), per-branch arity (combo
/// search), input-statement count (debounce bypass), and hold policy (predicted
/// holds).
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecipeMeta {
  pub iterators: Vec<IterMeta>,
  /// Whether the recipe anchors on the action's `root`.
  pub root: bool,
  /// Bitmask of top-level branches referenced (bit `n` = branch `n`).
  pub branches: u16,
  pub root_slot_hold: bool,
  pub root_position_hold: bool,
  /// Number of `@input` statements (≤1 → single-input, debounce-bypass).
  pub input_count: u32,
  /// Required card count per top-level branch (`branch_counts[b]` = max offset
  /// referenced in branch `b`, +1). Index 0..=2.
  pub branch_counts: [u8; 3],
}

/// `(slot, position)` hold a verb claims. Mirrors `dsl_recipe::kinds`.
fn verb_holds(verb: &str) -> Option<(bool, bool)> {
  match verb {
    "use" => Some((true, false)),
    "claim" => Some((true, true)),
    "share" => Some((false, true)),
    "borrow" => Some((false, false)),
    _ => None,
  }
}

/// The `(parent, branch)` of a slot path's LAST `slot.<b>.<o>` triple — which
/// iterator a held `&path` belongs to. `None` for `root` / pathless.
fn path_iter(path: &str) -> Option<(String, u8)> {
  let segs: Vec<&str> = path.split('.').collect();
  let mut prefix = String::new();
  let mut last: Option<(String, u8)> = None;
  let mut i = 0;
  while i < segs.len() {
    if segs[i] == "slot" && i + 2 < segs.len() {
      if let (Ok(b), Ok(_o)) = (segs[i + 1].parse::<u8>(), segs[i + 2].parse::<u32>()) {
        last = Some((prefix.clone(), b));
        if !prefix.is_empty() {
          prefix.push('.');
        }
        prefix.push_str(&format!("slot.{}.{}", segs[i + 1], segs[i + 2]));
        i += 3;
        continue;
      }
    }
    if !prefix.is_empty() {
      prefix.push('.');
    }
    prefix.push_str(segs[i]);
    i += 1;
  }
  last
}

/// Build a recipe's full [`RecipeMeta`]. `None` if unknown.
pub fn recipe_meta(bundle: &Bundle, name: &str) -> Option<RecipeMeta> {
  use crate::parser::{Stmt, Token};
  let node = bundle.recipe(name)?;
  let iters = crate::recipe::iterators(node);
  let mut branches = 0u16;
  for it in &iters {
    if it.parent.is_empty() {
      branches |= 1 << it.branch;
    }
  }
  let mut meta = RecipeMeta {
    iterators: iters
      .iter()
      .map(|i| IterMeta { parent: i.parent.clone(), branch: i.branch, slot_hold: false, position_hold: false })
      .collect(),
    root: false,
    branches,
    root_slot_hold: false,
    root_position_hold: false,
    input_count: node.hook("input").map(|h| h.body.len() as u32).unwrap_or(0),
    branch_counts: [0; 3],
  };

  for hook in ["input", "output"] {
    let Some(h) = node.hook(hook) else { continue };
    for stmt in &h.body {
      let Stmt::Instr(toks) = stmt else { continue };
      // anchor + arity: scan every slot/value path's triples.
      for tok in toks {
        if let Token::Slot(p) | Token::Value(p) = tok {
          if p == "root" || p.starts_with("root.") {
            meta.root = true;
          }
          // top-level branch arity from each `slot.<b>.<o>` triple.
          let segs: Vec<&str> = p.split('.').collect();
          if segs.first() == Some(&"slot") && segs.len() >= 3 {
            if let (Ok(b), Ok(o)) = (segs[1].parse::<usize>(), segs[2].parse::<u8>()) {
              if b < 3 {
                meta.branch_counts[b] = meta.branch_counts[b].max(o + 1);
              }
            }
          }
        }
      }
      // hold policy: the `&<target> <verb>` write at the tail.
      let Some(Token::Word(verb)) = toks.last() else { continue };
      let Some((slot, pos)) = verb_holds(verb) else { continue };
      let Some(target) = toks.iter().rev().find_map(|t| match t {
        Token::Slot(p) => Some(p.clone()),
        _ => None,
      }) else {
        continue;
      };
      if target == "root" || target.starts_with("root.") {
        meta.root_slot_hold |= slot;
        meta.root_position_hold |= pos;
      } else if let Some((parent, branch)) = path_iter(&target) {
        if let Some(im) = meta.iterators.iter_mut().find(|im| im.parent == parent && im.branch == branch) {
          im.slot_hold |= slot;
          im.position_hold |= pos;
        }
      }
    }
  }
  Some(meta)
}

/// A blueprint catalog entry, resolved from the `<blueprint>` bucket.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct BlueprintInfo {
  /// Bundle blueprint id (the discovery-bit index + wire id).
  pub id: u16,
  /// Blueprint registry key (e.g. `"nd_furnace"`).
  pub key: String,
  /// The blueprint *card* spawned on request, + its packed def.
  pub blueprint_card: String,
  pub blueprint_packed: u16,
  /// The card it builds in-world, + its packed def.
  pub output_card: String,
  pub output_packed: u16,
}

/// Resolve a blueprint by Bundle id. `None` if unknown.
pub fn blueprint_info(bundle: &Bundle, id: u16) -> Option<BlueprintInfo> {
  let key = bundle.blueprint_name(id)?.to_string();
  let blueprint_card = bundle.blueprint_card(&key)?;
  let output_card = bundle.blueprint_output(&key)?;
  Some(BlueprintInfo {
    id,
    blueprint_packed: bundle.packed_def(&blueprint_card).unwrap_or(0),
    output_packed: bundle.packed_def(&output_card).unwrap_or(0),
    blueprint_card,
    output_card,
    key,
  })
}

/// Every blueprint in id order — the wrench-panel catalog.
pub fn all_blueprints(bundle: &Bundle) -> Vec<BlueprintInfo> {
  (1..=bundle.blueprint_ids.len() as u16).filter_map(|id| blueprint_info(bundle, id)).collect()
}

fn sym(c: Option<&Cell>) -> Option<String> {
  match c {
    Some(Cell::Sym(s)) => Some(s.clone()),
    _ => None,
  }
}

fn strip<'a>(s: &'a str, p: &str) -> &'a str {
  s.strip_prefix(p).unwrap_or(s)
}

/// Run a card facet's `@define` into a fresh [`Store`].
fn facet_store(bundle: &Bundle, card: &str, facet: &str) -> Store {
  let mut s = Store::default();
  if let Some(define) = bundle.card(card).and_then(|c| c.facet(facet)).and_then(|f| f.hook("define")) {
    let _ = run(&define.body, &mut s, &[], &bundle.catalog, &bundle.functions);
  }
  s
}

/// Build the [`CardRenderDef`] for a packed `[type:u4 | def_id:u12]`. `None` if
/// the packed def is unknown.
pub fn card_render_def(bundle: &Bundle, packed: u16) -> Option<CardRenderDef> {
  let name = bundle.name_for_packed(packed)?.to_string();
  let data = facet_store(bundle, &name, "data");
  let vis = facet_store(bundle, &name, "visuals");

  // visuals
  let shape = sym(vis.read("shape"))
    .map(|s| strip(&s, "shape.").to_string())
    .unwrap_or_else(|| {
      let nibble = bundle.card_type(&name).as_deref().and_then(crate::loader::type_nibble);
      if nibble.map(crate::inspect::is_hex_type).unwrap_or(false) { "hex".into() } else { "rect".into() }
    });
  let color = |k: &str| vis.read(k).map(Cell::as_int).unwrap_or(0);
  let objects = match vis.read("objects") {
    Some(Cell::Arr(v)) => v.iter().filter_map(|c| match c {
      Cell::Sym(s) => Some(strip(s, "asset::").to_string()),
      _ => None,
    }).collect(),
    _ => Vec::new(),
  };
  let texture = sym(vis.read("texture")).map(|s| strip(&s, "asset::").to_string());

  // Row-mutable stock slots. A stock-backed aspect is represented ONLY here —
  // never also as a static aspect — so a renderer reads its live per-row count
  // from the slot, not the meaningless static `0` the `stock` verb seeds (the
  // per-tile value is filled by @init at worldgen and stored in the zone /
  // tile-card, not the static def).
  // Effective display visibility for an aspect: the card's `:visuals` override,
  // else the aspect's registry default (1 = aspect slot if unknown).
  let visibility_of = |aspect: &str| -> u8 {
    vis.read(&format!("visibility.{aspect}"))
      .map(|c| c.as_int() as u8)
      .or_else(|| aspect_info(bundle, aspect).map(|i| i.visibility))
      .unwrap_or(1)
  };
  let stock: Vec<StockSlot> = crate::bridge::stock_schema(bundle, &name)
    .into_iter()
    .map(|(aspect, max)| {
      let default = data.read(&format!("aspect.{aspect}")).map(Cell::as_int).unwrap_or(0);
      let visibility = visibility_of(&aspect);
      StockSlot { aspect, max, default, visibility }
    })
    .collect();
  let is_stock = |k: &str| stock.iter().any(|s| s.aspect == k);

  // data: static aspects (numeric `aspect.*` entries, excluding `type` and any
  // stock-backed aspect — those live in `stock`, where the live value is read).
  let aspects = match data.read("aspect") {
    Some(Cell::Map(m)) => m.iter().filter_map(|(k, c)| {
      if is_stock(k) {
        return None;
      }
      match c {
        Cell::Int(n) => Some((k.clone(), *n, visibility_of(k))),
        Cell::Ranged { val, .. } => Some((k.clone(), *val, visibility_of(k))),
        _ => None,
      }
    }).collect(),
    _ => Vec::new(),
  };

  let lifecycle_recipe = sym(data.read("magnetic.recipe")).map(|s| strip(&s, "recipe::").to_string());
  let lifecycle_duration_ms = data.read("magnetic.duration").map(|c| c.as_int() as u64);

  Some(CardRenderDef {
    card_type: bundle.card_type(&name).as_deref().and_then(crate::loader::type_nibble).unwrap_or(0),
    def_id: bundle.type_def_id(&name).unwrap_or(0),
    type_name: sym(data.read("aspect.type")).unwrap_or_default(),
    key: name,
    shape,
    color_bg: color("color.bg"),
    color_title: color("color.title"),
    color_text: color("color.text"),
    texture,
    objects,
    aspects,
    stock,
    lifecycle_recipe,
    lifecycle_duration_ms,
  })
}

// ---------------------------------------------------------------------------
// Visual primitives — the DSL `:visuals` PrimList the client reconciler draws.
// ---------------------------------------------------------------------------

/// One reconcilable primitive emitted by `:visuals` (`&prims.<i>`). Mirrors the
/// client `VisualNode`. Numeric fields are floats (the VM now supports them);
/// `tint` is an `0xRRGGBB` color int; `texture` is an asset name; `text` is a
/// LOCALE KEY (the client resolves it — definitions never decode locales).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PrimNode {
  pub kind: String,
  pub pos: Vec2f,
  pub size: Vec2f,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub anchor: Option<Vec2f>,
  pub scale: f64,
  pub rot: f64,
  pub alpha: f64,
  pub tint: i64,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub texture: Option<String>,
  /// Variant index pinned from an `asset::pack:variant` texture (e.g.
  /// `requisite:axe` → 7). `None` → the client picks a variant by seed (tile
  /// objects). Lets the generic path render a SPECIFIC icon, not a random one.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub index: Option<i64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub text: Option<String>,
  /// `progress` primitive: the row to TRACK (an index into the card's progress
  /// list, `*d.progress.<i>.id`). The client resolves it to the row's timing and
  /// fills the bar live — the DSL doesn't compute the fraction.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub target: Option<i64>,
  /// `progress` primitive: the bar style (`*d.progress.<i>.style`; 1 = ltr,
  /// 2 = rtl). DSL-chosen.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub style: Option<i64>,
  /// Paint order WITHIN the card (`&h.z set`). Higher draws on top. `None` → the
  /// client falls back to push order (so `title`, pushed last, stays on top
  /// without anyone setting `z`). Independent of the card's z in its stack —
  /// that's the container's `stackZ` (nested sorts compose).
  #[serde(skip_serializing_if = "Option::is_none")]
  pub z: Option<i64>,
  /// `progress` primitive: which fraction SOURCE the client fills from. `None`/0
  /// = a progress row (`target` indexes `card_data.progress`); 1 = the action
  /// QUEUE/debounce fraction (the pre-propose bar). The DSL picks it (`&h.source`).
  #[serde(skip_serializing_if = "Option::is_none")]
  pub source: Option<i64>,
  /// Seed for the prim's `current` on FIRST creation, so it eases IN from this
  /// state instead of snapping to target. Each present field overrides the
  /// matching target field for the initial frame only (`&h.enter.h set`, …).
  /// Drives exit/entrance animations on freshly-spawned prims — e.g. a `^mask`
  /// with `enter.h` = the full card height and `size.y` (target) = 0 rolls up.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub enter: Option<EnterFields>,
}

/// Partial `current`-seed for a prim's `enter` (mirrors the client's
/// `Partial<AnimatableFields>`). Only the fields the DSL sets are emitted; the
/// rest fall back to the prim's target. Field names match the animatable set
/// (`w`/`h`, not `size.x`/`size.y` — `size` maps onto `w`/`h` client-side).
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct EnterFields {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub x: Option<f64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub y: Option<f64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub w: Option<f64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub h: Option<f64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub scale: Option<f64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub rot: Option<f64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub alpha: Option<f64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub tint: Option<i64>,
}

/// Run one facet hook into the (shared) store, if present.
fn run_into(bundle: &Bundle, card: &str, facet: &str, hook: &str, host: &[(String, Cell)], s: &mut Store) {
  if let Some(h) = bundle.card(card).and_then(|c| c.facet(facet)).and_then(|f| f.hook(hook)) {
    let _ = run(&h.body, s, host, &bundle.catalog, &bundle.functions);
  }
}

fn map_get<'a>(m: &'a [(String, Cell)], k: &str) -> Option<&'a Cell> {
  m.iter().find(|(kk, _)| kk == k).map(|(_, c)| c)
}

fn vec2_field(m: &[(String, Cell)], k: &str) -> Option<Vec2f> {
  match map_get(m, k) {
    Some(Cell::Map(mm)) => Some(Vec2f {
      x: map_get(mm, "x").map(Cell::as_f64).unwrap_or(0.0),
      y: map_get(mm, "y").map(Cell::as_f64).unwrap_or(0.0),
    }),
    _ => None,
  }
}

/// Convert one `prims` entry (`Cell::Map`) into a [`PrimNode`]. `None` if it
/// isn't a map or has no `kind`. Texture + index are resolved IN THE DSL (the
/// `:visuals` hook derefs `*pack.object` / `*pack.texture.<variant>`); this just
/// reads the fields the prim carries.
fn prim_from_cell(c: &Cell) -> Option<PrimNode> {
  let Cell::Map(m) = c else { return None };
  // `kind` is the symbol pushed by the `^hex`/`^rect`/`^sprite`/`^text` engine
  // intrinsic (`vm::PRIM_KINDS`) — already the bare kind, no prefix to strip.
  let kind = match map_get(m, "kind") {
    Some(Cell::Sym(s)) => s.clone(),
    _ => return None,
  };
  let f = |k: &str, dflt: f64| map_get(m, k).map(Cell::as_f64).unwrap_or(dflt);
  // texture: an object name the DSL already resolved (`*pack.object` →
  // `manifest::requisite`, or `*rec.art.object`); strip the namespace prefix to
  // the bare folder the client LOD keys on.
  let texture = match map_get(m, "texture") {
    Some(Cell::Sym(s)) => Some(strip(strip(s, "asset::"), "manifest::").to_string()),
    _ => None,
  };
  // index: the variant pinned in the DSL (`*pack.texture.<variant>`); unset →
  // the client picks a variant by seed (tile objects).
  let index = map_get(m, "index").map(Cell::as_int);
  Some(PrimNode {
    kind,
    pos: vec2_field(m, "pos").unwrap_or_default(),
    size: vec2_field(m, "size").unwrap_or_default(),
    anchor: vec2_field(m, "anchor"),
    scale: f("scale", 1.0),
    rot: f("rot", 0.0),
    alpha: f("alpha", 1.0),
    tint: map_get(m, "tint").map(Cell::as_int).unwrap_or(0xff_ffff),
    texture,
    index,
    // `text` is a locale key — kept verbatim (client resolves via locales).
    text: match map_get(m, "text") {
      Some(Cell::Sym(s)) => Some(s.clone()),
      _ => None,
    },
    // progress: which row to track + the bar style (DSL-set; client fills live).
    target: map_get(m, "target").map(Cell::as_int),
    style: map_get(m, "style").map(Cell::as_int),
    // intra-card paint order (unset → client uses push order).
    z: map_get(m, "z").map(Cell::as_int),
    // progress fill source (unset/0 = row, 1 = queue/debounce).
    source: map_get(m, "source").map(Cell::as_int),
    // enter: a partial `current`-seed map (`&h.enter.<field> set`). Each field
    // overrides the target for the prim's first frame only.
    enter: match map_get(m, "enter") {
      Some(Cell::Map(em)) => {
        let g = |k: &str| map_get(em, k).map(Cell::as_f64);
        Some(EnterFields {
          x: g("x"),
          y: g("y"),
          w: g("w"),
          h: g("h"),
          scale: g("scale"),
          rot: g("rot"),
          alpha: g("alpha"),
          tint: map_get(em, "tint").map(Cell::as_int),
        })
      }
      _ => None,
    },
  })
}

/// Run a card's `:visuals` hook against `host` and return its `&prims` list — the
/// client's reconcilable visual spec. Runs `:data` (`@define`+`@init`) then
/// `:visuals` (`@define`+`hook`) into one store, so a visual hook can read the
/// card's data aspects + static colours. `hook` is `"init"` (first draw, client
/// snaps current=target) or `"update"` (client eases to the new targets).
pub fn draw_visuals(bundle: &Bundle, packed: u16, host: &[(String, Cell)], hook: &str) -> Vec<PrimNode> {
  let Some(name) = bundle.name_for_packed(packed).map(str::to_string) else {
    return Vec::new();
  };
  let mut s = Store::default();
  run_into(bundle, &name, "data", "define", &[], &mut s);
  run_into(bundle, &name, "data", "init", host, &mut s);
  // Seed the card's identity so `:visuals` can author locale-keyed text (titles,
  // and later descriptions) WITHOUT hardcoding the key per card — the engine
  // hands the DSL the parts (`*sys.type`/`*sys.key`) plus the ready label key
  // (`*sys.label`). The `cards.<type>.<key>.label` scheme mirrors the client's
  // `DefinitionManager.label`; the DSL only forwards the key (the client still
  // resolves the string — definitions never decode locales). The data hook ran
  // first, so `aspect.type` is set.
  let type_name = sym(s.read("aspect.type")).unwrap_or_default();
  s.write("sys.key", Cell::Sym(name.clone()));
  s.write("sys.type", Cell::Sym(type_name.clone()));
  s.write("sys.label", Cell::Sym(format!("cards.{type_name}.{name}.label")));
  run_into(bundle, &name, "visuals", "define", &[], &mut s);
  run_into(bundle, &name, "visuals", hook, host, &mut s);
  match s.read("prims") {
    Some(Cell::Arr(v)) => v.iter().filter_map(prim_from_cell).collect(),
    _ => Vec::new(),
  }
}

/// Render a world tile through `:visuals` → `PrimList`, with the **stored** stock
/// overlaid (not recomputed from biome — the client has no world seed, and a
/// harvested tile's stock differs from its biome default). `stock` is the tile's
/// per-slot counts (positional, by `stock_schema`). Raw aspects are NOT folded
/// here (unlike `card_view`) — `ring_prims` pulls each aspect's own sprite, so
/// `pine` stays `pine`, not rolled up to `wood`. The LOD variant + faction are
/// chosen client-side (the sprite's `texture` is the bare object name). `seed`
/// is the tile's `(q,r)` hash, exposed to `:visuals` as `^seed` so the scatter
/// (ring-slot angles, per-object scale) is deterministic per tile.
pub fn tile_prims(bundle: &Bundle, packed: u16, stock: &[i64], seed: i64) -> Vec<PrimNode> {
  let Some(name) = bundle.name_for_packed(packed).map(str::to_string) else {
    return Vec::new();
  };
  let host = [("seed".to_string(), Cell::Int(seed))];
  let mut s = Store::default();
  run_into(bundle, &name, "data", "define", &[], &mut s);
  for (i, (aspect, _)) in crate::bridge::stock_schema(bundle, &name).iter().enumerate() {
    if let Some(v) = stock.get(i) {
      s.write(&format!("aspect.{aspect}"), Cell::Int(*v));
    }
  }
  run_into(bundle, &name, "visuals", "define", &[], &mut s);
  run_into(bundle, &name, "visuals", "init", &host, &mut s);
  match s.read("prims") {
    Some(Cell::Arr(v)) => v.iter().filter_map(prim_from_cell).collect(),
    _ => Vec::new(),
  }
}

/// The `<aspect>` record for `name`, resolving inherited `icon`/`color` up the
/// `satisfies` chain (first satisfied aspect wins). `None` if unknown.
pub fn aspect_info(bundle: &Bundle, name: &str) -> Option<AspectInfo> {
  let cell = bundle.catalog.aspect(name)?;
  let field = |k: &str| match cell {
    Cell::Map(m) => m.iter().find(|(kk, _)| kk == k).map(|(_, v)| v),
    _ => None,
  };
  let satisfies: Vec<String> = match field("satisfies") {
    Some(Cell::Arr(v)) => v.iter().filter_map(|c| match c {
      Cell::Sym(s) => Some(strip(s, "aspect::").to_string()),
      _ => None,
    }).collect(),
    _ => Vec::new(),
  };
  // icon/color inherit from the first satisfied aspect when omitted (bounded
  // walk up the satisfies chain). Returns the resolved cell so the caller
  // extracts a Sym (icon) or Int (color) as appropriate.
  let inherited = |key: &str| -> Option<Cell> {
    if let Some(v) = field(key) {
      return Some(v.clone());
    }
    let mut cur = satisfies.first().cloned();
    for _ in 0..8 {
      let parent = cur?;
      let Some(Cell::Map(m)) = bundle.catalog.aspect(&parent) else { break };
      if let Some(v) = m.iter().find(|(k, _)| k == key).map(|(_, v)| v) {
        return Some(v.clone());
      }
      cur = m.iter().find(|(k, _)| k == "satisfies").and_then(|(_, v)| match v {
        Cell::Arr(a) => a.first().and_then(|c| match c {
          Cell::Sym(s) => Some(strip(s, "aspect::").to_string()),
          _ => None,
        }),
        _ => None,
      });
    }
    None
  };
  Some(AspectInfo {
    name: name.to_string(),
    icon: inherited("icon").and_then(|c| match c { Cell::Sym(s) => Some(s), _ => None }).unwrap_or_default(),
    color: inherited("color").map(|c| c.as_int()).unwrap_or(0),
    visibility: field("visibility").map(|c| c.as_int() as u8).unwrap_or(1),
    satisfies,
    art: sym(field("art")).map(|s| strip(&s, "asset::").to_string()),
  })
}

/// The folded value of a named aspect on a packed card def — the static aspect
/// rolled up the `satisfies` hierarchy (what a recipe predicate reads). `None`
/// when the def doesn't carry the aspect at all; `Some(0)` when it carries the
/// aspect with value 0. Callers rely on that distinction (e.g. "has an
/// inventory" vs "infinite-capacity inventory"), so don't `unwrap_or(0)` here —
/// that collapses absent into `Some(0)`, making every card look like it carries
/// every aspect.
pub fn aspect_value(bundle: &Bundle, packed: u16, name: &str) -> Option<i64> {
  // Resolve packed → name → GLOBAL card def_id. `card_view`/`card_name` index a
  // global card list, so feeding the raw `unpack_def(packed).1` (the TYPE-LOCAL
  // 12-bit def_id) resolves the wrong card — it only happened to work when a
  // card's type-local id equalled its global index (true in tiny test bundles,
  // false in the real corpus, where it silently read a different card's
  // aspects). `name_for_packed` keys the full packed value, matching
  // `card_render_def`.
  let def_id = bundle.card_def_id(bundle.name_for_packed(packed)?)?;
  let view = crate::bridge::card_view(bundle, &crate::bridge::Card { def_id, stock: Vec::new() });
  Store::with_root(view).read(&format!("aspect.{name}")).map(Cell::as_int)
}

/// A card's stacking bit-fields (the bundle-aware lookup behind the canonical
/// [`resonantdust_codec::stacking`] bit math). `stack_joins` declared marks an
/// explicit config (tiles/events join only the hex stack, `0b0010`); absent →
/// the regular-card default (hosts `0b1110`, joins `0b1100`). `stack_hosts`
/// defaults to 0 when only joins is set (a tile hosts nothing). Bit `i` = stack
/// `i` (0 = loose).
pub fn stack_bits(bundle: &Bundle, packed: u16) -> resonantdust_codec::stacking::StackBits {
  use resonantdust_codec::stacking::{StackBits, DEFAULT_BITS};
  match aspect_value(bundle, packed, "stack_joins") {
    None => DEFAULT_BITS,
    Some(joins) => StackBits {
      hosts: aspect_value(bundle, packed, "stack_hosts").unwrap_or(0) as u8,
      joins: joins as u8,
    },
  }
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
  use super::*;
  use crate::loader::load;

  fn bundle() -> Bundle {
    let assets = "<asset>\n  ::pine>\n    @define>\n      256 &size set\n  ::log>\n    @define>\n      128 &size set\n";
    let aspects = "<aspect>\n\
      \x20 ::wood>\n    @define>\n      1 &visibility set\n      tree &icon set\n      #6B4423 &color set\n\
      \x20 ::pine>\n    @define>\n      1 &visibility set\n      1 &satisfies array\n      $aspect::wood &satisfies.0 set\n      $asset::pine &art set\n\
      \x20 ::type>\n    @define>\n      0 &visibility set\n\
      \x20 ::cost>\n    @define>\n      0 &visibility set\n";
    let cards = "<card>\n\
      \x20 ::log>\n    :data>\n      @define>\n        requisite &aspect.type set\n        2 &aspect.wood set\n    :visuals>\n      @define>\n        $shape.rect &shape set\n        #8B5E3C &color.bg set\n        #ecd6aa &color.title set\n        #0b1426 &color.text set\n        2 &visibility.wood set\n        1 &objects array\n        $asset::log &objects.0 set\n\
      \x20 ::forest>\n    :data>\n      @define>\n        2 &aspect.pine stock\n        tile &aspect.type set\n        30 &aspect.cost set\n    :visuals>\n      @define>\n        $shape.hex &shape set\n        #0b1426 &color.bg set\n        #0b1426 &color.title set\n        #0b1426 &color.text set\n";
    load(&[("s.rd".into(), assets.into()), ("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into())]).expect("load")
  }

  #[test]
  fn draw_visuals_builds_prim_list() {
    // :visuals @init builds one rect primitive via the `^rect` handle, tinted
    // from a value @define set — exercises the prim constructor + Ref handle
    // (`&h.field` writes through to the pushed prim) + float coords.
    let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n";
    let cards = "<card>\n  ::panel>\n    :data>\n      @define>\n        requisite &aspect.type set\n    :visuals>\n      @define>\n        #112233 &bg set\n      @init>\n        ^rect call &h set\n        0 0 &h.pos vec2\n        100 50.5 &h.size vec2\n        50 50 &h.anchor vec2\n        *bg &h.tint set\n";
    let b = load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into())]).expect("load");
    let prims = draw_visuals(&b, b.packed_def("panel").unwrap(), &[], "init");
    assert_eq!(prims.len(), 1);
    let p = &prims[0];
    assert_eq!(p.kind, "rect");
    assert_eq!(p.size, Vec2f { x: 100.0, y: 50.5 });
    assert_eq!(p.anchor, Some(Vec2f { x: 50.0, y: 50.0 }));
    assert_eq!(p.tint, 0x112233);
    assert_eq!(p.scale, 1.0); // default
    assert_eq!(p.alpha, 1.0); // default
  }

  #[test]
  fn card_data_host_drives_prims() {
    // `^card_data` is a structured host record (avoids big-int fields): the DSL
    // reads NESTED fields (`*d.stack.index`, `*d.progress.0`) to drive prims —
    // the hook for DSL-side stack positioning / progress bars / overlays.
    let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n";
    let cards = "<card>\n  ::c>\n    :data>\n      @define>\n        requisite &aspect.type set\n    :visuals>\n      @init>\n        ^card_data call &d set\n        ^rect call &h set\n        *d.stack.index &h.tint set\n        *d.progress.0 &h.rot set\n";
    let b = load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into())]).expect("load");
    let host = vec![(
      "card_data".to_string(),
      Cell::Map(vec![
        ("stack".into(), Cell::Map(vec![("index".into(), Cell::Int(3))])),
        ("progress".into(), Cell::Arr(vec![Cell::Int(42)])),
      ]),
    )];
    let prims = draw_visuals(&b, b.packed_def("c").unwrap(), &host, "init");
    assert_eq!(prims.len(), 1);
    assert_eq!(prims[0].tint, 3); // from *d.stack.index
    assert_eq!(prims[0].rot, 42.0); // from *d.progress.0
  }

  #[test]
  fn card_data_stack_index_fans_prim_y() {
    // The generic stack fan lives in the DSL: a stacked card sits at its chain
    // root (engine-placed); its prims shift y by `index · dir · step`. Verifies
    // the arithmetic + nested `^card_data` reads + that a `vec2` takes a computed
    // y. (step is a literal here; the real builders use `$globals::title_height`.)
    let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n";
    let cards = "<card>\n  ::c>\n    :data>\n      @define>\n        requisite &aspect.type set\n    :visuals>\n      @init>\n        ^card_data call &d set\n        *d.stack.index *d.stack.dir mul 10 mul &stack_dy set\n        ^rect call &h set\n        0.0 0.0 *stack_dy add &h.pos vec2\n";
    let b = load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into())]).expect("load");
    let host = vec![(
      "card_data".to_string(),
      Cell::Map(vec![(
        "stack".into(),
        Cell::Map(vec![("index".into(), Cell::Int(2)), ("dir".into(), Cell::Int(-1))]),
      )]),
    )];
    let prims = draw_visuals(&b, b.packed_def("c").unwrap(), &host, "init");
    assert_eq!(prims.len(), 1);
    assert_eq!(prims[0].pos.y, -20.0); // 2 · -1 · 10
  }

  #[test]
  fn destroy_hook_emits_exit_prims() {
    // `@destroy` is a real `:visuals` hook (parser allows it; `draw_visuals` runs
    // `visuals/<hook>`). The client runs it when a row goes `dead === 1` and eases
    // the prims to these EXIT targets before removing the card. Here the exit fades
    // the rect to alpha 0; a card with no `@destroy` would yield an empty list.
    let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n";
    let cards = "<card>\n  ::c>\n    :data>\n      @define>\n        requisite &aspect.type set\n    :visuals>\n      @init>\n        ^rect call &h set\n      @destroy>\n        ^rect call &h set\n        0.0 &h.alpha set\n";
    let b = load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into())]).expect("load");
    let alive = draw_visuals(&b, b.packed_def("c").unwrap(), &[], "init");
    assert_eq!(alive.len(), 1);
    assert_eq!(alive[0].alpha, 1.0);
    let exit = draw_visuals(&b, b.packed_def("c").unwrap(), &[], "destroy");
    assert_eq!(exit.len(), 1);
    assert_eq!(exit[0].alpha, 0.0);
  }

  #[test]
  fn prim_z_carries_intra_card_paint_order() {
    // `&h.z set` rides through to the prim so the client can sort intra-card paint
    // order (text over art, …); unset → None (client falls back to push order).
    let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n";
    let cards = "<card>\n  ::c>\n    :data>\n      @define>\n        requisite &aspect.type set\n    :visuals>\n      @init>\n        ^rect call &h set\n        ^text call &h set\n        9 &h.z set\n";
    let b = load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into())]).expect("load");
    let prims = draw_visuals(&b, b.packed_def("c").unwrap(), &[], "init");
    assert_eq!(prims.len(), 2);
    assert_eq!(prims[0].z, None); // unset
    assert_eq!(prims[1].z, Some(9)); // the text
  }

  #[test]
  fn progress_prim_carries_target_and_style() {
    // `^progress call &h set` + `*d.progress.0.id &h.target set` /
    // `*d.progress.0.style &h.style set` → a progress prim carrying the row to
    // TRACK + the style. The client fills it live from the row's timing (the DSL
    // never sets a 0..1 value — it isn't run per-frame).
    let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n";
    let cards = "<card>\n  ::c>\n    :data>\n      @define>\n        requisite &aspect.type set\n    :visuals>\n      @init>\n        ^card_data call &d set\n        ^progress call &h set\n        *d.progress.0.id &h.target set\n        *d.progress.0.style &h.style set\n";
    let b = load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into())]).expect("load");
    let host = vec![(
      "card_data".to_string(),
      Cell::Map(vec![(
        "progress".into(),
        Cell::Arr(vec![Cell::Map(vec![("id".into(), Cell::Int(0)), ("style".into(), Cell::Int(2))])]),
      )]),
    )];
    let prims = draw_visuals(&b, b.packed_def("c").unwrap(), &host, "init");
    assert_eq!(prims.len(), 1);
    assert_eq!(prims[0].kind, "progress");
    assert_eq!(prims[0].target, Some(0));
    assert_eq!(prims[0].style, Some(2));
  }

  #[test]
  fn progress_source_selects_queue() {
    // `&h.source set` picks the fill SOURCE: unset/0 = a progress row, 1 = the
    // action queue/debounce. The client routes to `deps.queue` for source 1.
    let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n";
    let cards = "<card>\n  ::c>\n    :data>\n      @define>\n        requisite &aspect.type set\n    :visuals>\n      @init>\n        ^progress call &p set\n        1 &p.source set\n";
    let b = load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into())]).expect("load");
    let prims = draw_visuals(&b, b.packed_def("c").unwrap(), &[], "init");
    assert_eq!(prims.len(), 1);
    assert_eq!(prims[0].source, Some(1));
  }

  #[test]
  fn dsl_resolves_asset_variant_to_texture_index() {
    // The CARD resolves its art in the DSL (no engine `resolve_texture`): the
    // pack ref derefs `*pack.object` → the manifest folder and `*pack.texture.axe`
    // → the variant index (7). `prim_from_cell` just reads the fields.
    let manifest = "<manifest>\n  ::requisite>\n    :neutral>\n      @define>\n        1 &texture array\n        a.png &texture.0 set\n";
    let assets = "<asset>\n  ::requisite>\n    @define>\n      $manifest::requisite &object set\n      128 &size set\n      7 &texture.axe set\n";
    let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n";
    let cards = "<card>\n  ::axe>\n    :data>\n      @define>\n        requisite &aspect.type set\n    :visuals>\n      @init>\n        $asset::requisite &pack set\n        ^sprite call &h set\n        *pack.object &h.texture set\n        *pack.texture.axe &h.index set\n";
    let b = load(&[
      ("m.rd".into(), manifest.into()),
      ("s.rd".into(), assets.into()),
      ("a.rd".into(), aspects.into()),
      ("c.rd".into(), cards.into()),
    ])
    .expect("load");
    let prims = draw_visuals(&b, b.packed_def("axe").unwrap(), &[], "init");
    assert_eq!(prims.len(), 1);
    assert_eq!(prims[0].kind, "sprite");
    assert_eq!(prims[0].texture.as_deref(), Some("requisite"));
    assert_eq!(prims[0].index, Some(7));
  }

  #[test]
  fn sys_label_carries_card_title_key() {
    // The engine seeds the card's identity before `:visuals` so a shared title
    // builder can author the label locale KEY without per-card hardcoding:
    // `*sys.label` → `cards.<type>.<key>.label` (client resolves the string).
    let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n";
    let cards = "<card>\n  ::axe>\n    :data>\n      @define>\n        requisite &aspect.type set\n    :visuals>\n      @init>\n        ^text call &h set\n        *sys.label &h.text set\n";
    let b = load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into())]).expect("load");
    let prims = draw_visuals(&b, b.packed_def("axe").unwrap(), &[], "init");
    assert_eq!(prims.len(), 1);
    assert_eq!(prims[0].kind, "text");
    assert_eq!(prims[0].text.as_deref(), Some("cards.requisite.axe.label"));
  }

  #[test]
  fn render_def_extracts_visuals_and_data() {
    let b = bundle();
    let d = card_render_def(&b, b.packed_def("log").unwrap()).unwrap();
    assert_eq!(d.key, "log");
    assert_eq!(d.type_name, "requisite");
    assert_eq!(d.shape, "rect");
    assert_eq!(d.color_bg, 0x8B5E3C);
    assert_eq!(d.color_title, 0xecd6aa);
    assert_eq!(d.objects, vec!["log".to_string()]);
    // static aspect `wood = 2` present (type symbol excluded); log's :visuals
    // overrides wood's visibility to 2 (a function slot).
    assert!(d.aspects.iter().any(|(k, v, vis)| k == "wood" && *v == 2 && *vis == 2));
    assert!(!d.aspects.iter().any(|(k, _, _)| k == "type"));
  }

  #[test]
  fn render_def_stock_and_shape_for_tile() {
    let b = bundle();
    let d = card_render_def(&b, b.packed_def("forest").unwrap()).unwrap();
    assert_eq!(d.shape, "hex");
    // forest declares a `pine` stock slot (bits 2)
    assert_eq!(d.stock.len(), 1);
    assert_eq!(d.stock[0].aspect, "pine");
    // ...and a stock-backed aspect must NOT also appear as a static aspect
    // (else a renderer reads its static `0` default and masks the live stock).
    assert!(!d.aspects.iter().any(|(k, _, _)| k == "pine"));
  }

  #[test]
  fn aspect_info_inherits_icon_and_color() {
    let b = bundle();
    let wood = aspect_info(&b, "wood").unwrap();
    assert_eq!(wood.icon, "tree");
    assert_eq!(wood.color, 0x6B4423);
    assert_eq!(wood.visibility, 1);
    // pine omits icon/color → inherits wood's via satisfies
    let pine = aspect_info(&b, "pine").unwrap();
    assert_eq!(pine.satisfies, vec!["wood".to_string()]);
    assert_eq!(pine.icon, "tree");
    assert_eq!(pine.color, 0x6B4423);
    assert_eq!(pine.art.as_deref(), Some("pine"));
  }

  #[test]
  fn recipe_meta_iterators_and_anchors() {
    let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n  ::cost>\n    @define>\n      traits &section set\n";
    let cards = "<card>\n  ::corpus>\n    :data>\n      @define>\n        faculty &aspect.type set\n        1 &aspect.cost set\n";
    let recipes = "<recipe>\n\
      \x20 ::triple>\n    @input>\n      $card::corpus *slot.1.0.def_id eq if &slot.1.0 use\n      $card::corpus *slot.1.1.def_id eq if &slot.1.1 use\n    @output>\n      &slot.1.0 destroy\n\
      \x20 ::rooted>\n    @input>\n      *root.aspect.cost 1 ge if &root use\n    @output>\n      &root destroy\n";
    let b = load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into()), ("r.rd".into(), recipes.into())]).unwrap();
    let m = recipe_meta(&b, "triple").unwrap();
    assert_eq!(m.iterators.len(), 1);
    assert_eq!(m.iterators[0].branch, 1);
    assert_eq!(m.iterators[0].parent, "");
    assert_eq!(m.branches, 1 << 1);
    assert!(!m.root);
    assert!(recipe_meta(&b, "rooted").unwrap().root);
  }

  #[test]
  fn aspect_value_folds_satisfies() {
    // `pine` satisfies `wood`; a card that statically sets `pine = 2` folds to
    // `wood = 2` through `card_view`'s satisfies roll-up. (The old version of
    // this test used `forest`, whose `pine` is a *stock* slot seeded to 0
    // statically — so nothing folded; it only appeared to pass because the
    // buggy packed→def_id resolution read a *different* card's `wood`.)
    let aspects = "<aspect>\n\
      \x20 ::wood>\n    @define>\n      aspects &section set\n\
      \x20 ::pine>\n    @define>\n      aspects &section set\n      1 &satisfies array\n      $aspect::wood &satisfies.0 set\n\
      \x20 ::type>\n    @define>\n      traits &section set\n";
    let cards = "<card>\n\
      \x20 ::twig>\n    :data>\n      @define>\n        requisite &aspect.type set\n        2 &aspect.pine set\n    :visuals>\n      @define>\n        $shape.rect &shape set\n";
    let b = load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into())]).expect("load");
    let twig = b.packed_def("twig").unwrap();
    assert_eq!(aspect_value(&b, twig, "pine"), Some(2));
    assert_eq!(aspect_value(&b, twig, "wood"), Some(2)); // folded up `satisfies`
    // An aspect the def doesn't carry resolves to `None`, NOT `Some(0)` — the
    // client gate (`aspectValue !== null`) depends on this to tell "no
    // inventory" from "infinite-capacity inventory".
    assert_eq!(aspect_value(&b, twig, "definitely_not_an_aspect"), None);
  }

  #[test]
  fn aspect_value_resolves_packed_not_typelocal_id() {
    // Regression: a bundle where the inventory-bearing card's TYPE-LOCAL def_id
    // (1 — first `soul`) differs from its GLOBAL card index (3 — after two
    // `requisite`s). The old resolution fed `unpack_def(packed).1` straight to
    // `card_view`, which indexes the GLOBAL card list, so it read global card #1
    // (a requisite, no inventory) and returned `None` for the soul. The tiny
    // single-type test bundles masked this because type-local == global there.
    let aspects = "<aspect>\n\
      \x20 ::type>\n    @define>\n      traits &section set\n\
      \x20 ::inventory>\n    @define>\n      features &section set\n";
    let cards = "<card>\n\
      \x20 ::ra>\n    :data>\n      @define>\n        requisite &aspect.type set\n    :visuals>\n      @define>\n        $shape.rect &shape set\n\
      \x20 ::rb>\n    :data>\n      @define>\n        requisite &aspect.type set\n    :visuals>\n      @define>\n        $shape.rect &shape set\n\
      \x20 ::hero>\n    :data>\n      @define>\n        soul &aspect.type set\n        1 &aspect.inventory set\n    :visuals>\n      @define>\n        $shape.rect &shape set\n";
    let b = load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into())]).expect("load");
    let hero = b.packed_def("hero").unwrap();
    // The mismatch the bug needs: type-local def_id 1, global index 3.
    assert_eq!(resonantdust_codec::bits::unpack_def(hero).1, 1);
    assert_eq!(b.card_def_id("hero"), Some(3));
    assert_eq!(aspect_value(&b, hero, "inventory"), Some(1));
    // A requisite that doesn't declare inventory still resolves to `None` — not
    // the soul's value, and not a stale read of the wrong card.
    let ra = b.packed_def("ra").unwrap();
    assert_eq!(aspect_value(&b, ra, "inventory"), None);
  }

  #[test]
  fn mask_prim_emits_with_enter_seed() {
    // The `^mask` exit prim: `size.y` (target) is the rolled-up height, `enter.h`
    // seeds the open height so the client eases full → 0. Verifies the new kind
    // is a real PRIM_KIND and `enter` survives the serializer.
    let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n";
    let cards = "<card>\n  ::dummy>\n    :data>\n      @define>\n        requisite &aspect.type set\n    :visuals>\n      @define>\n        #112233 &color.bg set\n      @destroy>\n        ^mask call &m set\n        0 0 &m.pos vec2\n        72 0 &m.size vec2\n        90 &m.enter.h set\n";
    let b = load(&[("a.rd".into(), aspects.into()), ("c.rd".into(), cards.into())]).expect("load");
    let prims = draw_visuals(&b, b.packed_def("dummy").unwrap(), &[], "destroy");
    assert_eq!(prims.len(), 1);
    assert_eq!(prims[0].kind, "mask");
    assert_eq!(prims[0].size.y, 0.0);
    assert_eq!(prims[0].enter.as_ref().and_then(|e| e.h), Some(90.0));
  }
}
