import assert from "node:assert/strict";
import { readdir, readFile, writeFile } from "node:fs/promises";

const distDirectory = new URL("../dist/", import.meta.url);
const declarationFiles = (await readdir(distDirectory)).filter((file) =>
  file.endsWith(".d.ts"),
);

assert.ok(declarationFiles.includes("index.d.ts"), "Missing ESM entry declaration");

for (const file of declarationFiles) {
  const esmDeclaration = new URL(file, distDirectory);
  const cjsDeclaration = new URL(file.replace(/\.d\.ts$/, ".d.cts"), distDirectory);
  const declaration = await readFile(esmDeclaration, "utf8");
  const commonJsDeclaration = declaration.replace(
    /(from\s+["']\.\/.+?)\.js(["'])/g,
    "$1.cjs$2",
  );

  await writeFile(cjsDeclaration, commonJsDeclaration);
}

const declaration = await readFile(new URL("index.d.ts", distDirectory), "utf8");

for (const publicType of ["AgentKernel", "BrowserSession", "SandboxSession"]) {
  assert.match(
    declaration,
    new RegExp(`\\b${publicType}\\b`),
    `Bundled declarations are missing ${publicType}`,
  );
}

console.log(
  `Node SDK ESM and CommonJS declarations ready (${declarationFiles.length} modules)`,
);
