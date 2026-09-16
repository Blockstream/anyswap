use std::sync::atomic::{AtomicBool, Ordering};

use anyswap_core::{api::*, types::SwapNetwork};
use async_trait::async_trait;
use bitcoin::secp256k1::SecretKey;

use super::*;
use crate::{client::ClientError, swap_script::Terms, wallet::Wallet};

struct OfflineServer;

#[async_trait]
impl ServerClient for OfflineServer {
    async fn get_info(&self) -> Result<InfoResponse, ClientError> {
        unreachable!()
    }
    async fn create_swap(&self, _: &CreateSwapRequest) -> Result<(), ClientError> {
        unreachable!()
    }
    async fn get_swap(&self, _: Uuid) -> Result<SwapState, ClientError> {
        Err(ClientError::Transport("offline fixture".into()))
    }
    async fn list_swaps(&self, _: &SwapListQuery) -> Result<SwapListResponse, ClientError> {
        unreachable!()
    }
    async fn cancel(&self, _: Uuid, _: &CancelRequest) -> Result<(), ClientError> {
        unreachable!()
    }
    async fn reveal_claim(&self, _: Uuid, _: &RevealClaimRequest) -> Result<(), ClientError> {
        unreachable!()
    }
    async fn reveal_preimage(&self, _: Uuid, _: &RevealPreimageRequest) -> Result<(), ClientError> {
        unreachable!()
    }
}

#[derive(Default)]
struct TestChain {
    fail: AtomicBool,
}

#[async_trait]
impl ChainClient for TestChain {
    async fn height(&self) -> Result<Height, ChainError> {
        if self.fail.load(Ordering::SeqCst) {
            Err(ChainError("unavailable fixture".into()))
        } else {
            Ok(100)
        }
    }
    async fn fee_rate(&self, _: u32) -> Result<f64, ChainError> {
        Ok(1.0)
    }
    async fn address_history(&self, _: &str) -> Result<Vec<HistoryTx>, ChainError> {
        Ok(vec![])
    }
    async fn broadcast(&self, _: &str) -> Result<String, ChainError> {
        unreachable!()
    }
}

#[derive(Default)]
struct TestStore {
    inner: MemoryStore,
    fail_load: AtomicBool,
    fail_list: AtomicBool,
}

#[async_trait]
impl SwapStore for TestStore {
    async fn save(&self, record: &SwapRecord) -> Result<(), StoreError> {
        self.inner.save(record).await
    }
    async fn load(&self, id: &Uuid) -> Result<Option<SwapRecord>, StoreError> {
        if self.fail_load.load(Ordering::SeqCst) {
            Err(StoreError("load failed".into()))
        } else {
            self.inner.load(id).await
        }
    }
    async fn list(&self) -> Result<Vec<SwapRecord>, StoreError> {
        if self.fail_list.load(Ordering::SeqCst) {
            Err(StoreError("list failed".into()))
        } else {
            self.inner.list().await
        }
    }
}

async fn fixture() -> (Driver<Wallet>, Arc<TestStore>, Arc<TestChain>, SwapRecord) {
    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
        .parse()
        .unwrap();
    let wallet = Wallet::from_mnemonic(&mnemonic, SwapNetwork::Regtest).unwrap();
    let store = Arc::new(TestStore::default());
    let chain = Arc::new(TestChain::default());
    let driver = Driver::new(SwapContext::new(wallet), OfflineServer, store.clone()).with_chain(
        Chain::Bitcoin,
        chain.clone(),
        ChainConfig::bitcoin(),
    );
    let id = Uuid::from_u128(1);
    let started_at = now();
    let record = SwapRecord {
        version: SwapRecord::VERSION,
        swap_id: id,
        payment_hash: [7; 32],
        source: Endpoint::on_chain(
            Terms {
                swap_id: id,
                side: Side::Source,
                chain: Chain::Bitcoin,
                asset: "native".into(),
                csv: 144,
                payment_hash: [7; 32],
                server_pubkey: SecretKey::from_slice(&[21; 32])
                    .unwrap()
                    .public_key(anyswap_core::secp()),
                server_blinding_pubkey: None,
            },
            None,
        ),
        dest: Endpoint::lightning("unused fixture invoice".into()),
        send_amount: 101_000,
        receive_amount: 100_000,
        started_at,
        status: Status::Funding {
            expires_at: started_at + driver.config.funding_timeout.as_secs(),
            cancel_sent: false,
        },
        committed_at: None,
        preimage: None,
        sent: Sent::default(),
    };
    store.save(&record).await.unwrap();
    (driver, store, chain, record)
}

#[tokio::test]
async fn reports_recovery_without_a_record_change() {
    let (driver, store, chain, record) = fixture().await;
    let mut updates = driver.subscribe();
    chain.fail.store(true, Ordering::SeqCst);
    let _ = driver.sweep().await;
    assert!(matches!(
        updates.0.try_recv().unwrap().error(),
        Some(DriverError::Chain(_))
    ));

    chain.fail.store(false, Ordering::SeqCst);
    let _ = driver.sweep().await;
    let recovered = updates.0.try_recv().expect("missing recovery update");
    assert!(recovered.error().is_none());
    assert_eq!(recovered.record(), &record);
    assert_eq!(store.load(&record.swap_id).await.unwrap(), Some(record));

    let _ = driver.sweep().await;
    assert!(matches!(
        updates.0.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn reports_load_errors_and_recovery() {
    let (driver, store, _, record) = fixture().await;
    let mut updates = driver.subscribe();
    store.fail_load.store(true, Ordering::SeqCst);
    let _ = driver.sweep().await;
    let failed = updates.0.try_recv().expect("missing load error update");
    assert!(matches!(failed.error(), Some(DriverError::Store(_))));
    assert_eq!(failed.swap_id(), record.swap_id);

    store.fail_load.store(false, Ordering::SeqCst);
    let _ = driver.sweep().await;
    assert!(
        updates
            .0
            .try_recv()
            .expect("missing recovery update")
            .error()
            .is_none()
    );
}

#[tokio::test]
async fn propagates_list_errors_and_can_retry() {
    let (driver, store, _, record) = fixture().await;
    store.fail_list.store(true, Ordering::SeqCst);
    assert_eq!(
        driver.sweep().await.unwrap_err().to_string(),
        "store: list failed"
    );
    store.fail_list.store(false, Ordering::SeqCst);
    driver.sweep().await.unwrap();
    assert_eq!(store.load(&record.swap_id).await.unwrap(), Some(record));
}
