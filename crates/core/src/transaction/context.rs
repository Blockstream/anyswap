//! Per-chain execution context threaded to the transaction builders.
//!
//! [`ChainCtx`] carries the chain-specific inputs the builders need: Bitcoin
//! needs none, while Liquid carries a [`LiquidChainCtx`] (genesis hash and
//! policy asset).

use elements::{AssetId, BlockHash};

/// Liquid execution context: the network genesis hash (its signature hashes
/// commit to it) and the policy asset (the fee asset, and the swap asset in the
/// current single-asset model). Both come from configuration / the connected
/// `elementsd`, never a hardcoded constant, so regtest, testnet, and mainnet all
/// work.
#[derive(Debug, Clone, Copy)]
pub struct LiquidChainCtx {
    pub genesis_hash: BlockHash,
    pub policy_asset: AssetId,
}

/// Per-chain execution context threaded to the transaction builders. Bitcoin
/// needs none.
#[derive(Debug, Clone, Copy)]
pub enum ChainCtx {
    Bitcoin,
    Liquid(LiquidChainCtx),
}
