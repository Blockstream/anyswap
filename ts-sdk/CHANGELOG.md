# Changelog

## [1.10.1]

No changes.

## [1.10.0]

- `BitcoinEsplora` and `LiquidEsplora` ask again when a read is answered 429, 500
  or 503, or a broadcast 429: up to 6 times, waiting from 256 ms and doubling. A
  driver over a rate limit slows down instead of failing the swaps at the end of
  its list.
- `swapIn` accepts a claim invoice shorter than the chain's `fundingTimeout`
  and `minConfs + 1` blocks. It only has to outlast those blocks, and a
  shorter one brings `expires_at` forward.

## [1.9.0]

- **Breaking:** `BitcoinEsplora.getFeeRate` throws when the indexer has no
  estimate within the target, instead of returning 1 sat/vB, so a SwapOut
  holds its preimage until there is one. On regtest, which has no estimates,
  the driver takes the chain's relay minimum when a fee read fails.
- **Breaking:** `fundingTimeout` moves from `DriverConfig` to the chain options
  of `withChain` and defaults to 1200 s on Bitcoin and 900 s on Liquid, where
  it was 86400 s, so a swap-in's `expires_at` no longer runs past the server's
  wait for the deposit. A swap-out's `expires_at` is its hold invoice's expiry
  alone.
- **Breaking:** `swapIn` throws `Refused` when the quote's `await_opening_tx`
  or the claim invoice does not last the chain's `fundingTimeout` and
  `minConfs + 1` blocks more. `swapOut` throws `Refused`, and cancels the swap
  at the server, when the hold invoice outlasts the chain's `fundingTimeout`.
- **Breaking:** `new Driver` throws an `Invalid` `DriverError` outside a secure
  context, HTTPS or localhost. A page served over plain HTTP has no Web Locks,
  so tabs over one store could not take turns on a swap.
- A SwapOut holds its preimage while a claim at the fee estimate would cost
  more than its funds can pay, with the estimate as `fee_rate_spike`, and its
  claim never pays a higher rate than the funds can.
- A restored SwapOut reprices its claim still in the mempool even when the
  server does not report its key.
- `run` keeps driving an ended SwapIn while a fund is `Locked`, and takes it up
  again once a `Dust` fund is worth its refund, also with no refund address or
  `Destinations` hook, so the fund is seen turn `Refundable`.
- A SwapIn refund still in the mempool is replaced at the fee estimate, capped
  at `maxFeeRate`, once the estimate is above its rate.
- The `Destinations` hook is asked for a SwapIn's refund address once, and
  every later refund of the swap goes there.
- `run` and `advance` take a pass for a server signal no sooner than 5 s into
  their wait, so a flood of signals costs one pass per 5 s. A swap created or
  restored still gets its first pass at once.
- `SwapHandle.serverError` is the error of the last pass that could not read
  the swap at the server, so a stale `server_status` shows as stale. A read
  that works or the swap's end clears it, and the pass that sets or clears it
  yields the swap.
- The Esplora clients fetch a transaction's hex for the driver once and keep
  it, one request at a time, so a pass no longer fetches the hex of every
  transaction at a SwapIn's address and stays under an explorer's rate limit.
- `SwapHandle.refund` and the `Destinations` hook's refund address are checked
  as a refund address at creation is: one of another network, or on Liquid one
  that is not confidential, is refused instead of paid.
- `run` takes the pass that claims a swap-out as soon as its preimage has gone
  out, rather than after `pollInterval`.

## [1.8.0]

- **Breaking:** `AnySwapClient` takes the `wallet` that signs protected
  requests instead of `auth.identityPrivkey`. `AnySwapAuth` is gone.
- **Breaking:** `signals` and `subscribe` take no `userId`; they use the
  client's wallet.
- **Breaking:** `Wallet.identityPrivkey` is gone, and so are the bindings'
  `signRequest` and `getPubkey`: the client signs inside the wasm module, so
  the identity key never leaves it.

## [1.7.0]

- **Breaking:** the SDK exports what an app driving swaps uses. `SwapIn`,
  `SwapOut`, `SwapUtxo`, `EconomicalSplit`, `DepositClassification`,
  `validateSwap`, `sourceAsset`, `AnySwapClient.watchChanges` and
  `SwapChanges`, the Esplora clients' `addressUtxos` and `addressUtxoListing`
  with `EsploraUtxo`, and `signRequest` with its header constants,
  `AuthHeaders` and `RawBody` are gone. The claim and refund builders under the
  driver stay public in the Rust `anyswap-sdk` crate.
- **Breaking:** `Wallet` keeps `userId`, `network`, the identity keys,
  `sourceRefundPubkey` and `destClaimPubkey`. Its private swap keys, its
  blinding keys, `preimage` and `paymentHash` are gone; the driver derives them
  itself.
- **Breaking:** `@blockstream/anyswap-wasm` keeps the driver and what this SDK
  calls. `SwapIn`, `SwapOut`, `SwapUtxo`, `EconomicalSplit`, `serviceFee`,
  `minSendForReceive`, `maxReceiveForSend`, and `Wallet`'s private swap keys,
  blinding keys, `preimage` and `paymentHash` are gone.
- A swap-in fund is `Dust` unless it is worth its refund fee plus 546 sat on
  Bitcoin.
