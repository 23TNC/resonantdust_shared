//! resonantdust-protocol — the client↔gate wire types (JSON over the WS).
//!
//! Was the `protocol` feature of `resonantdust-data`; now its own crate so the
//! SpacetimeDB modules (which want the codec but not `serde_json`) simply don't
//! depend on it. Both message types derive `Serialize` + `Deserialize` so the
//! gateway and the client share one contract.

pub mod protocol;
