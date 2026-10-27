# Changelog

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
