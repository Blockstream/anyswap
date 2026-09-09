# AnySwap Rust SDK

`anyswap-sdk` is the Rust client for AnySwap: swaps between Lightning and
on-chain Bitcoin or Liquid. Its centrepiece is a driver that runs any number
of swaps to completion and verifies everything against the chain, so an
integration creates swaps and watches statuses rather than implementing the
protocol.

## Install

```toml
[dependencies]
anyswap-sdk = { version = "1.2.0-rc.2", features = ["esplora"] }
```

| Feature   | Default | Adds                                                          |
|-----------|---------|---------------------------------------------------------------|
| `client`  | yes     | `HttpClient`, the server API over HTTP                        |
| `ws`      | yes     | The WebSocket subscription that paces the driver; native only |
| `driver`  | yes     | The swap driver                                               |
| `esplora` | no      | `BitcoinEsplora` and `LiquidEsplora` as ready chain backends  |

## Where to go next

1. [Integration overview][overview] for what the driver does, what it
   trusts, and the setup every integration shares.
2. [Running swaps][swaps] for the two flows, the statuses they move
   through, cancels, refunds, and the configuration knobs.
3. [Persistence and recovery][recovery] for the store, restarts, and
   rebuilding swaps from the mnemonic alone.
4. [Liquid][liquid] for the two extra values a Liquid integration needs.
5. [Boltz: comparison and migration][boltz] if you come from
   `boltz-client` or `lwk_boltz`.
6. `cargo doc -p anyswap-sdk --open` for the API reference.

A runnable end-to-end program lives at [`examples/swaps.rs`][example]: it
drives any mix of SwapIns and SwapOuts on both chains against a regtest
deployment. [`examples/advance.rs`][advance] is the other shape: one swap
driven by hand to its outcome over a store on disk, resumable after a
restart.

[overview]: https://github.com/Blockstream/anyswap/blob/main/docs/overview.md
[swaps]: https://github.com/Blockstream/anyswap/blob/main/docs/swaps.md
[recovery]: https://github.com/Blockstream/anyswap/blob/main/docs/recovery.md
[liquid]: https://github.com/Blockstream/anyswap/blob/main/docs/liquid.md
[boltz]: https://github.com/Blockstream/anyswap/blob/main/docs/boltz.md
[example]: https://github.com/Blockstream/anyswap/blob/main/crates/sdk/examples/swaps.rs
[advance]: https://github.com/Blockstream/anyswap/blob/main/crates/sdk/examples/advance.rs
