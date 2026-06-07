//! Resonant Dust client-side wasm crate.
//!
//! Compiled to a browser wasm bundle (see the `wasm` service in `compose.yml`)
//! and imported by the pixijs client. The server does NOT consume this crate —
//! it links `resonantdust-data` directly as an rlib. The bindings live only
//! here, where they're needed.
//!
//! The substance ([`Content`] and its methods) is plain Rust — feature-
//! independent, so `cargo check`/`test` exercise it natively without the wasm
//! toolchain. The browser surface is a thin `#[wasm_bindgen]` layer, gated on
//! `js`, that marshals JSON in/out and delegates to the plain methods.

use resonantdust_data::bridge::{card_view, operating_set, Card};
use resonantdust_data::defs;
use resonantdust_data::loader::{load, Bundle};
use resonantdust_data::locales::Locales as DataLocales;
use resonantdust_data::vm::{match_recipe as vm_match, plan_recipe as vm_plan, Cell, Plan};
use resonantdust_data::worldgen;

#[cfg(feature = "js")]
use wasm_bindgen::prelude::*;

/// A loaded content runtime: the [`Bundle`] plus the operations the client
/// calls (card render-view, client-side recipe match/plan). Opaque to JS —
/// constructed once from the `.rd` sources, then queried.
#[derive(Debug)]
#[cfg_attr(feature = "js", wasm_bindgen)]
pub struct Content {
    bundle: Bundle,
}

impl Content {
    /// Load `(name, source)` `.rd` pairs into a runtime handle. Returns the
    /// corpus problems (parse/validate/resolve) as a message on failure.
    pub fn load(sources: Vec<(String, String)>) -> Result<Content, String> {
        load(&sources).map(|bundle| Content { bundle }).map_err(|errs| {
            let mut msg = format!("{} load problem(s):", errs.len());
            for e in errs.iter().take(8) {
                msg.push_str(&format!("\n  {}: {}", e.file, e.message));
            }
            msg
        })
    }

    /// Packed `def_id` for a card name (`None` if unknown).
    pub fn card_def_id(&self, name: &str) -> Option<u16> {
        self.bundle.card_def_id(name)
    }

    /// The VM view of a stored card — `def_id` + folded aspects (what a recipe
    /// reads, and what the renderer walks for objects).
    pub fn card_view(&self, card: &Card) -> Cell {
        card_view(&self.bundle, card)
    }

    /// The bundle's global `def_id` for a packed definition — the id the client
    /// puts in a [`Card`] for `match_recipe` (packed → name → card def id).
    pub fn def_id_for_packed(&self, packed: u16) -> Option<u16> {
        self.bundle.name_for_packed(packed).and_then(|n| self.bundle.card_def_id(n))
    }

    /// Run a card's `:visuals` hook against `host` (instance state — stock,
    /// faction, UI flags) and return its primitive list: the client reconciler's
    /// render spec. `hook` is `"init"` (first draw) or `"update"` (on change).
    pub fn draw_visuals(&self, packed: u16, host: Vec<(String, Cell)>, hook: &str) -> Vec<defs::PrimNode> {
        defs::draw_visuals(&self.bundle, packed, &host, hook)
    }

    /// Render a world tile to a primitive list from its stored stock (overlaid,
    /// not biome-recomputed). LOD variant + faction are chosen client-side. See
    /// `defs::tile_prims`.
    pub fn tile_prims(&self, packed: u16, stock: Vec<i64>, seed: i64) -> Vec<defs::PrimNode> {
        defs::tile_prims(&self.bundle, packed, &stock, seed)
    }

    /// Match a recipe's `@input` against an operating-set frame of
    /// `(slot path, card)` placements — the client-side matcher.
    pub fn match_recipe(&self, placed: &[(String, Card)], recipe: &str) -> Option<Plan> {
        self.run_recipe(placed, recipe, true)
    }

    /// Run a (matched) recipe's `@output` tape against the frame.
    pub fn plan_recipe(&self, placed: &[(String, Card)], recipe: &str) -> Option<Plan> {
        self.run_recipe(placed, recipe, false)
    }

    fn run_recipe(&self, placed: &[(String, Card)], recipe: &str, input: bool) -> Option<Plan> {
        let refs: Vec<(&str, &Card)> = placed.iter().map(|(p, c)| (p.as_str(), c)).collect();
        let mut frame = operating_set(&self.bundle, &refs);
        let hook = if input { "input" } else { "output" };
        let body = &self.bundle.recipe(recipe)?.hook(hook)?.body;
        let run = if input { vm_match } else { vm_plan };
        run(body, &mut frame, &self.bundle.catalog, &self.bundle.functions).ok()
    }

    /// The packed definition `[type:u4 | def_id:u12]` for a card name (`None` if
    /// unknown) — the wire id the client decodes / packs against.
    pub fn packed_def(&self, name: &str) -> Option<u16> {
        self.bundle.packed_def(name)
    }

    /// Generate a world hex's terrain: the tile's `def_id` (within the `tile`
    /// type) and its two 2-bit stock values — what a Zone tile slot stores.
    /// Selects the biome, runs the default tile's `@init` with the `^biome`
    /// host. Pure over `(q, r, seed)`, identical on gate + client.
    pub fn generate_tile(&self, q: i32, r: i32, seed: u64) -> (u16, [u8; 2]) {
        worldgen::generate_tile(&self.bundle, q, r, seed)
    }

    /// The render/UI definition for a packed card (visuals + data, DSL-native).
    pub fn card_def(&self, packed: u16) -> Option<defs::CardRenderDef> {
        defs::card_render_def(&self.bundle, packed)
    }

    /// An `<aspect>` record (icon/color/section/satisfies/art) by name.
    pub fn aspect_info(&self, name: &str) -> Option<defs::AspectInfo> {
        defs::aspect_info(&self.bundle, name)
    }

    /// Folded value of a named aspect on a packed def (recipe-relevant magnitude).
    pub fn aspect_value(&self, packed: u16, name: &str) -> Option<i64> {
        defs::aspect_value(&self.bundle, packed, name)
    }

    /// The blueprint catalog (wrench panel).
    pub fn all_blueprints(&self) -> Vec<defs::BlueprintInfo> {
        defs::all_blueprints(&self.bundle)
    }

    /// The texture registry — every `<asset>` pack's render metadata.
    pub fn all_textures(&self) -> Vec<defs::TextureDef> {
        defs::all_textures(&self.bundle)
    }

    /// Recipe names in Bundle-id order (`id = index + 1`) — the candidate list the
    /// client iterates for client-side matching.
    pub fn recipe_names(&self) -> Vec<String> {
        self.bundle.recipe_ids.clone()
    }
    /// Recipe name for a wire id, or `None`.
    pub fn recipe_name(&self, id: u16) -> Option<String> {
        self.bundle.recipe_name(id).map(str::to_string)
    }
    /// Wire id for a recipe name, or `None`.
    pub fn recipe_id(&self, name: &str) -> Option<u16> {
        self.bundle.recipe_def_id(name)
    }

    /// Every aspect name, sorted — the client indexes this into a local numeric
    /// id space for its id-keyed render/UI code (aspect ids are client-only).
    pub fn aspect_names(&self) -> Vec<String> {
        self.bundle.catalog.aspect_names()
    }

    /// A recipe's iterators + anchor set — the structure the client's discovery
    /// matcher needs to build the placed frame + `bindings` (predicate eval runs
    /// on the VM via `match_recipe`, so no path-statement IR is exposed).
    pub fn recipe_meta(&self, name: &str) -> Option<defs::RecipeMeta> {
        defs::recipe_meta(&self.bundle, name)
    }
}

/// Hot-loadable locale catalog — label / description / message strings the
/// client renders. Independent of the [`Content`] bundle so it can reload on its
/// own: the client fetches `content/locales/<domain>/<lang>.json` at runtime and
/// hands the text in, so editing a locale file and re-constructing picks up the
/// change without recompiling. Keys are flattened `domain.path`
/// (`cards.requisite.log.label`).
#[derive(Debug)]
#[cfg_attr(feature = "js", wasm_bindgen)]
pub struct Locales {
    inner: DataLocales,
}

impl Locales {
    /// Load `(domain, json)` pairs into a lookup handle.
    pub fn load(sources: Vec<(String, String)>) -> Result<Locales, String> {
        DataLocales::load(&sources).map(|inner| Locales { inner })
    }

    /// The localized string for a flattened key, or `None`.
    pub fn string(&self, key: &str) -> Option<String> {
        self.inner.get(key).map(str::to_string)
    }
}

// ---------- Browser surface (js feature) ----------

#[cfg(feature = "js")]
fn jserr<E: std::fmt::Display>(e: E) -> JsValue {
    JsValue::from_str(&e.to_string())
}

/// One JSON value → a `Cell`. Recursive, so NESTED host inputs work: an object
/// becomes a `Cell::Map`, an array a `Cell::Arr` — which is how `^card_data`
/// hands the DSL a structured record (`*d.stack.index`, `*d.progress.0`, …)
/// without passing big ints field-by-field. `null` is dropped.
#[cfg(feature = "js")]
fn json_to_cell(v: serde_json::Value) -> Option<Cell> {
    Some(match v {
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Cell::Int(i),
            None => Cell::Float(n.as_f64()?),
        },
        serde_json::Value::String(s) => Cell::Sym(s),
        serde_json::Value::Bool(b) => Cell::Int(b as i64),
        serde_json::Value::Array(a) => Cell::Arr(a.into_iter().filter_map(json_to_cell).collect()),
        serde_json::Value::Object(m) => {
            Cell::Map(m.into_iter().filter_map(|(k, v)| json_to_cell(v).map(|c| (k, c))).collect())
        }
        serde_json::Value::Null => return None,
    })
}

#[cfg(feature = "js")]
fn parse_host(json: &str) -> Vec<(String, Cell)> {
    let map: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(json).unwrap_or_default();
    map.into_iter().filter_map(|(k, v)| json_to_cell(v).map(|c| (k, c))).collect()
}

#[cfg(feature = "js")]
#[wasm_bindgen]
impl Locales {
    /// `new Locales(sourcesJson)` — `sourcesJson` is `[[domain, json], …]`,
    /// e.g. `[["cards", "{…}"], ["recipes", "{…}"]]`.
    #[wasm_bindgen(constructor)]
    pub fn new(sources_json: &str) -> Result<Locales, JsValue> {
        let sources: Vec<(String, String)> = serde_json::from_str(sources_json).map_err(jserr)?;
        Locales::load(sources).map_err(|e| JsValue::from_str(&e))
    }

    /// `string(key)` → the localized string, or `undefined`.
    #[wasm_bindgen(js_name = string)]
    pub fn string_js(&self, key: &str) -> Option<String> {
        self.string(key)
    }
}

#[cfg(feature = "js")]
#[wasm_bindgen]
impl Content {
    /// `new Content(sourcesJson)` — `sourcesJson` is `[[name, text], …]`.
    #[wasm_bindgen(constructor)]
    pub fn new(sources_json: &str) -> Result<Content, JsValue> {
        let sources: Vec<(String, String)> = serde_json::from_str(sources_json).map_err(jserr)?;
        Content::load(sources).map_err(|e| JsValue::from_str(&e))
    }

    #[wasm_bindgen(js_name = cardDefId)]
    pub fn card_def_id_js(&self, name: &str) -> Option<u16> {
        self.card_def_id(name)
    }

    /// `cardView(cardJson)` → the view cell as JSON. `cardJson` = `{def_id, stock}`.
    #[wasm_bindgen(js_name = cardView)]
    pub fn card_view_js(&self, card_json: &str) -> Result<String, JsValue> {
        let card: Card = serde_json::from_str(card_json).map_err(jserr)?;
        serde_json::to_string(&self.card_view(&card)).map_err(jserr)
    }

    /// `defIdForPacked(packed)` → the global card def id (or `undefined`).
    #[wasm_bindgen(js_name = defIdForPacked)]
    pub fn def_id_for_packed_js(&self, packed: u16) -> Option<u16> {
        self.def_id_for_packed(packed)
    }

    /// `matchRecipe(placedJson, recipe)` → the `Plan` as JSON (`null` if no
    /// such recipe). `placedJson` = `[[slotPath, {def_id, stock}], …]`.
    #[wasm_bindgen(js_name = matchRecipe)]
    pub fn match_recipe_js(&self, placed_json: &str, recipe: &str) -> Result<String, JsValue> {
        let placed: Vec<(String, Card)> = serde_json::from_str(placed_json).map_err(jserr)?;
        serde_json::to_string(&self.match_recipe(&placed, recipe)).map_err(jserr)
    }

    #[wasm_bindgen(js_name = planRecipe)]
    pub fn plan_recipe_js(&self, placed_json: &str, recipe: &str) -> Result<String, JsValue> {
        let placed: Vec<(String, Card)> = serde_json::from_str(placed_json).map_err(jserr)?;
        serde_json::to_string(&self.plan_recipe(&placed, recipe)).map_err(jserr)
    }

    /// `packedDef(name)` → the packed `[type:u4 | def_id:u12]` (or `undefined`).
    #[wasm_bindgen(js_name = packedDef)]
    pub fn packed_def_js(&self, name: &str) -> Option<u16> {
        self.packed_def(name)
    }

    /// `generateTile(q, r, seed)` → the packed Zone tile slot
    /// `[def_id:u12 | stock0:u2 | stock1:u2]` for a world hex. `seed` is a JS
    /// number (the world seed is small; values beyond 2^53 lose precision).
    #[wasm_bindgen(js_name = generateTile)]
    pub fn generate_tile_js(&self, q: i32, r: i32, seed: f64) -> u16 {
        let (def_id, [s0, s1]) = self.generate_tile(q, r, seed as u64);
        resonantdust_data::bits::pack_tile_slot(def_id, s0, s1)
    }

    /// `cardDef(packed)` → the render def as JSON (`null` if unknown).
    #[wasm_bindgen(js_name = cardDef)]
    pub fn card_def_js(&self, packed: u16) -> Result<String, JsValue> {
        serde_json::to_string(&self.card_def(packed)).map_err(jserr)
    }

    /// `drawVisuals(packed, hostJson, hook)` → the `PrimList` JSON for a card's
    /// `:visuals` hook. `hostJson` is a plain object of instance state
    /// (`{"poison": 1, "faction": "chorus"}`) — numbers become Int/Float, strings
    /// Sym. `hook` is "init" | "update".
    #[wasm_bindgen(js_name = drawVisuals)]
    pub fn draw_visuals_js(&self, packed: u16, host_json: &str, hook: &str) -> Result<String, JsValue> {
        serde_json::to_string(&self.draw_visuals(packed, parse_host(host_json), hook)).map_err(jserr)
    }

    /// `tilePrims(packed, stock0, stock1, seed)` → the world tile's `PrimList`
    /// JSON from its stored stock (the two zone stock slots). `seed` is the
    /// tile's `(q,r)` hash, driving the `:visuals` scatter (ring angles, scale).
    #[wasm_bindgen(js_name = tilePrims)]
    pub fn tile_prims_js(&self, packed: u16, stock0: i32, stock1: i32, seed: i32) -> Result<String, JsValue> {
        serde_json::to_string(&self.tile_prims(packed, vec![stock0 as i64, stock1 as i64], seed as i64))
            .map_err(jserr)
    }

    /// Batched `tilePrims`: `reqs` is a flat `[packed, stock0, stock1, seed]`
    /// quadruplet per tile. Returns a JSON array of per-tile PrimNode arrays in
    /// request order — one boundary crossing + one serialize for a whole drain
    /// batch instead of one call per tile.
    #[wasm_bindgen(js_name = tilePrimsBatch)]
    pub fn tile_prims_batch_js(&self, reqs: &[i32]) -> Result<String, JsValue> {
        let lists: Vec<Vec<defs::PrimNode>> = reqs
            .chunks_exact(4)
            .map(|c| self.tile_prims(c[0] as u16, vec![c[1] as i64, c[2] as i64], c[3] as i64))
            .collect();
        serde_json::to_string(&lists).map_err(jserr)
    }

    /// `aspectInfo(name)` → the aspect record as JSON (`null` if unknown).
    #[wasm_bindgen(js_name = aspectInfo)]
    pub fn aspect_info_js(&self, name: &str) -> Result<String, JsValue> {
        serde_json::to_string(&self.aspect_info(name)).map_err(jserr)
    }

    /// `aspectValue(packed, name)` → the folded magnitude (or `undefined`).
    #[wasm_bindgen(js_name = aspectValue)]
    pub fn aspect_value_js(&self, packed: u16, name: &str) -> Option<i64> {
        self.aspect_value(packed, name)
    }

    /// `allBlueprints()` → the blueprint catalog as JSON.
    #[wasm_bindgen(js_name = allBlueprints)]
    pub fn all_blueprints_js(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.all_blueprints()).map_err(jserr)
    }

    /// `allTextures()` → the texture registry as JSON.
    #[wasm_bindgen(js_name = allTextures)]
    pub fn all_textures_js(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.all_textures()).map_err(jserr)
    }

    /// `globals()` → the `<globals>` constants as a JSON `{ id: number }` map
    /// (card_width, card_height, title_height, hex_*). The client reads card/cell
    /// dimensions from here instead of hardcoding `RECT_CARD_*` / hex radius.
    #[wasm_bindgen(js_name = globals)]
    pub fn globals_js(&self) -> Result<String, JsValue> {
        let map: std::collections::BTreeMap<String, f64> = self
            .bundle
            .catalog
            .global_entries()
            .into_iter()
            .map(|(k, v)| (k, v.as_f64()))
            .collect();
        serde_json::to_string(&map).map_err(jserr)
    }

    /// `recipeNames()` → the recipe candidate list (Bundle-id order).
    #[wasm_bindgen(js_name = recipeNames)]
    pub fn recipe_names_js(&self) -> Vec<String> {
        self.recipe_names()
    }

    #[wasm_bindgen(js_name = recipeName)]
    pub fn recipe_name_js(&self, id: u16) -> Option<String> {
        self.recipe_name(id)
    }

    #[wasm_bindgen(js_name = recipeId)]
    pub fn recipe_id_js(&self, name: &str) -> Option<u16> {
        self.recipe_id(name)
    }

    /// `aspectNames()` → all aspect names, sorted (client builds its id map).
    #[wasm_bindgen(js_name = aspectNames)]
    pub fn aspect_names_js(&self) -> Vec<String> {
        self.aspect_names()
    }

    /// `recipeMeta(name)` → the recipe's iterators + anchors as JSON (`null` if
    /// unknown). Drives client discovery + binding construction.
    #[wasm_bindgen(js_name = recipeMeta)]
    pub fn recipe_meta_js(&self, name: &str) -> Result<String, JsValue> {
        serde_json::to_string(&self.recipe_meta(name)).map_err(jserr)
    }
}

// ---------- Flag / type introspection (free functions, js) ----------
//
// Model-stable lookups the client's `DefinitionManager` needs that don't require
// a loaded bundle (flag bit layout + card-type nibbles). They replace the legacy
// `resonantdust_content` free functions one-for-one.

#[cfg(feature = "js")]
#[wasm_bindgen(js_name = cardFlagBit)]
pub fn card_flag_bit(name: &str) -> Option<u8> {
    resonantdust_data::inspect::card_flag_bit(name)
}

#[cfg(feature = "js")]
#[wasm_bindgen(js_name = cardFlagBitIn)]
pub fn card_flag_bit_in(field: &str, name: &str) -> Option<u8> {
    resonantdust_data::flags::flag_bit(field, name)
}

#[cfg(feature = "js")]
#[wasm_bindgen(js_name = cardFlagFieldShape)]
pub fn card_flag_field_shape(field: &str, name: &str) -> Option<Vec<u8>> {
    resonantdust_data::inspect::card_flag_field_shape(field, name).map(|(s, w)| vec![s, w])
}

#[cfg(feature = "js")]
#[wasm_bindgen(js_name = hasCardFlag)]
pub fn has_card_flag(flags: u32, flags_bk: u32, name: &str) -> bool {
    resonantdust_data::inspect::has_card_flag(flags, flags_bk, name)
}

#[cfg(feature = "js")]
#[wasm_bindgen(js_name = cardFlagFieldValueIn)]
pub fn card_flag_field_value_in(field: &str, host: u32, name: &str) -> Option<u32> {
    resonantdust_data::inspect::card_flag_field_value_in(field, host, name)
}

#[cfg(feature = "js")]
#[wasm_bindgen(js_name = cardFlagFieldValueAny)]
pub fn card_flag_field_value_any(flags: u32, stock: u32, name: &str) -> Option<u32> {
    resonantdust_data::inspect::card_flag_field_value_any(flags, stock, name)
}

#[cfg(feature = "js")]
#[wasm_bindgen(js_name = cardTypeId)]
pub fn card_type_id(name: &str) -> Option<u8> {
    resonantdust_data::inspect::card_type_id(name)
}

#[cfg(feature = "js")]
#[wasm_bindgen(js_name = isHexType)]
pub fn is_hex_type(type_id: u8) -> bool {
    resonantdust_data::inspect::is_hex_type(type_id)
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    fn content() -> Content {
        let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n";
        let cards = "<card>\n  ::corpus>\n    :data>\n      @define>\n        faculty &aspect.type set\n";
        let recipes = "<recipe>\n  ::use_corpus>\n    @input>\n      $card::corpus *slot.1.0.def_id eq if &slot.1.0 use\n    @output>\n      10 &sys.duration set\n      &slot.1.0 destroy\n";
        Content::load(vec![
            ("a.rd".into(), aspects.into()),
            ("c.rd".into(), cards.into()),
            ("r.rd".into(), recipes.into()),
        ])
        .expect("load")
    }

    #[test]
    fn loads_and_matches_a_recipe() {
        let c = content();
        let id = c.card_def_id("corpus").expect("corpus id");
        let plan = c
            .match_recipe(&[("slot.1.0".into(), Card { def_id: id, stock: vec![] })], "use_corpus")
            .expect("matched");
        assert!(plan.matched);
        let out = c
            .plan_recipe(&[("slot.1.0".into(), Card { def_id: id, stock: vec![] })], "use_corpus")
            .expect("planned");
        assert_eq!(out.duration, 10);
    }

    #[test]
    fn bad_content_reports_problems() {
        let err = Content::load(vec![("b.rd".into(), "<functions:f>\n  2 &aspect.ghost set\n".into())]).unwrap_err();
        assert!(err.contains("ghost"), "{err}");
    }

    #[test]
    fn generates_terrain_and_packs_defs() {
        let aspects = "<aspect>\n  ::type>\n    @define>\n      traits &section set\n  ::cost>\n    @define>\n      traits &section set\n  ::pine>\n    @define>\n      aspects &section set\n";
        let biomes = "<biome>\n  ::forest>\n    @define>\n      30 75 &elevation range\n      20 70 &temperature range\n      55 95 &humidity range\n      1 &tile array\n      $card::forest &tile.0 set\n";
        let cards = "<card>\n  ::forest>\n    :data>\n      @define>\n        2 &aspect.pine stock\n        tile &aspect.type set\n        30 &aspect.cost set\n      @init>\n        ^biome call &biome set\n        0 3 &aspect.pine range\n        &aspect.pine *biome.humidity normalize\n";
        let c = Content::load(vec![
            ("a.rd".into(), aspects.into()),
            ("b.rd".into(), biomes.into()),
            ("c.rd".into(), cards.into()),
        ])
        .expect("load");
        // tile type nibble 7, first (only) tile → def_id 1
        assert_eq!(c.packed_def("forest"), Some((7 << 12) | 1));
        // origin lands in the forest envelope; pine stock = humidity-driven
        let (def_id, stock) = c.generate_tile(0, 0, 0x27);
        assert_eq!(def_id, 1);
        assert_eq!(stock, [2, 0]);
    }
}
