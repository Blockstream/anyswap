use std::{collections::BTreeMap, sync::Mutex};

use anyswap_core::types::{Chain, SwapType};
use async_trait::async_trait;
use bitcoin::secp256k1::{PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
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
/// `swap_id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapRecord {
    pub version: u32,
    pub swap_id: Uuid,
    pub payment_hash: [u8; 32],
    pub source: Endpoint,
    pub dest: Endpoint,
    pub send_amount: u64,
    pub receive_amount: u64,
    /// Our own clock at acceptance.
    pub started_at: Timestamp,
    pub status: Status,
    /// SwapOut: when our preimage was written down as about to leave. What
    /// `coop_timeout` counts from.
    pub committed_at: Option<Timestamp>,
    /// SwapIn: the server's preimage, once it hashed to `payment_hash`.
    pub preimage: Option<[u8; 32]>,
    pub sent: Sent,
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
    pub const VERSION: u32 = 1;

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

    /// The script terms of the end on `side`.
    pub fn terms(&self, side: Side) -> Result<Terms, DriverError> {
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

    fn missing(&self, what: &str) -> DriverError {
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
    Funding {
        expires_at: Timestamp,
        cancel_sent: bool,
    },
    Confirming {
        txid: String,
        confs: u32,
        needed: u32,
        matures_at: Option<Height>,
    },
    CoopKey {
        expires_at: Timestamp,
        matures_at: Height,
    },
    Spending {
        txid: String,
        confs: u32,
    },
    /// SwapIn only. Never left; its contents move with the chain.
    Refund {
        expires_at: Timestamp,
        refundable: u64,
        matures_at: Option<Height>,
        refunded: Vec<(String, bool)>,
    },
    Completed {
        opening: String,
        payout: Option<String>,
    },
    Lost {
        opening: String,
    },
    Canceled {
        opening: Option<String>,
    },
}

impl Status {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Lost { .. } | Self::Canceled { .. }
        )
    }

    /// Nothing a pass could move: terminal, or a `Refund` with nothing at
    /// stake past its window.
    pub fn is_quiet(&self, now: Timestamp) -> bool {
        match self {
            Self::Refund {
                expires_at,
                refundable,
                refunded,
                ..
            } => {
                *refundable == 0
                    && refunded.iter().all(|(_, confirmed)| *confirmed)
                    && now >= *expires_at
            }
            _ => self.is_terminal(),
        }
    }
}

#[derive(Debug, Error)]
#[error("store: {0}")]
pub struct StoreError(pub String);

/// Durable records, wanted by id or in bulk and nothing else.
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait SwapStore: MaybeSend + MaybeSync {
    /// Replaces any record already stored under the same swap id.
    async fn save(&self, record: &SwapRecord) -> Result<(), StoreError>;
    async fn load(&self, swap_id: &Uuid) -> Result<Option<SwapRecord>, StoreError>;
    async fn list(&self) -> Result<Vec<SwapRecord>, StoreError>;
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

    async fn list(&self) -> Result<Vec<SwapRecord>, StoreError> {
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

    async fn list(&self) -> Result<Vec<SwapRecord>, StoreError> {
        Ok(self.0.lock().unwrap().values().cloned().collect())
    }
}
