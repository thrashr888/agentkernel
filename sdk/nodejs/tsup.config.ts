import { defineConfig } from "tsup";

export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm", "cjs"],
  // eventsource-parser 4 is ESM-only. Bundle it so the CommonJS SDK export does
  // not rely on Node's experimental require(ESM) interoperability at our floor.
  noExternal: ["eventsource-parser"],
  // Declarations are emitted separately by TypeScript itself. Keeping tsup on
  // runtime bundles avoids its rollup-plugin-dts dependency, which is not
  // compatible with TypeScript 7 compiler internals.
  dts: false,
  clean: true,
  sourcemap: true,
  target: "node22",
});
