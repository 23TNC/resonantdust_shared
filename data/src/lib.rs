//! resonantdust-data — transitional facade.
//!
//! The shared crate was split into `codec` / `dsl` / `state` / `protocol` (see
//! `docs/shared_crate_split`). This re-exports them under the old module paths so
//! existing `use resonantdust_data::X` keeps compiling while consumers migrate to
//! the member crates directly. Deleted once narrowed.

pub use resonantdust_codec::{bits, card_model, flags, packed, plan};
pub use resonantdust_dsl::{
    bridge, defs, inspect, loader, locales, noise, parser, recipe, resolve, validate, vm, worldgen,
};
pub use resonantdust_state::recipe_state;

/// Wire protocol — behind the `protocol` feature (gateway + client), exactly as
/// before. The SpacetimeDB modules don't enable it, so they don't pull serde_json.
#[cfg(feature = "protocol")]
pub use resonantdust_protocol::protocol;
