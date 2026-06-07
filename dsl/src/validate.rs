//! Static (per-file) validation of the parsed block tree.
//!
//! Intra-body checks derivable from one file's [`crate::parser::Node`] tree:
//!
//! 1. **Stack neutrality** — each instruction line, simulated as straight-line
//!  postfix, never underflows and ends at depth 0. The language has
//!  `goto`/labels, so every line must net 0 for jump targets to be reached at
//!  a consistent depth. `if`/`!if` consume the condition and gate the rest of
//!  the line; the gated remainder is itself net-zero, so a straight pass
//!  validates both branches. A bare word is an op if known, else a literal
//!  constant (an enum/type token like `faculty`) that pushes one value — so a
//!  misspelled op reads as a literal and surfaces here as an imbalance.
//!
//! 2. **Local label resolution** — a `:name goto` resolves to a `:name>` marker
//!  in the same body.
//!
//! Cross-symbol `$` resolution (cards, recipes, functions, catalog) is a
//! whole-corpus concern and lives in [`crate::resolve`].

use crate::parser::{Node, Stmt, Token};
use std::collections::HashSet;

/// A single validation problem, tagged with a `/`-joined block breadcrumb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
  pub path: String,
  pub message: String,
}

/// Operand-stack effect of an op: `(pops, pushes)`. `None` for an unknown word.
fn op_effect(word: &str) -> Option<(u32, u32)> {
  let e = match word {
    "eq" | "ne" | "gt" | "ge" | "lt" | "le" => (2, 1),
    "and" | "or" => (2, 1),
    "not" => (1, 1),
    "add" | "sub" | "mul" | "div" | "mod" => (2, 1),
    // trig (radians) + the `pi` constant — for DSL-computed ring/layout positions.
    "sin" | "cos" | "sqrt" => (1, 1),
    "pi" => (0, 1),
    // deterministic pseudo-random: hash of a seed (pure, so server/client
    // agree). `<seed> random` -> hashed value; range it with `mod`.
    "random" => (1, 1),
    "within" => (3, 1),
    "count" => (1, 1),
    "key" => (2, 1),    // map, index -> key name
    "recall" => (2, 1), // id, namespace -> catalog record
    "inc" | "dec" => (1, 0),
    "set" => (2, 0), // value, addr
    "range" => (3, 0),
    "vec2" => (3, 0), // x, y, addr -> {x, y}
    "normalize" => (2, 0),
    "scatter" => (5, 0), // input, lo, hi, seed, addr -> band-relative count + jitter
    "stock" => (2, 0),
    "array" => (2, 0),
    "destroy" => (1, 0),
    "create" => (2, 0),
    "borrow" | "use" | "claim" | "share" => (1, 0),
    "if" | "!if" => (1, 0),
    "goto" => (1, 0),
    // `call` runs a function or `^system` call and leaves its return on the
    // stack — consume it (`… &x set`) or discard it (`… drop`). `ret` returns
    // exactly one value (use `0 ret` for void). So call/ret are arity-uniform
    // and the per-line neutrality check needs no knowledge of the callee.
    "call" => (1, 1),
    "ret" => (1, 0),
    "drop" => (1, 0),
    "rtl" | "ltr" => (0, 1),
    _ => return None,
  };
  Some(e)
}

/// Validate one file's tree. Empty result means clean.
pub fn validate(root: &Node) -> Vec<Diagnostic> {
  let mut diags = Vec::new();
  walk(root, &mut Vec::new(), &mut diags);
  diags
}

fn header_label(h: &crate::parser::Header) -> String {
  use crate::parser::Header::*;
  match h {
    Block(n) if n.is_empty() => "(root)".into(),
    Block(n) => n.clone(),
    Bucket(n) => format!("<{n}>"),
    Def(n) => format!("::{n}"),
    Facet(n) => format!(":{n}"),
    Hook(n) => format!("@{n}"),
  }
}

fn walk(node: &Node, path: &mut Vec<String>, diags: &mut Vec<Diagnostic>) {
  path.push(header_label(&node.header));
  if !node.body.is_empty() {
    check_body(node, &path.join("/"), diags);
  }
  for c in &node.children {
    walk(c, path, diags);
  }
  path.pop();
}

fn check_body(node: &Node, path: &str, diags: &mut Vec<Diagnostic>) {
  let locals: HashSet<&str> = node
    .body
    .iter()
    .filter_map(|s| match s {
      Stmt::LabelDef(n) => Some(n.as_str()),
      _ => None,
    })
    .collect();

  for stmt in &node.body {
    let toks = match stmt {
      Stmt::Instr(t) => t,
      Stmt::LabelDef(_) => continue,
    };

    // --- stack neutrality ---
    let mut depth: i64 = 0;
    let mut underflowed = false;
    for tok in toks {
      // A bare word is an op if known; otherwise a literal constant (an
      // enum/type token like `faculty`/`rtl`) that pushes one value.
      let (pops, pushes) = match tok {
        Token::Word(w) => op_effect(w).unwrap_or((0, 1)),
        _ => (0, 1),
      };
      if depth < pops as i64 {
        underflowed = true;
      }
      depth = depth - pops as i64 + pushes as i64;
    }
    if underflowed {
      diags.push(Diagnostic {
        path: path.into(),
        message: format!("stack underflow in: {}", render(toks)),
      });
    } else if depth != 0 {
      diags.push(Diagnostic {
        path: path.into(),
        message: format!("line not stack-neutral (ends at depth {depth}): {}", render(toks)),
      });
    }

    // --- local labels + slot-path sanity ---
    for tok in toks {
      match tok {
        Token::Label(name) if !locals.contains(name.as_str()) => {
          diags.push(Diagnostic {
            path: path.into(),
            message: format!("unresolved label `:{name}` in: {}", render(toks)),
          });
        }
        // `::` is the definition-lookup separator (`$card::x`); a writable
        // `&` or readable `*` slot is a member path and must use `.`.
        Token::Slot(s) | Token::Value(s) if s.contains("::") => {
          let sig = if matches!(tok, Token::Slot(_)) { '&' } else { '*' };
          diags.push(Diagnostic {
            path: path.into(),
            message: format!(
              "`::` in slot path `{sig}{s}` — use `.` (`::` is only for `$` lookups) in: {}",
              render(toks)
            ),
          });
        }
        _ => {}
      }
    }
  }
}

/// Re-render an instruction line roughly as written, for diagnostics.
pub(crate) fn render(toks: &[Token]) -> String {
  toks.iter()
    .map(|t| match t {
      Token::Const(s) => format!("${s}"),
      Token::Slot(s) => format!("&{s}"),
      Token::Value(s) => format!("*{s}"),
      Token::Label(s) => format!(":{s}"),
      Token::System(s) => format!("^{s}"),
      Token::Color(s) => s.clone(),
      Token::Number(n) => n.to_string(),
      Token::Float(f) => f.to_string(),
      Token::Word(s) => s.clone(),
    })
    .collect::<Vec<_>>()
    .join(" ")
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
  use super::*;
  use crate::parser::parse;

  fn diags(src: &str) -> Vec<Diagnostic> {
    validate(&parse(src).unwrap())
  }

  #[test]
  fn clean_body_has_no_diagnostics() {
    let src = "\
<recipe>
  ::triple_corpus>
  @input>
    $card::corpus *slot.1.0.def_id eq if &slot.1.0 use
  @output>
    10 &sys.duration set
    &slot.1.0 destroy
";
    assert_eq!(diags(src), vec![]);
  }

  #[test]
  fn bareword_value_is_literal() {
    let src = "<functions:f>\n  faculty &card.type set\n";
    assert_eq!(diags(src), vec![]);
  }

  #[test]
  fn detects_stack_underflow() {
    let src = "<functions:f>\n  &a set\n";
    assert!(diags(src).iter().any(|d| d.message.contains("underflow")));
  }

  #[test]
  fn detects_non_neutral_line() {
    let src = "<functions:f>\n  1 2\n";
    assert!(diags(src).iter().any(|d| d.message.contains("not stack-neutral")));
  }

  #[test]
  fn detects_unresolved_label() {
    let src = "<functions:f>\n  :missing goto\n";
    assert!(diags(src).iter().any(|d| d.message.contains("unresolved label `:missing`")));
  }

  #[test]
  fn detects_double_colon_in_slot_path() {
    // `$asset::pine` (lookup) is fine; `&asset::pine` (a writable slot) is not.
    let src = "<functions:f>\n  $asset::pine &asset::pine set\n";
    assert!(
      diags(src).iter().any(|d| d.message.contains("`::` in slot path `&asset::pine`")),
      "{:?}", diags(src)
    );
  }

  #[test]
  fn system_call_balances() {
    // `^biome call &biome set` — system call's return consumed by set
    let src = "<functions:f>\n  ^biome call &biome set\n";
    assert_eq!(diags(src), vec![]);
  }

  #[test]
  fn call_return_must_be_consumed() {
    // a bare `call` leaves its return on the stack → not neutral (use drop/set)
    assert!(diags("<functions:f>\n  $functions:f call\n").iter().any(|d| d.message.contains("not stack-neutral")));
    assert_eq!(diags("<functions:f>\n  $functions:f call drop\n"), vec![]);
  }

  #[test]
  fn random_balances() {
    // `<seed> random <n> mod` — deterministic pick in 0..n-1.
    let src = "<functions:f>\n  *biome.rarity *var.0 add random 6 mod &var.1 set\n";
    assert_eq!(diags(src), vec![]);
  }

  #[test]
  fn within_and_normalize_balance() {
    let src = "\
<card>
  ::forest:data>
  @init>
    *biome.rarity 0 10 within !if :r10 goto
    0 1 &aspect.pine range
    :r10>
    &aspect.pine *biome.humidity normalize
";
    assert_eq!(diags(src), vec![]);
  }
}
