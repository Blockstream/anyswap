# Changelog

## [1.5.0]

- **Breaking:** `Status::Confirming` gains `fee_rate_spike`, the fee estimate
  while it is above `max_fee_rate` and holds a SwapOut's preimage.
- **Breaking:** `with_chain` returns a `Result`, refusing a `max_fee_rate`
  that is not finite and positive with `DriverError::InvalidChainConfig`.

### Minor

- A SwapOut claims its opening by the preimage path only once the preimage
  was sent or seen on chain. With only the server's key it claims by the
  keypath, even to outbid a pending CSV refund, where it used to switch to
  the preimage path.
- A pass whose save fails still refunds what the chain allows and fails
  only at its end, where it used to stop at the save.
- A record keeps at most 64 bytes of the server's status and 256 of its
  cancel message.
- A SwapIn refund spends only confirmed outputs, so a fund is `Locked` until
  its first confirmation even once the server's key is home, where the
  keypath used to refund it from the mempool.

## [1.4.0]

- **Breaking:** the driver reports the miner fee of the user's own spends as
  `fee`, read from the transaction on chain: a SwapOut claim's in
  `Status::Claiming` and `Status::Completed`, a SwapIn refund's in
  `FundState::Refunding` and `FundState::Refunded`, where a batched refund's
  fee is the whole transaction's on every fund it took. `Spender` carries it
  too. A stored swap still loads, with no `fee` until a pass reads it again.
- **Breaking:** `Config` gains `fee_rate_cache_ttl`, how long a chain's fee
  estimate is reused, 30 seconds by default, so a sweep reads it once per
  chain rather than once per swap. Zero reads it on every pass, as before.
- **Breaking:** `Config` gains `server_timeout`, 30 seconds by default. A call
  to the server that goes unanswered that long fails as a transport error,
  where the driver used to wait on it without limit.
- **Breaking:** `max_htlc_hold` moves from `ChainConfig` to `Config` and caps
  a SwapOut hold invoice's `min_final_cltv_expiry_delta` on its own, 1008
  Lightning blocks by default, where it used to be added to the server's
  `dest_csv`.
- **Breaking:** `CancelReason` gains `ClaimWindowClosed { at }`. A SwapOut
  with less than `claim_margin` of the CSV left before its preimage went out
  ends `Canceled` at once, its payment failing back once the server's CSV
  refund takes the opening from block `at`. It used to stay `Confirming`
  until that refund confirmed.
- A SwapOut's `Funding.expires_at` is no later than the expiry of
  `claim_invoice`, so it is the time to pay by, and the driver's cancel goes
  out then.
- A SwapOut holds its preimage while the fee estimate is above
  `max_fee_rate`, so its claim pays the current rate. The swap stays
  `Confirming` meanwhile.
- `cancel` answers `Requested` for a swap already canceled, where it answered
  `TooLate`. Against a 1.4.0 server, a cancel retried after a lost answer
  gets `Requested` too.
- `run` takes a quiet SwapIn up again once the chain's cached fee rate makes
  one of its `Dust` funds worth refunding, where it used to leave the fund
  `Dust` until a manual `step` or `refund`.
- A SwapOut claim that replaces a pending preimage claim or the server's
  refund goes through the preimage path even once the server's key is known,
  so the replacement pays more in all and is accepted.
- A restored swap's `ended_at` is the server's `transitioned_at` only up to
  the time of the restore.
- On wasm32 the driver holds a Web Lock per swap from loading its record to
  saving it, where `navigator.locks` exists, so tabs and workers over one
  store take turns on a swap. Natively a store still belongs to one process.
- `restore_all` stops paging at a page with no swap it has not seen whose
  request the wallet signed, where it used to follow the server's `has_next`
  alone, and reports a swap listed twice once.
- `BitcoinEsplora` and `LiquidEsplora` give up on a request after 30 seconds,
  where they used to wait on it without limit; `with_timeout` sets another
  limit. A `ChainClient` of your own should bound its requests the same way,
  since the driver waits on a chain call for as long as it takes.
- The driver refunds at most 8 of a SwapIn's funds in one transaction, the
  most valuable first, and the rest once that refund is in a block, so funds
  can stay `Refundable` while another refund is `Refunding`. A pass calls
  `outspend` for as many strays, those in the pending refund first, and for
  none still `Locked`.
- `subscribe` yields a swap's handle again when a pass clears its error,
  where a pass that changed nothing else used to stay silent.
- `DriverError::retryable` is true for an HTTP 429 from the server or
  anything in front of it.

## [1.3.0]

- **Breaking:** the wallet derives its keys from a seed of its own, so the same
  mnemonic gives a different `user_id` and different swap keys. Swaps made
  with an earlier SDK are not listed or restored; finish them with the SDK
  that made them.
- **Breaking:** `SwapWallet::pubkey` and `privkey` are split into
  `source_refund_pubkey`/`source_refund_privkey` and
  `dest_claim_pubkey`/`dest_claim_privkey`.
- **Breaking:** the driver's `Config::trust_unsigned` is removed, so `restore`
  refuses a swap the server holds no signed request for.
- **Breaking:** the driver has one swap handle type, `Handle`, in place of
  `Swap`, `SwapInHandle` and `SwapOutHandle`. It keeps no copy of the swap,
  so `record`, `status`, `address` and `claim_invoice` read the store and are
  async, and a call meant for the other swap type fails with `WrongSwapType`.
- **Breaking:** the server's status is kept in the swap record as
  `server_status`, so it survives a restart, and `ServerStatus` is now the
  status name alone. Updates now also fire when only the server's status
  changes.
- **Breaking:** the `Destinations` methods are async and return a `Result`;
  an `Err` fails the call as `DriverError::Destination`.
- **Breaking:** `restore_all` takes no query. It restores every swap the
  server lists for the wallet that the store lacks.
- **Breaking:** `ClientError::Header` and `ClientError::Json` carry a message
  instead of the source error, so every error can be cloned.
- **Breaking:** amounts in sats serialize as decimal strings, in the API
  types and in the driver's `Quote`, `SwapRecord` and `Status`. The Rust
  fields are unchanged, but a store written before this holds numbers and no
  longer loads; finish or drop those swaps first.
- **Breaking:** `SwapScript::spend_path` reads from an input's witness how it
  spent the swap output: `SpendPath::Keypath`, `Preimage` with the verified
  preimage, or `Csv`. The driver's `Spender` carries it as `path` in place of
  `preimage` and `pays_address`, and `Tx::preimage_at`, `Tx::pays` and
  `SwapScript::destination_script` are gone.
- **Breaking:** a SwapOut's claim is a spend of the opening through the
  keypath or the preimage leaf, which the server's key alone cannot open. It
  used to be any spend paying the claim address.
- **Breaking:** the driver's opening, a SwapIn deposit or a SwapOut opening,
  is the oldest output paying exactly the swap's amount, and every other
  output is a stray. Two such outputs used to mean none. `Observed::unique`
  is `Observed::split`, which returns the opening and the strays.
- **Breaking:** a SwapOut sends its preimage once, at the release gate, then
  claims every pass until a claim of the opening is in a block, rebuilt at the
  current fee rate: by keypath when the server answered with its key, by
  preimage once the opening is at `min_confs` when it did not. A spend still in
  the mempool is outbid, the server's CSV refund at `max_fee_rate`, where the
  driver used to wait on it and to resend its own claim unchanged. The
  preimage is marked tried and saved before the request, and any answer
  counts as tried. `CoopKey` and `Spending` are one `Claiming`, `Lost` is a
  `Canceled` with its `opening`, and `Config::coop_timeout`,
  `SwapRecord::committed_at`, `Action::Commit`, `Spend::Rebroadcast` and
  `Spender::tx_hex` are gone. A store holding a SwapOut in `CoopKey`,
  `Spending` or `Lost` no longer loads; finish those swaps first.
- **Breaking:** a restored SwapOut no longer takes the server's `Claimed` as
  its preimage having left. It counts the preimage as out only when the
  server's key verifies or a claim of ours on chain shows it, and otherwise
  sends it through the release gate like any other. `SwapRecord::preimage`
  holds a SwapOut's own preimage once a claim shows it.
- **Breaking:** a SwapIn's status is worked out from the chain and the
  server's key and preimage on every pass, and it ends `Completed` or
  `Canceled`. `Status::Refund` is gone, along with the early returns that
  kept a stored `Completed` or `Refund` forever. What under the script is the
  user's is the record's `funds`, read with `Handle::funds`: one `Fund` per
  output, `Locked`, `Refundable`, `Dust`, `Refunding` or `Refunded`. Every
  pass refunds whatever is `Refundable` in any status, where strays used to
  come back only in `Refund` or by hand. Whose a spend is reads its path, and
  a keypath refund is taken only while our refund key is home.
  `Handle::refund` is a pass with the given destination. `Action::Close` is
  gone, `SwapRecord` gains `funds` and `ended_at`, and `refund_timeout`
  counts from a SwapIn's end. `Confirming` no longer carries `matures_at`,
  which nothing read. A store holding a SwapIn in `Refund` no longer loads;
  finish those swaps first.
- **Breaking:** `ChainClient` has a new required method, `outspend(txid,
  vout)`: the transaction spending an output, mempool included, or `None`
  while it is unspent. Each pass asks it about the opening and, on a SwapIn,
  each output worth its refund fee, when the address history shows them
  unspent. Only a failure on the opening fails the pass. `BitcoinEsplora` and
  `LiquidEsplora` answer it from `/tx/:txid/outspend/:vout`.
- **Breaking:** `SwapRecord` gains `base_fee` and `service_fee`, the server's
  split of `send_amount - receive_amount` as acceptance checked it. A store
  written before this no longer loads.
- **Breaking:** `Status::Canceled` carries a `reason`: `ServerCanceled` with
  the server's `error_type` and `cancel_message`, `CsvMatured` for a SwapIn
  whose deposit's CSV matured with no word from the server, or
  `ServerRefunded` for a SwapOut the server's CSV refund took. The server's
  cancel is kept in the record as `server_cancel` from the first pass that
  reads it, so a SwapOut canceled before any opening stays `Canceled` through
  a pass the server does not answer, where it used to read `Funding` again.
- **Breaking:** a restored swap is `Status::Restoring` until its first pass,
  the loop's or a `step`, where it used to read `Funding` whatever it stood
  at, so a page could show an ended SwapIn's deposit address as open.
  `cancel` takes it as open, and the server refuses what is past.
- **Breaking:** the driver sends the protocol version built into core,
  `PROTOCOL_VERSION`, the release's major and minor version, where it used to
  send back whatever the server's `/v1/info` reported. A quote from a server
  on another version fails with `Refusal::ProtocolVersion`, so the SDK and
  the server have to share a minor version. `Quote` no longer carries
  `protocol_version`.
- **Breaking:** both crates declare `rust-version = "1.91"`, up from 1.85.
- **Breaking:** `SwapStore::list` returns the stored swap ids instead of the
  records. The driver loads each record on its own, so one that fails to load
  stops only its own swap. It used to stop every pass of `run` and fail
  `restore_all`.

### Minor

- A pass whose message or spend fails still saves what it saw and refunds
  what is refundable, then returns the error. A cancel or refund key the
  server could not take used to end the pass before the refund, so with the
  server gone a SwapIn's stray deposit never came back, even past its CSV,
  and `Handle::refund` failed the same way. `Funding`'s `cancel_sent` now
  turns true only once the cancel is delivered.
- A pass whose refund fails after it saved a change still announces that
  change, where the next pass used to see nothing new and never did.
- The driver's `run` loop ends on `stop`, and `wait_stopped` resolves once it
  has exited; `state` says where the loop is. `Updates` end once the driver
  is stopped and drained.
- Creating or restoring a swap wakes the loop, and a second `run` is a no-op.
- `PROTOCOL_VERSION` is re-exported, and the wasm crate returns it from
  `protocolVersion()`, for a request built by hand.

## [1.2.0]

- **Breaking:** `POST /v1/swap` takes `send_amount` instead of `swap_fee_limit`
  and accepts only a `send_amount` that buys exactly `receive_amount`.
  `swap_fee_limit` is gone from the swap state and the SDKs.
- **Breaking:** the SDK driver's `quote` pins either the amount to receive or
  the amount to send. The driver cannot restore swaps created before this
  release.

### Minor

- The driver no longer fails a step when resending an observed, unconfirmed
  spend is refused, as when the chain source lags a block behind the node.
  The status stays `Spending` with the observed txid.
- `LiquidEsplora` keeps the transaction bodies it has fetched, so reading an
  address history costs one request per pass once its transactions are known,
  instead of one more per transaction.

## [1.2.0-rc.2]

- First release on crates.io. Both crates declare `rust-version = "1.85"`,
  checked in CI with every feature on.

## [1.2.0-rc.1]

### Core

- Swap scripts, transactions and MuSig2 signing for Bitcoin and Liquid, fee
  estimation, and the server API types. Esplora chain backends sit behind the
  `esplora` feature, OpenAPI derives behind `openapi`.

### SDK

- `HttpClient` for the server API (`client`), a WebSocket subscription that
  paces the driver (`ws`, native only), and the swap driver (`driver`) that runs
  any number of swaps to completion, verifying statuses against the chain rather
  than the server's reports. `esplora` adds `BitcoinEsplora` and `LiquidEsplora`.
  Builds for wasm32.
