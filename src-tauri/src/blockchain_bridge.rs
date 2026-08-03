//! Compatibility shim.
//!
//! The chain backend moved from Solana + MagicBlock Ephemeral Rollups to the
//! Avalanche C-Chain — see [`crate::avax_bridge`]. Every call site and the
//! frozen IPC contract reference this module by name, so it re-exports the
//! AVAX bridge's types and implementation wholesale. Nothing lives here
//! anymore.

pub use crate::avax_bridge::*;
