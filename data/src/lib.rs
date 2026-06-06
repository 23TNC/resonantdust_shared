//! Resonant Dust shared **data-manipulation** crate.
//!
//! Pure logic over bytes and rows handed in at call time: packing /
//! unpacking and bit-field helpers, and eventually the recipe matcher.
//! There is deliberately **no embedded catalog JSON** here — catalog
//! *values* live in `content` today and move to SpacetimeDB tables under
//! the table-promotion plan. This crate only knows how to *manipulate*
//! them, so editing card/recipe data never touches it.
//!
//! It is built two ways, like the old shared crate:
//!   - linked as an **rlib** into the SpacetimeDB modules (server), and
//!   - wrapped by the sibling `resonantdust-wasm` crate and compiled to a
//!     browser bundle for the pixijs client.
//!
//! Seed contents only. The real codec / matcher port lands here
//! incrementally, in parallel with current development.

pub mod bits;
pub mod bridge;
pub mod card_model;
pub mod defs;
pub mod flags;
pub mod inspect;
pub mod loader;
pub mod locales;
pub mod noise;
pub mod packed;
pub mod parser;
pub mod plan;
pub mod recipe;
pub mod recipe_state;
pub mod resolve;
pub mod validate;
pub mod vm;
pub mod worldgen;

