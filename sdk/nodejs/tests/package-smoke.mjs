import assert from "node:assert/strict";
import { createRequire } from "node:module";

const esm = await import("../dist/index.js");
const require = createRequire(import.meta.url);
const cjs = require("../dist/index.cjs");

const expectedRuntimeExports = [
  "AgentKernel",
  "AgentKernelError",
  "AuthError",
  "BROWSER_SETUP_CMD",
  "BrowserSession",
  "NetworkError",
  "NotFoundError",
  "SandboxSession",
  "ServerError",
  "StreamError",
  "ValidationError",
];

assert.deepEqual(Object.keys(esm).sort(), expectedRuntimeExports);
assert.deepEqual(Object.keys(cjs).sort(), expectedRuntimeExports);
assert.equal(typeof esm.AgentKernel, "function");
assert.equal(typeof cjs.AgentKernel, "function");

console.log("Node SDK ESM and CommonJS package exports OK");
