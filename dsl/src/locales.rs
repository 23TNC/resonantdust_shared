//! Locale string loading — hot-loadable label / description / message catalogs.
//!
//! Locales stay as JSON in `content/locales/<domain>/<lang>.json` (cards,
//! recipes, aspects, panels). This module is the *code* that ingests them: the
//! caller (the client) reads the JSON at runtime and hands it in, so editing a
//! locale file and reloading picks up the change **without recompiling** — the
//! same hot-load model as the `.rd` bundle ([`crate::loader::load`]). New
//! content goes live by shipping JSON, not code.
//!
//! Nested objects flatten to dot-joined keys, prefixed by their domain:
//! `{"requisite":{"log":{"label":"Log"}}}` loaded under domain `cards` becomes
//! `cards.requisite.log.label`. `_comment` keys and non-string leaves
//! (numbers / bools / arrays) are skipped — locales are strings.
//!
//! Dependency-free (a tiny JSON scanner, not `serde_json`) so the crate adds no
//! dependencies to the SpacetimeDB modules that link it as an rlib.

use std::collections::HashMap;

/// A loaded set of localized strings, keyed by flattened `domain.path` key.
/// Construct once from the JSON sources; re-construct to hot-reload.
#[derive(Default, Debug, Clone)]
pub struct Locales {
  strings: HashMap<String, String>,
}

impl Locales {
  /// Ingest `(domain, json)` sources — each domain's top-level JSON object
  /// flattens into `domain.<path>` keys. Errors name the domain + byte offset
  /// on malformed JSON. Idempotent per call: builds a fresh table, so reloading
  /// edited content is just another `load`.
  pub fn load(sources: &[(String, String)]) -> Result<Locales, String> {
    let mut strings = HashMap::new();
    for (domain, json) in sources {
      Parser::new(json)
        .parse_root(domain, &mut strings)
        .map_err(|e| format!("{domain}: {e}"))?;
    }
    Ok(Locales { strings })
  }

  /// The string for a flattened key (`cards.requisite.log.label`), or `None`.
  pub fn get(&self, key: &str) -> Option<&str> {
    self.strings.get(key).map(String::as_str)
  }

  /// Number of strings loaded.
  pub fn len(&self) -> usize {
    self.strings.len()
  }
  pub fn is_empty(&self) -> bool {
    self.strings.is_empty()
  }
}

/// A minimal JSON scanner — enough for locale catalogs (nested objects of
/// strings). Records string leaves into the flat map; skips everything else.
struct Parser<'a> {
  s: &'a [u8],
  i: usize,
}

impl<'a> Parser<'a> {
  fn new(s: &'a str) -> Self {
    Parser { s: s.as_bytes(), i: 0 }
  }

  fn err(&self, msg: &str) -> String {
    format!("{msg} at byte {}", self.i)
  }

  fn peek(&self) -> Option<u8> {
    self.s.get(self.i).copied()
  }

  fn ws(&mut self) {
    while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
      self.i += 1;
    }
  }

  /// Parse the top-level object into `out`, prefixing keys with `domain`.
  fn parse_root(&mut self, domain: &str, out: &mut HashMap<String, String>) -> Result<(), String> {
    self.parse_object(domain, out)
  }

  fn parse_object(&mut self, prefix: &str, out: &mut HashMap<String, String>) -> Result<(), String> {
    self.ws();
    if self.peek() != Some(b'{') {
      return Err(self.err("expected '{'"));
    }
    self.i += 1;
    self.ws();
    if self.peek() == Some(b'}') {
      self.i += 1;
      return Ok(());
    }
    loop {
      self.ws();
      let key = self.parse_string()?;
      self.ws();
      if self.peek() != Some(b':') {
        return Err(self.err("expected ':'"));
      }
      self.i += 1;
      self.ws();
      if key == "_comment" {
        self.skip_value()?;
      } else {
        let full = if prefix.is_empty() { key } else { format!("{prefix}.{key}") };
        match self.peek() {
          Some(b'{') => self.parse_object(&full, out)?,
          Some(b'"') => {
            let val = self.parse_string()?;
            out.insert(full, val);
          }
          _ => self.skip_value()?, // number / bool / null / array — not a locale string
        }
      }
      self.ws();
      match self.peek() {
        Some(b',') => self.i += 1,
        Some(b'}') => {
          self.i += 1;
          return Ok(());
        }
        _ => return Err(self.err("expected ',' or '}'")),
      }
    }
  }

  /// Parse a `"…"` JSON string, decoding escapes. Assumes the next byte is `"`.
  fn parse_string(&mut self) -> Result<String, String> {
    if self.peek() != Some(b'"') {
      return Err(self.err("expected string"));
    }
    self.i += 1;
    let mut out = String::new();
    loop {
      // Copy a run of ordinary bytes (not `"` / `\`) as one UTF-8 slice. The
      // run only ever splits at ASCII `"`/`\`, never mid-codepoint, so the
      // slice is always valid UTF-8.
      let start = self.i;
      while let Some(c) = self.peek() {
        if c == b'"' || c == b'\\' {
          break;
        }
        self.i += 1;
      }
      if self.i > start {
        out.push_str(std::str::from_utf8(&self.s[start..self.i]).map_err(|_| self.err("bad utf-8"))?);
      }
      match self.peek() {
        Some(b'"') => {
          self.i += 1;
          return Ok(out);
        }
        Some(b'\\') => {
          self.i += 1;
          self.read_escape(&mut out)?;
        }
        _ => return Err(self.err("unterminated string")),
      }
    }
  }

  /// Decode one escape sequence (the `\` already consumed) into `out`.
  fn read_escape(&mut self, out: &mut String) -> Result<(), String> {
    let e = self.peek().ok_or_else(|| self.err("unterminated escape"))?;
    self.i += 1;
    match e {
      b'"' => out.push('"'),
      b'\\' => out.push('\\'),
      b'/' => out.push('/'),
      b'n' => out.push('\n'),
      b't' => out.push('\t'),
      b'r' => out.push('\r'),
      b'b' => out.push('\u{0008}'),
      b'f' => out.push('\u{000C}'),
      b'u' => self.read_unicode(out)?,
      _ => return Err(self.err("bad escape")),
    }
    Ok(())
  }

  /// Decode a `\uXXXX` escape (the `\u` already consumed), joining a surrogate
  /// pair if present.
  fn read_unicode(&mut self, out: &mut String) -> Result<(), String> {
    let cp = self.hex4()?;
    let scalar = if (0xD800..=0xDBFF).contains(&cp) {
      // high surrogate → expect a `\uXXXX` low surrogate
      if self.peek() != Some(b'\\') {
        return Err(self.err("expected low surrogate"));
      }
      self.i += 1;
      if self.peek() != Some(b'u') {
        return Err(self.err("expected \\u low surrogate"));
      }
      self.i += 1;
      let lo = self.hex4()?;
      if !(0xDC00..=0xDFFF).contains(&lo) {
        return Err(self.err("bad low surrogate"));
      }
      0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00)
    } else {
      cp
    };
    out.push(char::from_u32(scalar).ok_or_else(|| self.err("invalid codepoint"))?);
    Ok(())
  }

  fn hex4(&mut self) -> Result<u32, String> {
    let mut v = 0u32;
    for _ in 0..4 {
      let c = self.peek().ok_or_else(|| self.err("truncated \\u"))?;
      self.i += 1;
      let d = match c {
        b'0'..=b'9' => (c - b'0') as u32,
        b'a'..=b'f' => (c - b'a' + 10) as u32,
        b'A'..=b'F' => (c - b'A' + 10) as u32,
        _ => return Err(self.err("bad hex digit")),
      };
      v = v * 16 + d;
    }
    Ok(v)
  }

  /// Skip any JSON value without recording it (non-string locale leaves).
  fn skip_value(&mut self) -> Result<(), String> {
    self.ws();
    match self.peek() {
      Some(b'"') => {
        self.parse_string()?;
        Ok(())
      }
      Some(b'{') => self.skip_container(b'{', b'}'),
      Some(b'[') => self.skip_container(b'[', b']'),
      // A structural closer here means the value is missing (`{"x": }`).
      Some(b',' | b'}' | b']') => Err(self.err("expected value")),
      Some(_) => {
        // number / true / false / null — consume to the next structural byte.
        while let Some(c) = self.peek() {
          if matches!(c, b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n') {
            break;
          }
          self.i += 1;
        }
        Ok(())
      }
      None => Err(self.err("expected value")),
    }
  }

  fn skip_container(&mut self, open: u8, close: u8) -> Result<(), String> {
    self.i += 1; // consume the opener
    let mut depth = 1;
    while depth > 0 {
      match self.peek() {
        Some(b'"') => {
          self.parse_string()?; // skip strings so braces inside don't miscount
        }
        Some(c) if c == open => {
          self.i += 1;
          depth += 1;
        }
        Some(c) if c == close => {
          self.i += 1;
          depth -= 1;
        }
        Some(_) => self.i += 1,
        None => return Err(self.err("unterminated container")),
      }
    }
    Ok(())
  }
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn flattens_nested_objects_with_domain_prefix() {
    let cards = r#"{
      "_comment": "ignored",
      "requisite": { "log": { "label": "Log", "description": { "simple": "A cut length of wood." } } }
    }"#;
    let l = Locales::load(&[("cards".into(), cards.into())]).unwrap();
    assert_eq!(l.get("cards.requisite.log.label"), Some("Log"));
    assert_eq!(l.get("cards.requisite.log.description.simple"), Some("A cut length of wood."));
    // _comment is dropped, not stored
    assert_eq!(l.get("cards._comment"), None);
  }

  #[test]
  fn multiple_domains_share_one_table() {
    let cards = r#"{ "requisite": { "log": { "label": "Log" } } }"#;
    let recipes = r#"{ "cut_tree": { "label": "cut tree", "success": { "simple": "{actor} chopped {root}." } } }"#;
    let l = Locales::load(&[("cards".into(), cards.into()), ("recipes".into(), recipes.into())]).unwrap();
    assert_eq!(l.get("cards.requisite.log.label"), Some("Log"));
    assert_eq!(l.get("recipes.cut_tree.label"), Some("cut tree"));
    // placeholders survive verbatim
    assert_eq!(l.get("recipes.cut_tree.success.simple"), Some("{actor} chopped {root}."));
  }

  #[test]
  fn decodes_escapes_and_unicode() {
    let src = r#"{ "k": { "q": "a \"quote\" and é and 😀", "nl": "line1\nline2" } }"#;
    let l = Locales::load(&[("d".into(), src.into())]).unwrap();
    assert_eq!(l.get("d.k.q"), Some("a \"quote\" and é and 😀"));
    assert_eq!(l.get("d.k.nl"), Some("line1\nline2"));
  }

  #[test]
  fn skips_non_string_leaves() {
    let src = r#"{ "a": { "n": 42, "b": true, "arr": [1, 2, {"x": "y"}], "s": "kept" } }"#;
    let l = Locales::load(&[("d".into(), src.into())]).unwrap();
    assert_eq!(l.get("d.a.s"), Some("kept"));
    // numbers / bools / arrays are not locale strings
    assert_eq!(l.get("d.a.n"), None);
    assert_eq!(l.get("d.a.b"), None);
    assert_eq!(l.get("d.a.arr"), None);
  }

  #[test]
  fn empty_object_and_reload() {
    let l = Locales::load(&[("d".into(), "{}".into())]).unwrap();
    assert!(l.is_empty());
    // reloading with new content is just another load
    let l2 = Locales::load(&[("d".into(), r#"{"x":"y"}"#.into())]).unwrap();
    assert_eq!(l2.get("d.x"), Some("y"));
    assert_eq!(l2.len(), 1);
  }

  #[test]
  fn malformed_json_errors_with_domain() {
    let err = Locales::load(&[("cards".into(), r#"{ "x": }"#.into())]).unwrap_err();
    assert!(err.starts_with("cards:"), "{err}");
  }
}
