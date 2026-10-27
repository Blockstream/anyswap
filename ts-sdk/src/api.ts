/**
 * HTTP client for the AnySwap server API.
 *
 * The {@link Driver!Driver | Driver} takes this client as its
 * {@link Driver!ServerClient | ServerClient} and makes the swap calls below
 * itself; an app calls {@link AnySwapClient.getInfo},
 * {@link AnySwapClient.getSwaps} and {@link AnySwapClient.signals}.
 *
 * The server is always the responder; the user always initiates. POST methods
 * trigger server-side transitions and return an empty body - the resulting state
 * is always read back via {@link AnySwapClient.getSwap}, never from the POST
 * response. Track progress with {@link AnySwapClient.subscribe}: it nudges you
 * on connect and on every transition, and you read the current state over HTTP
 * (typically {@link AnySwapClient.getSwaps}) in response.
 *
 * ## Swap-in (on-chain to Lightning)
 *
 * After creating a swap-in ({@link AnySwapClient.createSwap} with an on-chain
 * `source_chain` and no destination), follow the status via
 * {@link AnySwapClient.getSwap}:
 *
 * `AwaitOpeningTxConfirmation` (deposit to the swap address; the server watches
 * for it) -> `PayInvoice` -> `AwaitClaimKeyRevealMessage` (reveal the refund key
 * via {@link AnySwapClient.revealClaim} for a cooperative keypath claim) ->
 * `AwaitClaimTxConfirmation` -> `Claimed`.
 *
 * {@link AnySwapClient.cancel} before a successful claim yields `Canceled`, where
 * the server reveals `source_claim_privkey` for an immediate keypath refund.
 *
 * ## Swap-out (Lightning to on-chain)
 *
 * After creating a swap-out ({@link AnySwapClient.createSwap} with a `dest_chain`
 * and no source), follow the status via {@link AnySwapClient.getSwap}:
 *
 * `AwaitClaimInvoicePay` (pay the held `claim_invoice`) -> `OpeningTxBroadcast`
 * -> `AwaitPreimageRevealed` (claim cooperatively via
 * {@link AnySwapClient.revealPreimage} then keypath spend, or unilaterally
 * on-chain) -> `Claimed`.
 *
 * If the user never claims, the server refunds after the CSV timeout
 * (`AwaitRefundTxConfirmation` -> `Refunded`). There is no way to abort a
 * swap-out once its opening tx is broadcast.
 *
 * @packageDocumentation
 * @module Api
 */

import type {
  ApiError,
  CancelRequest,
  CreateSwapRequest,
  InfoResponse,
  RevealClaimRequest,
  RevealPreimageRequest,
  SwapErrorCode,
  SwapListQuery,
  SwapListResponse,
  SwapState,
  SwapStateUpdate,
  SwapStatus,
  SwapType,
} from "./types";
import {
  IDENTITY_PUBKEY_HEADER,
  SIGNATURE_HEADER,
  TIMESTAMP_HEADER,
  signRequest,
} from "./auth";
import type { Wallet } from "./wallet";

/** Options for {@link AnySwapClient}. */
export interface AnySwapClientOptions {
  /** Optional `fetch` implementation. Defaults to the global `fetch`. */
  fetchFn?: typeof fetch;
  /**
   * The wallet whose identity key signs protected requests, inside the wasm
   * module. Without one, only the public endpoints work.
   */
  wallet?: Wallet;
}

const RECONNECT_MIN_MS = 1_000;
const RECONNECT_MAX_MS = 30_000;

/** Minimal structural view of a `WebSocket` (browser / Node >= 22). */
type WebSocketLike = {
  close(): void;
  onopen: (() => void) | null;
  onmessage: ((event: { data: unknown }) => void) | null;
  onclose: (() => void) | null;
};

/**
 * Error thrown by {@link AnySwapClient} when the server returns a non-2xx
 * response. `detail` carries the server's `{ code, message }` body when
 * available; branch on `code` rather than `status` or the message text. The
 * codes and what to do about each are listed in the integration overview.
 *
 * @hidden
 */
export class AnySwapError extends Error {
  constructor(
    public status: number,
    public detail: ApiError,
  ) {
    super(detail.message ?? `request failed with status ${status}`);
    this.name = "AnySwapError";
  }

  /** The server's error code, when it sent one. */
  get code() {
    return this.detail.code ?? undefined;
  }
}

/**
 * Parameters for {@link AnySwapClient.getSwaps}.
 *
 * Every field below `offset` is an optional filter. They combine with AND and
 * apply before `limit`/`offset`, so `pagination.total` counts the swaps that
 * matched rather than every swap the user owns. An omitted or empty filter is
 * simply not applied.
 */
export interface GetSwapsParams {
  /** User identifier, typically {@link Wallet!Wallet.userId | Wallet.userId}. */
  userId: string;
  /** Maximum number of swaps to return, at most 100. Defaults to 20. */
  limit?: number;
  /** Number of swaps to skip, newest first. Defaults to 0. */
  offset?: number;
  /**
   * `true` returns only swaps still in flight, `false` only those that reached a
   * terminal status (`Claimed`, `Canceled`, `Refunded`); omit for both.
   */
  isActive?: boolean;
  /** Keeps only swaps of this type. */
  swapType?: SwapType;
  /** Keeps swaps in any of these statuses, e.g. `["Claimed", "Refunded"]`. */
  status?: SwapStatus[];
  /**
   * Keeps swaps cancelled for any of these reasons, e.g. `["Timeout"]`. Only a
   * cancelled swap has one, so this drops every other swap too.
   */
  errorType?: SwapErrorCode[];
  /** Keeps swaps created at or after this unix timestamp, in seconds. */
  createdAfter?: number;
  /** Keeps swaps created at or before this unix timestamp, in seconds. */
  createdBefore?: number;
  /** Keeps swaps whose received amount is at least this many sats. */
  minAmount?: number;
  /** Keeps swaps whose received amount is at most this many sats. */
  maxAmount?: number;
  /**
   * Keeps swaps where one of `swapId`, `paymentHash`, `claimInvoice`,
   * `sourceTxid`, `destTxid`, or `userClaimAddress` equals this value. The
   * match ignores case but is exact, so a prefix of a txid finds nothing.
   */
  search?: string;
  /**
   * `true` includes each swap's `create_request_*` fields, as `getSwap` does;
   * a page omits them otherwise.
   */
  withCreateRequest?: boolean;
}

/** Options for {@link AnySwapClient.subscribe} and {@link AnySwapClient.signals}. */
export interface SubscribeOptions {
  /**
   * Called with `false` when the socket drops and `true` when one opens, once
   * per change and never after `stop()`. It starts disconnected, so the first
   * call is `true`.
   */
  onConnectionChange?: (connected: boolean) => void;
}

/** A WebSocket subscription handle. */
export interface Subscription {
  /** Closes the subscription and stops reconnecting. */
  stop(): void;
}

/**
 * The WASM driver's `Signals`, returned by {@link AnySwapClient.signals}: a
 * push channel naming swaps worth reading sooner.
 */
export interface SwapSignals extends Subscription {
  /**
   * Resolves with the id of the next swap the server reported, waiting for
   * one when none is queued, and with `undefined` once stopped.
   */
  next(): Promise<string | undefined>;
}

/** HTTP client for the AnySwap server API. */
export class AnySwapClient {
  private readonly baseUrl: string;
  private readonly fetchFn: typeof fetch;
  private readonly wallet?: Wallet;

  /**
   * @param url Base URL of the swap server. Trailing slashes are stripped.
   * @param options Optional `fetch` implementation and the wallet that signs
   * protected swap endpoints.
   */
  constructor(url: string, options: AnySwapClientOptions = {}) {
    this.baseUrl = url.replace(/\/+$/, "");
    const resolvedFetchFn = options.fetchFn ?? fetch;
    this.fetchFn = (...args) => resolvedFetchFn(...args);
    this.wallet = options.wallet;
  }

  /**
   * Fetches server info: the current {@link Types!Policy | policy}, base-fee
   * estimates, and a `quote_id` (valid for `quote_ttl`) locking them.
   */
  async getInfo(): Promise<InfoResponse> {
    const res = await this.fetchFn(`${this.baseUrl}/v1/info`);
    return this.handleResponse(res);
  }

  /**
   * Initiates a swap. The server may cancel the swap right after creation; the
   * actual state is discovered via {@link AnySwapClient.getSwap}.
   *
   * An optional {@link Types!SwapWebhook | webhook} is refused with a `400` when
   * the server has webhooks off or the url breaks its rules; `webhook` on
   * {@link AnySwapClient.getInfo} reports both.
   */
  async createSwap(body: CreateSwapRequest): Promise<void> {
    await this.post("/v1/swap", body, body.swap_id);
  }

  /** Reads the current state of a swap. */
  async getSwap(swapId: string): Promise<SwapState> {
    return this.get(`/v1/swap/${encodeURIComponent(swapId)}`, swapId);
  }

  /**
   * Lists swaps for a user, newest first. Used during recovery to discover swap
   * history after restoring a wallet from its mnemonic.
   */
  async getSwaps(params: GetSwapsParams): Promise<SwapListResponse> {
    const q = new URLSearchParams({
      user_id: params.userId,
      limit: String(params.limit ?? 20),
      offset: String(params.offset ?? 0),
    });
    const filter = (
      key: string,
      value: string | number | boolean | undefined,
    ) => {
      if (value !== undefined && value !== "") {
        q.set(key, String(value));
      }
    };
    filter("is_active", params.isActive);
    filter("swap_type", params.swapType);
    filter("status", params.status?.join(","));
    filter("error_type", params.errorType?.join(","));
    filter("created_after", params.createdAfter);
    filter("created_before", params.createdBefore);
    filter("min_amount", params.minAmount);
    filter("max_amount", params.maxAmount);
    filter("search", params.search);
    filter("with_create_request", params.withCreateRequest);
    return this.get(`/v1/swap?${q.toString()}`);
  }

  /**
   * The listing over a raw {@link SwapListQuery}, the form the WASM driver's
   * `ServerClient` calls; {@link AnySwapClient.getSwaps} is the typed one.
   */
  async listSwaps(query: SwapListQuery): Promise<SwapListResponse> {
    const q = new URLSearchParams();
    for (const [key, value] of Object.entries(query)) {
      if (value !== undefined && value !== null) q.set(key, String(value));
    }
    return this.get(`/v1/swap?${q.toString()}`);
  }

  /**
   * Requests swap cancellation. The outcome is observed via
   * {@link AnySwapClient.getSwap}: `Canceled` reveals the server's claim privkey
   * (swap-in) for an immediate keypath refund of any deposit.
   */
  async cancel(swapId: string, body: CancelRequest): Promise<void> {
    await this.post(
      `/v1/swap/${encodeURIComponent(swapId)}/cancel`,
      body,
      swapId,
    );
  }

  /**
   * Reveals the user's refund privkey after the claim invoice was paid (swap-in
   * only), letting the server claim cooperatively via the keypath instead of the
   * preimage script path.
   */
  async revealClaim(swapId: string, body: RevealClaimRequest): Promise<void> {
    await this.post(
      `/v1/swap/${encodeURIComponent(swapId)}/reveal/claim`,
      body,
      swapId,
    );
  }

  /**
   * Reveals the preimage off-chain (swap-out only) instead of claiming via the
   * script path. The server settles the held invoice and exposes its refund
   * privkey, letting the user finish with a cheap cooperative keypath claim.
   * Reveal only after the opening tx has the required confirmations.
   */
  async revealPreimage(
    swapId: string,
    body: RevealPreimageRequest,
  ): Promise<void> {
    await this.post(
      `/v1/swap/${encodeURIComponent(swapId)}/reveal/preimage`,
      body,
      swapId,
    );
  }

  /**
   * Subscribes to the wallet's swap-change signals over the server's WebSocket.
   * `onUpdate` gets the `{ swap_id, status }` signal per transition, or
   * `undefined` on (re)connect and on an unparseable frame ("resync, swap
   * unknown"). It is a nudge: read authoritative state over HTTP
   * ({@link AnySwapClient.getSwap} / {@link AnySwapClient.getSwaps}) in response;
   * do not trust the pushed `status` for safety-critical decisions. On a drop
   * it reconnects with a freshly signed upgrade after a second, doubling to 30
   * seconds while attempts fail and back to a second once one opens, until the
   * returned handle's `stop()` is called, and `onConnectionChange` hears of
   * each drop and each return. The server pings every 30 seconds to keep the
   * socket alive and allows 16 subscriptions per wallet; a browser tab holds
   * one.
   *
   * Requires a global `WebSocket` (browsers, Node >= 22).
   *
   * @returns a {@link Subscription}; call `stop()` to close it.
   */
  subscribe(
    onUpdate: (update?: SwapStateUpdate) => void,
    options: SubscribeOptions = {},
  ): Subscription {
    const ctor = (
      globalThis as { WebSocket?: new (url: string) => WebSocketLike }
    ).WebSocket;
    if (!ctor) {
      throw new Error("no global WebSocket available (Node < 22 has none)");
    }
    const { userId } = this.requireWallet();
    const path = `/v1/subscribe/${encodeURIComponent(userId)}`;
    let closed = false;
    let ws: WebSocketLike | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let delay = RECONNECT_MIN_MS;
    let connected = false;
    const report = (now: boolean) => {
      if (closed || now === connected) return;
      connected = now;
      options.onConnectionChange?.(now);
    };
    // A wait from the upper half of the delay, so clients dropped together
    // come back apart.
    const retry = () => {
      if (closed) return;
      timer = setTimeout(open, delay * (0.5 + Math.random() / 2));
      delay = Math.min(delay * 2, RECONNECT_MAX_MS);
    };
    const open = async () => {
      try {
        const url = await this.buildWebSocketUrl(path);
        if (closed) return;
        ws = new ctor(url);
      } catch {
        retry();
        return;
      }
      ws.onopen = () => {
        delay = RECONNECT_MIN_MS;
        report(true);
        onUpdate(); // (re)connect: resync, swap unknown
      };
      ws.onmessage = ({ data }) => {
        try {
          onUpdate(JSON.parse(String(data)) as SwapStateUpdate);
        } catch {
          onUpdate();
        }
      };
      ws.onclose = () => {
        retry();
        report(false);
      };
    };
    void open();
    return {
      stop: () => {
        closed = true;
        clearTimeout(timer);
        ws?.close();
      },
    };
  }

  /**
   * Wraps {@link AnySwapClient.subscribe} as the WASM driver's `Signals`: pass
   * the result to `Driver.withSignals`. Every pushed swap id is queued until
   * `next` takes it; the reconnect nudge carries none and is dropped, since the
   * driver reads fresh state on every pass anyway.
   */
  signals(options: SubscribeOptions = {}): SwapSignals {
    const queue: string[] = [];
    let wake: ((id?: string) => void) | null = null;
    let stopped = false;
    const { stop } = this.subscribe((update) => {
      const id = update?.swap_id;
      if (!id) return;
      if (wake) {
        const resolve = wake;
        wake = null;
        resolve(id);
      } else {
        queue.push(id);
      }
    }, options);
    return {
      next(): Promise<string | undefined> {
        if (stopped) return Promise.resolve(undefined);
        const id = queue.shift();
        if (id !== undefined) return Promise.resolve(id);
        return new Promise((resolve) => {
          wake = resolve;
        });
      },
      stop() {
        stopped = true;
        stop();
        wake?.(undefined);
        wake = null;
      },
    };
  }

  private async get<T>(path: string, swapId = ""): Promise<T> {
    const res = await this.fetchFn(`${this.baseUrl}${path}`, {
      headers: await this.authHeaders("GET", path, "", swapId),
    });
    return this.handleResponse(res);
  }

  private async post<T>(
    path: string,
    body: unknown,
    swapId: string,
  ): Promise<T> {
    const rawBody = JSON.stringify(body);
    const res = await this.fetchFn(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...(await this.authHeaders("POST", path, rawBody, swapId)),
      },
      body: rawBody,
    });
    return this.handleResponse(res);
  }

  private async authHeaders(
    method: string,
    target: string,
    rawBody: string,
    swapId: string,
  ): Promise<Record<string, string>> {
    return signRequest(this.requireWallet(), method, target, rawBody, swapId);
  }

  private requireWallet(): Wallet {
    if (!this.wallet) {
      throw new Error("a wallet is required for protected swap endpoints");
    }
    return this.wallet;
  }

  private async buildWebSocketUrl(path: string): Promise<string> {
    // signs the bare path: the query below carries the signature itself
    const headers = await this.authHeaders("GET", path, "", "");
    const q = new URLSearchParams({
      [IDENTITY_PUBKEY_HEADER]: headers[IDENTITY_PUBKEY_HEADER],
      [TIMESTAMP_HEADER]: headers[TIMESTAMP_HEADER],
      [SIGNATURE_HEADER]: headers[SIGNATURE_HEADER],
    });
    return `${this.baseUrl.replace(/^http/, "ws")}${path}?${q.toString()}`;
  }

  private async handleResponse<T>(res: Response): Promise<T> {
    if (!res.ok) {
      const detail = (await res.json().catch(() => ({}))) as ApiError;
      throw new AnySwapError(res.status, detail);
    }
    return (await res.json().catch(() => ({}))) as T;
  }
}
