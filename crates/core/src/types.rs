//! Shared core types.

use serde::{Deserialize, Serialize};

/// Native-asset sentinel used by Bitcoin and Lightning. Liquid assets always
/// use their canonical 32-byte `AssetId` hex instead.
pub const NATIVE_ASSET: &str = "native";

/// Swap kind. Directions are relative to Lightning: `SwapIn` is on-chain to
/// Lightning, `SwapOut` is Lightning to on-chain, `SwapChain` is on-chain to
/// on-chain across two different chains.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Copy,
    PartialEq,
    Eq,
    Hash,
    strum::Display,
    strum::IntoStaticStr,
    strum::EnumString,
    strum::VariantNames,
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum SwapType {
    SwapIn,
    SwapOut,
    SwapChain,
}

/// On-chain network for swap operations.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Copy,
    PartialEq,
    Eq,
    Hash,
    strum::Display,
    strum::IntoStaticStr,
    strum::EnumString,
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum SwapNetwork {
    Mainnet,
    Testnet,
    Signet,
    Regtest,
}

impl SwapNetwork {
    pub fn bitcoin_network(self) -> bitcoin::Network {
        match self {
            Self::Mainnet => bitcoin::Network::Bitcoin,
            Self::Testnet => bitcoin::Network::Testnet,
            Self::Signet => bitcoin::Network::Signet,
            Self::Regtest => bitcoin::Network::Regtest,
        }
    }

    pub fn liquid_address_params(self) -> &'static elements::AddressParams {
        match self {
            Self::Mainnet => &elements::AddressParams::LIQUID,
            Self::Testnet | Self::Signet => &elements::AddressParams::LIQUID_TESTNET,
            Self::Regtest => &elements::AddressParams::ELEMENTS,
        }
    }
}

impl From<bitcoin::Network> for SwapNetwork {
    fn from(n: bitcoin::Network) -> Self {
        match n {
            bitcoin::Network::Bitcoin => Self::Mainnet,
            bitcoin::Network::Testnet => Self::Testnet,
            bitcoin::Network::Signet => Self::Signet,
            _ => Self::Regtest,
        }
    }
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Copy,
    PartialEq,
    Eq,
    Hash,
    strum::Display,
    strum::IntoStaticStr,
    strum::EnumString,
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum Chain {
    Bitcoin,
    Liquid,
}

/// Chain-neutral reference to a transaction output.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Outpoint {
    pub txid: String,
    pub vout: u32,
}

impl Outpoint {
    pub fn new(txid: impl Into<String>, vout: u32) -> Self {
        Self {
            txid: txid.into(),
            vout,
        }
    }
}

impl std::fmt::Display for Outpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.txid, self.vout)
    }
}

/// Machine-readable cancellation reason. Each variant maps 1:1 to the terminal
/// event that produced it; stored on the swap row alongside `cancel_message`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    strum::Display,
    strum::IntoStaticStr,
    strum::EnumString,
    strum::VariantNames,
)]
pub enum ErrorType {
    /// Deadline elapsed in a time-sensitive state.
    Timeout,
    /// User explicitly cancelled.
    UserCanceled,
    /// A watched deposit failed validation, e.g. two or more paid the exact swap amount.
    TxValidationFailed,
    /// The `OpeningTxBroadcast` phase failed before the tx propagated, e.g. the wallet could not
    /// fund it.
    TxBroadcastFailed,
    /// Lightning invoice payment failed terminally.
    PayInvoiceFailed,
}

/// Swap-in lifecycle states.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    strum::Display,
    strum::IntoStaticStr,
    strum::EnumString,
    strum::VariantNames,
)]
pub enum SwapInStatus {
    /// Watching the swap address for the opening tx and its confirmations.
    AwaitOpeningTxConfirmation,
    /// Opening tx confirmed; paying the claim invoice.
    PayInvoice,
    /// Invoice paid; awaiting refund key for cooperative claim.
    AwaitClaimKeyRevealMessage,
    /// Claim tx broadcast; awaiting on-chain confirmation.
    AwaitClaimTxConfirmation,
    /// Terminal: cancelled before successful claim.
    Canceled,
    /// Terminal: claim tx confirmed.
    Claimed,
}

/// Swap-out lifecycle states.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    strum::Display,
    strum::IntoStaticStr,
    strum::EnumString,
    strum::VariantNames,
)]
pub enum SwapOutStatus {
    /// Awaiting the user to pay the held claim invoice.
    AwaitClaimInvoicePay,
    /// Invoice paid; broadcasting the destination opening tx.
    OpeningTxBroadcast,
    /// Opening tx broadcast; watching for claim or CSV timeout.
    AwaitPreimageRevealed,
    /// CSV timeout matured; awaiting refund tx confirmation.
    AwaitRefundTxConfirmation,
    /// Terminal: cancelled before or immediately after opening tx broadcast.
    Canceled,
    /// Terminal: preimage revealed and hold invoice settled.
    Claimed,
    /// Terminal: destination output refunded after CSV timeout.
    Refunded,
}
