The driver: runs swaps to completion against the chain, in the SDK's wasm,
over a server client, a chain client and a store the application supplies.

It creates swaps from a quote, watches the swap script's address, sends the
protocol messages when their moment comes, claims a swap-out and refunds a
swap-in through the cheapest path available, and saves every change to your
store, so a restart resumes where it left off. It trusts only what it can
verify: the terms are checked against your own request at creation and
frozen, and from each later read it takes the server's key and preimage on
their own merit, and its status label and cancel reason for display only.

Two ways to drive. [Driver.run](#run) drives every stored swap in the
background and [Driver.subscribe](#subscribe) yields each change; a
[SwapHandle](#swaphandle) drives one swap by hand with [SwapHandle.advance](#advance).

What you supply is three interfaces, two of which the SDK's own classes
satisfy as they are: [AnySwapClient](Api.md#anyswapclient) is a
[ServerClient](#serverclient), [BitcoinEsplora](Esplora.md#bitcoinesplora) and
[LiquidEsplora](Esplora.md#liquidesplora) are [ChainClient](#chainclient)s, and
the [SwapStore](#swapstore) is yours, with [MemoryStore](#memorystore) for tests.
[AnySwapClient.signals](Api.md#signals) is the optional
[Signals](#signals) channel that paces the loops, and [Destinations](#destinations) the
optional hook for a wallet that mints its addresses on demand.

## Classes

### Driver

Drives every swap in its store. Build one at startup, register a chain
client per chain, then either spawn [Driver.run](#run) and watch
[Driver.subscribe](#subscribe), or drive one swap at a time through its
[SwapHandle](#swaphandle).

#### Constructors

##### Constructor

```ts
new Driver(
   ctx: SwapContext, 
   server: ServerClient, 
   store: SwapStore): Driver;
```

###### Parameters

| Parameter | Type | Description |
| ------ | ------ | ------ |
| `ctx` | [`SwapContext`](Context.md#swapcontext) | The wallet and chain parameters every swap shares. |
| `server` | [`ServerClient`](#serverclient) | The server API; an [AnySwapClient](Api.md#anyswapclient) with auth. |
| `store` | [`SwapStore`](#swapstore) | Where records live between passes and across restarts. |

###### Returns

[`Driver`](#driver)

#### Accessors

##### state

###### Get Signature

```ts
get state(): RunState;
```

Where `run` is: `Idle` before it, then `Running`, `Stopping` and `Stopped`.

###### Returns

[`RunState`](#runstate)

#### Methods

##### get()

```ts
get(swapId: string): Promise<SwapHandle | undefined>;
```

The stored swap as a fresh handle, or `undefined`.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `swapId` | `string` |

###### Returns

`Promise`&lt;[`SwapHandle`](#swaphandle) \| `undefined`&gt;

##### quote()

```ts
quote(
   source: SwapEndpoint, 
   dest: SwapEndpoint, 
   target: TargetAmount): Promise<Quote>;
```

A quote for a swap from `source` to `dest`: one read of the server's
terms for one amount, with the `quote_id` that locks its base fee until
`expires_at`. `target` pins either end, `{ receive }` or `{ send }`. Show
it, create the swap when the user accepts.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `source` | [`SwapEndpoint`](Types.md#swapendpoint) |
| `dest` | [`SwapEndpoint`](Types.md#swapendpoint) |
| `target` | [`TargetAmount`](#targetamount) |

###### Returns

`Promise`&lt;[`Quote`](#quote-1)&gt;

##### restore()

```ts
restore(swapId: string): Promise<SwapHandle>;
```

Rebuilds one swap from the server's copy of the signed request that
created it, verified against this wallet's identity key, for a store that
lost it. It is `Restoring` until its first pass, which [Driver.run](#run)
makes, or [SwapHandle.step](#step). A record already stored is returned as
is.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `swapId` | `string` |

###### Returns

`Promise`&lt;[`SwapHandle`](#swaphandle)&gt;

##### restoreAll()

```ts
restoreAll(): Promise<RestoreReport>;
```

Rebuilds every swap the server lists for this wallet that the store
lacks, as [Driver.restore](#restore) does one, and reports each swap restored
or the check that failed it.

###### Returns

`Promise`&lt;[`RestoreReport`](#restorereport)&gt;

##### run()

```ts
run(): void;
```

Drives every swap in the store that is not quiet, in the background,
until [Driver.stop](#stop). Every pass re-reads the store, so a swap
created or restored is driven from the next pass on; a second call is a
no-op. Watch [Driver.subscribe](#subscribe) for what changed.

###### Returns

`void`

##### signal()

```ts
signal(swapId: string): void;
```

Wakes any loop waiting on `swapId`, as a signal from the channel would.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `swapId` | `string` |

###### Returns

`void`

##### stop()

```ts
stop(): void;
```

Ends [Driver.run](#run) after the pass in flight. Final: a stopped driver
does not run again, and its [Updates](#updates) end once drained. Handles
still `step` and `advance`.

###### Returns

`void`

##### subscribe()

```ts
subscribe(): Updates;
```

Every swap a pass changed, from [Driver.run](#run) and from [SwapHandle.step](#step).

###### Returns

[`Updates`](#updates)

##### swapIn()

```ts
swapIn(
   quote: Quote, 
   claimInvoice: string, 
   refundAddress?: string): Promise<SwapHandle>;
```

Creates a swap-in against `quote` with the invoice minted for its
`receive_amount`. The handle carries where and how much to deposit. A
`refundAddress` is signed with the request and paid by any refund; without
one the [Destinations](#destinations) hook is asked when there is something to take,
or [SwapHandle.refund](#refund) is called with an address.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `quote` | [`Quote`](#quote-1) |
| `claimInvoice` | `string` |
| `refundAddress?` | `string` |

###### Returns

`Promise`&lt;[`SwapHandle`](#swaphandle)&gt;

##### swapOut()

```ts
swapOut(quote: Quote, claimAddress?: string): Promise<SwapHandle>;
```

Creates a swap-out against `quote` paying `claimAddress`, or the address
[Destinations](#destinations) gives without one. The handle carries the hold invoice
to pay.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `quote` | [`Quote`](#quote-1) |
| `claimAddress?` | `string` |

###### Returns

`Promise`&lt;[`SwapHandle`](#swaphandle)&gt;

##### waitStopped()

```ts
waitStopped(): Promise<void>;
```

Resolves once `run` has exited and no pass writes to the store.

###### Returns

`Promise`&lt;`void`&gt;

##### withChain()

```ts
withChain(
   chain: Chain, 
   client: ChainClient, 
   config?: ChainConfig): this;
```

Registers a chain with its client, under the chain's default policy with
`config` applied over it; a chain without one cannot be swapped on.
Throws an `Invalid` [DriverError](#drivererror) for a `maxFeeRate` that is not
finite and positive.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `chain` | [`Chain`](Types.md#chain) |
| `client` | [`ChainClient`](#chainclient) |
| `config?` | [`ChainConfig`](#chainconfig) |

###### Returns

`this`

##### withConfig()

```ts
withConfig(config: DriverConfig): this;
```

Sets the driver-wide knobs, each one left out at its default.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `config` | [`DriverConfig`](#driverconfig) |

###### Returns

`this`

##### withDestinations()

```ts
withDestinations(destinations: Destinations): this;
```

Takes the hook asked for an address when a swap is created without one:
[Driver.swapOut](#swapout) with no `claimAddress`, and a swap-in with no
refund address once there is something to take.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `destinations` | [`Destinations`](#destinations) |

###### Returns

`this`

##### withSignals()

```ts
withSignals(signals: Signals): this;
```

Takes a push channel to pace the loops by, typically
[AnySwapClient.signals](Api.md#signals). Without one they
pace on the poll interval alone.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `signals` | [`Signals`](#signals) |

###### Returns

`this`

***

### DriverError

What a driver call failed with. `code` names the kind of failure, keeping
the server's own code names for its answers; `retryable` means the chain or
server was unreachable and the next call sees the same world, where anything
else needs attention before acting again.

#### Extends

- `Error`

#### Constructors

##### Constructor

```ts
new DriverError(
   message: string, 
   code: DriverErrorCode, 
   retryable: boolean, 
   httpStatus?: number): DriverError;
```

###### Parameters

| Parameter | Type | Description |
| ------ | ------ | ------ |
| `message` | `string` | - |
| `code` | [`DriverErrorCode`](#drivererrorcode) | - |
| `retryable` | `boolean` | - |
| `httpStatus?` | `number` | The HTTP status the server answered with, `undefined` when the failure is not an answer from the server. |

###### Returns

[`DriverError`](#drivererror)

###### Overrides

```ts
Error.constructor
```

#### Properties

| Property | Modifier | Type | Description | Inherited from |
| ------ | ------ | ------ | ------ | ------ |
| <a id="property-cause"></a> `cause?` | `public` | `unknown` | - | `Error.cause` |
| <a id="property-code"></a> `code` | `readonly` | [`DriverErrorCode`](#drivererrorcode) | - | - |
| <a id="property-httpstatus"></a> `httpStatus?` | `readonly` | `number` | The HTTP status the server answered with, `undefined` when the failure is not an answer from the server. | - |
| <a id="property-message"></a> `message` | `public` | `string` | - | `Error.message` |
| <a id="property-name"></a> `name` | `public` | `string` | - | `Error.name` |
| <a id="property-retryable"></a> `retryable` | `readonly` | `boolean` | - | - |
| <a id="property-stack"></a> `stack?` | `public` | `string` | - | `Error.stack` |
| <a id="property-stacktracelimit"></a> `stackTraceLimit` | `static` | `number` | The `Error.stackTraceLimit` property specifies the number of stack frames collected by a stack trace (whether generated by `new Error().stack` or `Error.captureStackTrace(obj)`). The default value is `10` but may be set to any valid JavaScript number. Changes will affect any stack trace captured _after_ the value has been changed. If set to a non-number value, or set to a negative number, stack traces will not capture any frames. | `Error.stackTraceLimit` |

#### Methods

##### captureStackTrace()

```ts
static captureStackTrace(targetObject: object, constructorOpt?: Function): void;
```

Creates a `.stack` property on `targetObject`, which when accessed returns
a string representing the location in the code at which
`Error.captureStackTrace()` was called.

```js
const myObject = {};
Error.captureStackTrace(myObject);
myObject.stack;  // Similar to `new Error().stack`
```

The first line of the trace will be prefixed with
`${myObject.name}: ${myObject.message}`.

The optional `constructorOpt` argument accepts a function. If given, all frames
above `constructorOpt`, including `constructorOpt`, will be omitted from the
generated stack trace.

The `constructorOpt` argument is useful for hiding implementation
details of error generation from the user. For instance:

```js
function a() {
  b();
}

function b() {
  c();
}

function c() {
  // Create an error without stack trace to avoid calculating the stack trace twice.
  const { stackTraceLimit } = Error;
  Error.stackTraceLimit = 0;
  const error = new Error();
  Error.stackTraceLimit = stackTraceLimit;

  // Capture the stack trace above function b
  Error.captureStackTrace(error, b); // Neither function c, nor b is included in the stack trace
  throw error;
}

a();
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `targetObject` | `object` |
| `constructorOpt?` | `Function` |

###### Returns

`void`

###### Inherited from

```ts
Error.captureStackTrace
```

##### prepareStackTrace()

```ts
static prepareStackTrace(err: Error, stackTraces: CallSite[]): any;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `err` | `Error` |
| `stackTraces` | `CallSite`[] |

###### Returns

`any`

###### See

https://v8.dev/docs/stack-trace-api#customizing-stack-traces

###### Inherited from

```ts
Error.prepareStackTrace
```

***

### MemoryStore

Records kept in memory, for tests and short-lived tools.

#### Implements

- [`SwapStore`](#swapstore)

#### Constructors

##### Constructor

```ts
new MemoryStore(): MemoryStore;
```

###### Returns

[`MemoryStore`](#memorystore)

#### Methods

##### list()

```ts
list(): Promise<string[]>;
```

Every stored `swap_id`, without reading the records, so one that fails
to load stops only its own swap.

###### Returns

`Promise`&lt;`string`[]&gt;

###### Implementation of

[`SwapStore`](#swapstore).[`list`](#list-1)

##### load()

```ts
load(swapId: string): Promise<SwapRecord | undefined>;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `swapId` | `string` |

###### Returns

`Promise`&lt;[`SwapRecord`](#swaprecord) \| `undefined`&gt;

###### Implementation of

[`SwapStore`](#swapstore).[`load`](#load-1)

##### save()

```ts
save(record: SwapRecord): Promise<void>;
```

Replaces any record already stored under the same `swap_id`.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `record` | [`SwapRecord`](#swaprecord) |

###### Returns

`Promise`&lt;`void`&gt;

###### Implementation of

[`SwapStore`](#swapstore).[`save`](#save-1)

***

### SwapHandle

A swap bound to its driver: its id and type, and the calls that act on it.
It keeps no copy of the swap, so every read is the store's newest.

#### Accessors

##### error

###### Get Signature

```ts
get error(): DriverError | undefined;
```

The error of this swap's last failing pass under [Driver.run](#run),
`undefined` after a working one.

###### Returns

[`DriverError`](#drivererror) \| `undefined`

##### swapId

###### Get Signature

```ts
get swapId(): string;
```

###### Returns

`string`

##### swapType

###### Get Signature

```ts
get swapType(): SwapType;
```

###### Returns

[`SwapType`](Types.md#swaptype)

#### Methods

##### address()

```ts
address(): Promise<string>;
```

Swap-in: the deposit address; the record's `send_amount` goes here.

###### Returns

`Promise`&lt;`string`&gt;

##### advance()

```ts
advance(): Promise<DriverStatus>;
```

Waits for a signal or the poll interval, whichever comes first, then makes
one pass. Resolves to the status it left.

###### Returns

`Promise`&lt;[`DriverStatus`](#driverstatus)&gt;

##### cancel()

```ts
cancel(): Promise<Cancellation>;
```

###### Returns

`Promise`&lt;[`Cancellation`](#cancellation)&gt;

##### claimInvoice()

```ts
claimInvoice(): Promise<string>;
```

Swap-out: the hold invoice to pay; the record's `send_amount` is its amount.

###### Returns

`Promise`&lt;`string`&gt;

##### funds()

```ts
funds(): Promise<Fund[]>;
```

Swap-in: what under the script is the user's, as the last pass left it.

###### Returns

`Promise`&lt;[`Fund`](#fund)[]&gt;

##### isTerminal()

```ts
isTerminal(): Promise<boolean>;
```

`Completed` or `Canceled`: the deal is settled. A swap-in's
[SwapHandle.funds](#funds) may still be moving.

###### Returns

`Promise`&lt;`boolean`&gt;

##### record()

```ts
record(): Promise<SwapRecord>;
```

The record as the last pass left it.

###### Returns

`Promise`&lt;[`SwapRecord`](#swaprecord)&gt;

##### refund()

```ts
refund(destination: string): Promise<string | undefined>;
```

Swap-in: one pass that refunds the `Refundable` funds to `destination`,
in any status. The refund's txid, or `undefined` with nothing refundable
yet or an earlier refund still `Refunding`.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `destination` | `string` |

###### Returns

`Promise`&lt;`string` \| `undefined`&gt;

##### status()

```ts
status(): Promise<DriverStatus>;
```

The status as the last pass left it.

###### Returns

`Promise`&lt;[`DriverStatus`](#driverstatus)&gt;

##### step()

```ts
step(): Promise<DriverStatus>;
```

One pass, now. Resolves to the status it left.

###### Returns

`Promise`&lt;[`DriverStatus`](#driverstatus)&gt;

***

### Updates

Every swap a pass changed, as [Driver.run](#run) and [SwapHandle.step](#step)
produce them, plus a handle for every failing pass under `run`. Iterate it
with `for await`.

#### Implements

- `AsyncIterable`&lt;[`SwapHandle`](#swaphandle)&gt;

#### Methods

##### \[asyncIterator\]()

```ts
asyncIterator: AsyncIterator<SwapHandle>;
```

###### Returns

`AsyncIterator`&lt;[`SwapHandle`](#swaphandle)&gt;

###### Implementation of

```ts
AsyncIterable.[asyncIterator]
```

##### next()

```ts
next(): Promise<SwapHandle | undefined>;
```

The next changed swap, or `undefined` once the driver is stopped and drained, or gone.

###### Returns

`Promise`&lt;[`SwapHandle`](#swaphandle) \| `undefined`&gt;

## Interfaces

### ChainClient

Query and broadcast on one chain. A throw fails the pass, and the next pass asks again.

#### Methods

##### addressHistory()

```ts
addressHistory(address: string): Promise<HistoryTx[]>;
```

Every transaction paying or spending `address`, mempool included.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `address` | `string` |

###### Returns

`Promise`&lt;`HistoryTx`[]&gt;

##### broadcast()

```ts
broadcast(txHex: string): Promise<string>;
```

Broadcasts a signed transaction and returns its txid.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txHex` | `string` |

###### Returns

`Promise`&lt;`string`&gt;

##### getFeeRate()

```ts
getFeeRate(target: number): Promise<number>;
```

Estimate for confirmation within `target` blocks, in sat/vB.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `target` | `number` |

###### Returns

`Promise`&lt;`number`&gt;

##### getHeight()

```ts
getHeight(): Promise<number>;
```

###### Returns

`Promise`&lt;`number`&gt;

##### outspend()

```ts
outspend(txid: string, vout: number): Promise<HistoryTx | undefined>;
```

The transaction spending output `vout` of `txid`, mempool included, or
`undefined` while it is unspent: Esplora's `/tx/:txid/outspend/:vout`
with the spending transaction fetched. Read from the output itself, so it
is found however many other transactions crowd the address's history.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |
| `vout` | `number` |

###### Returns

`Promise`&lt;`HistoryTx` \| `undefined`&gt;

***

### ChainConfig

The knobs that mean something per chain, for [Driver.withChain](#withchain). Each
one left out keeps the chain's default. Depths are in blocks, counted from
a confirmation, since a timelock on an unconfirmed output has not started.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-blocktime"></a> `blockTime?` | `number` | Expected block interval in seconds, to compare a block deadline with a wall-clock one. Defaults to 600 on Bitcoin, 60 on Liquid. |
| <a id="property-claimmargin"></a> `claimMargin?` | `number` | How much CSV must remain when a swap-out's preimage goes out. Defaults to 252 on Bitcoin, 2520 on Liquid. |
| <a id="property-maxfeerate"></a> `maxFeeRate?` | `number` | The rate no spend goes above, in sat/vB, finite and positive. A swap-out holds its preimage while the estimate is above it. Defaults to 100 on Bitcoin, 1 on Liquid. |
| <a id="property-maxrefundcsv"></a> `maxRefundCsv?` | `number` | The longest CSV accepted on a swap-in deposit. Defaults to 1008 on Bitcoin, 10080 on Liquid. |
| <a id="property-minconfs"></a> `minConfs?` | `number` | The depth that releases a swap-out's preimage, and that a spend reaches before it closes a swap. Defaults to 3 on Bitcoin, 2 on Liquid. |

***

### Destinations

Where funds land when a swap is created without an address, for a wallet
that mints one on demand. A throw or a rejection leaves no address and fails
whatever asked for one. Resolve promptly: the loop ticks swaps one at a time
and waits on it.

#### Methods

##### claimAddress()

```ts
claimAddress(chain: Chain): string | Promise<string>;
```

Where a swap-out on `chain` claims to; asked at creation.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `chain` | [`Chain`](Types.md#chain) |

###### Returns

`string` \| `Promise`&lt;`string`&gt;

##### refundAddress()

```ts
refundAddress(chain: Chain): string | Promise<string>;
```

Where a swap-in on `chain` refunds to; asked with something refundable under the script.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `chain` | [`Chain`](Types.md#chain) |

###### Returns

`string` \| `Promise`&lt;`string`&gt;

***

### DriverConfig

The knobs true of the driver whatever it swaps, for
[Driver.withConfig](#withconfig). Each one left out keeps its default. Times are
seconds on the driver's own clock, never one the server reports.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-feeconftarget"></a> `feeConfTarget?` | `number` | The target every fee estimate is read at, in blocks. Defaults to 3. |
| <a id="property-feeratecachettl"></a> `feeRateCacheTtl?` | `number` | How long a chain's fee estimate is reused before it is read again; zero reads it every pass. Defaults to 30. |
| <a id="property-fundingtimeout"></a> `fundingTimeout?` | `number` | From creation, the latest a swap with nothing on chain asks the server to cancel; the server's window for a swap-in deposit, or a swap-out's claim invoice expiry, ends it sooner. Defaults to 86400. |
| <a id="property-maxhtlchold"></a> `maxHtlcHold?` | `number` | The longest a swap-out's hold invoice may hold the payment, in Lightning blocks. Defaults to 1008. |
| <a id="property-pollinterval"></a> `pollInterval?` | `number` | The longest wait between passes without a signal. Defaults to 15. |
| <a id="property-refundtimeout"></a> `refundTimeout?` | `number` | From a swap-in's end, how long it keeps getting passes once nothing of the user's is left to move. Defaults to 3600. |
| <a id="property-servertimeout"></a> `serverTimeout?` | `number` | How long a call to the server may go unanswered before it fails as `Transport`. Defaults to 30. |

***

### Endpoint

One end of a swap: on chain when `chain` is set, Lightning when it is not.
Keys are hex strings.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-asset"></a> `asset` | `string` \| `null` | - |
| <a id="property-chain"></a> `chain` | [`Chain`](Types.md#chain) \| `null` | - |
| <a id="property-csv"></a> `csv` | `number` \| `null` | - |
| <a id="property-destination"></a> `destination` | `string` \| `null` | On chain, where the end pays out: the refund address of a source, the claim address of a dest. |
| <a id="property-invoice"></a> `invoice` | `string` \| `null` | Lightning: the invoice, paid to a swap-in, paid by a swap-out. |
| <a id="property-server_blinding_pubkey"></a> `server_blinding_pubkey` | `string` \| `null` | - |
| <a id="property-server_privkey"></a> `server_privkey` | `string` \| `null` | The server's key, once it arrived and verified against `server_pubkey`. |
| <a id="property-server_pubkey"></a> `server_pubkey` | `string` \| `null` | - |

***

### Fund

Swap-in: one output under the script that is the user's. The deposit the
server was paid for is not one, nor is an output the server took by its
own path.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-amount"></a> `amount` | `string` | In sats, as a decimal string. |
| <a id="property-outpoint"></a> `outpoint` | [`OutPoint`](Types.md#outpoint) | - |
| <a id="property-state"></a> `state` | [`FundState`](#fundstate) | - |

***

### Quote

A quote: one read of the server's terms, locked by `quote_id` until `expires_at`. Amounts in sats, as decimal strings.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-await_opening_tx"></a> `await_opening_tx` | `number` \| `null` | Swap-in: how long after creation the server waits for the deposit to confirm, in seconds, and so the expiry to give the claim invoice. |
| <a id="property-base_fee"></a> `base_fee` | `string` | - |
| <a id="property-dest_asset_id"></a> `dest_asset_id` | `string` \| `null` | - |
| <a id="property-dest_chain"></a> `dest_chain` | `string` \| `null` | - |
| <a id="property-expires_at"></a> `expires_at` | `number` | Unix seconds. |
| <a id="property-fee"></a> `fee` | `string` | The whole spread, `send_amount - receive_amount`. |
| <a id="property-quote_id"></a> `quote_id` | `string` | - |
| <a id="property-receive_amount"></a> `receive_amount` | `string` | - |
| <a id="property-send_amount"></a> `send_amount` | `string` | - |
| <a id="property-service_fee"></a> `service_fee` | `string` | `send_amount - receive_amount - base_fee`. |
| <a id="property-source_asset_id"></a> `source_asset_id` | `string` \| `null` | - |
| <a id="property-source_chain"></a> `source_chain` | `string` \| `null` | - |

***

### RestoreReport

What [Driver.restoreAll](#restoreall) found among the swaps the server lists and the store lacks.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-failed"></a> `failed` | \{ `error`: [`DriverError`](#drivererror); `swapId`: `string`; \}[] | Each swap that failed a check, with the check; `Unsigned` for one the server holds no signed request for. |
| <a id="property-restored"></a> `restored` | `string`[] | The ids of the swaps now in the store. |

***

### Sent

The messages already delivered, since a message leaves no trace on chain.

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-cancel"></a> `cancel` | `boolean` |
| <a id="property-preimage"></a> `preimage` | `boolean` |
| <a id="property-refund_key"></a> `refund_key` | `boolean` |

***

### ServerCancel

The server's cancel in its words: its [error code](Types.md#swaperrorcode) and the message the cancel came with. Shown, never verified.

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-cancel_message"></a> `cancel_message` | `string` \| `null` |
| <a id="property-error_type"></a> `error_type` | [`SwapErrorCode`](Types.md#swaperrorcode) \| `null` |

***

### ServerClient

The server API as the driver calls it, over the wire objects as they are.
Amounts in a body are decimal strings, as on the wire; every integer
is a number. A throw with a numeric `status` is the server's
answer, its `code` the server's error code; any other throw is the server
being unreachable.

#### Methods

##### cancel()

```ts
cancel(swapId: string, body: unknown): Promise<void>;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `swapId` | `string` |
| `body` | `unknown` |

###### Returns

`Promise`&lt;`void`&gt;

##### createSwap()

```ts
createSwap(body: unknown): Promise<void>;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `body` | `unknown` |

###### Returns

`Promise`&lt;`void`&gt;

##### getInfo()

```ts
getInfo(): Promise<unknown>;
```

###### Returns

`Promise`&lt;`unknown`&gt;

##### getSwap()

```ts
getSwap(swapId: string): Promise<unknown>;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `swapId` | `string` |

###### Returns

`Promise`&lt;`unknown`&gt;

##### listSwaps()

```ts
listSwaps(query: unknown): Promise<unknown>;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `query` | `unknown` |

###### Returns

`Promise`&lt;`unknown`&gt;

##### revealClaim()

```ts
revealClaim(swapId: string, body: unknown): Promise<void>;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `swapId` | `string` |
| `body` | `unknown` |

###### Returns

`Promise`&lt;`void`&gt;

##### revealPreimage()

```ts
revealPreimage(swapId: string, body: unknown): Promise<void>;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `swapId` | `string` |
| `body` | `unknown` |

###### Returns

`Promise`&lt;`void`&gt;

***

### Signals

A push channel naming swaps worth reading sooner; resolves `undefined` while it is down.

#### Methods

##### next()

```ts
next(): Promise<string | undefined>;
```

###### Returns

`Promise`&lt;`string` \| `undefined`&gt;

***

### SwapRecord

One swap as the driver keeps it: both ends frozen at acceptance, the status,
and what the chain cannot report. The JSON form of the Rust record, and the
document a [SwapStore](#swapstore) holds. Amounts are sats as decimal strings.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-base_fee-1"></a> `base_fee` | `string` | The server's split of `send_amount - receive_amount`: `base_fee` for its transactions, `service_fee` the rest. |
| <a id="property-dest"></a> `dest` | [`Endpoint`](#endpoint) | - |
| <a id="property-ended_at"></a> `ended_at` | `number` \| `null` | When the status first became final: the driver's clock, or the server's on restore. |
| <a id="property-funds"></a> `funds` | [`Fund`](#fund)[] | Swap-in: what under the script is the user's, as the last pass left it. |
| <a id="property-payment_hash"></a> `payment_hash` | `number`[] | - |
| <a id="property-preimage-1"></a> `preimage` | `number`[] \| `null` | The preimage once seen and hashed to `payment_hash`: a swap-in's from the server, a swap-out's own from a claim of ours on chain. |
| <a id="property-receive_amount-1"></a> `receive_amount` | `string` | - |
| <a id="property-send_amount-1"></a> `send_amount` | `string` | - |
| <a id="property-sent"></a> `sent` | [`Sent`](#sent) | - |
| <a id="property-server_cancel"></a> `server_cancel` | [`ServerCancel`](#servercancel) \| `null` | The server's cancel, kept from the first pass that read one. |
| <a id="property-server_status"></a> `server_status` | `string` \| `null` | The server's status label as the last pass that reached it read it. Shown, never decided on. |
| <a id="property-service_fee-1"></a> `service_fee` | `string` | - |
| <a id="property-source"></a> `source` | [`Endpoint`](#endpoint) | - |
| <a id="property-started_at"></a> `started_at` | `number` | The driver's own clock at acceptance. |
| <a id="property-status"></a> `status` | [`DriverStatus`](#driverstatus) | - |
| <a id="property-swap_id"></a> `swap_id` | `string` | - |
| <a id="property-version"></a> `version` | `number` | - |

***

### SwapStore

Durable swap records, wanted by id and nothing else. Back it with anything
that outlives the page, keeping each record as the object given: a future
SDK can upgrade old records only if they kept their self-describing form.

#### Methods

##### list()

```ts
list(): Promise<string[]>;
```

Every stored `swap_id`, without reading the records, so one that fails
to load stops only its own swap.

###### Returns

`Promise`&lt;`string`[]&gt;

##### load()

```ts
load(swapId: string): Promise<SwapRecord | undefined>;
```

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `swapId` | `string` |

###### Returns

`Promise`&lt;[`SwapRecord`](#swaprecord) \| `undefined`&gt;

##### save()

```ts
save(record: SwapRecord): Promise<void>;
```

Replaces any record already stored under the same `swap_id`.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `record` | [`SwapRecord`](#swaprecord) |

###### Returns

`Promise`&lt;`void`&gt;

## Type Aliases

### Cancellation

```ts
type Cancellation = "Requested" | "TooLate";
```

The answer to [SwapHandle.cancel](#cancel): `Requested` means the swap is
canceled or closes when the server confirms it, and `TooLate` means the
payment landed first and the swap continues.

***

### CancelReason

```ts
type CancelReason = 
  | {
  kind: "ServerCanceled";
} & ServerCancel
  | {
  kind: "CsvMatured";
}
  | {
  kind: "ServerRefunded";
}
  | {
  at: Height;
  kind: "ClaimWindowClosed";
};
```

Why a swap ended `Canceled`, named by `kind`. `ServerCanceled`: the server
gave the swap up, with its [ServerCancel](#servercancel) words beside. `CsvMatured`:
a swap-in's deposit matured its CSV with no word from the server.
`ServerRefunded`: the server's CSV refund took a swap-out's opening.
`ClaimWindowClosed`: less than `claimMargin` of a swap-out's CSV was left
before its preimage went out, so the payment fails back once the server's
CSV refund takes the opening, from block `at`.

***

### DriverErrorCode

```ts
type DriverErrorCode = 
  | "Internal"
  | "BadRequest"
  | "NotImplemented"
  | "QuoteExpired"
  | "SwapNotFound"
  | "SwapAlreadyExists"
  | "TransitionRejected"
  | "PolicyError"
  | "TooManyConnections"
  | "Unavailable"
  | "Forbidden"
  | "PayloadTooLarge"
  | "InsufficientLiquidity"
  | "Transport"
  | "Chain"
  | "Store"
  | "Wallet"
  | "Destination"
  | "Refused"
  | "Unsigned"
  | "BadSignature"
  | "ChainNotConfigured"
  | "UnknownSwap"
  | "Unsupported"
  | "WrongSwapType"
  | "NotQuotable"
  | "AmountOutOfRange"
  | "Invalid";
```

The kind of a `DriverError`. The server's codes keep the server's names;
the rest name what failed on this side.

***

### DriverStatus

```ts
type DriverStatus = 
  | {
  status: "Restoring";
}
  | {
  cancel_sent: boolean;
  expires_at: Timestamp;
  status: "Funding";
}
  | {
  confs: number;
  fee_rate_spike?: number;
  max_fee_rate: number;
  needed: number;
  status: "Confirming";
  txid: string;
}
  | {
  claim: string | null;
  confs: number;
  fee: string | null;
  opening: string;
  status: "Claiming";
}
  | {
  fee: string | null;
  opening: string;
  payout: string | null;
  status: "Completed";
}
  | {
  opening: string | null;
  reason: CancelReason;
  status: "Canceled";
};
```

Where a swap stands, resolved from the chain on every pass. `status` names
the variant and its fields sit beside it, so `status.status === "Canceled"`
narrows. Deadlines are absolute, so two quiet passes produce an equal
status. `expires_at` is the time to pay by: a swap-in's is `minConfs + 1`
blocks before its invoice expires, a swap-out's no later than the expiry of
its claim invoice. `Restoring` is no pass's answer: [Driver.restore](#restore)
sets it and the first pass replaces it. `fee` is what a swap-out's claim pays, in sats
as a decimal string, read from the transaction on chain; `null` on a
swap-in's `Completed`, whose payout is the server's. `Confirming` carries
the chain's `maxFeeRate` as `max_fee_rate`, and a swap-out's carries
`fee_rate_spike`, the fee estimate in sat/vB, while it is above that and
holds the preimage, and leaves it out otherwise.

***

### FundState

```ts
type FundState = 
  | {
  at: Height | null;
  state: "Locked";
}
  | {
  state: "Refundable";
}
  | {
  state: "Dust";
}
  | {
  fee: string | null;
  state: "Refunding";
  txid: string;
}
  | {
  fee: string | null;
  state: "Refunded";
  txid: string;
};
```

Where the refund of one output of the user's stands, named by `state`.
`Locked` is under the CSV until block `at`, or unconfirmed (`null`);
`Refundable` is spendable and worth its refund, so the next refund takes it;
`Dust` is spendable but not worth its fee plus the smallest refund output
at the last rate, and `run` refunds it once the rate falls far enough;
`Refunding` and `Refunded` are in our refund `txid`, out of
a block and in one. Their `fee` is the whole refund transaction's, in sats
as a decimal string, so every fund it took shows the same one: group by
`txid` before adding fees up.

***

### Height

```ts
type Height = number;
```

A block height on the swap's chain.

***

### RunState

```ts
type RunState = "Idle" | "Running" | "Stopping" | "Stopped";
```

Where `run` is. `Stopped` is final.

***

### TargetAmount

```ts
type TargetAmount = 
  | {
  receive: bigint;
}
  | {
  send: bigint;
};
```

The end a quote is pinned by: the amount to receive, or the amount to send, in sats.

***

### Timestamp

```ts
type Timestamp = number;
```

Unix seconds.
