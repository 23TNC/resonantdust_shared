//! resonantdust-codec — the bit-packing + flag-layout foundation.
//!
//! Pure encoding logic: `packed` (id/zone/macro packing), `bits` (bit-field
//! helpers), `flags` (the bit layout — single source of truth), `card_model`
//! (the `Micro` placement enum + flag accessors), and `plan` (ActionPlan /
//! effect value types). No dependency on the DSL — the SpacetimeDB modules link
//! only this; `dsl` and `state` build on top.

pub mod bits;
pub mod card_model;
pub mod flags;
pub mod packed;
pub mod plan;
pub mod stacking;
