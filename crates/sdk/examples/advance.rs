//! One swap, driven by hand to its outcome, over records that survive a
//! restart: the shape of an `lwk_boltz` session, with the store and the loop
//! an integrator writes.
//!
//! ```text
//! cargo run --example advance --features esplora -- in <bolt11> <refund address>
//! cargo run --example advance --features esplora -- out <sats> <claim address>
//! cargo run --example advance --features esplora -- resume <swap id>
//! cargo run --example advance --features esplora -- list
//! ```
//!
//! `in` creates a SwapIn paying the invoice and prints the deposit to make,
//! `out` a SwapOut of that many sats to the address and prints the hold
//! invoice to pay; both then loop over `advance` until the swap settles. The
//! mnemonic and one JSON file per record live under `./anyswap-advance`, so
//! `resume` continues a swap after a restart, and `list` shows every one on
//! file. Bitcoin regtest; see `swaps.rs` for the Liquid setup.

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyswap_sdk::{
    bitcoin::PrivateKey,
    client::HttpClient,
    context::SwapContext,
    driver::{ChainConfig, Driver, Handle, Status, StoreError, SwapRecord, SwapStore},
    esplora::BitcoinEsplora,
    invoice,
    types::{Chain, SwapNetwork},
    wallet::Wallet,
};
use async_trait::async_trait;
use bip39::Mnemonic;
use uuid::Uuid;

const SERVER_URL: &str = "http://localhost:8083";
const ESPLORA_URL: &str = "http://localhost:8080/regtest/api";
const NETWORK: SwapNetwork = SwapNetwork::Regtest;
const DIR: &str = "anyswap-advance";

/// One JSON file per record, the record's serde form unchanged, written whole
/// and renamed into place so a crash mid-write leaves the old record.
struct FileStore(PathBuf);

impl FileStore {
    fn path(&self, swap_id: &Uuid) -> PathBuf {
        self.0.join(format!("{swap_id}.json"))
    }
}

fn store_error(e: impl std::fmt::Display) -> StoreError {
    StoreError(e.to_string())
}

#[async_trait]
impl SwapStore for FileStore {
    async fn save(&self, record: &SwapRecord) -> Result<(), StoreError> {
        let json = serde_json::to_vec_pretty(record).map_err(store_error)?;
        let path = self.path(&record.swap_id);
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, json).map_err(store_error)?;
        fs::rename(tmp, path).map_err(store_error)
    }

    async fn load(&self, swap_id: &Uuid) -> Result<Option<SwapRecord>, StoreError> {
        match fs::read(self.path(swap_id)) {
            Ok(json) => serde_json::from_slice(&json).map(Some).map_err(store_error),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(store_error(e)),
        }
    }

    async fn list(&self) -> Result<Vec<SwapRecord>, StoreError> {
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.0).map_err(store_error)? {
            let path = entry.map_err(store_error)?.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let json = fs::read(&path).map_err(store_error)?;
                records.push(serde_json::from_slice(&json).map_err(store_error)?);
            }
        }
        Ok(records)
    }
}

/// The mnemonic on file, made on the first run. Regtest only: a real
/// integration keeps it where it keeps the wallet's.
fn mnemonic(dir: &Path) -> anyhow::Result<Mnemonic> {
    let path = dir.join("mnemonic");
    if let Ok(words) = fs::read_to_string(&path) {
        return Ok(words.trim().parse()?);
    }
    let mnemonic = Wallet::generate_mnemonic(12)?;
    fs::write(&path, mnemonic.to_string())?;
    Ok(mnemonic)
}

fn setup(dir: PathBuf) -> anyhow::Result<(Driver<Wallet>, Arc<FileStore>)> {
    fs::create_dir_all(&dir)?;
    let wallet = Wallet::from_mnemonic(&mnemonic(&dir)?, NETWORK)?;
    let identity = PrivateKey::new(wallet.identity_privkey()?, NETWORK.bitcoin_network());
    let client = HttpClient::new(SERVER_URL, Some(identity));
    let store = Arc::new(FileStore(dir));
    let driver = Driver::new(SwapContext::new(wallet), client.clone(), store.clone())
        .with_chain(
            Chain::Bitcoin,
            BitcoinEsplora::new(ESPLORA_URL)?,
            ChainConfig::bitcoin(),
        )
        .with_signals(client.signals());
    Ok((driver, store))
}

/// `advance` until the swap settles: a terminal status, or a `Refund` with
/// nothing left to take and every sweep confirmed. A chain or server outage
/// is waited out, since `advance` paces itself.
async fn complete(swap: &mut Handle<Wallet>) -> anyhow::Result<Status> {
    loop {
        match swap.advance().await {
            Ok(status) => {
                println!("{status:?}");
                if settled(&status) {
                    return Ok(status);
                }
            }
            Err(e) if e.retryable() => println!("retrying: {e}"),
            Err(e) => return Err(e.into()),
        }
    }
}

fn settled(status: &Status) -> bool {
    match status {
        Status::Refund {
            refundable,
            refunded,
            ..
        } => *refundable == 0 && refunded.iter().all(|(_, confirmed)| *confirmed),
        _ => status.is_terminal(),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (driver, store) = setup(PathBuf::from(DIR))?;
    match args[..] {
        ["in", bolt11, refund_address] => {
            let receive_amount = invoice::amount_msat(bolt11)? / 1000;
            let quote = driver
                .quote(Some(Chain::Bitcoin), None, receive_amount)
                .await?;
            let mut swap = driver
                .swap_in(quote, bolt11.to_owned(), Some(refund_address.to_owned()))
                .await?;
            println!(
                "{}: deposit {} sats to {}",
                swap.swap_id(),
                swap.record().send_amount,
                swap.address()?
            );
            complete(&mut swap).await?;
        }
        ["out", sats, claim_address] => {
            let quote = driver
                .quote(None, Some(Chain::Bitcoin), sats.parse()?)
                .await?;
            let mut swap = driver
                .swap_out(quote, Some(claim_address.to_owned()))
                .await?;
            println!("{}: pay {}", swap.swap_id(), swap.claim_invoice());
            complete(&mut swap).await?;
        }
        ["resume", swap_id] => {
            let mut swap = driver
                .get(swap_id.parse()?)
                .await?
                .ok_or_else(|| anyhow::anyhow!("no swap {swap_id} on file"))?;
            complete(&mut swap).await?;
        }
        ["list"] => {
            for record in store.list().await? {
                println!(
                    "{} {} {:?}",
                    record.swap_id,
                    record.swap_type()?,
                    record.status
                );
            }
        }
        _ => anyhow::bail!(
            "usage: advance in <bolt11> <refund address> | out <sats> <claim address> | resume <swap id> | list"
        ),
    }
    Ok(())
}
