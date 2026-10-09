# anyswap-wasm

WASM bindings for the AnySwap client SDK. Published to npm as `@blockstream/anyswap-wasm`.

## Build

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
cd crates/sdk-wasm && CC_wasm32_unknown_unknown="$(brew --prefix llvm)/bin/clang" wasm-pack build --target web --scope blockstream
```

The bundle is written to `pkg/`.
