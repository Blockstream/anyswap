Client-wide setup: the wallet and the chain parameters the SDK needs.

Build one [SwapContext](#swapcontext) at startup and pass it to every swap method that
touches keys or the chain. The swap already knows its own chain, so the caller
never chooses anything per call; the context supplies what a swap cannot know
by itself.

Liquid swaps additionally need the network's genesis block hash and its policy
asset (L-BTC). Read both from your own chain source, never from the swap
server: the genesis hash is committed in every taproot signature the wallet
produces, and the policy asset decides what a transaction pays its fee in, so a
server that supplied them could make the user sign transactions that never
confirm. A context built without them serves Bitcoin swaps only; Liquid methods
on it throw.

## Classes

### SwapContext

The wallet and chain parameters shared by every swap of one client.

#### Constructors

##### Constructor

```ts
new SwapContext(wallet: Wallet, liquid?: LiquidParams): SwapContext;
```

###### Parameters

| Parameter | Type | Description |
| ------ | ------ | ------ |
| `wallet` | [`Wallet`](Wallet.md#wallet) | Wallet whose keys sign this client's swaps. |
| `liquid?` | [`LiquidParams`](#liquidparams) | Liquid chain parameters. Omit for a Bitcoin-only client. |

###### Returns

[`SwapContext`](#swapcontext)

#### Properties

| Property | Modifier | Type | Description |
| ------ | ------ | ------ | ------ |
| <a id="property-inner"></a> `inner` | `readonly` | `SwapContext` | Underlying wasm context. Exposed so swap helpers can forward it. |

#### Accessors

##### network

###### Get Signature

```ts
get network(): string;
```

Network this client operates on, from the wallet.

###### Returns

`string`

## Interfaces

### LiquidParams

Liquid chain parameters. Both come from your own chain source, never from the
swap server - see the module docs.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-genesishash"></a> `genesisHash` | `string` | Genesis block hash of the Liquid network, hex-encoded. |
| <a id="property-policyasset"></a> `policyAsset` | `string` | Policy (fee) asset id of the Liquid network, hex-encoded. L-BTC. |
