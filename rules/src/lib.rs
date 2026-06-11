//! resonantdust-rules — gameplay rules on top of the DSL + state.
//!
//! `dsl_recipe` runs a proposal's recipe on the shared VM and translates the
//! result into the `ActionPlan` the gate (and later the client) applies. It
//! reads cards through the `CardStore` trait, so the *same* code serves the
//! gate's gathered snapshot and the client's in-memory world model — no drift.
//! The top crate of the workspace: depends on `dsl` (vm/recipe), `state`
//! (CardStore), and `codec` (plan).

pub mod dsl_recipe;
