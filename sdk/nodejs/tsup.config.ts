import { defineConfig } from "tsup";

export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm", "cjs"],
  // eventsource-parser 4 is ESM-only. Bundle it so the CommonJS SDK export does
  // not rely on Node's experimental require(ESM) interoperability at our floor.
  noExternal: ["eventsource-parser"],
  // tsup 8 injects baseUrl into its declaration-bundling compiler options.
  // TypeScript 6 deprecates that internal option; keep the suppression scoped
  // to declaration bundling so the SDK's own typecheck remains strict.
  dts: {
    compilerOptions: {
      ignoreDeprecations: "6.0",
    },
  },
  clean: true,
  sourcemap: true,
  target: "node22",
});
