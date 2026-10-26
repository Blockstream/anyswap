/**
 * Runtime loader for the `@blockstream/anyswap-wasm` module.
 *
 * Wasm initialization is async and environment-specific: browsers fetch the `.wasm`
 * binary over HTTP, Node reads it from the local filesystem. This module abstracts
 * that away behind {@link initializeWasm} and {@link getWasm}.
 *
 * Callers must `await initializeWasm()` once before using any SDK class or function
 * that touches wasm (which is most of them). The environment-specific source is
 * resolved via the `#wasm-loader` conditional import - see `loader-browser.ts` /
 * `loader-node.ts`.
 *
 * @packageDocumentation
 */

import type { InitInput, SwapContext, Wallet } from "@blockstream/anyswap-wasm";

import { resolveWasmSource } from "#wasm-loader";

export type { InitInput, SwapContext, Wallet };

let wasm: typeof import("@blockstream/anyswap-wasm") | null = null;
let initPromise: Promise<void> | null = null;

/**
 * Loads and initializes the wasm module. Idempotent: subsequent calls return
 * immediately if the module is already loaded, or await the in-flight initialization
 * if one is already running.
 *
 * @param source Optional override for the wasm binary. When omitted, the environment
 *   default is used: a CDN URL in browsers, the local `node_modules` path in Node.
 */
export async function initializeWasm(
  source?: InitInput | Promise<InitInput>,
): Promise<void> {
  if (wasm) return;
  if (initPromise) return initPromise;
  initPromise = (async () => {
    const mod = await import("@blockstream/anyswap-wasm");
    const input = source ?? (await resolveWasmSource());
    await mod.default({ module_or_path: await input });
    wasm = mod;
  })();
  try {
    await initPromise;
  } catch (err) {
    initPromise = null;
    throw err;
  }
}

/** Whether {@link initializeWasm} has completed successfully. */
export function isWasmInitialized(): boolean {
  return wasm !== null;
}

/**
 * Returns the initialized wasm module. Throws if {@link initializeWasm} has not been
 * awaited yet. Used internally by the SDK's class wrappers; users rarely call it
 * directly.
 *
 * @internal
 */
export function getWasm(): typeof import("@blockstream/anyswap-wasm") {
  if (!isWasmInitialized())
    throw new Error(
      "WASM not initialized. Call `await initializeWasm()` first.",
    );
  return wasm!;
}
