/**
 * Mnemonic, invoice, swap-id, protocol-version, and swap-fee utilities - thin
 * wrappers over the wasm free functions, plus a client-side invoice validator.
 *
 * @packageDocumentation
 * @module Utils
 */

import { getWasm } from "./wasm";
import { NATIVE_ASSET } from "./types";
import type { Chain, InfoResponse, SwapEndpoint } from "./types";

/**
 * Generates a random BIP39 mnemonic. Pass the result to
 * {@link Wallet!Wallet.fromMnemonic | Wallet.fromMnemonic} and show it to the
 * user for backup.
 *
 * @param wordCount Phrase length: `12`, `15`, `18`, `21`, or `24`. 24 words
 *   (256 bits of entropy) is the recommended default.
 */
export function generateMnemonic(wordCount: number): string {
  return getWasm().generateMnemonic(wordCount);
}

/** Validates a BIP39 mnemonic. Returns `true` if valid, `false` otherwise. */
export function validateMnemonic(mnemonic: string): boolean {
  return getWasm().validateMnemonic(mnemonic);
}

/** Extracts the payment hash from a BOLT11 invoice, as hex. */
export function paymentHashFromInvoice(bolt11: string): string {
  return getWasm().paymentHashFromInvoice(bolt11);
}

/**
 * Extracts the amount from a BOLT11 invoice, in millisatoshis. Throws if the
 * invoice does not specify an amount.
 */
export function amountMsatFromInvoice(bolt11: string): bigint {
  return getWasm().amountMsatFromInvoice(bolt11);
}

/**
 * Extracts the invoice expiration time, as seconds since the Unix epoch. Throws
 * if the invoice does not specify an expiry.
 */
export function expiryAtFromInvoice(bolt11: string): number {
  return getWasm().expiryAtFromInvoice(bolt11);
}

/** Extracts the network string from a BOLT11 invoice (e.g. `"regtest"`). */
export function networkFromInvoice(bolt11: string): string {
  return getWasm().networkFromInvoice(bolt11);
}

/**
 * Generates a new random swap id: a UUIDv4 as a 36-character hyphenated string.
 * Pass to {@link Api!AnySwapClient.createSwap | AnySwapClient.createSwap}.
 */
export function newSwapId(): string {
  return getWasm().newSwapId();
}

/**
 * The protocol version this SDK speaks: the release's version, of which only the
 * major and minor version must match the server's. Pass as `protocol_version` to
 * {@link Api!AnySwapClient.createSwap | AnySwapClient.createSwap}; a server on
 * another minor version refuses the swap.
 */
export function protocolVersion(): string {
  return getWasm().protocolVersion();
}

/**
 * The swap fee and total send amount for a swap delivering `receiveAmount` from
 * `source` to `dest`, with the rates read from
 * {@link Api!AnySwapClient.getInfo | getInfo}. The base fee is the sum of the
 * on-chain legs (`fee_estimates[source].source` if the source is on-chain, plus
 * `fee_estimates[dest].dest` if the destination is on-chain); the service rate
 * comes from the destination (`policy.chain[dest].service_ppm`, or
 * `policy.lightning.service_ppm` when the destination is Lightning). Returns
 * satoshis, with `sendAmount = receiveAmount + swapFee`. This one formula covers a
 * SwapIn (`chain -> lightning`), SwapOut (`lightning -> chain`), and SwapChain
 * (`chain -> chain`).
 */
export function swapAmounts(
  receiveAmount: bigint,
  info: InfoResponse,
  source: SwapEndpoint,
  dest: SwapEndpoint,
): { swapFee: bigint; sendAmount: bigint } {
  const { servicePpm, baseFee } = feeInputs(info, source, dest);
  const swapFee = getWasm().swapFee(receiveAmount, servicePpm, baseFee);
  return { swapFee, sendAmount: receiveAmount + swapFee };
}

/**
 * Validates a BOLT11 claim invoice for a swap into Lightning (a SwapIn) whose
 * on-chain `source`, against {@link Api!AnySwapClient.getInfo | getInfo}: the
 * network matches `policy.network` (case-insensitively), the amount is a whole
 * number of sats, and the invoice does not expire before the deposit can confirm
 * on the source chain (`> (confs + 1) * block_time` seconds). The driver's quote
 * checks the amount limits. Throws on failure.
 */
export function validateInvoice(
  bolt11: string,
  info: InfoResponse,
  source: SwapEndpoint,
): void {
  if (source === "lightning") {
    throw new Error(
      "validateInvoice needs an on-chain source (the deposit chain)",
    );
  }
  const invNetwork = networkFromInvoice(bolt11);
  if (invNetwork.toLowerCase() !== info.policy.network.toLowerCase()) {
    throw new Error(
      `invoice network ${invNetwork} does not match ${info.policy.network}`,
    );
  }
  const msat = amountMsatFromInvoice(bolt11);
  if (msat % 1000n !== 0n) {
    throw new Error(
      `invoice amount ${msat} msat is not a whole number of sats`,
    );
  }
  const cp = chainPolicy(info, source.chain);
  const minLifetime = (cp.confs + 1) * cp.block_time;
  const remaining = expiryAtFromInvoice(bolt11) - Math.floor(Date.now() / 1000);
  if (remaining <= minLifetime) {
    throw new Error(
      `invoice expires in ${remaining}s, must exceed ${minLifetime}s`,
    );
  }
}

/**
 * The `dest_chain` and `dest_asset_id` pair naming a swap's on-chain
 * destination.
 *
 * The two fields have to agree, so they come from one lookup rather than being
 * written out separately. The asset id is read from
 * {@link Api!AnySwapClient.getInfo | getInfo} because the chains identify their
 * native asset differently - Bitcoin uses the `"native"` sentinel, Liquid the
 * asset's 32-byte hex, which is network-specific - so resolving it from the
 * server's policy keeps it correct on every network. A Liquid
 * {@link Context!LiquidParams | LiquidParams}' `policyAsset` is this id.
 *
 * Pass `asset` only to name a non-native asset by its policy label.
 */
export function destAsset(
  info: InfoResponse,
  chain: Chain,
  asset?: string,
): { dest_chain: Chain; dest_asset_id: string } {
  return {
    dest_chain: chain,
    dest_asset_id: assetPolicy(info, { chain, asset }).asset_id,
  };
}

/** Resolves the service rate and total base fee for a `source -> dest` swap. */
function feeInputs(
  info: InfoResponse,
  source: SwapEndpoint,
  dest: SwapEndpoint,
): { servicePpm: number; baseFee: bigint } {
  let baseFee = 0n;
  if (source !== "lightning") {
    baseFee += BigInt(feeEstimate(info, source.chain).source);
  }
  if (dest !== "lightning") {
    baseFee += BigInt(feeEstimate(info, dest.chain).dest);
  }
  const servicePpm =
    dest === "lightning"
      ? info.policy.lightning.service_ppm
      : chainPolicy(info, dest.chain).service_ppm;
  return { servicePpm, baseFee };
}

/** The asset policy an on-chain endpoint names, looked up by its policy label. */
function assetPolicy(
  info: InfoResponse,
  endpoint: { chain: Chain; asset?: string },
) {
  const label = endpoint.asset ?? NATIVE_ASSET;
  const policy = chainPolicy(info, endpoint.chain).asset[label];
  if (!policy) {
    throw new Error(
      `no policy for asset "${label}" on chain "${endpoint.chain}"`,
    );
  }
  return policy;
}

function feeEstimate(info: InfoResponse, chain: Chain) {
  const estimate = info.fee_estimates?.[chain];
  if (!estimate) {
    throw new Error(`no fee estimate for chain "${chain}"`);
  }
  return estimate;
}

function chainPolicy(info: InfoResponse, chain: Chain) {
  const policy = info.policy.chain[chain];
  if (!policy) {
    throw new Error(`no policy for chain "${chain}"`);
  }
  return policy;
}
