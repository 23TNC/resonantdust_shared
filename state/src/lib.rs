//! resonantdust-state — recipe binding **state** validation.
//!
//! The stack/world checks orthogonal to recipe semantics: every bound card
//! exists, isn't dead, isn't conflictingly held, is caller- (or world-) owned,
//! and appears once. Reads cards through the `CardStore` trait; depends on
//! `resonantdust-codec` only (the flag layout), NOT the DSL. The audited
//! gameplay-state movers (card/zone views, synthetic-tile derivation, effect
//! semantics, the host/join stacking resolver) land here next.

pub mod recipe_state;
