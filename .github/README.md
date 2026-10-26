# AnySwap SDKs

Clients for AnySwap: swaps between Lightning and on-chain Bitcoin or Liquid.
Each SDK carries a driver that runs any number of swaps to completion and
verifies everything against the chain, so an integration creates swaps and
watches statuses rather than implementing the protocol.

Documentation: <https://blockstream.github.io/anyswap/>

| Package | Language | Source | Docs |
|---|---|---|---|
| [`anyswap-sdk`](https://crates.io/crates/anyswap-sdk) | Rust | [`crates/sdk`](https://github.com/Blockstream/anyswap/tree/main/crates/sdk) | [guide](https://blockstream.github.io/anyswap/overview/?lang=rust), [API](https://docs.rs/anyswap-sdk) |
| [`@blockstream/anyswap-sdk`](https://www.npmjs.com/package/@blockstream/anyswap-sdk) | TypeScript | [`ts-sdk`](https://github.com/Blockstream/anyswap/tree/main/ts-sdk) | [guide](https://blockstream.github.io/anyswap/overview/?lang=ts), [API](https://blockstream.github.io/anyswap/typescript/reference/) |

`anyswap-core`, the shared on-chain primitives and API types, and
`@blockstream/anyswap-wasm`, the TypeScript SDK's bindings to the Rust driver
in `crates/sdk-wasm`, are published for the SDKs above. `api.yaml` describes
the server API they speak. The docs site is built from
[`sdk-docs`](https://github.com/Blockstream/anyswap/tree/main/sdk-docs).

Each [release](https://github.com/Blockstream/anyswap/releases) lists what
changed in both SDKs; their changelogs are
[`crates/sdk/CHANGELOG.md`](https://github.com/Blockstream/anyswap/blob/main/crates/sdk/CHANGELOG.md)
and
[`ts-sdk/CHANGELOG.md`](https://github.com/Blockstream/anyswap/blob/main/ts-sdk/CHANGELOG.md).

This repository is exported from Blockstream's internal repository at each
release.

## License

MIT, see [`LICENSE`](https://github.com/Blockstream/anyswap/blob/main/LICENSE).
