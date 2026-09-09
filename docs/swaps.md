# Running swaps

## Quotes

Every swap starts from a quote: one read of the server's terms for a source,
a destination, and one amount. An end is a chain, or `None` for Lightning.

```rust
let quote = driver.quote(Some(Chain::Bitcoin), None, 100_000).await?;
```

`Quote` carries the full price: `send_amount` is `receive_amount` plus
`fee`, split into `base_fee` (the server's on-chain costs at current fee
rates) and `service_fee` (proportional). Its `quote_id` pins the base fee,
so the price cannot move between quoting and creating; a quote older than
`expires_at` is refused and you fetch a new one. Show the quote to the user,
create the swap when they accept.

## SwapIn: on-chain to Lightning

Mint an invoice on your Lightning node for exactly `quote.receive_amount`
and create the swap with it. Acceptance refuses an invoice whose amount
differs or whose expiry does not outlast the funding window, so give it a
comfortable expiry.

```rust
let swap = driver.swap_in(quote, bolt11, Some(refund_address)).await?;
println!("deposit {} to {}", swap.record().send_amount, swap.address()?);
```

The refund address is optional; see [SwapIn refunds](#swapin-refunds) for
what happens without one.

The user sends `send_amount` to the address as one output of exactly that
value. From there the driver waits for the invoice to be paid, reveals the
refund key once the preimage is on the table, and closes the swap. Anything
else that lands under the script, a wrong amount or a second deposit, is
never treated as the deposit; it is swept back by the refund machinery
below.

A SwapIn ends in `Completed`, or in `Refund` when the deposit comes back:
because the swap was canceled, because the server cooperated with a key, or
because the CSV matured with the deposit unclaimed.

## SwapOut: Lightning to on-chain

Create the swap with the address the funds should land on, or `None` to
take one from the destinations hook:

```rust
let swap = driver.swap_out(quote, Some(claim_address)).await?;
println!("pay {}", swap.claim_invoice());
```

The user pays `claim_invoice`, a hold invoice for `send_amount`. The server
then opens the on-chain output; the driver waits for it to confirm, commits,
hands the server the preimage that releases the payment, and claims the
output to `claim_address`, cooperatively through the keypath when the server
sends its key and through the preimage path when it does not.

A SwapOut ends in `Completed`, in `Canceled` when it closes with the payment
safe (the hold invoice expires back), or in `Lost` if a foreign spend
confirms after the preimage went out, which a correct server never does.

## Statuses

`swap.status()` is resolved from the chain on every pass:

| Status       | Flow    | Meaning                                                                 |
|--------------|---------|-------------------------------------------------------------------------|
| `Funding`    | both    | Nothing on chain yet. Past `expires_at` the driver sends a cancel and keeps watching. |
| `Confirming` | both    | The opening has `confs` of `needed` confirmations; `matures_at` is where its CSV runs out. |
| `CoopKey`    | SwapOut | Committed to claim. Waiting for the server's key until `expires_at`, then claiming by preimage. |
| `Spending`   | SwapOut | Our claim is broadcast with `confs` confirmations.                       |
| `Refund`     | SwapIn  | Everything under the script is ours: `refundable` waits, `refunded` lists the sweeps. Never left; its contents move with the chain. |
| `Completed`  | both    | Paid out; `opening` and `payout` are the txids.                          |
| `Canceled`   | SwapOut | Closed with the payment safe.                                            |
| `Lost`       | SwapOut | A foreign spend confirmed after the preimage went out.                   |

`Completed`, `Canceled` and `Lost` are terminal (`status.is_terminal()`). A
`Refund` instead goes quiet: once nothing spendable remains, every sweep has
confirmed and its window has passed, `run` stops making passes for it, but
any coin that later lands under the script wakes it back up within
`refund_timeout` of entering the status.

`swap.server_status()` is the server's own label from the last pass. Show it
so a swap waiting on the server says whom it waits for; the driver never
acts on it.

## Canceling

```rust
match swap.cancel().await? {
    Cancellation::Requested => {} // closes once the server confirms
    Cancellation::TooLate => {}   // the payment already left; the swap continues
}
```

A swap can only be canceled while nothing irreversible has happened: before
a SwapIn deposit is claimed and before a SwapOut commits to its preimage.

## SwapIn refunds

The driver builds and broadcasts refunds itself, keypath when it holds the
server's key and CSV otherwise, and only when the coins are worth more than
the fee to move them. It pays the refund address given to `swap_in`, which
travels in the signed request. A swap created without one falls back to a
`Destinations`, asked once per broadcast:

```rust
impl Destinations for MyWallet {
    fn claim_address(&self, chain: Chain) -> String { self.next_address(chain) }
    fn refund_address(&self, chain: Chain) -> String { self.next_address(chain) }
}

let driver = driver.with_destinations(my_wallet);
```

Registered like this, `run` sweeps anything spendable the moment it can, and
`swap_out` may pass `None` for its claim address. Without either, call
`swap.refund(&destination).await?` on a `SwapIn` handle yourself; it returns
the sweep's txid, or `None` while there is nothing spendable.

## Configuration

`Config` (via `with_config`) holds the flow-independent knobs:

| Knob              | Default | Meaning                                                        |
|-------------------|---------|----------------------------------------------------------------|
| `funding_timeout` | 20 min  | From creation: when a swap with nothing on chain cancels       |
| `coop_timeout`    | 5 min   | SwapOut: how long to wait for the server's key before claiming by preimage |
| `poll_interval`   | 15 s    | The longest wait between passes without a server signal        |
| `fee_conf_target` | 3 blocks| The target every fee estimate is read at                       |
| `refund_timeout`  | 1 h     | How long an empty `Refund` keeps getting passes                |
| `trust_unsigned`  | off     | Whether `restore` takes an unsigned swap on the server's word, see [recovery](recovery.md#losing-the-store) |

`ChainConfig` (passed with each `with_chain`) holds the per-chain ones, with
`ChainConfig::bitcoin()` and `ChainConfig::liquid()` as defaults:

| Knob             | Bitcoin | Liquid | Meaning                                             |
|------------------|---------|--------|-----------------------------------------------------|
| `min_confs`      | 3       | 2      | Depth that releases a SwapOut claim, and closes a swap on a spend |
| `claim_margin`   | 252     | 2520   | How much CSV must remain when the preimage goes out |
| `max_refund_csv` | 1008    | 10080  | The longest CSV accepted on a SwapIn deposit        |
| `max_htlc_hold`  | 504     | 5040   | How far past the CSV a hold invoice may hold the payment |
| `max_fee_rate`   | 100     | 1      | The rate no spend goes above, in sat/vB             |
| `block_time`     | 600 s   | 60 s   | Expected block interval, for mixed deadline math    |
