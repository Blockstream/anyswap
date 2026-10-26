# anyswap-core

What the AnySwap server and SDKs share: the swap script and the transactions
that spend it on Bitcoin and Liquid, key derivation, MuSig2, invoice parsing,
fee arithmetic, request signing and the server API's types.

An integration wants [`anyswap-sdk`](https://crates.io/crates/anyswap-sdk)
instead, which re-exports what it needs from here and drives swaps to
completion. The [docs](https://blockstream.github.io/anyswap/) cover it, and
the API reference is on [docs.rs](https://docs.rs/anyswap-core).

| Feature   | Default | Adds                                              |
|-----------|---------|---------------------------------------------------|
| `esplora` | no      | Esplora clients for Bitcoin and Liquid            |
| `openapi` | no      | OpenAPI schemas for the API types                 |
