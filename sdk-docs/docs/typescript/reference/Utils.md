Mnemonic, invoice, swap-id, protocol-version, and swap-fee utilities - thin
wrappers over the wasm free functions, plus a client-side invoice validator.

## Functions

### amountMsatFromInvoice()

```ts
function amountMsatFromInvoice(bolt11: string): bigint;
```

Extracts the amount from a BOLT11 invoice, in millisatoshis. Throws if the
invoice does not specify an amount.

#### Parameters

| Parameter | Type |
| ------ | ------ |
| `bolt11` | `string` |

#### Returns

`bigint`

***

### destAsset()

```ts
function destAsset(
   info: InfoResponse, 
   chain: Chain, 
   asset?: string): {
  dest_asset_id: string;
  dest_chain: Chain;
};
```

The `dest_chain` and `dest_asset_id` pair naming a swap's on-chain
destination.

The two fields have to agree, so they come from one lookup rather than being
written out separately. The asset id is read from
[getInfo](Api.md#getinfo) because the chains identify their
native asset differently - Bitcoin uses the `"native"` sentinel, Liquid the
asset's 32-byte hex, which is network-specific - so resolving it from the
server's policy keeps it correct on every network. A Liquid
[LiquidParams](Context.md#liquidparams)' `policyAsset` is this id.

Pass `asset` only to name a non-native asset by its policy label.

#### Parameters

| Parameter | Type |
| ------ | ------ |
| `info` | [`InfoResponse`](Types.md#inforesponse) |
| `chain` | [`Chain`](Types.md#chain) |
| `asset?` | `string` |

#### Returns

```ts
{
  dest_asset_id: string;
  dest_chain: Chain;
}
```

| Name | Type |
| ------ | ------ |
| `dest_asset_id` | `string` |
| `dest_chain` | [`Chain`](Types.md#chain) |

***

### expiryAtFromInvoice()

```ts
function expiryAtFromInvoice(bolt11: string): number;
```

Extracts the invoice expiration time, as seconds since the Unix epoch. Throws
if the invoice does not specify an expiry.

#### Parameters

| Parameter | Type |
| ------ | ------ |
| `bolt11` | `string` |

#### Returns

`number`

***

### generateMnemonic()

```ts
function generateMnemonic(wordCount: number): string;
```

Generates a random BIP39 mnemonic. Pass the result to
[Wallet.fromMnemonic](Wallet.md#frommnemonic) and show it to the
user for backup.

#### Parameters

| Parameter | Type | Description |
| ------ | ------ | ------ |
| `wordCount` | `number` | Phrase length: `12`, `15`, `18`, `21`, or `24`. 24 words (256 bits of entropy) is the recommended default. |

#### Returns

`string`

***

### networkFromInvoice()

```ts
function networkFromInvoice(bolt11: string): string;
```

Extracts the network string from a BOLT11 invoice (e.g. `"regtest"`).

#### Parameters

| Parameter | Type |
| ------ | ------ |
| `bolt11` | `string` |

#### Returns

`string`

***

### newSwapId()

```ts
function newSwapId(): string;
```

Generates a new random swap id: a UUIDv4 as a 36-character hyphenated string.
Pass to [AnySwapClient.createSwap](Api.md#createswap).

#### Returns

`string`

***

### paymentHashFromInvoice()

```ts
function paymentHashFromInvoice(bolt11: string): string;
```

Extracts the payment hash from a BOLT11 invoice, as hex.

#### Parameters

| Parameter | Type |
| ------ | ------ |
| `bolt11` | `string` |

#### Returns

`string`

***

### protocolVersion()

```ts
function protocolVersion(): string;
```

The protocol version this SDK speaks: the release's version, of which only the
major and minor version must match the server's. Pass as `protocol_version` to
[AnySwapClient.createSwap](Api.md#createswap); a server on
another minor version refuses the swap.

#### Returns

`string`

***

### swapAmounts()

```ts
function swapAmounts(
   receiveAmount: bigint, 
   info: InfoResponse, 
   source: SwapEndpoint, 
   dest: SwapEndpoint): {
  sendAmount: bigint;
  swapFee: bigint;
};
```

The swap fee and total send amount for a swap delivering `receiveAmount` from
`source` to `dest`, with the rates read from
[getInfo](Api.md#getinfo). The base fee is the sum of the
on-chain legs (`fee_estimates[source].source` if the source is on-chain, plus
`fee_estimates[dest].dest` if the destination is on-chain); the service rate
comes from the destination (`policy.chain[dest].service_ppm`, or
`policy.lightning.service_ppm` when the destination is Lightning). Returns
satoshis, with `sendAmount = receiveAmount + swapFee`. This one formula covers a
SwapIn (`chain -> lightning`), SwapOut (`lightning -> chain`), and SwapChain
(`chain -> chain`).

#### Parameters

| Parameter | Type |
| ------ | ------ |
| `receiveAmount` | `bigint` |
| `info` | [`InfoResponse`](Types.md#inforesponse) |
| `source` | [`SwapEndpoint`](Types.md#swapendpoint) |
| `dest` | [`SwapEndpoint`](Types.md#swapendpoint) |

#### Returns

```ts
{
  sendAmount: bigint;
  swapFee: bigint;
}
```

| Name | Type |
| ------ | ------ |
| `sendAmount` | `bigint` |
| `swapFee` | `bigint` |

***

### validateInvoice()

```ts
function validateInvoice(
   bolt11: string, 
   info: InfoResponse, 
   source: SwapEndpoint): void;
```

Validates a BOLT11 claim invoice for a swap into Lightning (a SwapIn) whose
on-chain `source`, against [getInfo](Api.md#getinfo): the
network matches `policy.network` (case-insensitively), the amount is a whole
number of sats, and the invoice does not expire before the deposit can confirm
on the source chain (`> (confs + 1) * block_time` seconds). The driver's quote
checks the amount limits. Throws on failure.

#### Parameters

| Parameter | Type |
| ------ | ------ |
| `bolt11` | `string` |
| `info` | [`InfoResponse`](Types.md#inforesponse) |
| `source` | [`SwapEndpoint`](Types.md#swapendpoint) |

#### Returns

`void`

***

### validateMnemonic()

```ts
function validateMnemonic(mnemonic: string): boolean;
```

Validates a BIP39 mnemonic. Returns `true` if valid, `false` otherwise.

#### Parameters

| Parameter | Type |
| ------ | ------ |
| `mnemonic` | `string` |

#### Returns

`boolean`
