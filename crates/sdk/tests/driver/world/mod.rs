//! A driver over a chain and a server in memory, on tokio's paused time.

mod chain;
mod lightning;
mod server;

use std::{sync::Arc, time::Duration};

use anyswap_sdk::{
    bitcoin::{
        Address, CompressedPublicKey, Network,
        secp256k1::{Keypair, Secp256k1, SecretKey},
    },
    context::SwapContext,
    driver::{
        ChainConfig, Clock, Config, Driver, DriverError, Handle, Height, MemoryStore, Status,
        TargetAmount, Timestamp,
    },
    types::{Chain, SwapNetwork},
    wallet::Wallet,
};
use bip39::Mnemonic;
use tokio::{task::JoinHandle, time::Instant};
use uuid::Uuid;

pub use self::{
    chain::FakeChain,
    lightning::Lightning,
    server::{Behavior, FakeServer, Reach},
};

pub const EPOCH: Timestamp = 1_700_000_000;
pub const TIP: Height = 1_000;
pub const FEE_RATE: f64 = 2.0;

/// The driver's clock on tokio's paused time, so `World::advance` moves its
/// deadlines, `poll_interval` and `server_timeout` together.
#[derive(Clone)]
pub struct TestClock {
    start: Instant,
}

impl Clock for TestClock {
    fn now(&self) -> Timestamp {
        EPOCH + self.start.elapsed().as_secs()
    }
}

pub struct World {
    pub chain: Arc<FakeChain>,
    pub server: Arc<FakeServer>,
    pub lightning: Arc<Lightning>,
    pub config: Config,
    pub chain_config: ChainConfig,
    clock: TestClock,
    ctx: SwapContext<Wallet>,
    store: Arc<MemoryStore>,
    driver: Driver<Wallet>,
}

impl World {
    /// Regtest Bitcoin under `ChainConfig::bitcoin()` and `Config::default()`,
    /// with the fee estimate read every pass, and an honest server.
    pub fn bitcoin() -> Self {
        let config = Config {
            fee_rate_cache_ttl: Duration::ZERO,
            ..Config::default()
        };
        Self::with(config, ChainConfig::bitcoin())
    }

    pub fn with(config: Config, chain_config: ChainConfig) -> Self {
        let mnemonic = Mnemonic::from_entropy(&[7; 16]).unwrap();
        let wallet = Wallet::from_mnemonic(&mnemonic, SwapNetwork::Regtest).unwrap();
        let identity =
            Keypair::from_secret_key(&Secp256k1::new(), &wallet.identity_privkey().unwrap());
        let clock = TestClock {
            start: Instant::now(),
        };
        let chain = Arc::new(FakeChain::new(TIP, FEE_RATE));
        let lightning = Arc::new(Lightning::default());
        let server = Arc::new(FakeServer::new(
            chain.clone(),
            lightning.clone(),
            clock.clone(),
            identity,
            chain_config,
        ));
        let ctx = SwapContext::new(wallet);
        let store = Arc::new(MemoryStore::default());
        let driver = driver(&ctx, &server, &chain, &store, &clock, &config, chain_config);
        Self {
            chain,
            server,
            lightning,
            config,
            chain_config,
            clock,
            ctx,
            store,
            driver,
        }
    }

    /// A SwapIn for `receive_amount` with no refund address, its claim
    /// invoice minted by `lightning` to expire when the server times out.
    pub async fn swap_in(&self, receive_amount: u64) -> Handle<Wallet> {
        self.try_swap_in(receive_amount, server::AWAIT_OPENING_TX)
            .await
            .unwrap()
    }

    /// Creates a SwapIn as `swap_in` does, its claim invoice expiring
    /// `expiry` seconds from now.
    pub async fn try_swap_in(
        &self,
        receive_amount: u64,
        expiry: u64,
    ) -> Result<Handle<Wallet>, DriverError> {
        let quote = self
            .driver
            .quote(
                Some(Chain::Bitcoin),
                None,
                TargetAmount::Receive(receive_amount),
            )
            .await
            .unwrap();
        let invoice = self.lightning.invoice(receive_amount, self.now(), expiry);
        self.driver.swap_in(quote, invoice, None).await
    }

    /// A SwapOut for `receive_amount` claiming to `user_address()`.
    pub async fn swap_out(&self, receive_amount: u64) -> Handle<Wallet> {
        self.try_swap_out(receive_amount).await.unwrap()
    }

    /// Creates a SwapOut as `swap_out` does.
    pub async fn try_swap_out(&self, receive_amount: u64) -> Result<Handle<Wallet>, DriverError> {
        let quote = self
            .driver
            .quote(
                None,
                Some(Chain::Bitcoin),
                TargetAmount::Receive(receive_amount),
            )
            .await
            .unwrap();
        self.driver.swap_out(quote, Some(user_address())).await
    }

    /// SwapIn: pays `send_amount` to the swap address, into the mempool.
    pub async fn deposit(&self, swap: &Handle<Wallet>) -> String {
        let address = swap.address().await.unwrap();
        let amount = swap.record().await.unwrap().send_amount;
        self.chain.pay(&address, amount)
    }

    /// SwapOut: pays the hold invoice, so the server holds the HTLC.
    pub fn pay(&self, swap: &Handle<Wallet>) {
        self.server.hold(swap.swap_id());
    }

    /// Signals `swap_id` every millisecond, as a server flooding the push
    /// channel would, until the task is aborted.
    pub fn flood(&self, swap_id: Uuid) -> JoinHandle<()> {
        let driver = self.driver.clone();
        tokio::spawn(async move {
            loop {
                driver.signal(swap_id);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
    }

    /// A new driver over an empty store, with the same wallet. Handles taken
    /// before keep the old driver.
    pub fn lose_store(&mut self) {
        self.store = Arc::new(MemoryStore::default());
        self.driver = self.driver(&self.store);
    }

    pub async fn restore(&self, swap_id: Uuid) -> Handle<Wallet> {
        self.driver.restore(swap_id).await.unwrap()
    }

    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    pub async fn advance(&self, by: Duration) {
        tokio::time::advance(by).await;
    }

    /// Steps the swap, then mines a block and lets a block time pass, until
    /// its status is final or twice the longer CSV has been mined. Returns
    /// the last status.
    pub async fn finish(&self, swap: &Handle<Wallet>) -> Status {
        let limit = 2 * self.server.source_csv().max(self.server.dest_csv());
        let block_time = Duration::from_secs(self.chain_config.block_time);
        for _ in 0..limit {
            let _ = swap.step().await;
            let status = swap.status().await.unwrap();
            if status.is_terminal() {
                return status;
            }
            self.chain.mine(1);
            self.advance(block_time).await;
        }
        swap.status().await.unwrap()
    }

    /// One sweep of `Driver::run`, `is_quiet` included. A stopped driver
    /// never runs again, so each sweep is a fresh driver over the store. A
    /// panic in the sweep, such as a failed check in the fake server, fails
    /// the test.
    pub async fn sweep(&self) {
        let driver = self.driver(&self.store);
        let run = tokio::spawn(driver.run());
        tokio::time::sleep(Duration::from_millis(1)).await;
        driver.stop();
        run.await.unwrap();
    }

    fn driver(&self, store: &Arc<MemoryStore>) -> Driver<Wallet> {
        driver(
            &self.ctx,
            &self.server,
            &self.chain,
            store,
            &self.clock,
            &self.config,
            self.chain_config,
        )
    }
}

fn driver(
    ctx: &SwapContext<Wallet>,
    server: &Arc<FakeServer>,
    chain: &Arc<FakeChain>,
    store: &Arc<MemoryStore>,
    clock: &TestClock,
    config: &Config,
    chain_config: ChainConfig,
) -> Driver<Wallet> {
    Driver::new(ctx.clone(), server.clone(), store.clone())
        .with_chain(Chain::Bitcoin, chain.clone(), chain_config)
        .unwrap()
        .with_config(config.clone())
        .with_clock(clock.clone())
}

/// The user's own address, where claims and refunds go.
pub fn user_address() -> String {
    let key = SecretKey::from_slice(&[9; 32]).unwrap();
    let pubkey = CompressedPublicKey(key.public_key(&Secp256k1::new()));
    Address::p2wpkh(&pubkey, Network::Regtest).to_string()
}
