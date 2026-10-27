# @blockstream/anyswap-sdk

TypeScript SDK for atomic swaps between Lightning and on-chain Bitcoin or Liquid.

Documentation: <https://blockstream.github.io/anyswap/overview/?lang=ts>

## Install

```bash
npm install @blockstream/anyswap-sdk
```

## Build from source

The SDK runs on the wasm bindings in `crates/sdk-wasm`, so they are built
first. This needs Node.js `24+`, Rust with the `wasm32-unknown-unknown`
target, `wasm-pack` and clang. From the repository root:

```bash
cd crates/sdk-wasm
CC_wasm32_unknown_unknown=clang wasm-pack build --dev --target web --scope blockstream
cd ../../ts-sdk
npm ci
npm run build
```

On macOS, Apple's clang has no wasm32 target; use Homebrew's LLVM with
`CC_wasm32_unknown_unknown="$(brew --prefix llvm)/bin/clang"`.

## Examples

`examples/swaps.ts` drives any mix of swaps on both chains, and
`examples/advance.ts` drives one swap by hand over a file store. Each
describes its arguments and environment at the top, and runs with
`npm run example:swaps` or `npm run example:advance`.
