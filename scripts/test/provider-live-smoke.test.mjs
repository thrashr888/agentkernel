import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

const provider = process.env.AGENTKERNEL_LIVE_PROVIDER;
const bridgePath = fileURLToPath(new URL("../remote-bridge.mjs", import.meta.url));

const credentialEnv = {
  daytona: ["DAYTONA_API_KEY"],
  runloop: ["RUNLOOP_API_KEY"],
  e2b: ["E2B_API_KEY"],
  modal: ["MODAL_TOKEN_ID", "MODAL_TOKEN_SECRET"],
};

function runBridge(request, tempRoot) {
  const payload = Buffer.from(JSON.stringify(request)).toString("base64");
  const result = spawnSync(process.execPath, [bridgePath, provider, payload], {
    encoding: "utf8",
    env: {
      ...process.env,
      AGENTKERNEL_REMOTE_TMPDIR: tempRoot,
    },
  });

  let response;
  try {
    response = JSON.parse(result.stdout.trim());
  } catch {
    throw new Error(`provider bridge returned invalid JSON (status ${result.status})`);
  }
  if (result.status !== 0 || response.success === false) {
    throw new Error(response.error || `provider bridge failed (status ${result.status})`);
  }
  return response;
}

test(
  `credentialed ${provider || "unknown"} provider smoke`,
  { skip: !provider },
  async () => {
  assert.ok(credentialEnv[provider], `unsupported live provider: ${provider}`);
  for (const envName of credentialEnv[provider]) {
    assert.ok(process.env[envName], `${envName} is required for live smoke`);
  }

  const tempParent = process.env.AGENTKERNEL_LIVE_ROOT || os.tmpdir();
  const tempRoot = await fs.mkdtemp(
    path.join(tempParent, `agentkernel-${provider}-live-`),
  );
  const sandboxName = `agentkernel-ci-${provider}-${Date.now()}-${process.pid}`;
  const request = { sandbox_name: sandboxName };
  let created;
  let destroyed = false;

  try {
    created = runBridge({ ...request, operation: "create", ports: [] }, tempRoot);
    assert.equal(created.success, true);
    assert.equal(created.running, true);
    await fs.writeFile(
      path.join(tempRoot, "live-resource.json"),
      JSON.stringify({ ...request, remote_id: created.remote_id, remote_metadata: created.remote_metadata }),
    );

    const status = runBridge({ ...request, operation: "status" }, tempRoot);
    assert.equal(status.remote_id, created.remote_id);

    const executed = runBridge(
      {
        ...request,
        operation: "exec",
        command: ["sh", "-lc", "printf live-smoke"],
      },
      tempRoot,
    );
    assert.equal(executed.exit_code, 0);
    assert.equal(executed.stdout, "live-smoke");

    const content = Buffer.from(`${provider}-live-smoke`).toString("base64");
    runBridge(
      {
        ...request,
        operation: "write_file",
        path: "/workspace/agentkernel-live-smoke.txt",
        content_base64: content,
      },
      tempRoot,
    );
    const read = runBridge(
      {
        ...request,
        operation: "read_file",
        path: "/workspace/agentkernel-live-smoke.txt",
      },
      tempRoot,
    );
    assert.equal(read.content_base64, content);

    runBridge({ ...request, operation: "stop" }, tempRoot);
    const resumed = runBridge({ ...request, operation: "resume" }, tempRoot);
    assert.equal(resumed.running, true);
  } finally {
    // Destroy is deliberately in finally: provider resources must not survive a
    // failed assertion or a partially completed smoke test.
    if (created?.remote_id) {
      try {
        runBridge({ ...request, operation: "destroy" }, tempRoot);
        destroyed = true;
      } catch (error) {
        console.error(`provider cleanup failed: ${error.message}`);
      }
    }
    // Keep the manifest when destroy failed so the workflow's always() cleanup
    // step can retry the provider deletion with the same remote identifier.
    if (!created?.remote_id || destroyed) {
      await fs.rm(tempRoot, { recursive: true, force: true });
    }
  }
  },
);
