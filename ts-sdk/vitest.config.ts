import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    alias: {
      "#wasm-loader": fileURLToPath(
        new URL("./src/wasm/loader-node.ts", import.meta.url),
      ),
    },
  },
});
