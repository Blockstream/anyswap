/**
 * Browser implementation of {@link resolveWasmSource}. Returns a CDN URL that
 * `wasm-bindgen`'s `init` function fetches at runtime.
 */

import type { InitInput } from "@blockstream/anyswap-wasm";

const WASM_CDN_URL =
  "https://unpkg.com/@blockstream/anyswap-wasm@1.10.1/anyswap_wasm_bg.wasm";

export async function resolveWasmSource(): Promise<InitInput> {
  return WASM_CDN_URL;
}
