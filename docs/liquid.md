# Liquid

The same protocol, script, statuses and driver as on Bitcoin. The driver
handles what confidentiality adds by itself: it derives and sends the
per-swap blinding pubkeys, unblinds amounts with the swap's own key, and
requires confidential destinations. An integration differs in exactly two
places.

## The context needs Liquid parameters

```rust
let genesis_hash = liquid_esplora.get_block_hash(0).await?;
let ctx = SwapContext::new(wallet).with_liquid(&genesis_hash.to_string(), POLICY_ASSET)?;
```

Both values must come from your own sources, never from the swap server.
Every signature the wallet produces commits to the genesis hash, so a wrong
one signs for a different chain; the policy asset is what fees are paid in,
so a wrong one builds transactions that never confirm. The genesis hash
comes from your own Esplora (or `elements-cli getblockhash 0`); the policy
asset id belongs in your network configuration next to the server and
Esplora URLs. A regtest Elements chain mints its own L-BTC, so there read it
with `elements-cli dumpassetlabels`.

The server also advertises an asset id in `/info`. The driver only uses it
to name the asset when quoting; the context's copy is the one it trusts,
and acceptance refuses a swap whose asset is any other.

A context built without `with_liquid` serves Bitcoin swaps only.

## The chain registration

```rust
let driver = driver.with_chain(Chain::Liquid, liquid_esplora, ChainConfig::liquid());
```

`ChainConfig::liquid()` carries the Liquid defaults: one-minute blocks, two
confirmations, and a 1 sat/vB fee ceiling. Both chains can be registered on
one driver; each swap runs on its own.

## Everything else

Unchanged. Quotes, creation, statuses, cancels, refunds and recovery read
exactly as the other pages describe them; `claim_address` and refund
destinations must simply be confidential Liquid addresses, and an unblinded
one is refused when the swap or the sweep is built.
