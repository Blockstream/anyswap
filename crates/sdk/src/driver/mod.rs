//! A driver that runs a swap to completion against the chain, taking from the
//! server only what the chain can verify: its cancel report, its preimage and
//! its key. The loop is in `tick.rs`, what a pass reads in `observe.rs`, what
//! it decides in `decide/`, what fixes a swap's terms in `accept.rs`, what
//! survives a crash in `record.rs`, the knobs in `config.rs`, and what an
//! integrator implements for the chain in `chain.rs` and for the server in
//! `server.rs`. See `docs/proposals/driver.md`.

mod accept;
mod chain;
mod config;
mod decide;
mod error;
mod observe;
mod record;
mod rt;
mod server;
mod tick;

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};

use anyswap_core::{
    PROTOCOL_VERSION,
    api::{CancelRequest, CreateSwapRequest},
    invoice,
    types::{Chain, SwapType},
    utils::{now, validate_user_address},
};
use async_trait::async_trait;
use tokio::sync::{
    broadcast::{self, error::RecvError},
    watch,
};
use uuid::Uuid;

pub use self::{
    accept::{Quote, RestoreReport, TargetAmount},
    chain::{ChainClient, ChainError, HistoryTx},
    config::{ChainConfig, Config},
    decide::{Action, Decision, Message, Path, Spend, decide, funds},
    error::{DriverError, Refusal},
    observe::{Observed, Output, Report, Server, Spender},
    record::{
        CancelReason, Endpoint, Fund, FundState, Height, MemoryStore, Sent, ServerCancel,
        ServerStatus, Status, StoreError, SwapRecord, SwapStore, Timestamp,
    },
    rt::{MaybeSend, MaybeSync, spawn},
    server::{ServerClient, Signals},
};
use crate::{
    context::SwapContext,
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
    errors: Mutex<HashMap<Uuid, DriverError>>,
    signals: broadcast::Sender<Uuid>,
    updates: broadcast::Sender<Handle<W>>,
    /// The push channel, until the listener takes it.
    source: Mutex<Option<Box<dyn Signals>>>,
    listener: OnceLock<()>,
    /// Where `run` is; `stop` and the loop move it.
    state: watch::Sender<RunState>,
}

/// Where `run` is: never started, looping, told to stop and finishing its
/// pass, or exited. Stopped is final.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    Idle,
    Running,
    Stopping,
    Stopped,
}

/// Resolves once the driver is `Stopped`, or gone.
async fn stopped(state: &mut watch::Receiver<RunState>) {
    let _ = state.wait_for(|state| *state == RunState::Stopped).await;
}

/// Drives every swap in its store. Cheap to clone; every clone shares the
/// store, the server client, the loop's channels and its run state.
pub struct Driver<W: SwapWallet> {
    core: Arc<Core<W>>,
    chains: HashMap<Chain, Registered>,
    config: Config,
    destinations: Option<Arc<dyn Destinations>>,
}

/// Where funds land when a swap is created without an address.
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait Destinations: MaybeSend + MaybeSync {
    /// The address a SwapOut on `chain` claims to; asked at creation.
    async fn claim_address(&self, chain: Chain) -> Result<String, String>;
    /// The address a SwapIn on `chain` refunds to; asked once per broadcast,
    /// only with something spendable under the script.
    async fn refund_address(&self, chain: Chain) -> Result<String, String>;
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
                state: watch::channel(RunState::Idle).0,
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
    /// when its chain is `None`, pinned by the amount the user receives or
    /// the amount they spend. Pinned by what they spend, the terms are
    /// inverted against the one read of `/v1/info` the quote comes from.
    pub async fn quote(
        &self,
        source: Option<Chain>,
        dest: Option<Chain>,
        target: TargetAmount,
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
            target,
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
    ) -> Result<Handle<W>, DriverError> {
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
            protocol_version: PROTOCOL_VERSION.into(),
            swap_id,
            user_id: wallet.user_id().map_err(wallet_error)?,
            source_chain: quote.source_chain,
            source_asset_id: quote.source_asset_id,
            dest_chain: quote.dest_chain,
            dest_asset_id: quote.dest_asset_id,
            receive_amount: quote.receive_amount,
            send_amount: quote.send_amount,
            quote_id: Some(quote.quote_id),
            claim_invoice: Some(claim_invoice),
            source_refund_pubkey: Some(
                wallet
                    .source_refund_pubkey(&swap_id)
                    .map_err(wallet_error)?
                    .to_string(),
            ),
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
        let record = self.adopt(&req, started_at).await?;
        Handle::new(self.clone(), &record)
    }

    /// Creates a SwapOut against `quote` paying `claim_address`, or the
    /// destinations hook's address without one. The handle carries the hold
    /// invoice to pay.
    pub async fn swap_out(
        &self,
        quote: Quote,
        claim_address: Option<String>,
    ) -> Result<Handle<W>, DriverError> {
        let chain = self.check_quote(&quote, SwapType::SwapOut)?;
        let claim_address = match (claim_address, &self.destinations) {
            (Some(address), _) => address,
            (None, Some(destinations)) => destinations
                .claim_address(chain)
                .await
                .map_err(DriverError::Destination)?,
            (None, None) => return Err(Refusal::Address("none given".into()).into()),
        };
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
            protocol_version: PROTOCOL_VERSION.into(),
            swap_id,
            user_id: wallet.user_id().map_err(wallet_error)?,
            source_chain: quote.source_chain,
            source_asset_id: quote.source_asset_id,
            dest_chain: quote.dest_chain,
            dest_asset_id: quote.dest_asset_id,
            receive_amount: quote.receive_amount,
            send_amount: quote.send_amount,
            quote_id: Some(quote.quote_id),
            claim_invoice: None,
            source_refund_pubkey: None,
            source_blinding_pubkey: None,
            dest_claim_pubkey: Some(
                wallet
                    .dest_claim_pubkey(&swap_id)
                    .map_err(wallet_error)?
                    .to_string(),
            ),
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
        let record = self.adopt(&req, started_at).await?;
        Handle::new(self.clone(), &record)
    }

    /// The stored swap, as a fresh handle.
    pub async fn get(&self, swap_id: Uuid) -> Result<Option<Handle<W>>, DriverError> {
        self.core
            .store
            .load(&swap_id)
            .await?
            .map(|record| Handle::new(self.clone(), &record))
            .transpose()
    }

    /// Drives every swap in the store that is not quiet until `stop`. Spawn
    /// it. Every pass re-reads the store, so a swap created, restored or
    /// cancelled is driven from the next pass on, and creating one wakes the
    /// loop. A second call while the loop runs, or any call after `stop`,
    /// returns at once.
    pub fn run(&self) -> impl Future<Output = ()> + MaybeSend + 'static {
        let driver = self.clone();
        async move {
            let started = driver.core.state.send_if_modified(|state| match state {
                RunState::Idle => {
                    *state = RunState::Running;
                    true
                }
                _ => false,
            });
            if !started {
                return;
            }
            driver.ensure_listener();
            let mut signals = driver.core.signals.subscribe();
            let mut state = driver.core.state.subscribe();
            while *state.borrow() == RunState::Running {
                driver.sweep().await;
                tokio::select! {
                    _ = rt::sleep(driver.config.poll_interval) => {}
                    _ = signals.recv() => {}
                    _ = state.changed() => {}
                }
            }
            driver.core.state.send_replace(RunState::Stopped);
        }
    }

    /// Ends `run` after the pass in flight, if any; a loop never started is
    /// stopped outright. Final: a stopped driver does not run again, and its
    /// `Updates` end once drained. Handles still `step` and `advance`.
    pub fn stop(&self) {
        self.core.state.send_if_modified(|state| match state {
            RunState::Idle => {
                *state = RunState::Stopped;
                true
            }
            RunState::Running => {
                *state = RunState::Stopping;
                true
            }
            RunState::Stopping | RunState::Stopped => false,
        });
    }

    /// Resolves once `run` has exited and no pass is in flight, so the
    /// store is no longer written to.
    pub async fn wait_stopped(&self) {
        stopped(&mut self.core.state.subscribe()).await;
    }

    /// Where `run` is now.
    pub fn state(&self) -> RunState {
        *self.core.state.borrow()
    }

    /// Every swap a pass saw change, on chain or at the server, from `run`
    /// and from `step`.
    pub fn subscribe(&self) -> Updates<W> {
        Updates {
            updates: self.core.updates.subscribe(),
            state: self.core.state.subscribe(),
        }
    }

    async fn sweep(&self) {
        let Ok(ids) = self.core.store.list().await else {
            return;
        };
        let now = now();
        let window = self.config.refund_timeout.as_secs();
        for id in ids {
            if *self.core.state.borrow() != RunState::Running {
                return;
            }
            let _guard = self.lock(id).await;
            let Ok(Some(mut record)) = self.core.store.load(&id).await else {
                continue;
            };
            if record.is_quiet(now, window, self.destinations.is_some()) {
                continue;
            }
            if let Err(error) = self.tick(&mut record).await {
                self.core.errors.lock().unwrap().insert(id, error);
                self.emit(&record);
            }
        }
    }

    async fn adopt(
        &self,
        req: &CreateSwapRequest,
        started_at: Timestamp,
    ) -> Result<SwapRecord, DriverError> {
        let state = self.core.client.get_swap(req.swap_id).await?;
        match self.accept(req, started_at, state) {
            Ok(record) => {
                self.core.store.save(&record).await?;
                self.signal(record.swap_id);
                Ok(record)
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

    /// The stored record, or `Unknown`.
    async fn load(&self, swap_id: Uuid) -> Result<SwapRecord, DriverError> {
        self.core
            .store
            .load(&swap_id)
            .await?
            .ok_or(DriverError::Unknown(swap_id))
    }

    fn emit(&self, record: &SwapRecord) {
        if let Ok(handle) = Handle::new(self.clone(), record) {
            let _ = self.core.updates.send(handle);
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
                    self.core.state.subscribe(),
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
/// it is down, until the driver is stopped or dropped.
async fn listen<W: SwapWallet + MaybeSend + MaybeSync + 'static>(
    core: Weak<Core<W>>,
    mut source: Box<dyn Signals>,
    retry: Duration,
    mut state: watch::Receiver<RunState>,
) {
    loop {
        let next = tokio::select! {
            next = source.next() => next,
            _ = stopped(&mut state) => return,
        };
        match next {
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
                tokio::select! {
                    _ = rt::sleep(retry) => {}
                    _ = stopped(&mut state) => return,
                }
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

/// A swap bound to its driver: its id and type, and the calls that act on
/// it. It keeps no copy of the swap, so every read is the store's or the
/// driver's newest.
pub struct Handle<W: SwapWallet> {
    driver: Driver<W>,
    swap_id: Uuid,
    swap_type: SwapType,
}

impl<W: SwapWallet> Clone for Handle<W> {
    fn clone(&self) -> Self {
        Self {
            driver: self.driver.clone(),
            swap_id: self.swap_id,
            swap_type: self.swap_type,
        }
    }
}

impl<W: SwapWallet + MaybeSend + MaybeSync + 'static> Handle<W> {
    fn new(driver: Driver<W>, record: &SwapRecord) -> Result<Self, DriverError> {
        let swap_type = match record.swap_type()? {
            swap_type @ (SwapType::SwapIn | SwapType::SwapOut) => swap_type,
            other => return Err(DriverError::Unsupported(other)),
        };
        Ok(Self {
            driver,
            swap_id: record.swap_id,
            swap_type,
        })
    }

    pub fn swap_id(&self) -> Uuid {
        self.swap_id
    }

    pub fn swap_type(&self) -> SwapType {
        self.swap_type
    }

    /// The stored record, as the last pass left it.
    pub async fn record(&self) -> Result<SwapRecord, DriverError> {
        self.driver.load(self.swap_id).await
    }

    pub async fn status(&self) -> Result<Status, DriverError> {
        Ok(self.record().await?.status)
    }

    /// The error of this swap's last failing tick under `run`, `None` after a
    /// working one.
    pub fn error(&self) -> Option<DriverError> {
        self.driver
            .core
            .errors
            .lock()
            .unwrap()
            .get(&self.swap_id)
            .cloned()
    }

    /// SwapIn: the deposit address; `record().send_amount` goes here.
    pub async fn address(&self) -> Result<String, DriverError> {
        self.expect(SwapType::SwapIn)?;
        let record = self.record().await?;
        let terms = record.terms(Side::Source)?;
        let script = self.driver.core.ctx.script(&terms)?;
        Ok(script.address())
    }

    /// SwapOut: the hold invoice to pay; `record().send_amount` is its amount.
    pub async fn claim_invoice(&self) -> Result<String, DriverError> {
        self.expect(SwapType::SwapOut)?;
        let mut record = self.record().await?;
        record
            .source
            .invoice
            .take()
            .ok_or_else(|| record.missing("source invoice"))
    }

    /// One pass, now.
    pub async fn step(&self) -> Result<Status, DriverError> {
        let id = self.swap_id;
        let _guard = self.driver.lock(id).await;
        let mut record = self.driver.load(id).await?;
        self.driver.tick(&mut record).await?;
        Ok(record.status)
    }

    /// Waits for a server notification or `poll_interval`, whichever comes
    /// first, then makes one pass.
    pub async fn advance(&self) -> Result<Status, DriverError> {
        self.driver.wait_for(self.swap_id).await;
        self.step().await
    }

    pub async fn cancel(&self) -> Result<Cancellation, DriverError> {
        let id = self.swap_id;
        let _guard = self.driver.lock(id).await;
        let mut record = self.driver.load(id).await?;
        let open = matches!(
            record.status,
            Status::Restoring | Status::Funding { .. } | Status::Confirming { .. }
        );
        if !open || record.sent.preimage {
            return Ok(Cancellation::TooLate);
        }
        let req = CancelRequest {
            cancel_message: "canceled by the user".into(),
        };
        match self.driver.core.client.cancel(id, &req).await {
            Ok(()) => {
                record.sent.cancel = true;
                self.driver.core.store.save(&record).await?;
                Ok(Cancellation::Requested)
            }
            Err(e) if e.transition_rejected() => Ok(Cancellation::TooLate),
            Err(e) => Err(e.into()),
        }
    }

    /// SwapIn: what under the script is ours, as the last pass left it.
    pub async fn funds(&self) -> Result<Vec<Fund>, DriverError> {
        self.expect(SwapType::SwapIn)?;
        Ok(self.record().await?.funds)
    }

    /// SwapIn: one pass that refunds every `Refundable` fund to
    /// `destination`, returning the refund's txid. `None` when nothing is
    /// refundable yet.
    pub async fn refund(&self, destination: &str) -> Result<Option<String>, DriverError> {
        self.expect(SwapType::SwapIn)?;
        let id = self.swap_id;
        let _guard = self.driver.lock(id).await;
        let mut record = self.driver.load(id).await?;
        self.driver.refund(&mut record, destination).await
    }

    fn expect(&self, swap_type: SwapType) -> Result<(), DriverError> {
        if self.swap_type == swap_type {
            Ok(())
        } else {
            Err(DriverError::WrongSwapType)
        }
    }
}

/// Every swap a pass saw change, on chain or at the server, as `run` and
/// `step` produce them, plus a handle for every failing tick under `run`.
pub struct Updates<W: SwapWallet> {
    updates: broadcast::Receiver<Handle<W>>,
    state: watch::Receiver<RunState>,
}

impl<W: SwapWallet> Updates<W> {
    /// The next changed swap, or `None` once the driver is stopped and
    /// drained, or gone.
    pub async fn next(&mut self) -> Option<Handle<W>> {
        loop {
            tokio::select! {
                biased;
                swap = self.updates.recv() => match swap {
                    Ok(swap) => return Some(swap),
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return None,
                },
                _ = stopped(&mut self.state) => return None,
            }
        }
    }
}
