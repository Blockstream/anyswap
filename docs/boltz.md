# Boltz: comparison and migration

The nearest neighbours of `anyswap-sdk` are the Rust clients of Boltz:
[`boltz-client`](https://github.com/SatoshiPortal/boltz-rust) (boltz-rust,
by Bull Bitcoin) and [`lwk_boltz`](https://github.com/Blockstream/lwk)
(part of Blockstream's Liquid Wallet Kit, built on `boltz-client`). They
are clients of the Boltz Exchange, while `anyswap-sdk` is the client of
AnySwap. The protocols are close relatives: both lock funds in
a taproot script spendable by preimage, cooperatively by MuSig2 keypath,
or by the owner after a timeout, and the flows correspond one to one, a
submarine swap is a SwapIn, a reverse swap a SwapOut. This page compares
the three and maps a Boltz integration onto `anyswap-sdk`, as of
`boltz-client` 0.4.1 and `lwk_boltz` 0.18.

## How they differ

`boltz-client` is a toolkit: it builds correct scripts, transactions and
API calls, and your code decides when each step happens and keeps the
state. `lwk_boltz` wraps it in a session that mints per-swap state
machines the integrator persists and drives with `advance` until each
completes. In `anyswap-sdk` the whole protocol sits behind the driver:
one loop runs every stored swap, claims and refunds included, and your
code creates swaps and renders updates, with `step` on a handle when you
want the per-swap control back.

All three verify the counterparty's script before funds move. They part
on what drives a swap after that. The Boltz state machines advance on the
server's WebSocket events; the `anyswap-sdk` driver re-derives the status
from the script's chain history on every pass, shows the server's report
as a label, and counts deadlines on its own clock. A stalled, lying or
missed event cannot wedge a swap into a wrong state, and a client that
was offline reconstructs where it stands from the chain alone.

The same design makes crashes boring. A record stores only what the chain
cannot report, the messages already delivered and the secrets the server
revealed; everything else is recomputed each pass, so the process can die
at any point and the next pass just re-observes. Restart is rebuilding
the driver over the store, where in `lwk_boltz` each swap is a persisted
state machine to resume with `restore_prepare_pay` or `restore_invoice`
and feed events again. A client that lost its store follows the same
line: `lwk_boltz` posts the session xpub to Boltz to get its swaps back,
while an AnySwap server returns your own signed create request and the
SDK verifies it locally, so a server can hide a swap but not alter one.

Last, the driver treats the swap script as a wallet rather than one
expected coin. boltz-rust documents its assumption plainly: one UTXO of
exactly the swap amount, and it does not claim otherwise. The
`anyswap-sdk` driver sweeps everything under the script that is worth its
fee, a wrong-amount deposit, a second one, a stray, through the keypath
when the server cooperates and the CSV path when it does not.

## The map

| With Boltz                                                    | With `anyswap-sdk`                                           |
|---------------------------------------------------------------|--------------------------------------------------------------|
| `BoltzApiClientV2` / `BoltzSession`                           | `HttpClient` and `Driver`                                    |
| Pair info and limits                                          | `driver.quote(source, dest, receive_amount)`             |
| A keypair and preimage you create and rotate per swap         | derived by `Wallet` from the mnemonic and the swap's id, internal to the driver |
| `next_index_to_use`, the key counter you persist              | nothing, keys need no counter                                |
| WebSocket status events driving your state machine            | `Status` resolved from the chain every pass; the server's label is `server_status()`, display only |
| `advance` / `complete_pay` loops per swap (`lwk_boltz`)       | one `driver.run()` for every stored swap                     |
| `BtcSwapTx` / `LBtcSwapTx` claim and refund building          | the driver claims and refunds itself                         |
| `DynStore` encrypted key-value store                          | `SwapStore`, three methods over serde records                |
| `swap_restore` with your xpub, the rescue file                | `restore_all`, verified locally                              |

## What you delete

Coming from `boltz-client`: the protocol steps themselves. Creating the
keys and the preimage, verifying the script, watching the address,
building and broadcasting the claim or refund, reacting to status events,
all of it is the driver's job in `anyswap-sdk`. Coming from `lwk_boltz`
the per-swap work is comparable, and what goes away is the fleet: the
loop per swap, and the restore-and-respawn wiring a restart needs, since
`run` drives every stored swap from one call. Either way the integration
becomes: create a swap, spawn `run` once, render what `subscribe`
yields. The [overview](overview.md) shows the whole shape in
two snippets. There is nothing to port your Boltz event handlers to; map
your UI to the [driver statuses](swaps.md#statuses) instead.

## What you write

- A `SwapStore` implementation over your database: `save`, `load`, `list`.
  Records carry the swap terms and the server's revealed secrets, never
  your own keys, so encryption is your policy rather than a requirement.
- The setup: `Wallet` from a mnemonic, `HttpClient` with the identity key,
  `with_chain` per chain. On Liquid the context needs the genesis hash and
  policy asset from your own sources, see [Liquid](liquid.md); the
  blinding itself, which `boltz-client` hands you, is internal to the
  driver.
- If your chain data comes from Electrum: `anyswap-sdk` bundles only
  Esplora backends, so implement the four-method `ChainClient` trait over
  your Electrum connection.

## What stays yours

The Lightning node and the money's endpoints, exactly as with Boltz: mint
the invoice a SwapIn pays out to, pay the hold invoice of a SwapOut, and
provide the on-chain addresses (a claim address per SwapOut and a refund
address per SwapIn, or a `Destinations` behind `with_destinations` for
both). Neither SDK wallet holds funds;
the `lwk_wollet` a `lwk_boltz` integration claims into is simply where
those addresses come from, and any wallet serves.
