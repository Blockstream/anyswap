//! Client-wide setup: the wallet and the chain parameters the SDK needs.
//!
//! A client builds one [`SwapContext`] at startup and passes it to every swap.
//! The swap knows its own chain, so the caller never chooses anything per call;
//! the context supplies what a swap cannot know by itself, which is the wallet
//! and the Liquid chain parameters.
//!
//! Read those parameters from your own chain source, such as Esplora's block 0
//! and your `elementsd`, not from the server. The genesis hash is committed in
//! every taproot signature hash the wallet signs, and the policy asset decides
//! what the transaction pays its fee in, so a server that supplied them could
//! make the user sign transactions that never confirm.

use std::str::FromStr;

use anyswap_core::{codec, transaction::LiquidChainCtx, types::SwapNetwork};
use elements::AssetId;
use thiserror::Error;

use crate::wallet::SwapWallet;

#[derive(Debug, Error)]
pub enum ContextError {
    #[error("invalid Liquid genesis hash: {0}")]
    GenesisHash(String),

    #[error("invalid Liquid policy asset: {0}")]
    PolicyAsset(String),
}

/// The wallet and chain parameters shared by every swap of one client.
///
/// It holds the wallet, so it holds key material: keep it out of logs and out
/// of anything you serialize.
#[derive(Clone)]
pub struct SwapContext<W: SwapWallet> {
    wallet: W,
    liquid: Option<LiquidChainCtx>,
}

impl<W: SwapWallet> SwapContext<W> {
    /// Creates a context for Bitcoin swaps. The network comes from the wallet.
    /// Call [`with_liquid`](Self::with_liquid) as well to serve Liquid swaps.
    pub fn new(wallet: W) -> Self {
        Self {
            wallet,
            liquid: None,
        }
    }

    /// Adds the Liquid parameters: the network's genesis block hash, which every
    /// signature commits to, and its policy asset, which swaps trade and pay
    /// fees in.
    pub fn with_liquid(
        mut self,
        genesis_hash: &str,
        policy_asset: &str,
    ) -> Result<Self, ContextError> {
        self.liquid = Some(LiquidChainCtx {
            genesis_hash: codec::parse_liquid_block_hash(genesis_hash)
                .map_err(|error| ContextError::GenesisHash(error.to_string()))?,
            policy_asset: AssetId::from_str(policy_asset)
                .map_err(|error| ContextError::PolicyAsset(error.to_string()))?,
        });
        Ok(self)
    }

    pub fn wallet(&self) -> &W {
        &self.wallet
    }

    pub fn network(&self) -> SwapNetwork {
        self.wallet.network()
    }

    /// The Liquid parameters, or `None` when the client is not set up for
    /// Liquid.
    pub(crate) fn liquid(&self) -> Option<LiquidChainCtx> {
        self.liquid
    }
}
