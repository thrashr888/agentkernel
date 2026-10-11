import assert from "node:assert/strict";
import http from "node:http";
import { once } from "node:events";
import test from "node:test";

const [major, minor] = process.versions.node.split(".").map(Number);
const supportsUndici8 = major > 22 || (major === 22 && minor >= 19);

// E2B prefers this optional alias on Node >=22.19. Older supported runtimes
// retain the SDK's undici 7 fallback and cannot load undici 8.
test("E2B's patched undici8 alias supports requests and cancellation", {
  skip: !supportsUndici8,
}, async (t) => {
  const { Agent, fetch } = await import("undici8");
  const server = http.createServer((request, response) => {
    if (request.url === "/wait") return;
    response.setHeader("content-type", "application/json");
    response.end(JSON.stringify({ ok: true }));
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const dispatcher = new Agent({ allowH2: true, connections: 1 });
  t.after(async () => {
    await dispatcher.destroy();
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
  });
  const url = `http://127.0.0.1:${server.address().port}`;
  const response = await fetch(url, { dispatcher });
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { ok: true });
  await assert.rejects(
    fetch(`${url}/wait`, { dispatcher, signal: AbortSignal.timeout(100) }),
    { name: "TimeoutError" },
  );
});
