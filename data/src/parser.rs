//! Lexer + parser for the definition language (cards + recipes).
//!
//! Turns `.recipe` / `.card` source into a block tree. Header grammar, by sigil:
//!
//!   <name>  bucket / compile-time export — `<card>`, `<recipe>`, and each
//!       global function `<functions:ring_objects>`. Holds `::` defs
//!       (structural) or, for `functions:*`, a code body.
//!   ::name>   a stored definition record (the lookup key): `::corpus`,
//!       `::triple_corpus`. Holds `:` facets (cards) or `@` hooks
//!       (recipes). A single-facet card may inline its facet: `::a:visuals>`.
//!   :name>  a structural facet — `:data` / `:visuals`. Holds `@` hooks.
//!       (Inside a *code body*, a `:name>` line is a jump label instead.)
//!   @name>  a code hook: `@define/@init/@update/@input/@output`.
//!
//! A code body (hook or function) is a flat stream of `:label>` markers and
//! postfix instructions; sub-indent there is cosmetic. A `:name>` is a facet in
//! structural position but a label in a body, so the parser is context-aware
//! ([`parse_structural`] vs [`parse_body`]).
//!
//! Path separators (inside references, resolved later — the lexer keeps them
//! verbatim): `::` definition · `:` tag · `.` member. e.g.
//! `$card::corpus:data.aspect.corpus+`.
//!
//! Grammar reference: content/data/SYNTAX.txt. The VM comes later.

/// A single token within an instruction line. Classified by leading sigil; any
/// `::`/`:`/`.` *inside* the token is part of its path and kept verbatim.
// No `Eq`: `Token::Float` carries an `f64`, which is `PartialEq` but not `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
  /// `$path` — resolve a global symbol (`$card::corpus`, `$functions:ring_objects`).
  Const(String),
  /// `&path` — writable slot reference (`&aspect.cost`, `&objects.0`).
  Slot(String),
  /// `*path` — read / deref (`*biome.humidity`, `*slot.1.0.def_id`).
  Value(String),
  /// `:name` — local label reference (`:loop`).
  Label(String),
  /// `^name` — a system call: an engine-native function invoked with `call`
  /// (`^biome`, `^seed`). The FFI boundary to native code we don't express in
  /// the DSL; must be deterministic so server and client agree.
  System(String),
  /// `#rrggbb` — hex color literal (kept verbatim, including the `#`).
  Color(String),
  /// integer literal, including negatives (`10`, `-1`).
  Number(i64),
  /// float literal — any `.`-bearing token that parses as `f64` (`1.5`, `-0.25`).
  /// Integers stay `Number`, so existing integer op semantics are unchanged.
  Float(f64),
  /// bare word: an op (`set`, `if`, `goto`, …) or a content constant (`rtl`).
  Word(String),
}

/// The header / role of a node.
#[derive(Debug, Clone, PartialEq)]
pub enum Header {
  /// `""` root, or a bare structural block.
  Block(String),
  /// `<name>` bucket / compile-time export (`card`, `recipe`, `functions:ring_objects`).
  Bucket(String),
  /// `::name` stored definition record (id, possibly with an inline `:facet`).
  Def(String),
  /// `:name` structural facet (`data`, `visuals`).
  Facet(String),
  /// `@name` code hook.
  Hook(String),
}

/// One item in a code body (hook or function).
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
  /// `:loop>` → `LabelDef("loop")` — a jump target in the stream.
  LabelDef(String),
  /// A postfix instruction line.
  Instr(Vec<Token>),
}

/// A node in the block tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
  pub header: Header,
  pub children: Vec<Node>,
  /// Flat code stream (only on hook / function nodes).
  pub body: Vec<Stmt>,
}

impl Node {
  fn find(&self, header: Header) -> Option<&Node> {
    self.children.iter().find(|c| c.header == header)
  }
  pub fn block(&self, name: &str) -> Option<&Node> {
    self.find(Header::Block(name.to_string()))
  }
  pub fn bucket(&self, name: &str) -> Option<&Node> {
    self.find(Header::Bucket(name.to_string()))
  }
  pub fn def(&self, name: &str) -> Option<&Node> {
    self.find(Header::Def(name.to_string()))
  }
  pub fn facet(&self, name: &str) -> Option<&Node> {
    self.find(Header::Facet(name.to_string()))
  }
  pub fn hook(&self, name: &str) -> Option<&Node> {
    self.find(Header::Hook(name.to_string()))
  }
}

/// `@`-hooks whose content is a code body.
fn is_code_hook(name: &str) -> bool {
  matches!(name, "define" | "init" | "update" | "input" | "output" | "destroy")
}


// ---------- Lexer ----------

#[derive(Debug, Clone, PartialEq)]
enum Raw {
  Angle(String),  // <name>
  DColon(String), // ::name>
  Colon(String),  // :name>
  At(String),   // @name>
  Bare(String),   // name>
  Instr(Vec<Token>),
}

struct LexLine {
  indent: usize,
  raw: Raw,
}

/// Everything from the first `;` to end of line is a comment.
fn strip_comment(line: &str) -> &str {
  match line.find(';') {
    Some(i) => &line[..i],
    None => line,
  }
}

fn classify_token(t: &str) -> Token {
  let first = t.as_bytes()[0] as char;
  match first {
    '$' => Token::Const(t[1..].to_string()),
    '&' => Token::Slot(t[1..].to_string()),
    '*' => Token::Value(t[1..].to_string()),
    ':' => Token::Label(t[1..].to_string()),
    '^' => Token::System(t[1..].to_string()),
    '#' => Token::Color(t.to_string()),
    _ => {
      if let Ok(n) = t.parse::<i64>() {
        Token::Number(n)
      } else if t.contains('.') {
        // Only `.`-bearing tokens try float — keeps bare words (and `inf`/`nan`)
        // as `Word`, and integers as `Number`.
        match t.parse::<f64>() {
          Ok(f) => Token::Float(f),
          Err(_) => Token::Word(t.to_string()),
        }
      } else {
        Token::Word(t.to_string())
      }
    }
  }
}

fn lex(input: &str) -> Vec<LexLine> {
  let mut out = Vec::new();
  for raw in input.lines() {
    let body = strip_comment(raw);
    let indent = body.len() - body.trim_start().len();
    let trimmed = body.trim();
    if trimmed.is_empty() {
      continue;
    }
    let toks: Vec<&str> = trimmed.split_whitespace().collect();
    // A lone token ending in `>` is a header; its leading sigil is its role.
    let r = if toks.len() == 1 && toks[0].ends_with('>') {
      let name = &toks[0][..toks[0].len() - 1];
      if let Some(rest) = name.strip_prefix('<') {
        Raw::Angle(rest.to_string())
      } else if let Some(rest) = name.strip_prefix("::") {
        Raw::DColon(rest.to_string())
      } else if let Some(rest) = name.strip_prefix(':') {
        Raw::Colon(rest.to_string())
      } else if let Some(rest) = name.strip_prefix('@') {
        Raw::At(rest.to_string())
      } else {
        Raw::Bare(name.to_string())
      }
    } else {
      Raw::Instr(toks.iter().map(|t| classify_token(t)).collect())
    };
    out.push(LexLine { indent, raw: r });
  }
  out
}

// ---------- Parser ----------

/// Parse source into the block tree (root is `Block("")`).
pub fn parse(input: &str) -> Result<Node, String> {
  let lines = lex(input);
  let mut i = 0;
  let children = parse_structural(&lines, &mut i, -1)?;
  if i != lines.len() {
    return Err(format!(
      "parser stopped at line index {i} of {} — likely an indentation error",
      lines.len()
    ));
  }
  Ok(Node { header: Header::Block(String::new()), children, body: Vec::new() })
}

/// Parse structural children (headers) more-indented than `parent_indent`.
/// Code hooks and function buckets descend into [`parse_body`]; everything else
/// recurses structurally. A bare instruction here is a grammar error.
fn parse_structural(lines: &[LexLine], i: &mut usize, parent_indent: i64) -> Result<Vec<Node>, String> {
  let mut children = Vec::new();
  while *i < lines.len() && (lines[*i].indent as i64) > parent_indent {
    let line_indent = lines[*i].indent as i64;
    let node = match &lines[*i].raw {
      Raw::Angle(name) => {
        let name = name.clone();
        *i += 1;
        if name == "functions" {
          // `<functions>` holds code-bodied `::name>` defs — each a function
          // (its body is a flat instruction stream, like a hook). Same `<>`/`::`
          // grammar as every other space, so functions catalogue + version like
          // cards. (`<card>`/`<recipe>` `::` defs nest structurally instead.)
          let c = parse_function_defs(lines, i, line_indent)?;
          Node { header: Header::Bucket(name), children: c, body: Vec::new() }
        } else if name.starts_with("functions:") {
          // Legacy bucket-per-function (`<functions:ring_objects>`): a deprecated
          // alias still accepted so older sources + the test harness keep
          // parsing. New content uses `<functions>` + `::name>`.
          let body = parse_body(lines, i, line_indent);
          Node { header: Header::Bucket(name), children: Vec::new(), body }
        } else {
          let c = parse_structural(lines, i, line_indent)?;
          Node { header: Header::Bucket(name), children: c, body: Vec::new() }
        }
      }
      Raw::DColon(name) => {
        let name = name.clone();
        *i += 1;
        let c = parse_structural(lines, i, line_indent)?;
        Node { header: Header::Def(name), children: c, body: Vec::new() }
      }
      Raw::Colon(name) => {
        let name = name.clone();
        *i += 1;
        let c = parse_structural(lines, i, line_indent)?;
        Node { header: Header::Facet(name), children: c, body: Vec::new() }
      }
      Raw::At(name) => {
        let name = name.clone();
        *i += 1;
        if is_code_hook(&name) {
          let body = parse_body(lines, i, line_indent);
          Node { header: Header::Hook(name), children: Vec::new(), body }
        } else {
          let c = parse_structural(lines, i, line_indent)?;
          Node { header: Header::Block(name), children: c, body: Vec::new() }
        }
      }
      Raw::Bare(name) => {
        let name = name.clone();
        *i += 1;
        let c = parse_structural(lines, i, line_indent)?;
        Node { header: Header::Block(name), children: c, body: Vec::new() }
      }
      Raw::Instr(_) => {
        return Err(format!(
          "instruction at structural level (indent {line_indent}) — instructions belong inside a hook or function"
        ));
      }
    };
    children.push(node);
  }
  Ok(children)
}

/// Parse the `::name>` defs inside a `<functions>` bucket. Each is a function: a
/// `Header::Def` whose body is a flat code stream (via [`parse_body`], so its
/// `:label>` markers stay labels) — unlike a `<card>`/`<recipe>` `::` def, whose
/// body nests structurally. This is the one place `::` carries code directly.
fn parse_function_defs(
  lines: &[LexLine],
  i: &mut usize,
  parent_indent: i64,
) -> Result<Vec<Node>, String> {
  let mut defs = Vec::new();
  while *i < lines.len() && (lines[*i].indent as i64) > parent_indent {
    let line_indent = lines[*i].indent as i64;
    match &lines[*i].raw {
      Raw::DColon(name) => {
        let name = name.clone();
        *i += 1;
        let body = parse_body(lines, i, line_indent);
        defs.push(Node { header: Header::Def(name), children: Vec::new(), body });
      }
      _ => {
        return Err(format!(
          "<functions> may only contain `::name>` defs (unexpected header at indent {line_indent})"
        ))
      }
    }
  }
  Ok(defs)
}

/// Collect a flat code body: `:label>` markers and instructions more-indented
/// than `parent_indent`. Sub-indentation among them is cosmetic.
fn parse_body(lines: &[LexLine], i: &mut usize, parent_indent: i64) -> Vec<Stmt> {
  let mut body = Vec::new();
  while *i < lines.len() && (lines[*i].indent as i64) > parent_indent {
    match &lines[*i].raw {
      Raw::Colon(name) => {
        body.push(Stmt::LabelDef(name.clone()));
        *i += 1;
      }
      Raw::Instr(toks) => {
        body.push(Stmt::Instr(toks.clone()));
        *i += 1;
      }
      // A new structural header ends the body.
      Raw::Angle(_) | Raw::DColon(_) | Raw::At(_) | Raw::Bare(_) => break,
    }
  }
  body
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
  use super::*;

  fn instr(node: &Node, idx: usize) -> &[Token] {
    match &node.body[idx] {
      Stmt::Instr(t) => t,
      other => panic!("expected Instr at {idx}, got {other:?}"),
    }
  }

  #[test]
  fn classifies_token_sigils() {
    let src = "<functions:f>\n  $card::corpus *slot.1.0.def_id &aspect.cost -1 #395C39 set :loop ^biome\n";
    let root = parse(src).unwrap();
    let toks = instr(&root.children[0], 0);
    assert_eq!(toks[0], Token::Const("card::corpus".into()));
    assert_eq!(toks[1], Token::Value("slot.1.0.def_id".into()));
    assert_eq!(toks[2], Token::Slot("aspect.cost".into()));
    assert_eq!(toks[3], Token::Number(-1));
    assert_eq!(toks[4], Token::Color("#395C39".into()));
    assert_eq!(toks[5], Token::Word("set".into()));
    assert_eq!(toks[6], Token::Label("loop".into()));
    assert_eq!(toks[7], Token::System("biome".into()));
  }

  #[test]
  fn parses_recipe_structure() {
    let src = "\
<recipe>
  ::triple_corpus>
    @input>
      $card::corpus *slot.1.0.def_id eq if &slot.1.0 use
    @output>
      10 &sys.duration set
      &slot.1.0 destroy
";
    let root = parse(src).unwrap();
    let recipe = root.bucket("recipe").unwrap();
    let tc = recipe.def("triple_corpus").unwrap();
    assert_eq!(tc.hook("input").unwrap().body.len(), 1);
    let output = tc.hook("output").unwrap();
    assert_eq!(output.body.len(), 2);
    assert_eq!(instr(output, 1)[1], Token::Word("destroy".into()));
  }

  #[test]
  fn parses_card_def_with_facets_and_labels() {
    let src = "\
<card>
  ::forest>
    :data>
      @define>
        tile &card.type set
        2 &aspect.pine stock
      @init>
        *biome.rarity 0 10 within !if :r10 goto
        0 1 &aspect.pine range
        :r10>
        0 3 &aspect.pine range
    :visuals>
      @define>
        $shape.hex &shape set
      @update>
        $functions:ring_objects call
";
    let root = parse(src).unwrap();
    let forest = root.bucket("card").unwrap().def("forest").unwrap();
    let data = forest.facet("data").unwrap();
    let init = data.hook("init").unwrap();
    assert_eq!(init.body.len(), 4);
    assert_eq!(init.body[2], Stmt::LabelDef("r10".into()));
    assert!(data.hook("define").is_some());
    let vis = forest.facet("visuals").unwrap();
    assert!(vis.hook("define").is_some());
    assert!(vis.hook("update").is_some());
  }

  #[test]
  fn parses_function_bucket_body_flattened() {
    let src = "\
<functions:ring_objects>
  0 &var.0 set
  :loop>
  &var.0 inc
  :skip>
  *var.0 6 ge if ret
  :loop goto
";
    let root = parse(src).unwrap();
    let ring = root.bucket("functions:ring_objects").unwrap();
    assert!(ring.children.is_empty(), "function body has no sub-blocks");
    assert_eq!(ring.body.len(), 6);
    assert_eq!(ring.body[1], Stmt::LabelDef("loop".into()));
    assert_eq!(ring.body[3], Stmt::LabelDef("skip".into()));
  }

  #[test]
  fn parses_inline_single_facet_def() {
    let src = "\
<card>
  ::aether:visuals>
    @define>
      $shape.rect &shape set
";
    let root = parse(src).unwrap();
    let card = root.bucket("card").unwrap();
    let aether = card.def("aether:visuals").unwrap();
    assert!(aether.hook("define").is_some());
  }

  #[test]
  fn instruction_at_structural_level_errors() {
    let src = "<card>\n  ::x>\n  10 &aspect.cost set\n";
    assert!(parse(src).is_err());
  }

  #[test]
  fn comments_and_blank_lines_are_ignored() {
    let src = "\
; a leading comment
<recipe>

  ::r>      ; trailing comment
    @output>
      10 &sys.duration set   ; set the duration
";
    let root = parse(src).unwrap();
    let out = root.bucket("recipe").unwrap().def("r").unwrap().hook("output").unwrap();
    assert_eq!(out.body.len(), 1);
    assert_eq!(instr(out, 0)[0], Token::Number(10));
  }
}
