# Integration overview

## What AnySwap does

AnySwap moves value between Lightning and on-chain Bitcoin or Liquid through
a single trust-minimised swap:

| Flow        | User starts with   | User ends with     |
|-------------|--------------------|--------------------|
| **SwapIn**  | On-chain BTC/L-BTC | Lightning balance  |
| **SwapOut** | Lightning balance  | On-chain BTC/L-BTC |

The funds sit in a taproot script with three spending paths: a preimage path
that completes the swap, a cooperative keypath for an early unlock when both
sides agree, and a CSV refund path the owner can use alone after a timeout.
The user never trusts the service with their money; the worst case is
waiting out the CSV.

## What the driver does

The protocol lives inside `Driver`. Per swap it:

- checks the server's copy of the terms at creation and freezes them,
- watches the swap script's address and interprets the history itself,
- sends the protocol messages when their moment comes,
- claims a SwapOut and refunds a SwapIn through the cheapest path available,
- saves every change to your store, so a restart resumes where it left off.

Your side of the contract is small: mint or pay an invoice, show statuses,
and give the driver somewhere durable to keep its records.

## What the driver trusts

Only what it can verify. The terms are checked against your own request at
creation and frozen; the server's copy of them is never read again. From
each later poll the driver takes three things, each on its own merit: the
status label (shown to you, never acted on), the server's key (verified
against the pubkey in the terms), and the preimage (verified against the
payment hash). Wall-clock deadlines count on the driver's own clock, block
deadlines from confirmation heights it read itself. A server can delay a
swap, but not redirect one.

## Setup

Everything long-lived is built once and handed to the driver:

| Object        | Role                                                              |
|---------------|-------------------------------------------------------------------|
| `Wallet`      | Key derivation from a BIP39 mnemonic, local                       |
| `SwapContext` | The wallet plus chain parameters (Liquid needs two, see [Liquid](liquid.md)) |
| `ServerClient` | The server API as the driver calls it; `HttpClient` implements it, request signing included |
| `ChainClient` | Chain reads and broadcasts, one per chain; `BitcoinEsplora` and `LiquidEsplora` implement it |
| `SwapStore`   | Durable swap records, implemented by you (see [Persistence](recovery.md)) |
| `Signals`     | Optional push channel that paces the loops; `client.signals()` is the server's WebSocket, and `driver.signal(id)` feeds one by hand |

The traits are `Send + Sync` except on wasm32, where the `MaybeSend` and
`MaybeSync` supertraits are empty and an implementation's attribute is
`#[async_trait(?Send)]`.

```rust
use anyswap_sdk::{
    bitcoin::PrivateKey,
    client::HttpClient,
    context::SwapContext,
    driver::{ChainConfig, Driver, MemoryStore},
    esplora::BitcoinEsplora,
    types::{Chain, SwapNetwork},
    wallet::Wallet,
};

const NETWORK: SwapNetwork = SwapNetwork::Regtest;

let (wallet, mnemonic) = Wallet::generate(NETWORK)?;
let identity = PrivateKey::new(wallet.identity_privkey()?, NETWORK.bitcoin_network());
let client = HttpClient::new("http://localhost:8083", Some(identity));

let driver = Driver::new(SwapContext::new(wallet), client.clone(), MemoryStore::default())
    .with_chain(
        Chain::Bitcoin,
        BitcoinEsplora::new("http://localhost:8080/regtest/api")?,
        ChainConfig::bitcoin(),
    )
    .with_signals(client.signals());
```

`MemoryStore` is for tests and short-lived tools; a real integration passes
its own `SwapStore`. A chain without a `with_chain` registration cannot be
swapped on. `with_signals` is optional: without it the loops pace on
`poll_interval` alone. The driver is cheap to clone and every clone shares
the store, the client, and the update stream.

## The integration shape

Create swaps, spawn the loop, watch the updates:

```rust
let quote = driver.quote(None, Some(Chain::Bitcoin), 50_000).await?;
let swap = driver.swap_out(quote, Some(claim_address)).await?;
println!("pay {}", swap.claim_invoice());

let mut updates = driver.subscribe();
tokio::spawn(driver.run());
while let Some(swap) = updates.next().await {
    match swap.error() {
        Some(error) => eprintln!("{} error: {error}", swap.swap_id()),
        None => println!("{} {:?}", swap.swap_id(), swap.status()),
    }
}
```

`run` drives every swap in the store that still has something to do,
forever. The `Signals` channel from setup is pacing only: a signal means a
pass is worth making sooner than `poll_interval`, and every decision still
comes from a fresh read.

## Updates and errors

`subscribe` yields a handle for every swap a pass changed, plus one for
every failing pass. A failing swap stays in the set and is retried each
pass, so a broken backend shows up as a heartbeat of updates with
`swap.error()` set; a pass that works clears it. `DriverError::retryable`
separates a chain or server outage, which the next pass may simply outlive,
from an error that needs attention. The stream drops updates under
sustained lag rather than blocking the driver, so treat it as a prompt to
render, not as a ledger; `driver.get(swap_id)` returns the current state of
any stored swap.

## Driving one swap by hand

Handles can pace the loop themselves, without `run`:

```rust
let mut swap = driver.swap_out(quote, Some(claim_address)).await?;
while !swap.status().is_terminal() {
    let status = swap.advance().await?;
    println!("{status:?}");
}
```

`step` makes one pass now, `advance` first waits for a server signal or
`poll_interval`, whichever comes first. A failed pass returns the error and
moves nothing; calling again retries. `run` and handles can be mixed freely,
a per-swap lock keeps passes from overlapping.

## Where to go next

| If you want to...                                   | Read                              |
|-----------------------------------------------------|-----------------------------------|
| The per-flow walkthroughs and every status          | [Running swaps](swaps.md)         |
| Survive restarts, or rebuild from the mnemonic      | [Persistence and recovery](recovery.md) |
| Swap on Liquid                                      | [Liquid](liquid.md)               |
| Look up a type or method                            | `cargo doc -p anyswap-sdk --open` |
