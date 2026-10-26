# AnySwap Rust SDK

`anyswap-sdk` is the Rust client for AnySwap: swaps between Lightning and
on-chain Bitcoin or Liquid. Its centrepiece is a driver that runs any number
of swaps to completion and verifies everything against the chain, so an
integration creates swaps and watches statuses rather than implementing the
protocol.

## Install

```toml
[dependencies]
anyswap-sdk = { version = "1.*", features = ["esplora"] }
```

| Feature   | Default | Adds                                                          |
|-----------|---------|---------------------------------------------------------------|
| `client`  | yes     | `HttpClient`, the server API over HTTP                        |
| `ws`      | yes     | The WebSocket subscription that paces the driver; native only |
| `driver`  | yes     | The swap driver                                               |
| `esplora` | no      | `BitcoinEsplora` and `LiquidEsplora` as ready chain backends  |

## Where to go next

1. [Quickstart][quickstart] for one swap, from setup to `Completed`.
2. [Swaps][swaps] for the two flows, the statuses they move through, and
   cancels.
3. [Persistence and recovery][recovery] for the store, restarts, and
   rebuilding swaps from the mnemonic alone.
4. [Liquid][liquid] for the two extra values a Liquid integration needs.
5. [Compared with Boltz][boltz] if you come from `boltz-client` or
   `lwk_boltz`.
6. [docs.rs][api] for the API reference.

A runnable end-to-end program lives at [`examples/swaps.rs`][example]: it
drives any mix of SwapIns and SwapOuts on both chains against a server.
[`examples/advance.rs`][advance] is the other shape: one swap
driven by hand to its outcome over a store on disk, resumable after a
restart.

[quickstart]: https://blockstream.github.io/anyswap/quickstart/?lang=rust
[swaps]: https://blockstream.github.io/anyswap/swaps/?lang=rust
[recovery]: https://blockstream.github.io/anyswap/recovery/?lang=rust
[liquid]: https://blockstream.github.io/anyswap/liquid/?lang=rust
[boltz]: https://blockstream.github.io/anyswap/boltz/?lang=rust
[api]: https://docs.rs/anyswap-sdk
[example]: https://github.com/Blockstream/anyswap/blob/main/crates/sdk/examples/swaps.rs
[advance]: https://github.com/Blockstream/anyswap/blob/main/crates/sdk/examples/advance.rs
