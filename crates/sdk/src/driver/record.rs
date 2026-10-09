use std::{collections::BTreeMap, sync::Mutex};

use anyswap_core::types::{Chain, Outpoint, SwapInStatus, SwapOutStatus, SwapType};
use async_trait::async_trait;
use bitcoin::secp256k1::{PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};
use thiserror::Error;
use uuid::Uuid;

use super::{
    error::DriverError,
    rt::{MaybeSend, MaybeSync},
};
use crate::swap_script::{Side, Terms};

/// Unix seconds.
pub type Timestamp = u64;

/// A block height on the swap's chain.
pub type Height = u32;

/// One swap as the driver keeps it: both ends frozen at acceptance, the
/// status, and what the chain cannot report. Our own keys derive from
/// `swap_id`. Amounts serialize as decimal strings, as the API's do.
#[serde_as]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapRecord {
    pub version: u32,
    pub swap_id: Uuid,
    pub payment_hash: [u8; 32],
    pub source: Endpoint,
    pub dest: Endpoint,
    #[serde_as(as = "DisplayFromStr")]
    pub send_amount: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub receive_amount: u64,
    /// The server's split of `send_amount - receive_amount`: `base_fee` for
    /// its transactions, `service_fee` the rest.
    #[serde_as(as = "DisplayFromStr")]
    pub base_fee: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub service_fee: u64,
    /// Our own clock at acceptance.
    pub started_at: Timestamp,
    pub status: Status,
    /// SwapIn: what under the script is ours, as the last pass left it.
    #[serde(default)]
    pub funds: Vec<Fund>,
    /// When the status first became final: our clock, or the server's on
    /// restore. What `refund_timeout` counts from.
    #[serde(default)]
    pub ended_at: Option<Timestamp>,
    /// The preimage once seen and hashed to `payment_hash`: on a SwapIn the
    /// server's, from its report or its claim, on a SwapOut ours, from a claim
    /// of ours on chain.
    pub preimage: Option<[u8; 32]>,
    pub sent: Sent,
    /// The server's status as the last pass that reached it read it, `None`
    /// before one. Shown, never decided on.
    #[serde(default)]
    pub server_status: Option<String>,
    /// The server's cancel, kept from the first pass that read one. Never
    /// cleared, since a cancel is final on the server.
    #[serde(default)]
    pub server_cancel: Option<ServerCancel>,
}

/// The server's status by name, `In` for a SwapIn and `Out` for a SwapOut.
#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::Display)]
pub enum ServerStatus {
    #[strum(default)]
    In(SwapInStatus),
    #[strum(default)]
    Out(SwapOutStatus),
}

impl ServerStatus {
    pub fn is_terminal(&self) -> bool {
        match self {
            Self::In(status) => matches!(status, SwapInStatus::Claimed | SwapInStatus::Canceled),
            Self::Out(status) => matches!(
                status,
                SwapOutStatus::Claimed | SwapOutStatus::Canceled | SwapOutStatus::Refunded
            ),
        }
    }
}

/// One end of a swap: on chain when `chain` is set, Lightning when it is
/// not. The server holds the opposite role on each end, the claim key of a
/// source and the refund key of a dest.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub chain: Option<Chain>,
    pub asset: Option<String>,
    pub csv: Option<u32>,
    pub server_pubkey: Option<PublicKey>,
    pub server_blinding_pubkey: Option<PublicKey>,
    /// On chain, where the end pays out: the refund address of a source, the
    /// claim address of a dest.
    pub destination: Option<String>,
    /// Lightning: the invoice, paid to a SwapIn, paid by a SwapOut.
    pub invoice: Option<String>,
    /// The server's key, once it arrived and verified against `server_pubkey`.
    pub server_privkey: Option<SecretKey>,
}

impl Endpoint {
    pub fn lightning(invoice: String) -> Self {
        Self {
            invoice: Some(invoice),
            ..Self::default()
        }
    }

    pub fn on_chain(terms: Terms, destination: Option<String>) -> Self {
        Self {
            chain: Some(terms.chain),
            asset: Some(terms.asset),
            csv: Some(terms.csv),
            server_pubkey: Some(terms.server_pubkey),
            server_blinding_pubkey: terms.server_blinding_pubkey,
            destination,
            invoice: None,
            server_privkey: None,
        }
    }
}

impl SwapRecord {
    /// The record form this SDK writes and drives.
    pub const VERSION: u32 = 1;

    /// Nothing a pass could move. A SwapOut once its status is final. A SwapIn
    /// once its status is final, no refund of ours is unconfirmed, nothing of
    /// ours waits for a destination to take it, and `window` has passed since
    /// it ended, so a deposit that crossed the end is still found. `hook` is
    /// whether the driver can ask for a refund address.
    pub fn is_quiet(&self, now: Timestamp, window: u64, hook: bool) -> bool {
        let destination = hook || self.source.destination.is_some();
        let busy = self.funds.iter().any(|fund| match fund.state {
            FundState::Refunding { .. } => true,
            FundState::Locked { .. } | FundState::Refundable => destination,
            FundState::Dust | FundState::Refunded { .. } => false,
        });
        let watched = self.ended_at.is_none_or(|ended| now < ended + window);
        let swap_in = matches!(self.swap_type(), Ok(SwapType::SwapIn));
        self.status.is_terminal() && !busy && !(swap_in && watched)
    }

    /// SwapIn: the server's key while our refund key is home, which is when a
    /// refund takes the keypath. Once our key leaves the server holds both, so
    /// a keypath spend from then on is read as the server's.
    pub(crate) fn refund_keypath(&self) -> Option<SecretKey> {
        self.source.server_privkey.filter(|_| !self.sent.refund_key)
    }

    pub fn server_status(&self) -> Option<ServerStatus> {
        let label = self.server_status.as_deref()?;
        match self.swap_type().ok()? {
            SwapType::SwapIn => label.parse().ok().map(ServerStatus::In),
            SwapType::SwapOut => label.parse().ok().map(ServerStatus::Out),
            SwapType::SwapChain => None,
        }
    }

    /// What the ends make: one on chain is a SwapIn or a SwapOut, both a
    /// SwapChain.
    pub fn swap_type(&self) -> Result<SwapType, DriverError> {
        match (self.source.chain, self.dest.chain) {
            (Some(_), None) => Ok(SwapType::SwapIn),
            (None, Some(_)) => Ok(SwapType::SwapOut),
            (Some(_), Some(_)) => Ok(SwapType::SwapChain),
            (None, None) => Err(self.missing("chain")),
        }
    }

    pub fn endpoint(&self, side: Side) -> &Endpoint {
        match side {
            Side::Source => &self.source,
            Side::Dest => &self.dest,
        }
    }

    pub fn endpoint_mut(&mut self, side: Side) -> &mut Endpoint {
        match side {
            Side::Source => &mut self.source,
            Side::Dest => &mut self.dest,
        }
    }

    /// The script terms of the end on `side`. Every use of our keys starts
    /// here, so a record of another version stops here.
    pub fn terms(&self, side: Side) -> Result<Terms, DriverError> {
        if self.version != Self::VERSION {
            return Err(DriverError::RecordVersion(self.swap_id, self.version));
        }
        let end = self.endpoint(side);
        let missing = |name: &str| self.missing(&format!("{side} {name}"));
        Ok(Terms {
            swap_id: self.swap_id,
            side,
            chain: end.chain.ok_or_else(|| missing("chain"))?,
            asset: end.asset.clone().ok_or_else(|| missing("asset"))?,
            csv: end.csv.ok_or_else(|| missing("csv"))?,
            payment_hash: self.payment_hash,
            server_pubkey: end.server_pubkey.ok_or_else(|| missing("server pubkey"))?,
            server_blinding_pubkey: end.server_blinding_pubkey,
        })
    }

    pub(crate) fn missing(&self, what: &str) -> DriverError {
        DriverError::MissingField(self.swap_id, what.to_owned())
    }
}

/// The messages already delivered, since a message leaves no trace on chain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sent {
    pub cancel: bool,
    pub refund_key: bool,
    pub preimage: bool,
}

/// Where a swap stands, resolved from the chain on every pass. Deadlines are
/// absolute so two quiet passes produce an equal status.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum Status {
    /// Restored, before its first pass: where it stands is not known yet.
    Restoring,
    Funding {
        expires_at: Timestamp,
        cancel_sent: bool,
    },
    Confirming {
        txid: String,
        confs: u32,
        needed: u32,
    },
    /// SwapOut only. The preimage may have left: claiming the opening, with
    /// `claim` once a claim of ours is broadcast.
    Claiming {
        opening: String,
        claim: Option<String>,
        confs: u32,
    },
    Completed {
        opening: String,
        payout: Option<String>,
    },
    /// The swap did not happen, for `reason`. On a SwapOut the invoice fails
    /// back: the server canceled before any opening, or its CSV refund took
    /// `opening`. On a SwapIn the deposit stays ours, with the rest of `funds`.
    Canceled {
        opening: Option<String>,
        reason: CancelReason,
    },
}

impl Status {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed { .. } | Self::Canceled { .. })
    }
}

/// Why a swap ended `Canceled`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum CancelReason {
    /// The server gave the swap up, in its words: a SwapIn's key, a SwapOut's
    /// report before any opening.
    ServerCanceled(ServerCancel),
    /// SwapIn: the deposit's CSV matured with neither the server's key nor
    /// its preimage.
    CsvMatured,
    /// SwapOut: the server's CSV refund took the opening.
    ServerRefunded,
}

/// The server's cancel in its words: its `ErrorType` name and the message
/// the cancel came with.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCancel {
    pub error_type: Option<String>,
    pub cancel_message: Option<String>,
}

/// SwapIn: one output under the script that is ours, and where its refund
/// stands. The deposit the server was paid for is not listed, nor is an
/// output the server took by its own path.
#[serde_as]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fund {
    pub outpoint: Outpoint,
    #[serde_as(as = "DisplayFromStr")]
    pub amount: u64,
    pub state: FundState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state")]
pub enum FundState {
    /// Under the CSV until `at`, or unconfirmed so its CSV has not started.
    Locked { at: Option<Height> },
    /// Spendable now and worth its fee: the next refund takes it.
    Refundable,
    /// Spendable, but worth less than its fee at the last rate.
    Dust,
    /// In our refund `txid`, not yet in a block.
    Refunding { txid: String },
    /// In our refund `txid`, in a block.
    Refunded { txid: String },
}

#[derive(Clone, Debug, Error)]
#[error("store: {0}")]
pub struct StoreError(pub String);

/// Durable records, wanted by id and nothing else.
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait SwapStore: MaybeSend + MaybeSync {
    /// Replaces any record already stored under the same swap id.
    async fn save(&self, record: &SwapRecord) -> Result<(), StoreError>;
    async fn load(&self, swap_id: &Uuid) -> Result<Option<SwapRecord>, StoreError>;
    /// Every stored swap id, without reading the records, so one that fails
    /// to load stops only its own swap.
    async fn list(&self) -> Result<Vec<Uuid>, StoreError>;
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl<S: SwapStore + ?Sized> SwapStore for std::sync::Arc<S> {
    async fn save(&self, record: &SwapRecord) -> Result<(), StoreError> {
        (**self).save(record).await
    }

    async fn load(&self, swap_id: &Uuid) -> Result<Option<SwapRecord>, StoreError> {
        (**self).load(swap_id).await
    }

    async fn list(&self) -> Result<Vec<Uuid>, StoreError> {
        (**self).list().await
    }
}

/// Records kept in memory, for tests and short-lived tools.
#[derive(Default)]
pub struct MemoryStore(Mutex<BTreeMap<Uuid, SwapRecord>>);

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl SwapStore for MemoryStore {
    async fn save(&self, record: &SwapRecord) -> Result<(), StoreError> {
        self.0
            .lock()
            .unwrap()
            .insert(record.swap_id, record.clone());
        Ok(())
    }

    async fn load(&self, swap_id: &Uuid) -> Result<Option<SwapRecord>, StoreError> {
        Ok(self.0.lock().unwrap().get(swap_id).cloned())
    }

    async fn list(&self) -> Result<Vec<Uuid>, StoreError> {
        Ok(self.0.lock().unwrap().keys().copied().collect())
    }
}
