/**
 * Shared types for API requests, responses, and swap state.
 *
 * Re-exports the OpenAPI-generated request/response types and enums, and adds
 * {@link SwapInStatus} and {@link SwapOutStatus} - discriminated unions keyed by
 * `status` that model each swap direction's lifecycle, carrying only the fields
 * available in each state.
 *
 * @packageDocumentation
 * @module Types
 */

import type {
  Chain,
  CreateSwapRequest as GeneratedCreateSwapRequest,
  ErrorCode,
} from "./generated/api";

export { SwapType, SwapNetwork, Chain, ErrorCode } from "./generated/api";

export type {
  AssetPolicyResponse,
  ChainPolicyResponse,
  LightningPolicyResponse,
  TimeoutResponse,
  FeeEstimate,
  Policy,
  InfoResponse,
  WebhookInfo,
  SwapState,
  CancelRequest,
  RevealClaimRequest,
  RevealPreimageRequest,
  SwapListQuery,
  SwapListResponse,
  Pagination,
  SwapStateUpdate,
} from "./generated/api";

/**
 * A swap endpoint: an on-chain asset, or Lightning.
 *
 * `asset` is the policy label the server files the asset under in
 * `policy.chain[chain].asset`, not the on-chain id. It defaults to
 * {@link NATIVE_ASSET}, which is the only label the server currently accepts.
 * The id that goes on the wire as `source_asset_id` / `dest_asset_id` is a
 * separate value, which the driver reads from the server's policy;
 * {@link Utils!destAsset | destAsset} resolves a destination's.
 */
export type SwapEndpoint = { chain: Chain; asset?: string } | "lightning";

/**
 * Policy label for a chain's native asset (BTC on Bitcoin, L-BTC on Liquid).
 * The server keys its per-chain asset policy under this label, and execution is
 * single-asset, so it is the only label a swap can name.
 *
 * On Bitcoin it doubles as the on-chain asset id: `source_asset_id` is literally
 * `"native"`. On Liquid the id is the asset's 32-byte hex instead, which lives in
 * `policy.chain.liquid.asset.native.asset_id`. Use
 * {@link Utils!destAsset | destAsset} rather than hardcoding this constant as
 * an asset id.
 */
export const NATIVE_ASSET = "native";

/**
 * Machine-readable reason a swap reached `Canceled`, carried by `errorCode`
 * alongside the human-readable `cancelMessage`. Each maps to the terminal event
 * that canceled the swap.
 *
 * - `UserCanceled`: the user aborted the swap.
 * - `Timeout`: a waiting state's deadline elapsed.
 * - `TxValidationFailed` (swap-in): two or more deposits paid the exact swap amount, so the service could not tell which to claim.
 * - `PayInvoiceFailed` (swap-in): the claim invoice payment failed terminally.
 * - `TxBroadcastFailed` (swap-out): the destination opening tx was never broadcast.
 */
export type SwapErrorCode =
  | "UserCanceled"
  | "Timeout"
  | "TxValidationFailed"
  | "PayInvoiceFailed"
  | "TxBroadcastFailed";

/**
 * Error body returned by the server on a non-2xx response. Branch on `code`
 * rather than `message` or the HTTP status; the fields are absent only when
 * the body could not be parsed.
 */
export interface ApiError {
  code?: ErrorCode;
  message?: string;
}

/** On-chain opening-transaction output that funds a swap. */
export interface OutPoint {
  /** Opening transaction id. */
  txid: string;
  /** Opening transaction output index. */
  vout: number;
}

/**
 * Swap-in lifecycle state, discriminated by `status`. Each variant carries only
 * the fields available in that state, so narrowing on `status` exposes them
 * safely. Normal flow: `AwaitOpeningTxConfirmation` -> `PayInvoice` ->
 * `AwaitClaimKeyRevealMessage` -> `AwaitClaimTxConfirmation` -> `Claimed`. The
 * server discovers the deposit by watching the swap address, so there is no
 * announcement step and `AwaitOpeningTxConfirmation` carries no outpoint.
 * `AwaitClaimKeyRevealMessage` carries the `preimage` the server obtained by
 * paying the invoice, which the driver checks before revealing the refund key.
 * `Canceled` is terminal: `errorCode` is the machine-readable reason, and the
 * server reveals its `claimPrivkey`; the user finds the deposit by re-deriving
 * the swap address and refunds it.
 */
export type SwapInStatus =
  | { status: "AwaitOpeningTxConfirmation" }
  | { status: "PayInvoice"; outpoint: OutPoint }
  | {
      status: "AwaitClaimKeyRevealMessage";
      outpoint: OutPoint;
      preimage: string;
    }
  | { status: "AwaitClaimTxConfirmation"; outpoint: OutPoint }
  | { status: "Claimed"; outpoint: OutPoint }
  | {
      status: "Canceled";
      errorCode: SwapErrorCode;
      claimPrivkey: string;
      cancelMessage: string;
    };

/**
 * Swap-out lifecycle state, discriminated by `status`. Normal flow:
 * `AwaitClaimInvoicePay` -> `OpeningTxBroadcast` -> `AwaitPreimageRevealed` ->
 * `Claimed` (the server reveals `refundPrivkey` for a cooperative keypath
 * claim). If the user never claims: `AwaitRefundTxConfirmation` -> `Refunded`.
 * `Canceled` is terminal.
 */
export type SwapOutStatus =
  | { status: "AwaitClaimInvoicePay" }
  | { status: "OpeningTxBroadcast" }
  | { status: "AwaitPreimageRevealed"; outpoint: OutPoint }
  | { status: "AwaitRefundTxConfirmation"; outpoint: OutPoint }
  | { status: "Claimed"; outpoint: OutPoint; refundPrivkey: string }
  | { status: "Canceled"; errorCode: SwapErrorCode; cancelMessage: string }
  | { status: "Refunded"; outpoint: OutPoint };

/**
 * Every status either flow reports, derived from {@link SwapInStatus} and
 * {@link SwapOutStatus} so it cannot drift from them. Used to filter a swap
 * listing by status.
 */
export type SwapStatus = SwapInStatus["status"] | SwapOutStatus["status"];

/**
 * Status webhook registered with a swap and fixed for its lifetime. The service
 * `POST`s `{ event: "swap.update", data: { id, status } }` to `url` on every
 * subscribed change. Deliveries are unsigned, unordered, and not guaranteed:
 * treat one as a prompt to read
 * {@link Api!AnySwapClient.getSwap | getSwap}, not as state.
 *
 * `S` bounds `statuses`; {@link SwapInWebhook} and {@link SwapOutWebhook} pin it
 * to one direction.
 */
export interface SwapWebhook<S extends SwapStatus = SwapStatus> {
  /** Destination the service posts to, at most 250 characters. */
  url: string;
  /** Statuses to deliver. Omitted or empty delivers every status change. */
  statuses?: S[];
  /** Send the SHA256 of the swap id instead of the id itself. */
  hash_swap_id?: boolean;
}

/** {@link SwapWebhook} restricted to the swap-in lifecycle. */
export type SwapInWebhook = SwapWebhook<SwapInStatus["status"]>;

/** {@link SwapWebhook} restricted to the swap-out lifecycle. */
export type SwapOutWebhook = SwapWebhook<SwapOutStatus["status"]>;

/**
 * Body of {@link Api!AnySwapClient.createSwap | createSwap}. As the service
 * documents it, amounts as decimal strings, except that `webhook.statuses` are
 * typed to the statuses a swap can reach, so a misspelled one fails to compile.
 */
export interface CreateSwapRequest extends Omit<
  GeneratedCreateSwapRequest,
  "webhook"
> {
  /** Status webhook to register with the swap. */
  webhook?: SwapWebhook | null;
}
