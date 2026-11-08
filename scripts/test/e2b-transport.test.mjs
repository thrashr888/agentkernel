import assert from "node:assert/strict";
import http from "node:http";
import { once } from "node:events";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import test from "node:test";

// E2B >=2.63 depends on undici 8 directly (no optional undici8 alias), which
// requires Node.js >=22.19. Check that the copy E2B resolves supports
// requests and cancellation.
test("E2B's undici 8 dependency supports requests and cancellation", async (t) => {
  const e2bRequire = createRequire(import.meta.resolve("e2b"));
  const undiciEntry = e2bRequire.resolve("undici");
  const { Agent, fetch } = await import(pathToFileURL(undiciEntry).href);
  const { version } = e2bRequire("undici/package.json");
  assert.equal(Number(version.split(".")[0]), 8);
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
