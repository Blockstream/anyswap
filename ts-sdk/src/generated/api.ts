/* eslint-disable */
/* tslint:disable */
// @ts-nocheck
/*
 * ---------------------------------------------------------------
 * ## THIS FILE WAS GENERATED VIA SWAGGER-TYPESCRIPT-API        ##
 * ##                                                           ##
 * ## AUTHOR: acacode                                           ##
 * ## SOURCE: https://github.com/acacode/swagger-typescript-api ##
 * ---------------------------------------------------------------
 */

/**
 * Swap kind. Directions are relative to Lightning: `SwapIn` is on-chain to
 * Lightning, `SwapOut` is Lightning to on-chain, `SwapChain` is on-chain to
 * on-chain across two different chains.
 */
export enum SwapType {
  SwapIn = "SwapIn",
  SwapOut = "SwapOut",
  SwapChain = "SwapChain",
}

/** On-chain network for swap operations. */
export enum SwapNetwork {
  Mainnet = "mainnet",
  Testnet = "testnet",
  Signet = "signet",
  Regtest = "regtest",
}

/**
 * Machine-readable `code` carried beside `message` in every error response,
 * so a client never infers semantics from the HTTP status.
 */
export enum ErrorCode {
  Internal = "Internal",
  BadRequest = "BadRequest",
  NotImplemented = "NotImplemented",
  QuoteExpired = "QuoteExpired",
  SwapNotFound = "SwapNotFound",
  SwapAlreadyExists = "SwapAlreadyExists",
  TransitionRejected = "TransitionRejected",
  PolicyError = "PolicyError",
  TooManyConnections = "TooManyConnections",
  Unavailable = "Unavailable",
  Forbidden = "Forbidden",
  PayloadTooLarge = "PayloadTooLarge",
  InsufficientLiquidity = "InsufficientLiquidity",
}

export enum Chain {
  Bitcoin = "bitcoin",
  Liquid = "liquid",
}

export interface AssetPolicyResponse {
  asset_id: string;
  max_amount: string;
  min_amount: string;
}

export interface CancelRequest {
  cancel_message: string;
}

export interface ChainPolicyResponse {
  asset: Partial<Record<string, AssetPolicyResponse>>;
  /**
   * @format int32
   * @min 0
   */
  block_time: number;
  /**
   * @format int32
   * @min 0
   */
  confs: number;
  /**
   * CSV timelock on a swap-out opening tx, in this chain's blocks.
   * @format int32
   * @min 0
   */
  dest_csv: number;
  /**
   * Dust-guard floor: minimum receive amount as a multiple of this chain's
   * base fee.
   * @format int64
   * @min 0
   */
  min_base_fee_multiplier: number;
  /**
   * @format int64
   * @min 0
   */
  service_ppm: number;
  /**
   * CSV timelock on a swap-in opening tx, in this chain's blocks.
   * @format int32
   * @min 0
   */
  source_csv: number;
  /**
   * Deadlines this chain enforces as a SwapIn source, in seconds. A swap left in
   * the matching status past its deadline is canceled. SwapOut has no deadline
   * here: once its opening tx confirms, the bound is the `dest_csv` timelock
   * in blocks.
   */
  timeouts: TimeoutResponse;
}

export interface CreateSwapRequest {
  /** swap-in only. */
  claim_invoice?: string | null;
  dest_asset_id?: string | null;
  /** swap-out and swap-chain, Liquid only. */
  dest_blinding_pubkey?: string | null;
  dest_chain?: Chain | null;
  /** swap-out and swap-chain. */
  dest_claim_pubkey?: string | null;
  /** swap-out and swap-chain: preimage hash for the destination HTLC. */
  payment_hash?: string | null;
  protocol_version: string;
  /**
   * Optional `GET /v1/info` quote locking the base fee.
   * @format uuid
   */
  quote_id?: string | null;
  receive_amount: string;
  send_amount: string;
  source_asset_id?: string | null;
  /** swap-in and swap-chain, Liquid only. */
  source_blinding_pubkey?: string | null;
  source_chain?: Chain | null;
  /** swap-in and swap-chain. */
  source_refund_pubkey?: string | null;
  /** @format uuid */
  swap_id: string;
  /** swap-out and swap-chain. */
  user_claim_address?: string | null;
  user_id: string;
  /** swap-in and swap-chain. */
  user_refund_address?: string | null;
  /** Optional webhook for swap status changes. */
  webhook?: WebhookRequest | null;
}

/** The body of every `4xx`/`5xx` response. */
export interface ErrorResponse {
  /**
   * Machine-readable `code` carried beside `message` in every error response,
   * so a client never infers semantics from the HTTP status.
   */
  code: ErrorCode;
  message: string;
}

export interface FeeEstimate {
  dest: string;
  source: string;
}

export interface InfoResponse {
  fee_estimates?: Partial<Record<"bitcoin" | "liquid", FeeEstimate>>;
  /**
   * Whether the Lightning node pays and receives; `false` while it runs in
   * offline mode.
   */
  online: boolean;
  policy: Policy;
  /** @format uuid */
  quote_id?: string | null;
  /**
   * Seconds a `quote_id` stays valid once issued.
   * @format int64
   * @min 0
   */
  quote_ttl: number;
  webhook?: WebhookInfo;
}

export interface LightningPolicyResponse {
  max_amount: string;
  min_amount: string;
  /**
   * @format int64
   * @min 0
   */
  service_ppm: number;
}

export interface Pagination {
  has_next: boolean;
  /**
   * @format int32
   * @min 0
   */
  limit: number;
  /**
   * @format int32
   * @min 0
   */
  offset: number;
  /**
   * @format int32
   * @min 0
   */
  total: number;
}

export interface Policy {
  chain: Partial<Record<"bitcoin" | "liquid", ChainPolicyResponse>>;
  /**
   * @format int64
   * @min 0
   */
  invoice_expiry: number;
  lightning: LightningPolicyResponse;
  /** On-chain network for swap operations. */
  network: SwapNetwork;
  protocol_version: string;
}

export interface RevealClaimRequest {
  source_refund_privkey: string;
}

export interface RevealPreimageRequest {
  preimage: string;
}

/**
 * Filters for `GET /v1/swap`. They intersect, and all of them apply before
 * `limit` and `offset`, so `Pagination::total` counts what they matched. A
 * filter sent with nothing in it means the same as one left out.
 */
export interface SwapListQuery {
  /**
   * Keeps swaps created at or after this unix timestamp, in seconds.
   * @format int64
   * @min 0
   */
  created_after?: number | null;
  /**
   * Keeps swaps created at or before this unix timestamp, in seconds.
   * @format int64
   * @min 0
   */
  created_before?: number | null;
  /**
   * Comma-separated cancellation reasons to keep, e.g. `UserCanceled,Timeout`.
   * Only a cancelled swap has one, so this filters out every other swap too.
   */
  error_type?: string | null;
  /** `true` keeps only active swaps, `false` only finished ones; omit for both. */
  is_active?: boolean | null;
  /**
   * Page size, at most 100. A larger one is rejected, not clamped.
   * @format int32
   * @min 0
   * @max 100
   */
  limit: number;
  /**
   * Keeps swaps whose `receive_amount` is at most this many sats.
   * @format int64
   * @min 0
   */
  max_amount?: number | null;
  /**
   * Keeps swaps whose `receive_amount` is at least this many sats.
   * @format int64
   * @min 0
   */
  min_amount?: number | null;
  /**
   * @format int32
   * @min 0
   */
  offset: number;
  /**
   * Keeps swaps where `swap_id`, `payment_hash`, `claim_invoice`,
   * `source_txid`, `dest_txid`, or `user_claim_address` equals this value.
   * The match ignores case but is exact, not a prefix.
   */
  search?: string | null;
  /** Comma-separated FSM statuses to keep, e.g. `Claimed,Refunded`. */
  status?: string | null;
  /** Keeps only swaps of this type: `SwapIn`, `SwapOut`, or `SwapChain`. */
  swap_type?: string | null;
  user_id: string;
  /**
   * `true` carries each swap's `create_request_*` fields as
   * `GET /v1/swap/{id}` does; a page omits them otherwise.
   */
  with_create_request?: boolean;
}

export interface SwapListResponse {
  pagination: Pagination;
  swaps: SwapState[];
}

export interface SwapState {
  base_fee: string;
  cancel_message?: string | null;
  claim_invoice?: string | null;
  /**
   * The `POST /v1/swap` body this swap was created from, verbatim. With the
   * two fields below it lets a client verify the swap's terms against its own
   * signature rather than against this response. Returned by
   * `GET /v1/swap/{id}`, and by the listing under `with_create_request`.
   */
  create_request_body?: string | null;
  /** Hex of the 64-byte Schnorr signature over that request. */
  create_request_signature?: string | null;
  /** @format int64 */
  create_request_timestamp?: number | null;
  /**
   * @format int64
   * @min 0
   */
  created_at: number;
  dest_asset?: string | null;
  dest_chain?: Chain | null;
  dest_claim_pubkey?: string | null;
  /**
   * @format int32
   * @min 0
   */
  dest_csv?: number | null;
  dest_refund_privkey?: string | null;
  dest_refund_pubkey?: string | null;
  /** Liquid: anyswap's blinding pubkey for the destination output. */
  dest_service_blinding_pubkey?: string | null;
  dest_txid?: string | null;
  /** Liquid: the user's blinding pubkey for the destination output. */
  dest_user_blinding_pubkey?: string | null;
  /**
   * @format int32
   * @min 0
   */
  dest_vout?: number | null;
  error_type?: string | null;
  payment_hash: string;
  preimage?: string | null;
  protocol_version: string;
  receive_amount: string;
  send_amount: string;
  service_fee: string;
  source_asset?: string | null;
  source_chain?: Chain | null;
  source_claim_privkey?: string | null;
  source_claim_pubkey?: string | null;
  /**
   * @format int32
   * @min 0
   */
  source_csv?: number | null;
  source_refund_pubkey?: string | null;
  /** Liquid: anyswap's blinding pubkey for the source output. */
  source_service_blinding_pubkey?: string | null;
  source_txid?: string | null;
  /** Liquid: the user's blinding pubkey for the source output. */
  source_user_blinding_pubkey?: string | null;
  /**
   * @format int32
   * @min 0
   */
  source_vout?: number | null;
  status: string;
  /** @format uuid */
  swap_id: string;
  /**
   * Swap kind. Directions are relative to Lightning: `SwapIn` is on-chain to
   * Lightning, `SwapOut` is Lightning to on-chain, `SwapChain` is on-chain to
   * on-chain across two different chains.
   */
  swap_type: SwapType;
  /**
   * @format int64
   * @min 0
   */
  transitioned_at: number;
  user_claim_address?: string | null;
  user_refund_address?: string | null;
}

export interface SwapStateUpdate {
  status: string;
  /** @format uuid */
  swap_id: string;
}

/**
 * Deadlines this chain enforces as a SwapIn source, in seconds. A swap left in
 * the matching status past its deadline is canceled. SwapOut has no deadline
 * here: once its opening tx confirms, the bound is the `dest_csv` timelock
 * in blocks.
 */
export interface TimeoutResponse {
  /**
   * Deadline to reveal the cooperative claim key.
   * @format int64
   * @min 0
   */
  await_claim_key_reveal_message: number;
  /**
   * Maximum wait for the opening tx to appear at the swap address and
   * confirm. The claim invoice expiry may end it sooner.
   * @format int64
   * @min 0
   */
  await_opening_tx: number;
}

export interface WebhookInfo {
  /** Whether this node accepts webhook registrations. */
  enabled: boolean;
  /** Whether a registered url must use https. */
  https_required: boolean;
}

/** Status webhook registration supplied with a swap. */
export interface WebhookRequest {
  /** Send SHA256 of the swap id instead of the id itself. */
  hash_swap_id?: boolean;
  /** Statuses to deliver. Omitted or empty delivers every status change. */
  statuses?: string[] | null;
  url: string;
}
