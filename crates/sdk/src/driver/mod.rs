//! A driver that runs a swap to completion against the chain, taking from the
//! server only what the chain can verify: its cancel report, its preimage and
//! its key. The loop is in `tick.rs`, what fixes a swap's terms in
//! `accept.rs`, what survives a crash in `record.rs`, the knobs in
//! `config.rs`, and what an integrator implements for the chain in `chain.rs`
//! and for the server in `server.rs`. See `docs/proposals/driver.md`.

mod accept;
mod chain;
mod config;
mod error;
mod record;
mod rt;
mod server;
mod tick;

use std::{
    collections::HashMap,
    future::Future,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};

use anyswap_core::{
    api::{CancelRequest, CreateSwapRequest},
    invoice,
    types::{Chain, SwapType},
    utils::{now, validate_user_address},
};
use tokio::sync::broadcast::{self, error::RecvError};
use uuid::Uuid;

use self::tick::ServerStatus as Reported;
pub use self::{
    accept::{Quote, RestoreReport},
    chain::{ChainClient, ChainError, HistoryTx},
    config::{ChainConfig, Config},
    error::{DriverError, Refusal},
    record::{
        Endpoint, Height, MemoryStore, Sent, Status, StoreError, SwapRecord, SwapStore, Timestamp,
    },
    rt::{MaybeSend, MaybeSync, spawn},
    server::{ServerClient, Signals},
    tick::{
        Action, Decision, Message, Observed, Output, Path, Server, ServerStatus, Spend, Spender,
        decide,
    },
};
use crate::{
    context::SwapContext,
    swap_in::SwapInStatus,
    swap_out::SwapOutStatus,
    swap_script::Side,
    wallet::{SwapWallet, SwapWalletExt},
};

pub(crate) fn wallet_error(e: impl std::fmt::Display) -> DriverError {
    DriverError::Wallet(e.to_string())
}

#[derive(Clone)]
pub(crate) struct Registered {
    client: Arc<dyn ChainClient>,
    config: ChainConfig,
}

pub(crate) struct Core<W: SwapWallet> {
    ctx: SwapContext<W>,
    client: Arc<dyn ServerClient>,
    store: Arc<dyn SwapStore>,
    locks: Mutex<HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>,
    errors: Mutex<HashMap<Uuid, Arc<DriverError>>>,
    signals: broadcast::Sender<Uuid>,
    updates: broadcast::Sender<Swap<W>>,
    /// The push channel, until the listener takes it.
    source: Mutex<Option<Box<dyn Signals>>>,
    listener: OnceLock<()>,
}

/// Drives every swap in its store. Cheap to clone; every clone shares the
/// store, the server client and the loop's channels.
pub struct Driver<W: SwapWallet> {
    core: Arc<Core<W>>,
    chains: HashMap<Chain, Registered>,
    config: Config,
    destinations: Option<Arc<dyn Destinations>>,
}

/// Where funds land when a swap is created without an address.
pub trait Destinations: MaybeSend + MaybeSync {
    /// The address a SwapOut on `chain` claims to; asked at creation.
    fn claim_address(&self, chain: Chain) -> String;
    /// The address a SwapIn on `chain` refunds to; asked once per broadcast,
    /// only with something spendable under the script.
    fn refund_address(&self, chain: Chain) -> String;
}

impl<W: SwapWallet> Clone for Driver<W> {
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
            chains: self.chains.clone(),
            config: self.config.clone(),
            destinations: self.destinations.clone(),
        }
    }
}

impl<W: SwapWallet + MaybeSend + MaybeSync + 'static> Driver<W> {
    pub fn new(
        ctx: SwapContext<W>,
        client: impl ServerClient + 'static,
        store: impl SwapStore + 'static,
    ) -> Self {
        Self {
            core: Arc::new(Core {
                ctx,
                client: Arc::new(client),
                store: Arc::new(store),
                locks: Mutex::default(),
                errors: Mutex::default(),
                signals: broadcast::channel(64).0,
                updates: broadcast::channel(256).0,
                source: Mutex::new(None),
                listener: OnceLock::new(),
            }),
            chains: HashMap::new(),
            config: Config::default(),
            destinations: None,
        }
    }

    /// Registers a chain with its policy; a chain without one cannot be swapped on.
    pub fn with_chain(
        mut self,
        chain: Chain,
        client: impl ChainClient + 'static,
        config: ChainConfig,
    ) -> Self {
        self.chains.insert(
            chain,
            Registered {
                client: Arc::new(client),
                config,
            },
        );
        self
    }

    pub fn with_config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }

    pub fn with_destinations(mut self, destinations: impl Destinations + 'static) -> Self {
        self.destinations = Some(Arc::new(destinations));
        self
    }

    /// Takes a push channel to pace the loops by; without one they pace on
    /// `poll_interval` alone. `HttpClient::signals` is the server's WebSocket.
    pub fn with_signals(self, signals: impl Signals + 'static) -> Self {
        *self.core.source.lock().unwrap() = Some(Box::new(signals));
        self
    }

    /// Wakes any loop waiting on `swap_id`, as a signal from the channel
    /// would. For a push source the driver cannot pump itself, a webhook
    /// receiver or a browser's socket.
    pub fn signal(&self, swap_id: Uuid) {
        let _ = self.core.signals.send(swap_id);
    }

    /// A quote for a swap from `source` to `dest`, an end being Lightning
    /// when its chain is `None`.
    pub async fn quote(
        &self,
        source: Option<Chain>,
        dest: Option<Chain>,
        receive_amount: u64,
    ) -> Result<Quote, DriverError> {
        for chain in [source, dest].into_iter().flatten() {
            self.chain(chain)?;
        }
        let info = self.core.client.get_info().await?;
        let policy_asset = self.core.ctx.liquid().map(|liquid| liquid.policy_asset);
        accept::quote(
            info,
            self.core.ctx.network(),
            policy_asset,
            source,
            dest,
            receive_amount,
            now(),
        )
    }

    /// Creates a SwapIn against `quote` with the invoice minted for its
    /// `receive_amount`. The handle carries where and how much to deposit.
    /// A `refund_address` is signed with the request and paid by any refund;
    /// without one the destinations hook is asked when there is something to
    /// take, or `refund` is called with one.
    pub async fn swap_in(
        &self,
        quote: Quote,
        claim_invoice: String,
        refund_address: Option<String>,
    ) -> Result<SwapInHandle<W>, DriverError> {
        let chain = self.check_quote(&quote, SwapType::SwapIn)?;
        let msat =
            invoice::amount_msat(&claim_invoice).map_err(|e| Refusal::Invoice(e.to_string()))?;
        if Some(msat) != quote.receive_amount.checked_mul(1000) {
            return Err(Refusal::Invoice("amount is not the quoted one".into()).into());
        }
        if let Some(refund_address) = &refund_address {
            validate_user_address(
                "user_refund_address",
                chain,
                refund_address,
                self.core.ctx.network(),
            )
            .map_err(|e| Refusal::Address(e.to_string()))?;
        }
        let wallet = self.core.ctx.wallet();
        let swap_id = Uuid::new_v4();
        let liquid = chain == Chain::Liquid;
        let req = CreateSwapRequest {
            protocol_version: quote.protocol_version,
            swap_id,
            user_id: wallet.user_id().map_err(wallet_error)?,
            source_chain: quote.source_chain,
            source_asset_id: quote.source_asset_id,
            dest_chain: quote.dest_chain,
            dest_asset_id: quote.dest_asset_id,
            receive_amount: quote.receive_amount,
            swap_fee_limit: quote.fee,
            quote_id: Some(quote.quote_id),
            claim_invoice: Some(claim_invoice),
            source_refund_pubkey: Some(wallet.pubkey(&swap_id).map_err(wallet_error)?.to_string()),
            source_blinding_pubkey: liquid
                .then(|| {
                    wallet
                        .source_blinding_pubkey(&swap_id)
                        .map(|k| k.to_string())
                })
                .transpose()
                .map_err(wallet_error)?,
            dest_claim_pubkey: None,
            dest_blinding_pubkey: None,
            user_claim_address: None,
            user_refund_address: refund_address,
            payment_hash: None,
            webhook: None,
        };
        let started_at = now();
        self.core.client.create_swap(&req).await?;
        let (record, status) = self.adopt(&req, started_at).await?;
        Ok(SwapInHandle {
            handle: Handle::new(self.clone(), record, Some(status)),
        })
    }

    /// Creates a SwapOut against `quote` paying `claim_address`, or the
    /// destinations hook's address without one. The handle carries the hold
    /// invoice to pay.
    pub async fn swap_out(
        &self,
        quote: Quote,
        claim_address: Option<String>,
    ) -> Result<SwapOutHandle<W>, DriverError> {
        let chain = self.check_quote(&quote, SwapType::SwapOut)?;
        let claim_address = claim_address
            .or_else(|| self.destinations.as_ref().map(|d| d.claim_address(chain)))
            .ok_or_else(|| Refusal::Address("none given".into()))?;
        validate_user_address(
            "user_claim_address",
            chain,
            &claim_address,
            self.core.ctx.network(),
        )
        .map_err(|e| Refusal::Address(e.to_string()))?;
        let wallet = self.core.ctx.wallet();
        let swap_id = Uuid::new_v4();
        let liquid = chain == Chain::Liquid;
        let req = CreateSwapRequest {
            protocol_version: quote.protocol_version,
            swap_id,
            user_id: wallet.user_id().map_err(wallet_error)?,
            source_chain: quote.source_chain,
            source_asset_id: quote.source_asset_id,
            dest_chain: quote.dest_chain,
            dest_asset_id: quote.dest_asset_id,
            receive_amount: quote.receive_amount,
            swap_fee_limit: quote.fee,
            quote_id: Some(quote.quote_id),
            claim_invoice: None,
            source_refund_pubkey: None,
            source_blinding_pubkey: None,
            dest_claim_pubkey: Some(wallet.pubkey(&swap_id).map_err(wallet_error)?.to_string()),
            dest_blinding_pubkey: liquid
                .then(|| wallet.dest_blinding_pubkey(&swap_id).map(|k| k.to_string()))
                .transpose()
                .map_err(wallet_error)?,
            user_claim_address: Some(claim_address),
            user_refund_address: None,
            payment_hash: Some(hex::encode(
                wallet.payment_hash(&swap_id).map_err(wallet_error)?,
            )),
            webhook: None,
        };
        let started_at = now();
        self.core.client.create_swap(&req).await?;
        let (record, status) = self.adopt(&req, started_at).await?;
        Ok(SwapOutHandle {
            handle: Handle::new(self.clone(), record, Some(status)),
        })
    }

    /// The stored swap, as a fresh handle.
    pub async fn get(&self, swap_id: Uuid) -> Result<Option<Swap<W>>, DriverError> {
        self.core
            .store
            .load(&swap_id)
            .await?
            .map(|record| Swap::new(self.clone(), record, None))
            .transpose()
    }

    /// Drives every swap in the store that is not quiet, forever. Spawn it.
    pub fn run(&self) -> impl Future<Output = ()> + MaybeSend + 'static {
        let driver = self.clone();
        async move {
            driver.ensure_listener();
            let mut signals = driver.core.signals.subscribe();
            loop {
                driver.sweep().await;
                tokio::select! {
                    _ = rt::sleep(driver.config.poll_interval) => {}
                    _ = signals.recv() => {}
                }
            }
        }
    }

    /// Every swap a pass changed, from `run` and from `step`.
    pub fn subscribe(&self) -> Updates<W> {
        Updates(self.core.updates.subscribe())
    }

    async fn sweep(&self) {
        let Ok(records) = self.core.store.list().await else {
            return;
        };
        let now = now();
        for record in records {
            if record.status.is_quiet(now) {
                continue;
            }
            let id = record.swap_id;
            let _guard = self.lock(id).await;
            let Ok(Some(mut record)) = self.core.store.load(&id).await else {
                continue;
            };
            if let Err(error) = self.tick(&mut record).await {
                self.core.errors.lock().unwrap().insert(id, Arc::new(error));
                self.emit(record, None);
            }
        }
    }

    async fn adopt(
        &self,
        req: &CreateSwapRequest,
        started_at: Timestamp,
    ) -> Result<(SwapRecord, Reported), DriverError> {
        let state = self.core.client.get_swap(req.swap_id).await?;
        match self.accept(req, started_at, state) {
            Ok((record, status)) => {
                self.core.store.save(&record).await?;
                Ok((record, status))
            }
            Err(error) => {
                let cancel = CancelRequest {
                    cancel_message: error.to_string(),
                };
                let _ = self.core.client.cancel(req.swap_id, &cancel).await;
                Err(error)
            }
        }
    }

    /// The quote's on-chain end, once the quote is for `swap_type` and fresh.
    fn check_quote(&self, quote: &Quote, swap_type: SwapType) -> Result<Chain, DriverError> {
        let chain = match (swap_type, quote.source_chain, quote.dest_chain) {
            (SwapType::SwapIn, Some(chain), None) | (SwapType::SwapOut, None, Some(chain)) => chain,
            _ => return Err(DriverError::WrongSwapType),
        };
        self.chain(chain)?;
        if now() >= quote.expires_at {
            return Err(DriverError::QuoteExpired);
        }
        Ok(chain)
    }

    fn chain(&self, chain: Chain) -> Result<&Registered, DriverError> {
        self.chains
            .get(&chain)
            .ok_or(DriverError::ChainNotConfigured(chain))
    }

    async fn lock(&self, swap_id: Uuid) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = self
            .core
            .locks
            .lock()
            .unwrap()
            .entry(swap_id)
            .or_default()
            .clone();
        lock.lock_owned().await
    }

    fn emit(&self, record: SwapRecord, server: Option<Reported>) {
        if let Ok(swap) = Swap::new(self.clone(), record, server) {
            let _ = self.core.updates.send(swap);
        }
    }

    /// Starts pumping the push channel once per driver. Its signals only say
    /// that reading sooner is worthwhile; the loops are paced by them and
    /// never fed by them.
    fn ensure_listener(&self) {
        self.core.listener.get_or_init(|| {
            if let Some(source) = self.core.source.lock().unwrap().take() {
                rt::spawn(listen(
                    Arc::downgrade(&self.core),
                    source,
                    self.config.poll_interval,
                ));
            }
        });
    }

    /// A signal for `swap_id` or `poll_interval`, whichever comes first.
    async fn wait_for(&self, swap_id: Uuid) {
        self.ensure_listener();
        let mut signals = self.core.signals.subscribe();
        let sleep = rt::sleep(self.config.poll_interval);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => return,
                signal = signals.recv() => match signal {
                    Ok(id) if id == swap_id => return,
                    Ok(_) | Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                },
            }
        }
    }
}

/// Forwards each id the channel yields to the loops, waiting `retry` while
/// it is down, until the driver is dropped.
async fn listen<W: SwapWallet + MaybeSend + MaybeSync + 'static>(
    core: Weak<Core<W>>,
    mut source: Box<dyn Signals>,
    retry: Duration,
) {
    loop {
        match source.next().await {
            Some(id) => {
                let Some(core) = core.upgrade() else {
                    return;
                };
                let _ = core.signals.send(id);
            }
            None => {
                if core.strong_count() == 0 {
                    return;
                }
                rt::sleep(retry).await;
            }
        }
    }
}

/// The answer to `cancel`: the swap closes when the server confirms it, and
/// `TooLate` means the payment landed first and the swap continues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cancellation {
    Requested,
    TooLate,
}

/// A swap bound to its driver: the record as this handle last saw it, and
/// the calls that act on it.
pub struct Handle<W: SwapWallet> {
    driver: Driver<W>,
    record: SwapRecord,
    server: Option<ServerStatus>,
    error: Option<Arc<DriverError>>,
}

impl<W: SwapWallet> Clone for Handle<W> {
    fn clone(&self) -> Self {
        Self {
            driver: self.driver.clone(),
            record: self.record.clone(),
            server: self.server.clone(),
            error: self.error.clone(),
        }
    }
}

impl<W: SwapWallet + MaybeSend + MaybeSync + 'static> Handle<W> {
    fn new(driver: Driver<W>, record: SwapRecord, server: Option<ServerStatus>) -> Self {
        let error = driver
            .core
            .errors
            .lock()
            .unwrap()
            .get(&record.swap_id)
            .cloned();
        Self {
            driver,
            record,
            server,
            error,
        }
    }

    pub fn swap_id(&self) -> Uuid {
        self.record.swap_id
    }

    pub fn record(&self) -> &SwapRecord {
        &self.record
    }

    pub fn status(&self) -> &Status {
        &self.record.status
    }

    /// The server's status as the last pass read it. Shown, never decided on.
    pub fn server_status(&self) -> Option<&ServerStatus> {
        self.server.as_ref()
    }

    /// The error of this swap's last failing tick under `run`, `None` after a
    /// working one; read when the handle was made.
    pub fn error(&self) -> Option<&DriverError> {
        self.error.as_deref()
    }

    /// One pass, now.
    pub async fn step(&mut self) -> Result<Status, DriverError> {
        let id = self.record.swap_id;
        let _guard = self.driver.lock(id).await;
        self.record = self
            .driver
            .core
            .store
            .load(&id)
            .await?
            .ok_or(DriverError::Unknown(id))?;
        self.server = self.driver.tick(&mut self.record).await?;
        self.error = None;
        Ok(self.record.status.clone())
    }

    /// Waits for a server notification or `poll_interval`, whichever comes
    /// first, then makes one pass.
    pub async fn advance(&mut self) -> Result<Status, DriverError> {
        self.driver.wait_for(self.record.swap_id).await;
        self.step().await
    }

    pub async fn cancel(&mut self) -> Result<Cancellation, DriverError> {
        let id = self.record.swap_id;
        let _guard = self.driver.lock(id).await;
        if let Some(record) = self.driver.core.store.load(&id).await? {
            self.record = record;
        }
        let open = matches!(
            self.record.status,
            Status::Funding { .. } | Status::Confirming { .. }
        );
        if !open || self.record.committed_at.is_some() {
            return Ok(Cancellation::TooLate);
        }
        let req = CancelRequest {
            cancel_message: "canceled by the user".into(),
        };
        match self.driver.core.client.cancel(id, &req).await {
            Ok(()) => {
                self.record.sent.cancel = true;
                self.driver.core.store.save(&self.record).await?;
                Ok(Cancellation::Requested)
            }
            Err(e) if e.transition_rejected() => Ok(Cancellation::TooLate),
            Err(e) => Err(e.into()),
        }
    }
}

/// A SwapIn: what the user must deposit, and where.
pub struct SwapInHandle<W: SwapWallet> {
    handle: Handle<W>,
}

impl<W: SwapWallet + MaybeSend + MaybeSync + 'static> SwapInHandle<W> {
    /// The deposit address; `record().send_amount` goes here.
    pub fn address(&self) -> Result<String, DriverError> {
        let terms = self.handle.record.terms(Side::Source)?;
        let script = self.handle.driver.core.ctx.script(&terms)?;
        Ok(script.address())
    }

    pub fn server_status(&self) -> Option<&SwapInStatus> {
        match self.handle.server.as_ref()? {
            ServerStatus::In(status) => Some(status),
            ServerStatus::Out(_) => None,
        }
    }

    /// Takes every output that is ours, spendable and worth its fee to
    /// `destination`. `None` when there is nothing to spend yet.
    pub async fn refund(&self, destination: &str) -> Result<Option<String>, DriverError> {
        let id = self.record.swap_id;
        let _guard = self.driver.lock(id).await;
        let record = self
            .driver
            .core
            .store
            .load(&id)
            .await?
            .ok_or(DriverError::Unknown(id))?;
        self.driver.refund(&record, destination).await
    }
}

/// A SwapOut: the hold invoice the user must pay.
pub struct SwapOutHandle<W: SwapWallet> {
    handle: Handle<W>,
}

impl<W: SwapWallet + MaybeSend + MaybeSync + 'static> SwapOutHandle<W> {
    /// The hold invoice to pay; `record().send_amount` is its amount.
    pub fn claim_invoice(&self) -> &str {
        self.handle
            .record
            .source
            .invoice
            .as_deref()
            .unwrap_or_default()
    }

    pub fn server_status(&self) -> Option<&SwapOutStatus> {
        match self.handle.server.as_ref()? {
            ServerStatus::Out(status) => Some(status),
            ServerStatus::In(_) => None,
        }
    }
}

pub enum Swap<W: SwapWallet> {
    In(SwapInHandle<W>),
    Out(SwapOutHandle<W>),
}

impl<W: SwapWallet + MaybeSend + MaybeSync + 'static> Swap<W> {
    fn new(
        driver: Driver<W>,
        record: SwapRecord,
        server: Option<ServerStatus>,
    ) -> Result<Self, DriverError> {
        let swap_type = record.swap_type()?;
        let handle = Handle::new(driver, record, server);
        match swap_type {
            SwapType::SwapIn => Ok(Self::In(SwapInHandle { handle })),
            SwapType::SwapOut => Ok(Self::Out(SwapOutHandle { handle })),
            other => Err(DriverError::Unsupported(other)),
        }
    }
}

macro_rules! deref_handle {
    ($type:ident, |$self:ident| $handle:expr, |$self_mut:ident| $handle_mut:expr) => {
        impl<W: SwapWallet> Deref for $type<W> {
            type Target = Handle<W>;

            fn deref(&$self) -> &Handle<W> {
                $handle
            }
        }

        impl<W: SwapWallet> DerefMut for $type<W> {
            fn deref_mut(&mut $self_mut) -> &mut Handle<W> {
                $handle_mut
            }
        }

        impl<W: SwapWallet> Clone for $type<W> {
            fn clone(&self) -> Self {
                self.clone_handle()
            }
        }
    };
}

deref_handle!(SwapInHandle, |self| &self.handle, |self| &mut self.handle);
deref_handle!(SwapOutHandle, |self| &self.handle, |self| &mut self.handle);
deref_handle!(
    Swap,
    |self| match self {
        Swap::In(swap) => &swap.handle,
        Swap::Out(swap) => &swap.handle,
    },
    |self| match self {
        Swap::In(swap) => &mut swap.handle,
        Swap::Out(swap) => &mut swap.handle,
    }
);

impl<W: SwapWallet> SwapInHandle<W> {
    fn clone_handle(&self) -> Self {
        Self {
            handle: self.handle.clone(),
        }
    }
}

impl<W: SwapWallet> SwapOutHandle<W> {
    fn clone_handle(&self) -> Self {
        Self {
            handle: self.handle.clone(),
        }
    }
}

impl<W: SwapWallet> Swap<W> {
    fn clone_handle(&self) -> Self {
        match self {
            Self::In(swap) => Self::In(swap.clone()),
            Self::Out(swap) => Self::Out(swap.clone()),
        }
    }
}

/// Every swap a pass changed, as `run` and `step` produce them, plus a
/// handle for every failing tick under `run`.
pub struct Updates<W: SwapWallet>(broadcast::Receiver<Swap<W>>);

impl<W: SwapWallet> Updates<W> {
    pub async fn next(&mut self) -> Option<Swap<W>> {
        loop {
            match self.0.recv().await {
                Ok(swap) => return Some(swap),
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return None,
            }
        }
    }
}
