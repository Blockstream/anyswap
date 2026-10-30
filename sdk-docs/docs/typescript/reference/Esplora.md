Esplora blockchain clients for fetching on-chain data needed during swap execution.

The driver's `ChainClient` on each chain: it reads address histories and spends,
broadcasts the user's claim or refund, and estimates fee rates.

One client per chain: [BitcoinEsplora](#bitcoinesplora) and [LiquidEsplora](#liquidesplora). The two
chains need separate backends and floor their fee estimates at different relay
minimums.

Optional: both implement [Esplora](#esplora), a structural interface, so anything
with those methods replaces them.

## Classes

### BitcoinEsplora

Esplora client for Bitcoin.

#### Extends

- `EsploraBase`

#### Constructors

##### Constructor

```ts
new BitcoinEsplora(
   url: string, 
   fetchFn?: (input: string | URL | Request, init?: RequestInit) => Promise<Response>, 
   options?: EsploraOptions): BitcoinEsplora;
```

###### Parameters

| Parameter | Type | Default value | Description |
| ------ | ------ | ------ | ------ |
| `url` | `string` | `undefined` | Esplora base URL including any path prefix. |
| `fetchFn` | (`input`: `string` \| `URL` \| `Request`, `init?`: `RequestInit`) => `Promise`&lt;`Response`&gt; | `fetch` | Optional `fetch` implementation. Defaults to the global `fetch`. |
| `options` | [`EsploraOptions`](#esploraoptions) | `{}` | Optional request settings, see [EsploraOptions](#esploraoptions). |

###### Returns

[`BitcoinEsplora`](#bitcoinesplora)

###### Inherited from

```ts
EsploraBase.constructor
```

#### Methods

##### addressHistory()

```ts
addressHistory(address: string): Promise<HistoryTx[]>;
```

Every transaction paying or spending `address`, mempool included, with its
raw hex and confirming height. Pages through the whole history, so a
deposit can never hide behind unrelated transactions; a transaction the
indexer has dropped since listing it is left out. Bodies are fetched one
at a time and kept as each arrives, so a later read fetches only what it
has not seen, and one that fails partway leaves the next less to fetch.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `address` | `string` |

###### Returns

`Promise`&lt;[`HistoryTx`](#historytx)[]&gt;

###### Inherited from

```ts
EsploraBase.addressHistory
```

##### addressTxs()

```ts
addressTxs(address: string, lastSeen?: string): Promise<EsploraTx[]>;
```

Transactions paying an address, newest first. `lastSeen` pages past 25.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `address` | `string` |
| `lastSeen?` | `string` |

###### Returns

`Promise`&lt;[`EsploraTx`](#esploratx)[]&gt;

###### Inherited from

```ts
EsploraBase.addressTxs
```

##### broadcast()

```ts
broadcast(txHex: string): Promise<string>;
```

Broadcasts a raw transaction (hex). Returns the broadcast txid.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txHex` | `string` |

###### Returns

`Promise`&lt;`string`&gt;

###### Inherited from

```ts
EsploraBase.broadcast
```

##### getBlockHash()

```ts
getBlockHash(height: number): Promise<string>;
```

Block hash at a given height, hex-encoded.

Height `0` is the genesis hash a
[SwapContext](Context.md#swapcontext) needs for Liquid. Read it from your
own Esplora, never from the swap server: every taproot signature the wallet
produces commits to it.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `height` | `number` |

###### Returns

`Promise`&lt;`string`&gt;

###### Inherited from

```ts
EsploraBase.getBlockHash
```

##### getFeeRate()

```ts
getFeeRate(target: number): Promise<number>;
```

Estimates the fee rate in sat/vB for a given confirmation target (in
blocks), fractional, floored at the chain's relay minimum. The indexer
reports a fixed set of targets, so this takes the cheapest one that still
confirms within `target`. With none, it throws on Bitcoin and takes the
relay minimum on Liquid.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `target` | `number` |

###### Returns

`Promise`&lt;`number`&gt;

###### Inherited from

```ts
EsploraBase.getFeeRate
```

##### getHeight()

```ts
getHeight(): Promise<number>;
```

Current chain tip height.

###### Returns

`Promise`&lt;`number`&gt;

###### Inherited from

```ts
EsploraBase.getHeight
```

##### getOutspend()

```ts
getOutspend(txid: string, vout: number): Promise<Outspend>;
```

Whether a specific output has been spent, and by which transaction.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |
| `vout` | `number` |

###### Returns

`Promise`&lt;[`Outspend`](#outspend-3)&gt;

###### Inherited from

```ts
EsploraBase.getOutspend
```

##### getTx()

```ts
getTx(txid: string): Promise<string | null>;
```

Returns the raw transaction as hex, or `null` if not found.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |

###### Returns

`Promise`&lt;`string` \| `null`&gt;

###### Inherited from

```ts
EsploraBase.getTx
```

##### getTxConfirmations()

```ts
getTxConfirmations(txid: string): Promise<number>;
```

Number of confirmations for a transaction, or `0` if unconfirmed.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |

###### Returns

`Promise`&lt;`number`&gt;

###### Inherited from

```ts
EsploraBase.getTxConfirmations
```

##### getTxFee()

```ts
getTxFee(txid: string): Promise<number | null>;
```

Miner fee paid by a transaction, in satoshis, or `null` if not found.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |

###### Returns

`Promise`&lt;`number` \| `null`&gt;

###### Inherited from

```ts
EsploraBase.getTxFee
```

##### getTxStatus()

```ts
getTxStatus(txid: string): Promise<TxStatus>;
```

Confirmation status of a transaction.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |

###### Returns

`Promise`&lt;[`TxStatus`](#txstatus)&gt;

###### Inherited from

```ts
EsploraBase.getTxStatus
```

##### outspend()

```ts
outspend(txid: string, vout: number): Promise<HistoryTx | undefined>;
```

The transaction spending output `vout` of `txid`, mempool included, with
its raw hex and confirming height, or `undefined` while the output is
unspent. Read from the output itself, so a spend is found however many
other transactions crowd the address's mempool history, which Esplora
caps at 50. A spend the indexer no longer serves counts as none.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |
| `vout` | `number` |

###### Returns

`Promise`&lt;[`HistoryTx`](#historytx) \| `undefined`&gt;

###### Inherited from

```ts
EsploraBase.outspend
```

##### scripthashTxs()

```ts
scripthashTxs(scriptHash: string, lastSeen?: string): Promise<EsploraTx[]>;
```

Transactions paying a scripthash, newest first. `lastSeen` pages past 25.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `scriptHash` | `string` |
| `lastSeen?` | `string` |

###### Returns

`Promise`&lt;[`EsploraTx`](#esploratx)[]&gt;

###### Inherited from

```ts
EsploraBase.scripthashTxs
```

***

### LiquidEsplora

Esplora client for Liquid.

#### Extends

- `EsploraBase`

#### Constructors

##### Constructor

```ts
new LiquidEsplora(
   url: string, 
   fetchFn?: (input: string | URL | Request, init?: RequestInit) => Promise<Response>, 
   options?: EsploraOptions): LiquidEsplora;
```

###### Parameters

| Parameter | Type | Default value | Description |
| ------ | ------ | ------ | ------ |
| `url` | `string` | `undefined` | Esplora base URL including any path prefix. |
| `fetchFn` | (`input`: `string` \| `URL` \| `Request`, `init?`: `RequestInit`) => `Promise`&lt;`Response`&gt; | `fetch` | Optional `fetch` implementation. Defaults to the global `fetch`. |
| `options` | [`EsploraOptions`](#esploraoptions) | `{}` | Optional request settings, see [EsploraOptions](#esploraoptions). |

###### Returns

[`LiquidEsplora`](#liquidesplora)

###### Inherited from

```ts
EsploraBase.constructor
```

#### Methods

##### addressHistory()

```ts
addressHistory(address: string): Promise<HistoryTx[]>;
```

Every transaction paying or spending `address`, mempool included, with its
raw hex and confirming height. Pages through the whole history, so a
deposit can never hide behind unrelated transactions; a transaction the
indexer has dropped since listing it is left out. Bodies are fetched one
at a time and kept as each arrives, so a later read fetches only what it
has not seen, and one that fails partway leaves the next less to fetch.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `address` | `string` |

###### Returns

`Promise`&lt;[`HistoryTx`](#historytx)[]&gt;

###### Inherited from

```ts
EsploraBase.addressHistory
```

##### addressTxs()

```ts
addressTxs(address: string, lastSeen?: string): Promise<EsploraTx[]>;
```

Transactions paying an address, newest first. `lastSeen` pages past 25.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `address` | `string` |
| `lastSeen?` | `string` |

###### Returns

`Promise`&lt;[`EsploraTx`](#esploratx)[]&gt;

###### Inherited from

```ts
EsploraBase.addressTxs
```

##### broadcast()

```ts
broadcast(txHex: string): Promise<string>;
```

Broadcasts a raw transaction (hex). Returns the broadcast txid.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txHex` | `string` |

###### Returns

`Promise`&lt;`string`&gt;

###### Inherited from

```ts
EsploraBase.broadcast
```

##### getBlockHash()

```ts
getBlockHash(height: number): Promise<string>;
```

Block hash at a given height, hex-encoded.

Height `0` is the genesis hash a
[SwapContext](Context.md#swapcontext) needs for Liquid. Read it from your
own Esplora, never from the swap server: every taproot signature the wallet
produces commits to it.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `height` | `number` |

###### Returns

`Promise`&lt;`string`&gt;

###### Inherited from

```ts
EsploraBase.getBlockHash
```

##### getFeeRate()

```ts
getFeeRate(target: number): Promise<number>;
```

Estimates the fee rate in sat/vB for a given confirmation target (in
blocks), fractional, floored at the chain's relay minimum. The indexer
reports a fixed set of targets, so this takes the cheapest one that still
confirms within `target`. With none, it throws on Bitcoin and takes the
relay minimum on Liquid.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `target` | `number` |

###### Returns

`Promise`&lt;`number`&gt;

###### Inherited from

```ts
EsploraBase.getFeeRate
```

##### getHeight()

```ts
getHeight(): Promise<number>;
```

Current chain tip height.

###### Returns

`Promise`&lt;`number`&gt;

###### Inherited from

```ts
EsploraBase.getHeight
```

##### getOutspend()

```ts
getOutspend(txid: string, vout: number): Promise<Outspend>;
```

Whether a specific output has been spent, and by which transaction.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |
| `vout` | `number` |

###### Returns

`Promise`&lt;[`Outspend`](#outspend-3)&gt;

###### Inherited from

```ts
EsploraBase.getOutspend
```

##### getTx()

```ts
getTx(txid: string): Promise<string | null>;
```

Returns the raw transaction as hex, or `null` if not found.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |

###### Returns

`Promise`&lt;`string` \| `null`&gt;

###### Inherited from

```ts
EsploraBase.getTx
```

##### getTxConfirmations()

```ts
getTxConfirmations(txid: string): Promise<number>;
```

Number of confirmations for a transaction, or `0` if unconfirmed.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |

###### Returns

`Promise`&lt;`number`&gt;

###### Inherited from

```ts
EsploraBase.getTxConfirmations
```

##### getTxFee()

```ts
getTxFee(txid: string): Promise<number | null>;
```

Miner fee paid by a transaction, in satoshis, or `null` if not found.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |

###### Returns

`Promise`&lt;`number` \| `null`&gt;

###### Inherited from

```ts
EsploraBase.getTxFee
```

##### getTxStatus()

```ts
getTxStatus(txid: string): Promise<TxStatus>;
```

Confirmation status of a transaction.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |

###### Returns

`Promise`&lt;[`TxStatus`](#txstatus)&gt;

###### Inherited from

```ts
EsploraBase.getTxStatus
```

##### outspend()

```ts
outspend(txid: string, vout: number): Promise<HistoryTx | undefined>;
```

The transaction spending output `vout` of `txid`, mempool included, with
its raw hex and confirming height, or `undefined` while the output is
unspent. Read from the output itself, so a spend is found however many
other transactions crowd the address's mempool history, which Esplora
caps at 50. A spend the indexer no longer serves counts as none.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |
| `vout` | `number` |

###### Returns

`Promise`&lt;[`HistoryTx`](#historytx) \| `undefined`&gt;

###### Inherited from

```ts
EsploraBase.outspend
```

##### scripthashTxs()

```ts
scripthashTxs(scriptHash: string, lastSeen?: string): Promise<EsploraTx[]>;
```

Transactions paying a scripthash, newest first. `lastSeen` pages past 25.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `scriptHash` | `string` |
| `lastSeen?` | `string` |

###### Returns

`Promise`&lt;[`EsploraTx`](#esploratx)[]&gt;

###### Inherited from

```ts
EsploraBase.scripthashTxs
```

## Interfaces

### Esplora

The chain reads a swap needs, as [BitcoinEsplora](#bitcoinesplora) and
[LiquidEsplora](#liquidesplora) provide them.

Program against this rather than a concrete class, and implement it yourself
to drive the SDK from your own indexer.

#### Methods

##### addressHistory()?

```ts
optional addressHistory(address: string): Promise<HistoryTx[]>;
```

Every transaction paying or spending an address, mempool included. Needed
only to serve the driver as its `ChainClient`.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `address` | `string` |

###### Returns

`Promise`&lt;[`HistoryTx`](#historytx)[]&gt;

##### addressTxs()

```ts
addressTxs(address: string, lastSeen?: string): Promise<EsploraTx[]>;
```

Transactions paying an address, newest first.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `address` | `string` |
| `lastSeen?` | `string` |

###### Returns

`Promise`&lt;[`EsploraTx`](#esploratx)[]&gt;

##### broadcast()

```ts
broadcast(txHex: string): Promise<string>;
```

Broadcasts a raw transaction (hex). Returns the broadcast txid.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txHex` | `string` |

###### Returns

`Promise`&lt;`string`&gt;

##### getBlockHash()

```ts
getBlockHash(height: number): Promise<string>;
```

Block hash at a given height, hex-encoded.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `height` | `number` |

###### Returns

`Promise`&lt;`string`&gt;

##### getFeeRate()

```ts
getFeeRate(target: number): Promise<number>;
```

Fee rate in sat/vB for a confirmation target, in blocks. Fractional.

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

Current chain tip height.

###### Returns

`Promise`&lt;`number`&gt;

##### getOutspend()

```ts
getOutspend(txid: string, vout: number): Promise<Outspend>;
```

Whether a specific output has been spent, and by which transaction.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |
| `vout` | `number` |

###### Returns

`Promise`&lt;[`Outspend`](#outspend-3)&gt;

##### getTx()

```ts
getTx(txid: string): Promise<string | null>;
```

Raw transaction hex, or `null` if not found.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |

###### Returns

`Promise`&lt;`string` \| `null`&gt;

##### getTxConfirmations()

```ts
getTxConfirmations(txid: string): Promise<number>;
```

Number of confirmations for a transaction, or `0` if unconfirmed.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |

###### Returns

`Promise`&lt;`number`&gt;

##### getTxFee()

```ts
getTxFee(txid: string): Promise<number | null>;
```

Miner fee paid by a transaction, in satoshis, or `null` if not found.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |

###### Returns

`Promise`&lt;`number` \| `null`&gt;

##### getTxStatus()

```ts
getTxStatus(txid: string): Promise<TxStatus>;
```

Confirmation status of a transaction.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |

###### Returns

`Promise`&lt;[`TxStatus`](#txstatus)&gt;

##### outspend()?

```ts
optional outspend(txid: string, vout: number): Promise<HistoryTx | undefined>;
```

The transaction spending an output, mempool included, or `undefined`
while it is unspent. Needed only to serve the driver as its
`ChainClient`.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `txid` | `string` |
| `vout` | `number` |

###### Returns

`Promise`&lt;[`HistoryTx`](#historytx) \| `undefined`&gt;

##### scripthashTxs()

```ts
scripthashTxs(scriptHash: string, lastSeen?: string): Promise<EsploraTx[]>;
```

Transactions paying a scripthash, newest first.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `scriptHash` | `string` |
| `lastSeen?` | `string` |

###### Returns

`Promise`&lt;[`EsploraTx`](#esploratx)[]&gt;

***

### EsploraOptions

How an Esplora client makes its requests.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-signal"></a> `signal?` | `AbortSignal` | Aborts every request of this client, beside the timeout, for good once aborted. Aborting is safe at any time, but a swap-out that is `Claiming` needs passes that finish, or the server's CSV refund can take the opening. |
| <a id="property-timeout"></a> `timeout?` | `number` | Seconds one request may go unanswered before it fails, above zero. Defaults to 30. A `fetch` passed in gets the timeout in `init.signal`, so its own error handling sees the `TimeoutError`. |

***

### EsploraTx

Minimal transaction record returned by address- and scripthash-lookup endpoints.

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-status"></a> `status` | [`TxStatus`](#txstatus) |
| <a id="property-txid"></a> `txid` | `string` |

***

### HistoryTx

One transaction of an address's history: its raw hex and confirming height,
`null` in the mempool. What the WASM driver's `ChainClient` reads.

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-height"></a> `height` | `number` \| `null` |
| <a id="property-hex"></a> `hex` | `string` |

***

### Outspend

Whether a specific output has been spent, and - if so - the spending transaction
and its status.

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-spent"></a> `spent` | `boolean` |
| <a id="property-status-1"></a> `status?` | [`TxStatus`](#txstatus) |
| <a id="property-txid-1"></a> `txid?` | `string` |
| <a id="property-vin"></a> `vin?` | `number` |

***

### TxStatus

Transaction confirmation status as returned by esplora.

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-block_height"></a> `block_height?` | `number` |
| <a id="property-confirmed"></a> `confirmed` | `boolean` |
