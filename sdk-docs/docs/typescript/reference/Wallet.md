BIP39/BIP32 HD wallet for swap key management.

The [Driver](Driver.md#driver) derives every swap's keys and preimage from this
wallet and signs with them inside the SDK, so a caller needs the mnemonic, the
[Wallet.userId](#userid) and the identity key that authenticates protected API
requests. The wallet never holds user balances (deposits go directly to the P2TR
swap address).

## Key derivation

The BIP39 seed is first domain-separated as
`HMAC-SHA512("anyswap/user-wallet/v2", seed)`, so no BIP32 wallet on the same
mnemonic shares a key with this one. Hardened branches `m/84'/COIN'/<branch>'`
are derived from the result, and each per-swap key is
`SHA-256(xpriv_secret_bytes || swap_id_bytes)` on its branch.

Branches 0 and 1 hold the source refund and destination claim keys, 2 and 3
the source and destination blinding keys (Liquid only), 4 the preimages and
5 the identity key. Every role has its own branch, so the two sides of a swap
under one `swap_id` never share a key. `COIN` is `0` for mainnet, `1` for
testnet/signet/regtest.

`user_id` is `SHA-256` of the identity pubkey (the public key on branch 5),
not of the xpub itself.

Wallets from SDKs before 1.3.0 derived from the untagged seed, so the
same mnemonic gives a different `user_id` and different keys here.

## Classes

### Wallet

BIP39/BIP32 HD wallet for swap key management.

#### Properties

| Property | Modifier | Type | Description |
| ------ | ------ | ------ | ------ |
| <a id="property-inner"></a> `inner` | `readonly` | `Wallet` | Underlying wasm wallet, which [Context!SwapContext](Context.md#swapcontext) forwards as it is. |

#### Accessors

##### identityPrivkey

###### Get Signature

```ts
get identityPrivkey(): string;
```

Stable identity private key used to authenticate protected API requests.

###### Returns

`string`

##### identityPubkey

###### Get Signature

```ts
get identityPubkey(): string;
```

Stable compressed identity public key sent with protected API requests.

###### Returns

`string`

##### network

###### Get Signature

```ts
get network(): string;
```

Network string the wallet was created for.

###### Returns

`string`

##### userId

###### Get Signature

```ts
get userId(): string;
```

Stable identifier sent to the server in every request that needs user identification.
Derived as `SHA-256` of the identity pubkey (the branch-5 public key) - the server
never sees that pubkey or the xpub, and cannot link `user_id` to any individual swap key.

###### Returns

`string`

#### Methods

##### destClaimPubkey()

```ts
destClaimPubkey(swapId: string): string;
```

Derives the swap-out public key that goes into the swap script on the
destination, sent as `dest_claim_pubkey`.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `swapId` | `string` |

###### Returns

`string`

##### sourceRefundPubkey()

```ts
sourceRefundPubkey(swapId: string): string;
```

Derives the swap-in public key that goes into the swap script on the
source, sent as `source_refund_pubkey`.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `swapId` | `string` |

###### Returns

`string`

##### fromMnemonic()

```ts
static fromMnemonic(mnemonic: string, network: SwapNetwork): Wallet;
```

Restores a wallet from an existing BIP39 mnemonic. Only the mnemonic is needed for
recovery - the server holds the swap history, queryable via
[AnySwapClient.getSwaps](Api.md#getswaps).

###### Parameters

| Parameter | Type | Description |
| ------ | ------ | ------ |
| `mnemonic` | `string` | 12- or 24-word BIP39 phrase. Generate a fresh one with [generateMnemonic](Utils.md#generatemnemonic). |
| `network` | [`SwapNetwork`](Types.md#swapnetwork) | Target [SwapNetwork](Types.md#swapnetwork) for derivation. |

###### Returns

[`Wallet`](#wallet)
