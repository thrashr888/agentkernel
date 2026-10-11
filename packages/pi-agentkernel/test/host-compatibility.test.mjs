import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { discoverAndLoadExtensions } from "@earendil-works/pi-coding-agent";

const packageDir = fileURLToPath(new URL("../", import.meta.url));
const manifest = JSON.parse(await readFile(new URL("../package.json", import.meta.url), "utf8"));
const hostEntry = import.meta.resolve("@earendil-works/pi-coding-agent");
const hostRequire = createRequire(hostEntry);
const semver = hostRequire("semver");
const host = JSON.parse(await readFile(new URL("../package.json", hostEntry), "utf8"));

test("peer range and installed host exclude the vulnerable host releases", () => {
  const range = manifest.peerDependencies["@earendil-works/pi-coding-agent"];
  for (const version of ["0.84.2", "0.99.2", "1.0.0", "2.0.0"]) {
    assert.equal(semver.satisfies(version, range), false, `${version} must be excluded`);
  }
  assert.equal(semver.satisfies("1.0.1", range), true, "keep the minimum patched host");
  assert.equal(semver.satisfies(host.version, range), true, `installed Pi ${host.version}`);

  // Check the actual vulnerable dependency path, not a possibly unrelated hoisted copy.
  const minimatchRequire = createRequire(hostRequire.resolve("minimatch"));
  const expansion = minimatchRequire("brace-expansion/package.json");
  assert.equal(semver.gte(expansion.version, "5.0.12"), true,
    `Pi's minimatch must resolve patched brace-expansion, found ${expansion.version}`);
});

test(`Pi ${host.version} loads and runs the extension contract without live services`, async (t) => {
  const isolatedDir = await mkdtemp(path.join(os.tmpdir(), "agentkernel-pi-contract-"));
  t.after(() => rm(isolatedDir, { recursive: true, force: true }));
  const calls = [];
  const notifications = [];
  let offline = false;

  t.mock.method(globalThis, "fetch", async (url, init = {}) => {
    const requestPath = new URL(url).pathname;
    const body = init.body ? JSON.parse(init.body) : undefined;
    calls.push({ path: requestPath, method: init.method, body });
    if (offline) throw new Error("mock API unavailable");
    let data;
    if (init.method === "POST" && requestPath === "/sandboxes") {
      data = { name: body.name, status: "running", backend: "mock" };
    } else if (init.method === "GET" && /^\/sandboxes\/pi-/.test(requestPath)) {
      data = { name: requestPath.split("/").pop(), status: "running", backend: "mock" };
    } else if (init.method === "POST" && (requestPath === "/run" || requestPath.endsWith("/exec"))) {
      data = { output: "sandbox-ok" };
    } else if (init.method === "DELETE" && /^\/sandboxes\/pi-/.test(requestPath)) {
      data = "deleted";
    } else {
      assert.fail(`Unexpected API request: ${init.method} ${requestPath}`);
    }
    return new Response(JSON.stringify({ success: true, data }), {
      headers: { "content-type": "application/json" },
    });
  });

  // The real public loader exercises Pi's legacy TypeBox alias. Both discovery roots
  // are temporary, so developer configuration and other extensions are never loaded.
  const loaded = await discoverAndLoadExtensions([packageDir], isolatedDir, isolatedDir);
  assert.deepEqual(loaded.errors, []);
  assert.equal(loaded.extensions.length, 1);
  const extension = loaded.extensions[0];
  assert.deepEqual([...extension.tools.keys()].sort(), ["bash", "sandbox_run"]);
  assert.deepEqual([...extension.commands.keys()], ["sandbox"]);
  assert.deepEqual([...extension.handlers.keys()].sort(), ["session_shutdown", "session_start"]);
  const bash = extension.tools.get("bash").definition;
  const oneShot = extension.tools.get("sandbox_run").definition;
  const context = { hasUI: true, ui: { notify: (...args) => notifications.push(args) } };
  const emit = async (event) => {
    for (const handler of extension.handlers.get(event)) await handler({ type: event }, context);
  };

  await emit("session_start");
  assert.equal(calls[0].path, "/sandboxes");
  assert.equal(calls[0].body.image, "node:24-alpine3.24");
  const sandboxName = calls[0].body.name;
  const updates = [];
  const result = await bash.execute("call", { command: "printf sandbox-ok" }, undefined,
    (update) => updates.push(update));
  assert.equal(result.content[0].text, "sandbox-ok");
  assert.equal(result.details.sandboxed, true);
  assert.equal(result.details.sandbox, sandboxName);
  assert.deepEqual(calls.at(-1).body.command, ["sh", "-c", "printf sandbox-ok"]);
  assert.equal(updates.length, 1);

  await extension.commands.get("sandbox").handler("", context);
  assert.match(notifications.at(-1)[0], /Backend: mock/);
  const single = await oneShot.execute("call", { command: "printf once" });
  assert.equal(single.details.error, false);
  assert.deepEqual(calls.at(-1), {
    path: "/run", method: "POST",
    body: { command: ["sh", "-c", "printf once"], image: "alpine:3.24", fast: true },
  });

  offline = true;
  const failure = await bash.execute("call", { command: "unused" });
  assert.equal(failure.details.error, true);
  assert.match(failure.content[0].text, /mock API unavailable/);
  offline = false;
  await emit("session_shutdown");
  assert.deepEqual(calls.at(-1), {
    path: `/sandboxes/${sandboxName}`, method: "DELETE", body: undefined,
  });
  const callCount = calls.length;
  await emit("session_shutdown");
  assert.equal(calls.length, callCount, "repeated shutdown must not delete again");
  await extension.commands.get("sandbox").handler("", context);
  assert.match(notifications.at(-1)[0], /No active sandbox/);
});

test("sandbox installers and documented images use the minimum patched host", async () => {
  const repoDir = fileURLToPath(new URL("../../../", import.meta.url));
  const read = (name) => readFile(path.join(repoDir, name), "utf8");
  const minimum = semver.minVersion(manifest.peerDependencies["@earendil-works/pi-coding-agent"]).version;
  const pinPattern = /@earendil-works\/pi-coding-agent@(\d+\.\d+\.\d+)/g;
  for (const name of ["src/vmm.rs", "templates/pi-sandbox.toml", "docs/agents/pi.md"]) {
    const pins = [...(await read(name)).matchAll(pinPattern)].map((match) => match[1]);
    assert.ok(pins.length > 0, `${name} must include its pinned Pi installer`);
    assert.ok(pins.every((version) => version === minimum), `${name}: ${pins.join(", ")}`);
  }
  const dockerfile = await read("examples/agents/pi/Dockerfile");
  assert.equal(dockerfile.match(/^ARG PI_VERSION=(.+)$/m)?.[1], minimum);
  assert.ok(dockerfile.includes('@earendil-works/pi-coding-agent@${PI_VERSION}'));

  const versions = JSON.parse(await read("examples/agents/tested-versions.json"));
  const pi = versions.agents.find((agent) => agent.id === "pi");
  assert.equal(pi.version, minimum);
  assert.equal(pi.expected_output, minimum);
  assert.ok(pi.install_command.endsWith(`@earendil-works/pi-coding-agent@${minimum}`));
});

test(`Pi ${host.version} also loads the CLI-distributed extension`, async (t) => {
  const isolatedDir = await mkdtemp(path.join(os.tmpdir(), "agentkernel-pi-cli-"));
  t.after(() => rm(isolatedDir, { recursive: true, force: true }));
  t.mock.method(globalThis, "fetch", () => assert.fail("extension loading must remain offline"));
  const extensionDir = fileURLToPath(new URL(
    "../../../plugins/pi/.pi/extensions/agentkernel/", import.meta.url,
  ));
  const loaded = await discoverAndLoadExtensions([extensionDir], isolatedDir, isolatedDir);
  assert.deepEqual(loaded.errors, []);
  assert.equal(loaded.extensions.length, 1);
  assert.deepEqual([...loaded.extensions[0].tools.keys()].sort(), ["bash", "sandbox_run"]);
  assert.deepEqual([...loaded.extensions[0].commands.keys()], ["sandbox"]);
});
