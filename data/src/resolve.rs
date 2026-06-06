//! Symbol table + cross-reference resolution (Phase A/B).
//!
//! A whole-corpus concern: a `$`-reference in one file may target a `::`
//! definition in another, so resolution runs against a table built from *all*
//! files. [`SymbolTable::collect`] is called once per parsed file to gather its
//! definitions; [`unresolved`] then walks a file's instruction bodies and
//! reports `$`-references that don't resolve.
//!
//! Namespaces checked: `card` / `recipe` (the `::` records), `functions`, and
//! `asset` (sprite packs from `<asset>`, each with an optional texture LUT — so
//! `$asset::symbols:corpus` checks both the pack and the texture). Heavy
//! registries still living as JSON — `aspect` / `shape` / `faction` / `type` —
//! are *deferred* (recognized, not resolved). An unrecognized root is a typo.

use crate::parser::{Header, Node, Stmt, Token};
use crate::validate::{render, Diagnostic};
use std::collections::{HashMap, HashSet};

/// Corpus-wide definitions, keyed by id (facets/types collapse to the record id).
#[derive(Debug, Default)]
pub struct SymbolTable {
  pub cards: HashSet<String>,
  pub recipes: HashSet<String>,
  pub functions: HashSet<String>,
  /// asset pack name -> its texture-LUT symbols (empty for single-sprite packs).
  pub assets: HashMap<String, HashSet<String>>,
  /// `<manifest>` object names (generated sprite-pack folders).
  pub manifest: HashSet<String>,
  /// `<aspect>` ids — the satisfies-LUT registry. Doubles as the set of valid
  /// `aspect.<name>` path members (so `aspect.corpus_lit` drift is caught).
  pub aspects: HashSet<String>,
  /// `<globals>` ids — shared constants referenced as `$globals::id`.
  pub globals: HashSet<String>,
}

/// Roots still living as JSON registries — resolution deferred (migrate later).
/// Visual-primitive kinds are NOT here: they're `^hex`/`^rect`/`^sprite`/`^text`
/// engine intrinsics (the `^` FFI boundary, see `vm::PRIM_KINDS`), not `$` refs.
/// `shape` is on its way out.
const DEFERRED_ROOTS: &[&str] = &["shape", "faction", "type"];

impl SymbolTable {
  /// Add one parsed file's definitions to the table.
  pub fn collect(&mut self, node: &Node) {
    match &node.header {
      // Every `::`-catalogued space registers each def name AND its lineage, so a
      // `$<space>::apple` ref resolves whether the corpus holds `apple` or only
      // versioned defs (`apple.0`, `apple.1`). Uniform across all `$`-addressable
      // spaces — content is versioned the same way everywhere.
      Header::Bucket(name) if name == "card" => {
        for c in &node.children {
          if let Header::Def(d) = &c.header {
            register(&mut self.cards, def_id(d));
          }
        }
      }
      Header::Bucket(name) if name == "recipe" => {
        for c in &node.children {
          if let Header::Def(d) = &c.header {
            register(&mut self.recipes, def_id(d));
          }
        }
      }
      Header::Bucket(name) if name == "asset" => {
        for c in &node.children {
          if let Header::Def(d) = &c.header {
            let id = def_id(d);
            let syms = texture_symbols(c);
            // lineage key shares the def's texture LUT (head wins once versioned).
            let lin = crate::loader::lineage(id);
            if lin != id {
              self.assets.entry(lin.to_string()).or_insert_with(|| syms.clone());
            }
            self.assets.insert(id.to_string(), syms);
          }
        }
      }
      Header::Bucket(name) if name == "manifest" => {
        for c in &node.children {
          if let Header::Def(d) = &c.header {
            register(&mut self.manifest, def_id(d));
          }
        }
      }
      Header::Bucket(name) if name == "aspect" => {
        for c in &node.children {
          if let Header::Def(d) = &c.header {
            register(&mut self.aspects, def_id(d));
          }
        }
      }
      Header::Bucket(name) if name == "globals" => {
        for c in &node.children {
          if let Header::Def(d) = &c.header {
            register(&mut self.globals, def_id(d));
          }
        }
      }
      Header::Bucket(name) if name == "functions" => {
        for c in &node.children {
          if let Header::Def(d) = &c.header {
            register(&mut self.functions, def_id(d));
          }
        }
      }
      // legacy bucket-per-function (`<functions:name>`), keyed by the bare name.
      Header::Bucket(name) if name.starts_with("functions:") => {
        self.functions.insert(name["functions:".len()..].to_string());
      }
      _ => {}
    }
    for c in &node.children {
      self.collect(c);
    }
  }
}

/// The record id of a `::` def header — the part before any inline `:facet`.
fn def_id(name: &str) -> &str {
  name.split(':').next().unwrap_or(name)
}

/// Register a def name in `set`, plus its [`crate::loader::lineage`] (the
/// version-stripped logical name) so a `$<space>::<lineage>` ref resolves even
/// when only versioned defs exist. No-op extra insert for a bare name.
fn register(set: &mut HashSet<String>, id: &str) {
  set.insert(id.to_string());
  let lin = crate::loader::lineage(id);
  if lin != id {
    set.insert(lin.to_string());
  }
}

/// The `&texture.<sym>` symbols an asset def's hooks declare (its LUT).
fn texture_symbols(def: &Node) -> HashSet<String> {
  let mut out = HashSet::new();
  for hook in &def.children {
    for stmt in &hook.body {
      if let Stmt::Instr(toks) = stmt {
        for t in toks {
          if let Token::Slot(s) = t {
            if let Some(sym) = s.strip_prefix("texture.") {
              out.insert(sym.to_string());
            }
          }
        }
      }
    }
  }
  out
}

/// Split a `$`-reference path into its root namespace and the segments after
/// it, dropping separators (`::`, `:`, `.`): `card::corpus:data` → ("card",
/// ["corpus","data"]); `asset::symbols:corpus` → ("asset",["symbols","corpus"]).
fn parse_path(v: &str) -> (&str, Vec<&str>) {
  let mut parts = v.split(|c| c == ':' || c == '.').filter(|s| !s.is_empty());
  let root = parts.next().unwrap_or("");
  (root, parts.collect())
}

/// Walk a file's bodies and report `$`-references that fail to resolve.
pub fn unresolved(root: &Node, table: &SymbolTable) -> Vec<Diagnostic> {
  let mut diags = Vec::new();
  walk(root, &mut Vec::new(), table, &mut diags);
  diags
}

fn header_label(h: &Header) -> String {
  match h {
    Header::Block(n) if n.is_empty() => "(root)".into(),
    Header::Block(n) => n.clone(),
    Header::Bucket(n) => format!("<{n}>"),
    Header::Def(n) => format!("::{n}"),
    Header::Facet(n) => format!(":{n}"),
    Header::Hook(n) => format!("@{n}"),
  }
}

fn walk(node: &Node, path: &mut Vec<String>, table: &SymbolTable, diags: &mut Vec<Diagnostic>) {
  path.push(header_label(&node.header));
  if !node.body.is_empty() {
    let p = path.join("/");
    for stmt in &node.body {
      if let Stmt::Instr(toks) = stmt {
        for tok in toks {
          match tok {
            Token::Const(v) => {
              if let Some(msg) = check_ref(v, table, toks) {
                diags.push(Diagnostic { path: p.clone(), message: msg });
              }
            }
            // `&…aspect.<name>` / `*…aspect.<name>` — the member must be a real
            // aspect (skip interpolated `*…` members like `aspect.*var.2`).
            Token::Slot(s) | Token::Value(s) => {
              if let Some(msg) = check_aspect_member(s, table, toks) {
                diags.push(Diagnostic { path: p.clone(), message: msg });
              }
            }
            _ => {}
          }
        }
      }
    }
  }
  for c in &node.children {
    walk(c, path, table, diags);
  }
  path.pop();
}

/// Check the `aspect.<name>` member of a `&`/`*` path against the registry. The
/// segment right after a literal `aspect` segment must be a known `<aspect>` id;
/// an interpolated member (`aspect.*var.2`) is dynamic, so it's left alone.
fn check_aspect_member(path: &str, table: &SymbolTable, toks: &[Token]) -> Option<String> {
  let segs: Vec<&str> = path.split(|c| c == ':' || c == '.').filter(|s| !s.is_empty()).collect();
  for w in segs.windows(2) {
    if w[0] == "aspect" {
      let name = w[1];
      if name.starts_with('*') {
        return None; // interpolated member — resolved at runtime
      }
      if !table.aspects.contains(name) {
        return Some(format!("unknown aspect `{name}` (not in <aspect> registry) in `{path}`: {}", render(toks)));
      }
    }
  }
  None
}

/// `None` if the reference resolves (or is a deferred root); otherwise a message.
fn check_ref(v: &str, table: &SymbolTable, toks: &[Token]) -> Option<String> {
  let (root, seg) = parse_path(v);
  let bad = |kind: &str| Some(format!("unresolved {kind} `${v}` in: {}", render(toks)));
  match root {
    "card" => match seg.first() {
      Some(id) if table.cards.contains(*id) => None,
      _ => bad("card"),
    },
    "recipe" => match seg.first() {
      Some(id) if table.recipes.contains(*id) => None,
      _ => bad("recipe"),
    },
    "functions" => match seg.first() {
      Some(id) if table.functions.contains(*id) => None,
      _ => bad("function"),
    },
    "asset" => match seg.first() {
      // Only the asset name is statically checked; deeper segments
      // (`.object`, `:faction`, `.texture.N`) are runtime deref through
      // the asset's `&object` -> manifest, not a static path.
      Some(id) if table.assets.contains_key(*id) => None,
      _ => bad("asset"),
    },
    "manifest" => match seg.first() {
      Some(id) if table.manifest.contains(*id) => None,
      _ => bad("manifest object"),
    },
    "aspect" => match seg.first() {
      Some(id) if table.aspects.contains(*id) => None,
      _ => bad("aspect"),
    },
    "globals" => match seg.first() {
      Some(id) if table.globals.contains(*id) => None,
      _ => bad("global"),
    },
    r if DEFERRED_ROOTS.contains(&r) => None,
    _ => Some(format!("unknown `$` namespace `{root}` in: {}", render(toks))),
  }
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
  use super::*;
  use crate::parser::parse;

  fn table(srcs: &[&str]) -> SymbolTable {
    let mut t = SymbolTable::default();
    for s in srcs {
      t.collect(&parse(s).unwrap());
    }
    t
  }

  const DEFS: &str = "\
<card>
  ::corpus>
    :data>
      @define>
        faculty &card.type set
  ::aether:visuals>
    @define>
      $shape.rect &shape set
<recipe>
  ::despair_success>
    @output>
      &root destroy
<functions:ring_objects>
  ret
<asset>
  ::pine>
    @define>
      $manifest::conifer &object set
      256 &size set
<manifest>
  ::conifer>
    :neutral>
      @define>
        2 &texture array
        1.png &texture.0 set
        2.png &texture.1 set
<aspect>
  ::wood>
    @define>
      aspects &section set
  ::pine>
    @define>
      1 &satisfies array
      $aspect::wood &satisfies.0 set
";

  #[test]
  fn collects_defs_across_namespaces() {
    let t = table(&[DEFS]);
    assert!(t.cards.contains("corpus"));
    assert!(t.cards.contains("aether")); // inline facet collapses to id
    assert!(t.recipes.contains("despair_success"));
    assert!(t.functions.contains("ring_objects"));
    assert!(t.assets.contains_key("pine"));
    assert!(t.manifest.contains("conifer"));
    assert!(t.aspects.contains("wood"));
    assert!(t.aspects.contains("pine")); // an aspect and a card can share an id
  }

  #[test]
  fn resolves_good_references() {
    let t = table(&[DEFS]);
    let user = parse("\
<recipe>
  ::r>
    @output>
      $card::corpus &slot.0.0.owner.inventory create
      $recipe::despair_success &magnetic.recipe set
      $functions:ring_objects call
      $asset::pine &asset.pine set
      $manifest::conifer &object set
      $shape.rect &shape set
      $aspect::wood &slot.0.0.aspect.pine set
").unwrap();
    assert_eq!(unresolved(&user, &t), vec![]);
  }

  #[test]
  fn resolves_and_flags_aspect_refs_and_members() {
    let t = table(&[DEFS]);
    // a known `$aspect::` ref and a known `aspect.<name>` member both resolve
    let ok = parse("<functions:f>\n  $aspect::pine &slot.0.0.aspect.wood set\n").unwrap();
    assert_eq!(unresolved(&ok, &t), vec![]);
    // unknown `$aspect::` ref
    let d = unresolved(&parse("<functions:f>\n  $aspect::ghost &a set\n").unwrap(), &t);
    assert!(d.iter().any(|d| d.message.contains("unresolved aspect `$aspect::ghost`")), "{d:?}");
    // unknown `aspect.<name>` member (the corpus_lit-style drift this catches)
    let d = unresolved(&parse("<functions:f>\n  *slot.0.0.aspect.ghost 1 ge if &slot.0.0 use\n").unwrap(), &t);
    assert!(d.iter().any(|d| d.message.contains("unknown aspect `ghost`")), "{d:?}");
    // an interpolated member (`aspect.*var.0`) is dynamic — not flagged
    assert_eq!(unresolved(&parse("<functions:f>\n  *aspect.*var.0 &a set\n").unwrap(), &t), vec![]);
  }

  #[test]
  fn flags_dangling_refs() {
    let t = table(&[DEFS]);
    let user = parse("\
<functions:f>
  $card::ghost &a set
  $recipe::ghost &a set
  $functions:ghost call
  $asset::nope &a set
  $manifest::ghost &a set
").unwrap();
    let d = unresolved(&user, &t);
    assert!(d.iter().any(|d| d.message.contains("unresolved card `$card::ghost`")), "{d:?}");
    assert!(d.iter().any(|d| d.message.contains("unresolved recipe `$recipe::ghost`")), "{d:?}");
    assert!(d.iter().any(|d| d.message.contains("unresolved function `$functions:ghost`")), "{d:?}");
    assert!(d.iter().any(|d| d.message.contains("unresolved asset `$asset::nope`")), "{d:?}");
    assert!(d.iter().any(|d| d.message.contains("unresolved manifest object `$manifest::ghost`")), "{d:?}");
  }

  #[test]
  fn flags_unknown_namespace() {
    let t = table(&[DEFS]);
    let user = parse("<functions:f>\n  $object::x &a set\n").unwrap();
    assert!(unresolved(&user, &t).iter().any(|d| d.message.contains("unknown `$` namespace `object`")));
  }

  #[test]
  fn parse_path_cases() {
    assert_eq!(parse_path("card::corpus:data"), ("card", vec!["corpus", "data"]));
    assert_eq!(parse_path("functions:ring_objects"), ("functions", vec!["ring_objects"]));
    assert_eq!(parse_path("asset::symbols:corpus"), ("asset", vec!["symbols", "corpus"]));
    assert_eq!(parse_path("corpus_dim"), ("corpus_dim", Vec::<&str>::new()));
  }
}
