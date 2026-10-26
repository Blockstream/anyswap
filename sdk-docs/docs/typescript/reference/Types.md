Shared types for API requests, responses, and swap state.

Re-exports the OpenAPI-generated request/response types and enums, and adds
[SwapInStatus](#swapinstatus) and [SwapOutStatus](#swapoutstatus) - discriminated unions keyed by
`status` that model each swap direction's lifecycle, carrying only the fields
available in each state.

## Enumerations

### Chain

#### Enumeration Members

| Enumeration Member | Value |
| ------ | ------ |
| <a id="enumeration-member-bitcoin"></a> `Bitcoin` | `"bitcoin"` |
| <a id="enumeration-member-liquid"></a> `Liquid` | `"liquid"` |

***

### ErrorCode

Machine-readable `code` carried beside `message` in every error response,
so a client never infers semantics from the HTTP status.

#### Enumeration Members

| Enumeration Member | Value |
| ------ | ------ |
| <a id="enumeration-member-badrequest"></a> `BadRequest` | `"BadRequest"` |
| <a id="enumeration-member-forbidden"></a> `Forbidden` | `"Forbidden"` |
| <a id="enumeration-member-insufficientliquidity"></a> `InsufficientLiquidity` | `"InsufficientLiquidity"` |
| <a id="enumeration-member-internal"></a> `Internal` | `"Internal"` |
| <a id="enumeration-member-notimplemented"></a> `NotImplemented` | `"NotImplemented"` |
| <a id="enumeration-member-payloadtoolarge"></a> `PayloadTooLarge` | `"PayloadTooLarge"` |
| <a id="enumeration-member-policyerror"></a> `PolicyError` | `"PolicyError"` |
| <a id="enumeration-member-quoteexpired"></a> `QuoteExpired` | `"QuoteExpired"` |
| <a id="enumeration-member-swapalreadyexists"></a> `SwapAlreadyExists` | `"SwapAlreadyExists"` |
| <a id="enumeration-member-swapnotfound"></a> `SwapNotFound` | `"SwapNotFound"` |
| <a id="enumeration-member-toomanyconnections"></a> `TooManyConnections` | `"TooManyConnections"` |
| <a id="enumeration-member-transitionrejected"></a> `TransitionRejected` | `"TransitionRejected"` |
| <a id="enumeration-member-unavailable"></a> `Unavailable` | `"Unavailable"` |

***

### SwapNetwork

On-chain network for swap operations.

#### Enumeration Members

| Enumeration Member | Value |
| ------ | ------ |
| <a id="enumeration-member-mainnet"></a> `Mainnet` | `"mainnet"` |
| <a id="enumeration-member-regtest"></a> `Regtest` | `"regtest"` |
| <a id="enumeration-member-signet"></a> `Signet` | `"signet"` |
| <a id="enumeration-member-testnet"></a> `Testnet` | `"testnet"` |

***

### SwapType

Swap kind. Directions are relative to Lightning: `SwapIn` is on-chain to
Lightning, `SwapOut` is Lightning to on-chain, `SwapChain` is on-chain to
on-chain across two different chains.

#### Enumeration Members

| Enumeration Member | Value |
| ------ | ------ |
| <a id="enumeration-member-swapchain"></a> `SwapChain` | `"SwapChain"` |
| <a id="enumeration-member-swapin"></a> `SwapIn` | `"SwapIn"` |
| <a id="enumeration-member-swapout"></a> `SwapOut` | `"SwapOut"` |

## Interfaces

### ApiError

Error body returned by the server on a non-2xx response. Branch on `code`
rather than `message` or the HTTP status; the fields are absent only when
the body could not be parsed.

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-code"></a> `code?` | [`ErrorCode`](#errorcode) |
| <a id="property-message"></a> `message?` | `string` |

***

### AssetPolicyResponse

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-asset_id"></a> `asset_id` | `string` |
| <a id="property-max_amount"></a> `max_amount` | `string` |
| <a id="property-min_amount"></a> `min_amount` | `string` |

***

### CancelRequest

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-cancel_message"></a> `cancel_message` | `string` |

***

### ChainPolicyResponse

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-asset"></a> `asset` | `Partial`&lt;`Record`&lt;`string`, [`AssetPolicyResponse`](#assetpolicyresponse)&gt;&gt; | - |
| <a id="property-block_time"></a> `block_time` | `number` | **Min** 0 |
| <a id="property-confs"></a> `confs` | `number` | **Min** 0 |
| <a id="property-dest_csv"></a> `dest_csv` | `number` | CSV timelock on a swap-out opening tx, in this chain's blocks. **Min** 0 |
| <a id="property-min_base_fee_multiplier"></a> `min_base_fee_multiplier` | `number` | Dust-guard floor: minimum receive amount as a multiple of this chain's base fee. **Min** 0 |
| <a id="property-service_ppm"></a> `service_ppm` | `number` | **Min** 0 |
| <a id="property-source_csv"></a> `source_csv` | `number` | CSV timelock on a swap-in opening tx, in this chain's blocks. **Min** 0 |
| <a id="property-timeouts"></a> `timeouts` | [`TimeoutResponse`](#timeoutresponse) | Deadlines this chain enforces as a SwapIn source, in seconds. A swap left in the matching status past its deadline is canceled. SwapOut has no deadline here: once its opening tx confirms, the bound is the `dest_csv` timelock in blocks. |

***

### CreateSwapRequest

Body of [createSwap](Api.md#createswap). As the service
documents it, amounts as decimal strings, except that `webhook.statuses` are
typed to the statuses a swap can reach, so a misspelled one fails to compile.

#### Extends

- `Omit`&lt;`GeneratedCreateSwapRequest`, `"webhook"`&gt;

#### Properties

| Property | Type | Description | Inherited from |
| ------ | ------ | ------ | ------ |
| <a id="property-claim_invoice"></a> `claim_invoice?` | `string` \| `null` | swap-in only. | `Omit.claim_invoice` |
| <a id="property-dest_asset_id"></a> `dest_asset_id?` | `string` \| `null` | - | `Omit.dest_asset_id` |
| <a id="property-dest_blinding_pubkey"></a> `dest_blinding_pubkey?` | `string` \| `null` | swap-out and swap-chain, Liquid only. | `Omit.dest_blinding_pubkey` |
| <a id="property-dest_chain"></a> `dest_chain?` | [`Chain`](#chain) \| `null` | - | `Omit.dest_chain` |
| <a id="property-dest_claim_pubkey"></a> `dest_claim_pubkey?` | `string` \| `null` | swap-out and swap-chain. | `Omit.dest_claim_pubkey` |
| <a id="property-payment_hash"></a> `payment_hash?` | `string` \| `null` | swap-out and swap-chain: preimage hash for the destination HTLC. | `Omit.payment_hash` |
| <a id="property-protocol_version"></a> `protocol_version` | `string` | - | `Omit.protocol_version` |
| <a id="property-quote_id"></a> `quote_id?` | `string` \| `null` | Optional `GET /v1/info` quote locking the base fee. | `Omit.quote_id` |
| <a id="property-receive_amount"></a> `receive_amount` | `string` | - | `Omit.receive_amount` |
| <a id="property-send_amount"></a> `send_amount` | `string` | - | `Omit.send_amount` |
| <a id="property-source_asset_id"></a> `source_asset_id?` | `string` \| `null` | - | `Omit.source_asset_id` |
| <a id="property-source_blinding_pubkey"></a> `source_blinding_pubkey?` | `string` \| `null` | swap-in and swap-chain, Liquid only. | `Omit.source_blinding_pubkey` |
| <a id="property-source_chain"></a> `source_chain?` | [`Chain`](#chain) \| `null` | - | `Omit.source_chain` |
| <a id="property-source_refund_pubkey"></a> `source_refund_pubkey?` | `string` \| `null` | swap-in and swap-chain. | `Omit.source_refund_pubkey` |
| <a id="property-swap_id"></a> `swap_id` | `string` | - | `Omit.swap_id` |
| <a id="property-user_claim_address"></a> `user_claim_address?` | `string` \| `null` | swap-out and swap-chain. | `Omit.user_claim_address` |
| <a id="property-user_id"></a> `user_id` | `string` | - | `Omit.user_id` |
| <a id="property-user_refund_address"></a> `user_refund_address?` | `string` \| `null` | swap-in and swap-chain. | `Omit.user_refund_address` |
| <a id="property-webhook"></a> `webhook?` | [`SwapWebhook`](#swapwebhook)&lt;[`SwapStatus`](#swapstatus)&gt; \| `null` | Status webhook to register with the swap. | - |

***

### FeeEstimate

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-dest"></a> `dest` | `string` |
| <a id="property-source"></a> `source` | `string` |

***

### InfoResponse

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-fee_estimates"></a> `fee_estimates?` | `Partial`&lt;`Record`&lt;`"bitcoin"` \| `"liquid"`, [`FeeEstimate`](#feeestimate)&gt;&gt; | - |
| <a id="property-online"></a> `online` | `boolean` | Whether the Lightning node pays and receives; `false` while it runs in offline mode. |
| <a id="property-policy"></a> `policy` | [`Policy`](#policy) | - |
| <a id="property-quote_id-1"></a> `quote_id?` | `string` \| `null` | - |
| <a id="property-quote_ttl"></a> `quote_ttl` | `number` | Seconds a `quote_id` stays valid once issued. **Min** 0 |
| <a id="property-webhook-1"></a> `webhook?` | [`WebhookInfo`](#webhookinfo) | - |

***

### LightningPolicyResponse

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-max_amount-1"></a> `max_amount` | `string` | - |
| <a id="property-min_amount-1"></a> `min_amount` | `string` | - |
| <a id="property-service_ppm-1"></a> `service_ppm` | `number` | **Min** 0 |

***

### OutPoint

On-chain opening-transaction output that funds a swap.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-txid"></a> `txid` | `string` | Opening transaction id. |
| <a id="property-vout"></a> `vout` | `number` | Opening transaction output index. |

***

### Pagination

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-has_next"></a> `has_next` | `boolean` | - |
| <a id="property-limit"></a> `limit` | `number` | **Min** 0 |
| <a id="property-offset"></a> `offset` | `number` | **Min** 0 |
| <a id="property-total"></a> `total` | `number` | **Min** 0 |

***

### Policy

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-chain"></a> `chain` | `Partial`&lt;`Record`&lt;`"bitcoin"` \| `"liquid"`, [`ChainPolicyResponse`](#chainpolicyresponse)&gt;&gt; | - |
| <a id="property-invoice_expiry"></a> `invoice_expiry` | `number` | **Min** 0 |
| <a id="property-lightning"></a> `lightning` | [`LightningPolicyResponse`](#lightningpolicyresponse) | - |
| <a id="property-network"></a> `network` | [`SwapNetwork`](#swapnetwork) | On-chain network for swap operations. |
| <a id="property-protocol_version-1"></a> `protocol_version` | `string` | - |

***

### RevealClaimRequest

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-source_refund_privkey"></a> `source_refund_privkey` | `string` |

***

### RevealPreimageRequest

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-preimage"></a> `preimage` | `string` |

***

### SwapListQuery

Filters for `GET /v1/swap`. They intersect, and all of them apply before
`limit` and `offset`, so `Pagination::total` counts what they matched. A
filter sent with nothing in it means the same as one left out.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-created_after"></a> `created_after?` | `number` \| `null` | Keeps swaps created at or after this unix timestamp, in seconds. **Min** 0 |
| <a id="property-created_before"></a> `created_before?` | `number` \| `null` | Keeps swaps created at or before this unix timestamp, in seconds. **Min** 0 |
| <a id="property-error_type"></a> `error_type?` | `string` \| `null` | Comma-separated cancellation reasons to keep, e.g. `UserCanceled,Timeout`. Only a cancelled swap has one, so this filters out every other swap too. |
| <a id="property-is_active"></a> `is_active?` | `boolean` \| `null` | `true` keeps only active swaps, `false` only finished ones; omit for both. |
| <a id="property-limit-1"></a> `limit` | `number` | Page size, at most 100. A larger one is rejected, not clamped. **Min** 0 **Max** 100 |
| <a id="property-max_amount-2"></a> `max_amount?` | `number` \| `null` | Keeps swaps whose `receive_amount` is at most this many sats. **Min** 0 |
| <a id="property-min_amount-2"></a> `min_amount?` | `number` \| `null` | Keeps swaps whose `receive_amount` is at least this many sats. **Min** 0 |
| <a id="property-offset-1"></a> `offset` | `number` | **Min** 0 |
| <a id="property-search"></a> `search?` | `string` \| `null` | Keeps swaps where `swap_id`, `payment_hash`, `claim_invoice`, `source_txid`, `dest_txid`, or `user_claim_address` equals this value. The match ignores case but is exact, not a prefix. |
| <a id="property-status"></a> `status?` | `string` \| `null` | Comma-separated FSM statuses to keep, e.g. `Claimed,Refunded`. |
| <a id="property-swap_type"></a> `swap_type?` | `string` \| `null` | Keeps only swaps of this type: `SwapIn`, `SwapOut`, or `SwapChain`. |
| <a id="property-user_id-1"></a> `user_id` | `string` | - |
| <a id="property-with_create_request"></a> `with_create_request?` | `boolean` | `true` carries each swap's `create_request_*` fields as `GET /v1/swap/{id}` does; a page omits them otherwise. |

***

### SwapListResponse

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-pagination"></a> `pagination` | [`Pagination`](#pagination) |
| <a id="property-swaps"></a> `swaps` | [`SwapState`](#swapstate)[] |

***

### SwapState

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-base_fee"></a> `base_fee` | `string` | - |
| <a id="property-cancel_message-1"></a> `cancel_message?` | `string` \| `null` | - |
| <a id="property-claim_invoice-1"></a> `claim_invoice?` | `string` \| `null` | - |
| <a id="property-create_request_body"></a> `create_request_body?` | `string` \| `null` | The `POST /v1/swap` body this swap was created from, verbatim. With the two fields below it lets a client verify the swap's terms against its own signature rather than against this response. Returned by `GET /v1/swap/{id}`, and by the listing under `with_create_request`. |
| <a id="property-create_request_signature"></a> `create_request_signature?` | `string` \| `null` | Hex of the 64-byte Schnorr signature over that request. |
| <a id="property-create_request_timestamp"></a> `create_request_timestamp?` | `number` \| `null` | - |
| <a id="property-created_at"></a> `created_at` | `number` | **Min** 0 |
| <a id="property-dest_asset"></a> `dest_asset?` | `string` \| `null` | - |
| <a id="property-dest_chain-1"></a> `dest_chain?` | [`Chain`](#chain) \| `null` | - |
| <a id="property-dest_claim_pubkey-1"></a> `dest_claim_pubkey?` | `string` \| `null` | - |
| <a id="property-dest_csv-1"></a> `dest_csv?` | `number` \| `null` | **Min** 0 |
| <a id="property-dest_refund_privkey"></a> `dest_refund_privkey?` | `string` \| `null` | - |
| <a id="property-dest_refund_pubkey"></a> `dest_refund_pubkey?` | `string` \| `null` | - |
| <a id="property-dest_service_blinding_pubkey"></a> `dest_service_blinding_pubkey?` | `string` \| `null` | Liquid: anyswap's blinding pubkey for the destination output. |
| <a id="property-dest_txid"></a> `dest_txid?` | `string` \| `null` | - |
| <a id="property-dest_user_blinding_pubkey"></a> `dest_user_blinding_pubkey?` | `string` \| `null` | Liquid: the user's blinding pubkey for the destination output. |
| <a id="property-dest_vout"></a> `dest_vout?` | `number` \| `null` | **Min** 0 |
| <a id="property-error_type-1"></a> `error_type?` | `string` \| `null` | - |
| <a id="property-payment_hash-1"></a> `payment_hash` | `string` | - |
| <a id="property-preimage-1"></a> `preimage?` | `string` \| `null` | - |
| <a id="property-protocol_version-2"></a> `protocol_version` | `string` | - |
| <a id="property-receive_amount-1"></a> `receive_amount` | `string` | - |
| <a id="property-send_amount-1"></a> `send_amount` | `string` | - |
| <a id="property-service_fee"></a> `service_fee` | `string` | - |
| <a id="property-source_asset"></a> `source_asset?` | `string` \| `null` | - |
| <a id="property-source_chain-1"></a> `source_chain?` | [`Chain`](#chain) \| `null` | - |
| <a id="property-source_claim_privkey"></a> `source_claim_privkey?` | `string` \| `null` | - |
| <a id="property-source_claim_pubkey"></a> `source_claim_pubkey?` | `string` \| `null` | - |
| <a id="property-source_csv-1"></a> `source_csv?` | `number` \| `null` | **Min** 0 |
| <a id="property-source_refund_pubkey-1"></a> `source_refund_pubkey?` | `string` \| `null` | - |
| <a id="property-source_service_blinding_pubkey"></a> `source_service_blinding_pubkey?` | `string` \| `null` | Liquid: anyswap's blinding pubkey for the source output. |
| <a id="property-source_txid"></a> `source_txid?` | `string` \| `null` | - |
| <a id="property-source_user_blinding_pubkey"></a> `source_user_blinding_pubkey?` | `string` \| `null` | Liquid: the user's blinding pubkey for the source output. |
| <a id="property-source_vout"></a> `source_vout?` | `number` \| `null` | **Min** 0 |
| <a id="property-status-1"></a> `status` | `string` | - |
| <a id="property-swap_id-1"></a> `swap_id` | `string` | - |
| <a id="property-swap_type-1"></a> `swap_type` | [`SwapType`](#swaptype) | Swap kind. Directions are relative to Lightning: `SwapIn` is on-chain to Lightning, `SwapOut` is Lightning to on-chain, `SwapChain` is on-chain to on-chain across two different chains. |
| <a id="property-transitioned_at"></a> `transitioned_at` | `number` | **Min** 0 |
| <a id="property-user_claim_address-1"></a> `user_claim_address?` | `string` \| `null` | - |
| <a id="property-user_refund_address-1"></a> `user_refund_address?` | `string` \| `null` | - |

***

### SwapStateUpdate

#### Properties

| Property | Type |
| ------ | ------ |
| <a id="property-status-2"></a> `status` | `string` |
| <a id="property-swap_id-2"></a> `swap_id` | `string` |

***

### SwapWebhook

Status webhook registered with a swap and fixed for its lifetime. The service
`POST`s `{ event: "swap.update", data: { id, status } }` to `url` on every
subscribed change. Deliveries are unsigned, unordered, and not guaranteed:
treat one as a prompt to read
[getSwap](Api.md#getswap), not as state.

`S` bounds `statuses`; [SwapInWebhook](#swapinwebhook) and [SwapOutWebhook](#swapoutwebhook) pin it
to one direction.

#### Type Parameters

| Type Parameter | Default type |
| ------ | ------ |
| `S` *extends* [`SwapStatus`](#swapstatus) | [`SwapStatus`](#swapstatus) |

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-hash_swap_id"></a> `hash_swap_id?` | `boolean` | Send the SHA256 of the swap id instead of the id itself. |
| <a id="property-statuses"></a> `statuses?` | `S`[] | Statuses to deliver. Omitted or empty delivers every status change. |
| <a id="property-url"></a> `url` | `string` | Destination the service posts to, at most 250 characters. |

***

### TimeoutResponse

Deadlines this chain enforces as a SwapIn source, in seconds. A swap left in
the matching status past its deadline is canceled. SwapOut has no deadline
here: once its opening tx confirms, the bound is the `dest_csv` timelock
in blocks.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-await_claim_key_reveal_message"></a> `await_claim_key_reveal_message` | `number` | Deadline to reveal the cooperative claim key. **Min** 0 |
| <a id="property-await_opening_tx"></a> `await_opening_tx` | `number` | Maximum wait for the opening tx to appear at the swap address and confirm. The claim invoice expiry may end it sooner. **Min** 0 |

***

### WebhookInfo

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-enabled"></a> `enabled` | `boolean` | Whether this node accepts webhook registrations. |
| <a id="property-https_required"></a> `https_required` | `boolean` | Whether a registered url must use https. |

## Type Aliases

### SwapEndpoint

```ts
type SwapEndpoint = 
  | {
  asset?: string;
  chain: Chain;
}
  | "lightning";
```

A swap endpoint: an on-chain asset, or Lightning.

`asset` is the policy label the server files the asset under in
`policy.chain[chain].asset`, not the on-chain id. It defaults to
[NATIVE\_ASSET](#native_asset), which is the only label the server currently accepts.
The id that goes on the wire as `source_asset_id` / `dest_asset_id` is a
separate value, which the driver reads from the server's policy;
[destAsset](Utils.md#destasset) resolves a destination's.

***

### SwapErrorCode

```ts
type SwapErrorCode = 
  | "UserCanceled"
  | "Timeout"
  | "TxValidationFailed"
  | "PayInvoiceFailed"
  | "TxBroadcastFailed";
```

Machine-readable reason a swap reached `Canceled`, carried by `errorCode`
alongside the human-readable `cancelMessage`. Each maps to the terminal event
that canceled the swap.

- `UserCanceled`: the user aborted the swap.
- `Timeout`: a waiting state's deadline elapsed.
- `TxValidationFailed` (swap-in): two or more deposits paid the exact swap amount, so the service could not tell which to claim.
- `PayInvoiceFailed` (swap-in): the claim invoice payment failed terminally.
- `TxBroadcastFailed` (swap-out): the destination opening tx was never broadcast.

***

### SwapInStatus

```ts
type SwapInStatus = 
  | {
  status: "AwaitOpeningTxConfirmation";
}
  | {
  outpoint: OutPoint;
  status: "PayInvoice";
}
  | {
  outpoint: OutPoint;
  preimage: string;
  status: "AwaitClaimKeyRevealMessage";
}
  | {
  outpoint: OutPoint;
  status: "AwaitClaimTxConfirmation";
}
  | {
  outpoint: OutPoint;
  status: "Claimed";
}
  | {
  cancelMessage: string;
  claimPrivkey: string;
  errorCode: SwapErrorCode;
  status: "Canceled";
};
```

Swap-in lifecycle state, discriminated by `status`. Each variant carries only
the fields available in that state, so narrowing on `status` exposes them
safely. Normal flow: `AwaitOpeningTxConfirmation` -> `PayInvoice` ->
`AwaitClaimKeyRevealMessage` -> `AwaitClaimTxConfirmation` -> `Claimed`. The
server discovers the deposit by watching the swap address, so there is no
announcement step and `AwaitOpeningTxConfirmation` carries no outpoint.
`AwaitClaimKeyRevealMessage` carries the `preimage` the server obtained by
paying the invoice, which the driver checks before revealing the refund key.
`Canceled` is terminal: `errorCode` is the machine-readable reason, and the
server reveals its `claimPrivkey`; the user finds the deposit by re-deriving
the swap address and refunds it.

***

### SwapInWebhook

```ts
type SwapInWebhook = SwapWebhook<SwapInStatus["status"]>;
```

[SwapWebhook](#swapwebhook) restricted to the swap-in lifecycle.

***

### SwapOutStatus

```ts
type SwapOutStatus = 
  | {
  status: "AwaitClaimInvoicePay";
}
  | {
  status: "OpeningTxBroadcast";
}
  | {
  outpoint: OutPoint;
  status: "AwaitPreimageRevealed";
}
  | {
  outpoint: OutPoint;
  status: "AwaitRefundTxConfirmation";
}
  | {
  outpoint: OutPoint;
  refundPrivkey: string;
  status: "Claimed";
}
  | {
  cancelMessage: string;
  errorCode: SwapErrorCode;
  status: "Canceled";
}
  | {
  outpoint: OutPoint;
  status: "Refunded";
};
```

Swap-out lifecycle state, discriminated by `status`. Normal flow:
`AwaitClaimInvoicePay` -> `OpeningTxBroadcast` -> `AwaitPreimageRevealed` ->
`Claimed` (the server reveals `refundPrivkey` for a cooperative keypath
claim). If the user never claims: `AwaitRefundTxConfirmation` -> `Refunded`.
`Canceled` is terminal.

***

### SwapOutWebhook

```ts
type SwapOutWebhook = SwapWebhook<SwapOutStatus["status"]>;
```

[SwapWebhook](#swapwebhook) restricted to the swap-out lifecycle.

***

### SwapStatus

```ts
type SwapStatus = 
  | SwapInStatus["status"]
  | SwapOutStatus["status"];
```

Every status either flow reports, derived from [SwapInStatus](#swapinstatus) and
[SwapOutStatus](#swapoutstatus) so it cannot drift from them. Used to filter a swap
listing by status.

## Variables

### NATIVE\_ASSET

```ts
const NATIVE_ASSET: "native" = "native";
```

Policy label for a chain's native asset (BTC on Bitcoin, L-BTC on Liquid).
The server keys its per-chain asset policy under this label, and execution is
single-asset, so it is the only label a swap can name.

On Bitcoin it doubles as the on-chain asset id: `source_asset_id` is literally
`"native"`. On Liquid the id is the asset's 32-byte hex instead, which lives in
`policy.chain.liquid.asset.native.asset_id`. Use
[destAsset](Utils.md#destasset) rather than hardcoding this constant as
an asset id.
