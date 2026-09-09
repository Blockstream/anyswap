use anyswap_core::{
    transaction::TxError,
    types::{Chain, SwapNetwork, SwapType},
};
use thiserror::Error;
use uuid::Uuid;

use super::{chain::ChainError, record::StoreError};
use crate::{client::ClientError, swap_script::ScriptError, utxo::UtxoError};

#[derive(Debug, Error)]
pub enum DriverError {
    #[error(transparent)]
    Chain(#[from] ChainError),

    #[error("server: {0}")]
    Server(#[from] ClientError),

    #[error(transparent)]
    Store(#[from] StoreError),

    #[error("wallet: {0}")]
    Wallet(String),

    #[error("refused: {0}")]
    Refused(#[from] Refusal),

    #[error(transparent)]
    Tx(#[from] TxError),

    #[error(transparent)]
    Utxo(#[from] UtxoError),

    #[error("chain {0} is not configured")]
    ChainNotConfigured(Chain),

    #[error("swap {0} is not in the store")]
    Unknown(Uuid),

    #[error("the record of swap {0} has no {1}")]
    MissingField(Uuid, String),

    #[error("swap type {0} is not driven")]
    Unsupported(SwapType),

    #[error("the server holds no signed request for swap {0}")]
    Unsigned(Uuid),

    #[error("the signed request of swap {0} does not verify")]
    BadSignature(Uuid),

    #[error("the quote expired")]
    QuoteExpired,

    #[error("the quote is for another swap type")]
    WrongSwapType,

    #[error("the server offers no quote: {0}")]
    NotQuotable(&'static str),

    #[error("amount outside {min}..={max}")]
    AmountOutOfRange { min: u64, max: u64 },
}

impl DriverError {
    /// Chain or server unreachable: the next pass sees the same world. Anything
    /// else means nothing may act before it is understood.
    pub fn retryable(&self) -> bool {
        match self {
            Self::Chain(_) | Self::Server(ClientError::Transport(_)) => true,
            Self::Server(ClientError::Api { status, .. }) => *status >= 500,
            _ => false,
        }
    }
}

impl From<ScriptError> for DriverError {
    fn from(e: ScriptError) -> Self {
        Self::Refused(e.into())
    }
}

/// An acceptance check the server's copy of a swap failed.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum Refusal {
    #[error("the server is on {found}, this wallet on {expected}")]
    Network {
        expected: SwapNetwork,
        found: SwapNetwork,
    },

    #[error("protocol version {found}, asked {expected}")]
    ProtocolVersion { expected: String, found: String },

    #[error("swap type {0}")]
    SwapType(SwapType),

    #[error("swap id {0}")]
    SwapId(Uuid),

    #[error("chain {0}")]
    Chain(Chain),

    #[error("asset {0}")]
    Asset(String),

    #[error("receive amount {found}, asked {expected}")]
    ReceiveAmount { expected: u64, found: u64 },

    #[error("send amount {send_amount} is not {receive_amount} + {base_fee} + {service_fee}")]
    SendAmount {
        send_amount: u64,
        receive_amount: u64,
        base_fee: u64,
        service_fee: u64,
    },

    #[error("fee {fee} above the limit {limit}")]
    FeeAboveLimit { fee: u64, limit: u64 },

    #[error("script: {0}")]
    Script(String),

    #[error("address: {0}")]
    Address(String),

    #[error("blinding pubkey")]
    BlindingPubkey,

    #[error("our pubkey")]
    OurPubkey,

    #[error("claim invoice differs from the one submitted")]
    ClaimInvoice,

    #[error("payment hash")]
    PaymentHash,

    #[error("invoice: {0}")]
    Invoice(String),

    #[error("csv {csv} outside {min}..={max}")]
    Csv { csv: u32, min: u32, max: u32 },

    #[error("wallet: {0}")]
    Wallet(String),

    #[error("Liquid is not configured")]
    LiquidNotConfigured,

    #[error("malformed: {0}")]
    Malformed(String),
}

impl From<ScriptError> for Refusal {
    fn from(e: ScriptError) -> Self {
        match e {
            ScriptError::LiquidNotConfigured => Self::LiquidNotConfigured,
            ScriptError::UnexpectedAsset { found, .. } => Self::Asset(found),
            ScriptError::MissingField(_) => Self::BlindingPubkey,
            ScriptError::Wallet(e) => Self::Wallet(e),
            ScriptError::Script(e) => Self::Script(e),
        }
    }
}
