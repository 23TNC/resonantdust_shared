//! resonantdust-dsl — the definition language + recipe runtime.
//!
//! One indivisible crate: `loader`, `vm`, `worldgen`, and `resolve`/`validate`
//! are mutually recursive, so the language machinery can't be subdivided.
//! Builds on `resonantdust-codec` (bit-field helpers). Holds the `@input`/
//! `@output` recipe matcher (`vm::match_recipe` / `plan_recipe`), the recipe
//! iterators (`recipe`), the catalog (`loader`/`defs`), and terrain gen
//! (`worldgen`/`noise`).

pub mod bridge;
pub mod defs;
pub mod inspect;
pub mod loader;
pub mod locales;
pub mod noise;
pub mod parser;
pub mod recipe;
pub mod resolve;
pub mod validate;
pub mod vm;
pub mod worldgen;
