# Changelog

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
