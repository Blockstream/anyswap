//! Any number of swaps in both directions and on both chains, driven together
//! by one driver.
//!
//! ```text
//! cargo run --example swaps --features esplora -- \
//!     bcrt1q... el1qq... 50000 lnbcrt1... liquid:30000
//! ```
//!
//! Arguments in any order. An address is recognised by its format and used
//! for the swaps of its chain: SwapOut claims pay it, and a SwapIn that ends
//! in `Refund` is swept to it by the driver; give the addresses your swaps
//! need. Every amount is a SwapOut receiving that many sats, everything else
//! is a SwapIn invoice from your node; a `liquid:` prefix swaps on Liquid
//! instead of Bitcoin, with the Liquid genesis and asset discovered from
//! esplora and the server. Pay the deposits and hold invoices by hand, watch
//! the statuses, Ctrl-C when done.

use std::{collections::HashMap, env};

use anyswap_sdk::{
    bitcoin::{self, PrivateKey},
    client::HttpClient,
    context::SwapContext,
    driver::{ChainConfig, Destinations, Driver, MemoryStore, Swap},
    elements,
    esplora::{BitcoinEsplora, LiquidEsplora},
    invoice,
    types::{Chain, NATIVE_ASSET, SwapNetwork},
    wallet::Wallet,
};

/// The addresses given on the command line, one per chain, for both roles.
struct Addresses(HashMap<Chain, String>);

impl Destinations for Addresses {
    fn claim_address(&self, chain: Chain) -> String {
        self.0.get(&chain).cloned().unwrap_or_default()
    }

    fn refund_address(&self, chain: Chain) -> String {
        self.claim_address(chain)
    }
}

const SERVER_URL: &str = "http://localhost:8083";
const BITCOIN_ESPLORA_URL: &str = "http://localhost:8080/regtest/api";
const LIQUID_ESPLORA_URL: &str = "http://localhost:8081/liquidregtest/api";
const NETWORK: SwapNetwork = SwapNetwork::Regtest;

fn address_chain(arg: &str) -> Option<Chain> {
    if arg
        .parse::<bitcoin::Address<bitcoin::address::NetworkUnchecked>>()
        .is_ok()
    {
        return Some(Chain::Bitcoin);
    }
    arg.parse::<elements::Address>().ok().map(|_| Chain::Liquid)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut addresses = HashMap::new();
    let mut swaps = Vec::new();
    for arg in env::args().skip(1) {
        let (chain, spec) = match arg.strip_prefix("liquid:") {
            Some(rest) => (Chain::Liquid, rest.to_owned()),
            None => (Chain::Bitcoin, arg),
        };
        match address_chain(&spec) {
            Some(chain) => {
                addresses.insert(chain, spec);
            }
            None => swaps.push((chain, spec)),
        }
    }
    if swaps.is_empty() {
        anyhow::bail!("usage: swaps [addresses] [liquid:]<sats or bolt11>...");
    }
    for (chain, _) in &swaps {
        if !addresses.contains_key(chain) {
            anyhow::bail!("give a {chain} address for the {chain} swaps");
        }
    }

    let (wallet, _mnemonic) = Wallet::generate(NETWORK)?;
    let identity = PrivateKey::new(wallet.identity_privkey()?, NETWORK.bitcoin_network());
    let client = HttpClient::new(SERVER_URL, Some(identity));
    let liquid_esplora = LiquidEsplora::new(LIQUID_ESPLORA_URL);

    let mut ctx = SwapContext::new(wallet);
    if swaps.iter().any(|(chain, _)| *chain == Chain::Liquid) {
        let genesis_hash = liquid_esplora.get_block_hash(0).await?;
        let info = client.get_info().await?;
        // Taking the asset from the server is a regtest convenience only: a
        // wrong asset id passes every later check, so outside regtest pin the
        // known asset id in configuration instead of asking the counterparty.
        let policy_asset = info
            .policy
            .chain
            .get(&Chain::Liquid)
            .and_then(|policy| policy.asset.get(NATIVE_ASSET))
            .ok_or_else(|| anyhow::anyhow!("the server offers no Liquid asset"))?;
        ctx = ctx.with_liquid(&genesis_hash.to_string(), &policy_asset.asset_id)?;
    }

    let driver = Driver::new(ctx, client.clone(), MemoryStore::default())
        .with_chain(
            Chain::Bitcoin,
            BitcoinEsplora::new(BITCOIN_ESPLORA_URL)?,
            ChainConfig::bitcoin(),
        )
        .with_chain(Chain::Liquid, liquid_esplora, ChainConfig::liquid())
        .with_destinations(Addresses(addresses))
        .with_signals(client.signals());

    for (chain, spec) in swaps {
        if let Ok(receive_amount) = spec.parse::<u64>() {
            let quote = driver.quote(None, Some(chain), receive_amount).await?;
            let swap = driver.swap_out(quote, None).await?;
            println!("pay {}", swap.claim_invoice());
        } else {
            let bolt11 = spec;
            let receive_amount = invoice::amount_msat(&bolt11)? / 1000;
            let quote = driver.quote(Some(chain), None, receive_amount).await?;
            let swap = driver.swap_in(quote, bolt11, None).await?;
            println!(
                "deposit {} sats to {}",
                swap.record().send_amount,
                swap.address()?
            );
        }
    }

    let mut updates = driver.subscribe();
    tokio::spawn(driver.run());
    while let Some(swap) = updates.next().await {
        match swap.error() {
            Some(error) => println!("{} error: {error}", swap.swap_id()),
            None => println!(
                "{} {:?} (server: {})",
                swap.swap_id(),
                swap.status(),
                server_label(&swap)
            ),
        }
    }
    Ok(())
}

/// The server's status name, shown so a swap waiting on the server says whom
/// it waits for.
fn server_label(swap: &Swap<Wallet>) -> &'static str {
    match swap {
        Swap::In(swap) => swap.server_status().map(<&str>::from),
        Swap::Out(swap) => swap.server_status().map(<&str>::from),
    }
    .unwrap_or("unreachable")
}
