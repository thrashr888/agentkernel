import { readdirSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { describe, it } from "node:test";
import { fileURLToPath } from "node:url";
import { compileFunction } from "node:vm";

const require = createRequire(import.meta.url);
const fixtures = new URL("./fixtures/braces-upstream/", import.meta.url);

// Execute the unmodified upstream 3.0.3 assertions using Node's built-in test
// runner. These synchronous fixtures use only describe/it from Mocha. Resolve
// their relative imports against our backport, and Bash through PATH, avoiding
// the obsolete upstream development dependency tree in production installs.
function upstreamRequire(id) {
  if (id === "mocha") return {};
  if (id === "bash-path") return () => "bash";
  if (id === "..") return require("braces");
  if (id.startsWith("../lib/")) return require(`braces/${id.slice(3)}`);
  return require(id);
}

for (const file of readdirSync(fixtures).filter((file) => file.endsWith(".js")).sort()) {
  const location = new URL(file, fixtures);
  compileFunction(readFileSync(location, "utf8"), ["require", "describe", "it"], {
    filename: fileURLToPath(location),
  })(upstreamRequire, describe, it);
}
