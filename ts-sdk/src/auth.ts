/**
 * Request-signing helpers for protected AnySwap API endpoints.
 *
 * Most consumers authenticate by passing a `wallet` to {@link Api!AnySwapClient},
 * which signs requests internally. Use these helpers directly only when driving the
 * API with your own HTTP layer instead of {@link Api!AnySwapClient}.
 *
 * @packageDocumentation
 * @module Auth
 */

import type { Wallet } from "./wallet";

/** Names of the auth fields, sent as HTTP headers or WebSocket query params. */
export const IDENTITY_PUBKEY_HEADER = "x-anyswap-identity-pubkey";
export const TIMESTAMP_HEADER = "x-anyswap-timestamp";
export const SIGNATURE_HEADER = "x-anyswap-signature";

/** Request body shapes accepted for hashing into the signed message. */
export type RawBody = string | ArrayBuffer | ArrayBufferView;

/** The three auth fields produced by {@link signRequest}, keyed by header name. */
export type AuthHeaders = Record<
  | typeof IDENTITY_PUBKEY_HEADER
  | typeof TIMESTAMP_HEADER
  | typeof SIGNATURE_HEADER,
  string
>;

const textEncoder = new TextEncoder();

/**
 * Builds the AnySwap auth message, Schnorr-signs it with the wallet's identity
 * key inside the wasm module, and returns the HTTP auth headers.
 *
 * The signature covers `tagged_hash("anyswap/auth/v1", message)`, where
 * `message` concatenates the request fields with a `u32` big-endian length
 * prefix on each variable-length one:
 *
 * `len(METHOD) || METHOD || len(target) || target || SHA256(raw_body_bytes) ||
 * len(swap_id) || swap_id || int64_be(timestamp) || compressed_pubkey`
 *
 * `target` is the URI path together with its query string, so filters and
 * pagination are signed too. Pass the path alone when the auth fields travel in
 * the query string, as the WebSocket upgrade does, since a signature cannot
 * cover the value carrying it.
 */
export async function signRequest(
  wallet: Wallet,
  method: string,
  target: string,
  rawBody: RawBody = new Uint8Array(),
  swapId?: string | null,
  timestamp?: number | null,
): Promise<AuthHeaders> {
  const body = bodyBytes(rawBody);
  const timestampSeconds =
    timestamp === undefined || timestamp === null
      ? Math.floor(Date.now() / 1000)
      : Math.trunc(timestamp);

  return {
    [IDENTITY_PUBKEY_HEADER]: wallet.identityPubkey,
    [TIMESTAMP_HEADER]: String(timestampSeconds),
    [SIGNATURE_HEADER]: wallet.inner.signRequest(
      method,
      target,
      body,
      swapId ?? "",
      BigInt(timestampSeconds),
    ),
  };
}

function bodyBytes(body: RawBody): Uint8Array {
  if (typeof body === "string") return textEncoder.encode(body);
  if (body instanceof ArrayBuffer) return new Uint8Array(body);
  return new Uint8Array(body.buffer, body.byteOffset, body.byteLength);
}
