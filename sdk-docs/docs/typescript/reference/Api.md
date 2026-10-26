HTTP client for the AnySwap server API.

The [Driver](Driver.md#driver) takes this client as its
[ServerClient](Driver.md#serverclient) and makes the swap calls below
itself; an app calls [AnySwapClient.getInfo](#getinfo),
[AnySwapClient.getSwaps](#getswaps) and [AnySwapClient.signals](#signals).

The server is always the responder; the user always initiates. POST methods
trigger server-side transitions and return an empty body - the resulting state
is always read back via [AnySwapClient.getSwap](#getswap), never from the POST
response. Track progress with [AnySwapClient.subscribe](#subscribe): it nudges you
on connect and on every transition, and you read the current state over HTTP
(typically [AnySwapClient.getSwaps](#getswaps)) in response.

## Swap-in (on-chain to Lightning)

After creating a swap-in ([AnySwapClient.createSwap](#createswap) with an on-chain
`source_chain` and no destination), follow the status via
[AnySwapClient.getSwap](#getswap):

`AwaitOpeningTxConfirmation` (deposit to the swap address; the server watches
for it) -> `PayInvoice` -> `AwaitClaimKeyRevealMessage` (reveal the refund key
via [AnySwapClient.revealClaim](#revealclaim) for a cooperative keypath claim) ->
`AwaitClaimTxConfirmation` -> `Claimed`.

[AnySwapClient.cancel](#cancel) before a successful claim yields `Canceled`, where
the server reveals `source_claim_privkey` for an immediate keypath refund.

## Swap-out (Lightning to on-chain)

After creating a swap-out ([AnySwapClient.createSwap](#createswap) with a `dest_chain`
and no source), follow the status via [AnySwapClient.getSwap](#getswap):

`AwaitClaimInvoicePay` (pay the held `claim_invoice`) -> `OpeningTxBroadcast`
-> `AwaitPreimageRevealed` (claim cooperatively via
[AnySwapClient.revealPreimage](#revealpreimage) then keypath spend, or unilaterally
on-chain) -> `Claimed`.

If the user never claims, the server refunds after the CSV timeout
(`AwaitRefundTxConfirmation` -> `Refunded`). There is no way to abort a
swap-out once its opening tx is broadcast.

## Classes

### AnySwapClient

HTTP client for the AnySwap server API.

#### Constructors

##### Constructor

```ts
new AnySwapClient(url: string, options?: AnySwapClientOptions): AnySwapClient;
```

###### Parameters

| Parameter | Type | Description |
| ------ | ------ | ------ |
| `url` | `string` | Base URL of the swap server. Trailing slashes are stripped. |
| `options` | [`AnySwapClientOptions`](#anyswapclientoptions) | Optional `fetch` implementation and auth key material for protected swap endpoints. |

###### Returns

[`AnySwapClient`](#anyswapclient)

#### Methods

##### cancel()

```ts
cancel(swapId: string, body: CancelRequest): Promise<void>;
```

Requests swap cancellation. The outcome is observed via
[AnySwapClient.getSwap](#getswap): `Canceled` reveals the server's claim privkey
(swap-in) for an immediate keypath refund of any deposit.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `swapId` | `string` |
| `body` | [`CancelRequest`](Types.md#cancelrequest) |

###### Returns

`Promise`&lt;`void`&gt;

##### createSwap()

```ts
createSwap(body: CreateSwapRequest): Promise<void>;
```

Initiates a swap. The server may cancel the swap right after creation; the
actual state is discovered via [AnySwapClient.getSwap](#getswap).

An optional [webhook](Types.md#swapwebhook) is refused with a `400` when
the server has webhooks off or the url breaks its rules; `webhook` on
[AnySwapClient.getInfo](#getinfo) reports both.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `body` | [`CreateSwapRequest`](Types.md#createswaprequest) |

###### Returns

`Promise`&lt;`void`&gt;

##### getInfo()

```ts
getInfo(): Promise<InfoResponse>;
```

Fetches server info: the current [policy](Types.md#policy), base-fee
estimates, and a `quote_id` (valid for `quote_ttl`) locking them.

###### Returns

`Promise`&lt;[`InfoResponse`](Types.md#inforesponse)&gt;

##### getSwap()

```ts
getSwap(swapId: string): Promise<SwapState>;
```

Reads the current state of a swap.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `swapId` | `string` |

###### Returns

`Promise`&lt;[`SwapState`](Types.md#swapstate)&gt;

##### getSwaps()

```ts
getSwaps(params: GetSwapsParams): Promise<SwapListResponse>;
```

Lists swaps for a user, newest first. Used during recovery to discover swap
history after restoring a wallet from its mnemonic.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `params` | [`GetSwapsParams`](#getswapsparams) |

###### Returns

`Promise`&lt;[`SwapListResponse`](Types.md#swaplistresponse)&gt;

##### listSwaps()

```ts
listSwaps(query: SwapListQuery): Promise<SwapListResponse>;
```

The listing over a raw [SwapListQuery](Types.md#swaplistquery), the form the WASM driver's
`ServerClient` calls; [AnySwapClient.getSwaps](#getswaps) is the typed one.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `query` | [`SwapListQuery`](Types.md#swaplistquery) |

###### Returns

`Promise`&lt;[`SwapListResponse`](Types.md#swaplistresponse)&gt;

##### revealClaim()

```ts
revealClaim(swapId: string, body: RevealClaimRequest): Promise<void>;
```

Reveals the user's refund privkey after the claim invoice was paid (swap-in
only), letting the server claim cooperatively via the keypath instead of the
preimage script path.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `swapId` | `string` |
| `body` | [`RevealClaimRequest`](Types.md#revealclaimrequest) |

###### Returns

`Promise`&lt;`void`&gt;

##### revealPreimage()

```ts
revealPreimage(swapId: string, body: RevealPreimageRequest): Promise<void>;
```

Reveals the preimage off-chain (swap-out only) instead of claiming via the
script path. The server settles the held invoice and exposes its refund
privkey, letting the user finish with a cheap cooperative keypath claim.
Reveal only after the opening tx has the required confirmations.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `swapId` | `string` |
| `body` | [`RevealPreimageRequest`](Types.md#revealpreimagerequest) |

###### Returns

`Promise`&lt;`void`&gt;

##### signals()

```ts
signals(userId: string, options?: SubscribeOptions): SwapSignals;
```

Wraps [AnySwapClient.subscribe](#subscribe) as the WASM driver's `Signals`: pass
the result to `Driver.withSignals`. Every pushed swap id is queued until
`next` takes it; the reconnect nudge carries none and is dropped, since the
driver reads fresh state on every pass anyway.

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `userId` | `string` |
| `options` | [`SubscribeOptions`](#subscribeoptions) |

###### Returns

[`SwapSignals`](#swapsignals)

##### subscribe()

```ts
subscribe(
   userId: string, 
   onUpdate: (update?: SwapStateUpdate) => void, 
   options?: SubscribeOptions): Subscription;
```

Subscribes to swap-change signals for `userId` over the server's WebSocket.
`onUpdate` gets the `{ swap_id, status }` signal per transition, or
`undefined` on (re)connect and on an unparseable frame ("resync, swap
unknown"). It is a nudge: read authoritative state over HTTP
([AnySwapClient.getSwap](#getswap) / [AnySwapClient.getSwaps](#getswaps)) in response;
do not trust the pushed `status` for safety-critical decisions. On a drop
it reconnects with a freshly signed upgrade after a second, doubling to 30
seconds while attempts fail and back to a second once one opens, until the
returned handle's `stop()` is called, and `onConnectionChange` hears of
each drop and each return. The server pings every 30 seconds to keep the
socket alive and allows 16 subscriptions per `userId`; a browser tab holds
one.

Requires a global `WebSocket` (browsers, Node >= 22).

###### Parameters

| Parameter | Type |
| ------ | ------ |
| `userId` | `string` |
| `onUpdate` | (`update?`: [`SwapStateUpdate`](Types.md#swapstateupdate)) => `void` |
| `options` | [`SubscribeOptions`](#subscribeoptions) |

###### Returns

[`Subscription`](#subscription)

a [Subscription](#subscription); call `stop()` to close it.

## Interfaces

### AnySwapAuth

Identity key material used to sign protected AnySwap requests.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-identityprivkey"></a> `identityPrivkey` | `string` | Stable wallet identity private key, hex-encoded. |

***

### AnySwapClientOptions

Options for [AnySwapClient](#anyswapclient).

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-auth"></a> `auth?` | [`AnySwapAuth`](#anyswapauth) | Identity key material for protected swap endpoints. |
| <a id="property-fetchfn"></a> `fetchFn?` | (`input`: `string` \| `URL` \| `Request`, `init?`: `RequestInit`) => `Promise`&lt;`Response`&gt; | Optional `fetch` implementation. Defaults to the global `fetch`. |

***

### GetSwapsParams

Parameters for [AnySwapClient.getSwaps](#getswaps).

Every field below `offset` is an optional filter. They combine with AND and
apply before `limit`/`offset`, so `pagination.total` counts the swaps that
matched rather than every swap the user owns. An omitted or empty filter is
simply not applied.

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-createdafter"></a> `createdAfter?` | `number` | Keeps swaps created at or after this unix timestamp, in seconds. |
| <a id="property-createdbefore"></a> `createdBefore?` | `number` | Keeps swaps created at or before this unix timestamp, in seconds. |
| <a id="property-errortype"></a> `errorType?` | [`SwapErrorCode`](Types.md#swaperrorcode)[] | Keeps swaps cancelled for any of these reasons, e.g. `["Timeout"]`. Only a cancelled swap has one, so this drops every other swap too. |
| <a id="property-isactive"></a> `isActive?` | `boolean` | `true` returns only swaps still in flight, `false` only those that reached a terminal status (`Claimed`, `Canceled`, `Refunded`); omit for both. |
| <a id="property-limit"></a> `limit?` | `number` | Maximum number of swaps to return, at most 100. Defaults to 20. |
| <a id="property-maxamount"></a> `maxAmount?` | `number` | Keeps swaps whose received amount is at most this many sats. |
| <a id="property-minamount"></a> `minAmount?` | `number` | Keeps swaps whose received amount is at least this many sats. |
| <a id="property-offset"></a> `offset?` | `number` | Number of swaps to skip, newest first. Defaults to 0. |
| <a id="property-search"></a> `search?` | `string` | Keeps swaps where one of `swapId`, `paymentHash`, `claimInvoice`, `sourceTxid`, `destTxid`, or `userClaimAddress` equals this value. The match ignores case but is exact, so a prefix of a txid finds nothing. |
| <a id="property-status"></a> `status?` | [`SwapStatus`](Types.md#swapstatus)[] | Keeps swaps in any of these statuses, e.g. `["Claimed", "Refunded"]`. |
| <a id="property-swaptype"></a> `swapType?` | [`SwapType`](Types.md#swaptype) | Keeps only swaps of this type. |
| <a id="property-userid"></a> `userId` | `string` | User identifier, typically [Wallet.userId](Wallet.md#userid). |
| <a id="property-withcreaterequest"></a> `withCreateRequest?` | `boolean` | `true` includes each swap's `create_request_*` fields, as `getSwap` does; a page omits them otherwise. |

***

### SubscribeOptions

Options for [AnySwapClient.subscribe](#subscribe) and [AnySwapClient.signals](#signals).

#### Properties

| Property | Type | Description |
| ------ | ------ | ------ |
| <a id="property-onconnectionchange"></a> `onConnectionChange?` | (`connected`: `boolean`) => `void` | Called with `false` when the socket drops and `true` when one opens, once per change and never after `stop()`. It starts disconnected, so the first call is `true`. |

***

### Subscription

A WebSocket subscription handle.

#### Extended by

- [`SwapSignals`](#swapsignals)

#### Methods

##### stop()

```ts
stop(): void;
```

Closes the subscription and stops reconnecting.

###### Returns

`void`

***

### SwapSignals

The WASM driver's `Signals`, returned by [AnySwapClient.signals](#signals): a
push channel naming swaps worth reading sooner.

#### Extends

- [`Subscription`](#subscription)

#### Methods

##### next()

```ts
next(): Promise<string | undefined>;
```

Resolves with the id of the next swap the server reported, waiting for
one when none is queued, and with `undefined` once stopped.

###### Returns

`Promise`&lt;`string` \| `undefined`&gt;

##### stop()

```ts
stop(): void;
```

Closes the subscription and stops reconnecting.

###### Returns

`void`

###### Inherited from

[`Subscription`](#subscription).[`stop`](#stop)
