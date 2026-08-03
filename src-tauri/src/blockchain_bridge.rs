//! Compatibility shim.
//!
//! The chain backend moved from Avalanche (alloy) to Solana + MagicBlock
//! Ephemeral Rollups — see [`crate::solana_bridge`]. Every call site and the
//! frozen IPC contract reference this module by name, so it re-exports the
//! Solana bridge's types and implementation wholesale. Nothing lives here
//! anymore.

pub use crate::solana_bridge::*;
