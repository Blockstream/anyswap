/**
 * BIP39/BIP32 HD wallet for swap key management.
 *
 * The {@link Driver!Driver | Driver} derives every swap's keys and preimage from this
 * wallet and signs with them inside the SDK, and
 * {@link Api!AnySwapClient | AnySwapClient} signs protected API requests with its
 * identity key, so no key leaves the wasm module and a caller keeps only the
 * mnemonic. The wallet never holds user balances (deposits go directly to the P2TR
 * swap address).
 *
 * ## Key derivation
 *
 * The BIP39 seed is first domain-separated as
 * `HMAC-SHA512("anyswap/user-wallet/v2", seed)`, so no BIP32 wallet on the same
 * mnemonic shares a key with this one. Hardened branches `m/84'/COIN'/<branch>'`
 * are derived from the result, and each per-swap key is
 * `SHA-256(xpriv_secret_bytes || swap_id_bytes)` on its branch.
 *
 * Branches 0 and 1 hold the source refund and destination claim keys, 2 and 3
 * the source and destination blinding keys (Liquid only), 4 the preimages and
 * 5 the identity key. Every role has its own branch, so the two sides of a swap
 * under one `swap_id` never share a key. `COIN` is `0` for mainnet, `1` for
 * testnet/signet/regtest.
 *
 * `user_id` is `SHA-256` of the identity pubkey (the public key on branch 5),
 * not of the xpub itself.
 *
 * Wallets from SDKs before 1.3.0 derived from the untagged seed, so the
 * same mnemonic gives a different `user_id` and different keys here.
 *
 * @packageDocumentation
 * @module Wallet
 */

import type { Wallet as WasmWallet } from "@blockstream/anyswap-wasm";
import { getWasm } from "./wasm";
import type { SwapNetwork } from "./types";

/** BIP39/BIP32 HD wallet for swap key management. */
export class Wallet {
  /** Underlying wasm wallet, which {@link Context!SwapContext} forwards as it is. */
  readonly inner: WasmWallet;

  private constructor(inner: WasmWallet) {
    this.inner = inner;
  }

  /**
   * Restores a wallet from an existing BIP39 mnemonic. Only the mnemonic is needed for
   * recovery - the server holds the swap history, queryable via
   * {@link Api!AnySwapClient.getSwaps | AnySwapClient.getSwaps}.
   *
   * @param mnemonic 12- or 24-word BIP39 phrase. Generate a fresh one with
   *   {@link Utils!generateMnemonic | generateMnemonic}.
   * @param network Target {@link Types!SwapNetwork | SwapNetwork} for derivation.
   */
  static fromMnemonic(mnemonic: string, network: SwapNetwork): Wallet {
    return new Wallet(getWasm().Wallet.fromMnemonic(mnemonic, network));
  }

  /**
   * Stable identifier sent to the server in every request that needs user identification.
   * Derived as `SHA-256` of the identity pubkey (the branch-5 public key) - the server
   * never sees that pubkey or the xpub, and cannot link `user_id` to any individual swap key.
   */
  get userId(): string {
    return this.inner.userId;
  }

  /** Network string the wallet was created for. */
  get network(): string {
    return this.inner.network;
  }

  /** Stable compressed identity public key sent with protected API requests. */
  get identityPubkey(): string {
    return this.inner.identityPubkey();
  }

  /**
   * Derives the swap-in public key that goes into the swap script on the
   * source, sent as `source_refund_pubkey`.
   */
  sourceRefundPubkey(swapId: string): string {
    return this.inner.sourceRefundPubkey(swapId);
  }

  /**
   * Derives the swap-out public key that goes into the swap script on the
   * destination, sent as `dest_claim_pubkey`.
   */
  destClaimPubkey(swapId: string): string {
    return this.inner.destClaimPubkey(swapId);
  }
}
