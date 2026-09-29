# Changelog

## [1.2.0]

- **Breaking:** `POST /v1/swap` takes `send_amount` instead of `swap_fee_limit`
  and accepts only a `send_amount` that buys exactly `receive_amount`.
  `swap_fee_limit` is gone from the swap state and the SDKs.
- **Breaking:** the SDK driver's `quote` pins either the amount to receive or
  the amount to send. The driver cannot restore swaps created before this
  release.

### Minor

- The driver no longer fails a step when resending an observed, unconfirmed
  spend is refused, as when the chain source lags a block behind the node.
  The status stays `Spending` with the observed txid.
- `LiquidEsplora` keeps the transaction bodies it has fetched, so reading an
  address history costs one request per pass once its transactions are known,
  instead of one more per transaction.

## [1.2.0-rc.2]

- First release on crates.io. Both crates declare `rust-version = "1.85"`,
  checked in CI with every feature on.

## [1.2.0-rc.1]

### Core

- Swap scripts, transactions and MuSig2 signing for Bitcoin and Liquid, fee
  estimation, and the server API types. Esplora chain backends sit behind the
  `esplora` feature, OpenAPI derives behind `openapi`.

### SDK

- `HttpClient` for the server API (`client`), a WebSocket subscription that
  paces the driver (`ws`, native only), and the swap driver (`driver`) that runs
  any number of swaps to completion, verifying statuses against the chain rather
  than the server's reports. `esplora` adds `BitcoinEsplora` and `LiquidEsplora`.
  Builds for wasm32.
