import { defineConfig } from "tsup";

export default defineConfig({
  entry: [
    "src/index.ts",
    "src/esplora.ts",
    "src/wasm/loader-node.ts",
    "src/wasm/loader-browser.ts",
  ],
  format: ["esm", "cjs"],
  dts: true,
  sourcemap: true,
  clean: true,
  target: "es2022",
  shims: true,
  external: ["#wasm-loader"],
});
