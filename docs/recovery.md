# Persistence and recovery

## The store

The driver keeps one record per swap: the terms frozen at acceptance, the
current status, and the few things the chain cannot report. It reads and
writes them through `SwapStore`:

```rust
#[async_trait]
pub trait SwapStore: MaybeSend + MaybeSync {
    async fn save(&self, record: &SwapRecord) -> Result<(), StoreError>;
    async fn load(&self, swap_id: &Uuid) -> Result<Option<SwapRecord>, StoreError>;
    async fn list(&self) -> Result<Vec<SwapRecord>, StoreError>;
}
```

Back it with anything durable, persisting the record's serde JSON form
unchanged: records carry a `version` field, and a future SDK can decode and
upgrade old stores only if they kept that self-describing form. A table of
JSON blobs keyed by `swap_id` is enough. The bundled `MemoryStore` is for
tests.

Restarting is then nothing at all: build the same driver over the same store
and spawn `run`. Every status is resolved from the chain on the next pass,
so a record is never stale, and messages already delivered are remembered in
the record rather than resent.

## Losing the store

A swap whose record is gone can be rebuilt from the server, because the
server keeps the signed request that created it:

```rust
let swap = driver.restore(swap_id).await?;
let report = driver.restore_all(SwapListQuery::default()).await?;
```

`restore` fetches the swap, verifies the stored request against your own
identity key, and runs the same acceptance checks as at creation, so a
tampered copy is refused rather than adopted. `restore_all` walks the
server's listing for your `user_id`, narrowed by the query's filters, and
restores every swap the store lacks,
reporting the restored and the failed with what failed them, `Unsigned` for
a swap created before the server kept signed requests, so nothing to verify
against. With `Config::trust_unsigned` those are restored on the server's
word instead, which is how a client from before the driver migrates.

This makes restore trustless about the content of a swap but not about its
existence: a server can omit one from the listing. Treat restore as the
fallback for a lost device, and the durable store as the plan.

## What the mnemonic determines

Everything key-shaped. From the mnemonic alone the `Wallet` reproduces the
`user_id`, and per `swap_id` the signing key, the blinding keys on Liquid,
and the SwapOut preimage. Nothing key-related is ever in a record or a
backup; a record names its swap and the wallet re-derives the rest.

The branches are hardened and separate, so revealing one swap's key, as the
protocol sometimes does cooperatively, exposes neither `user_id` nor any
other swap.

Keep the mnemonic, keep the store; the mnemonic alone recovers everything
the server still lists, and the store covers a server that is unreachable
or forgetful.
