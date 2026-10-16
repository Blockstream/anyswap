//! The pure choice of what a pass does, one module per direction: no IO, no
//! clock, no key, so every branch can be tested on its own.

mod swap_in;
mod swap_out;

use anyswap_core::invoice;
use bitcoin::secp256k1::SecretKey;

use super::{
    config::{ChainConfig, Config},
    observe::Observed,
    record::{Fund, Status, SwapRecord, Timestamp},
};
use crate::{
    swap_script::{Side, Terms},
    utxo::SwapUtxo,
};

/// Something that changes the world. Everything else a pass does is reading.
#[derive(Clone, Debug)]
pub enum Action {
    Send(Message),
    Spend(Spend),
}

/// The three things we ever tell the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    Cancel,
    RefundKey,
    Preimage,
}

#[derive(Clone, Debug)]
pub enum Spend {
    /// Claim the SwapOut opening. `replaces` is a spend of it still in the
    /// mempool, ours or the server's refund; a broadcast refused then is not
    /// an error, since that spend stands.
    Claim {
        utxo: SwapUtxo,
        path: Path,
        fee_rate: f64,
        replaces: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub enum Path {
    Keypath { server_privkey: SecretKey },
    Preimage,
}

/// What a pass does and the status it resolves, assuming the action succeeds.
/// A claim resolves to `Claiming` with the txid the broadcast returns.
#[derive(Clone, Debug)]
pub struct Decision {
    pub action: Option<Action>,
    pub status: Status,
}

/// `terms` are the on-chain end's, the one the loop drives.
pub fn decide(
    record: &SwapRecord,
    terms: &Terms,
    observed: &Observed,
    config: &Config,
    chain_config: &ChainConfig,
) -> Decision {
    match terms.side {
        Side::Source => swap_in::decide(record, terms, observed, config, chain_config),
        Side::Dest => swap_out::decide(record, terms, observed, config, chain_config),
    }
}

/// SwapIn: what under the script is ours as `status` leaves it, and where
/// each refund stands. A SwapOut has none.
pub fn funds(
    record: &SwapRecord,
    terms: &Terms,
    observed: &Observed,
    chain_config: &ChainConfig,
    status: &Status,
) -> Vec<Fund> {
    match terms.side {
        Side::Source => swap_in::funds(record, terms, observed, chain_config, status),
        Side::Dest => Vec::new(),
    }
}

/// When a swap with nothing on chain cancels: `funding_timeout` after
/// `started_at`, or sooner when `bolt11`, the invoice the user pays, expires.
pub(super) fn funding_deadline(
    started_at: Timestamp,
    bolt11: Option<&str>,
    config: &Config,
) -> Timestamp {
    let window = started_at + config.funding_timeout.as_secs();
    bolt11
        .and_then(|bolt11| invoice::expiry_at(bolt11).ok())
        .map_or(window, |expiry| expiry.min(window))
}
