const assert = require("node:assert/strict");
const test = require("node:test");
const { AgentKernelClient, AgentKernelError } = require("../out/client");

function response(status, body) {
  return {
    ok: status >= 200 && status < 300,
    status,
    text: async () => (body === undefined ? "" : JSON.stringify(body)),
  };
}

test("lists sandboxes and sends the optional bearer token", async () => {
  let request;
  const client = new AgentKernelClient(
    "http://localhost:18888/",
    "secret",
    async (url, init) => {
      request = { url, init };
      return response(200, {
        success: true,
        data: [
          { name: "dev", uuid: "uuid", status: "running", backend: "docker" },
        ],
      });
    },
  );

  const sandboxes = await client.listSandboxes();
  assert.equal(request.url, "http://localhost:18888/sandboxes");
  assert.equal(request.init.headers.Authorization, "Bearer secret");
  assert.deepEqual(sandboxes[0], {
    name: "dev",
    uuid: "uuid",
    status: "running",
    backend: "docker",
  });
});

test("reports API failures without leaking request credentials", async () => {
  const client = new AgentKernelClient(
    "http://localhost:18888",
    "secret",
    async () => response(401, { error: "unauthorized" }),
  );

  await assert.rejects(
    client.listSandboxes(),
    (error) =>
      error instanceof AgentKernelError &&
      error.statusCode === 401 &&
      error.message === "unauthorized" &&
      !error.message.includes("secret"),
  );
});

test("rejects malformed successful responses", async () => {
  const client = new AgentKernelClient(
    "http://localhost:18888",
    undefined,
    async () => response(200, { success: true, data: [{ name: "missing fields" }] }),
  );

  await assert.rejects(client.listSandboxes(), /invalid sandbox entry/);
});
