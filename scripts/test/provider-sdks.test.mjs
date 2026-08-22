import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  Devbox,
  DevboxCmdOps,
  DevboxFileOps,
  DevboxOps,
  RunloopSDK,
  Snapshot,
  SnapshotOps,
  toFile,
} from "@runloop/api-client";
import { FileType, Sandbox as E2bSandbox } from "e2b";
import {
  AppService,
  ContainerProcess,
  ImageService,
  ModalClient,
  Sandbox as ModalSandbox,
  SandboxFilesystem,
  SandboxService,
} from "modal";

function assertMethods(type, methods) {
  assert.equal(typeof type, "function");
  for (const method of methods) {
    assert.equal(
      typeof type.prototype[method],
      "function",
      `${type.name}.${String(method)} must remain callable`,
    );
  }
}

test("Runloop SDK preserves the bridge API", () => {
  assertMethods(DevboxOps, ["create", "fromId"]);
  assertMethods(Devbox, [
    "awaitRunning",
    "getInfo",
    "getTunnelUrl",
    "resume",
    "shutdown",
    "snapshotDisk",
    "suspend",
  ]);
  assertMethods(DevboxCmdOps, ["exec", "execAsync"]);
  assertMethods(DevboxFileOps, ["download", "upload"]);
  assertMethods(SnapshotOps, ["fromId"]);
  assertMethods(Snapshot, ["createDevbox", "delete"]);
  assert.equal(typeof toFile, "function");

  const sdk = new RunloopSDK({ bearerToken: "contract-test" });
  assert.equal(typeof sdk.devbox.create, "function");
  assert.equal(typeof sdk.snapshot.fromId, "function");
  assert.equal(typeof sdk.api.devboxes.executions.sendStdIn, "function");
});

test("E2B SDK preserves the bridge API", () => {
  for (const method of [
    "connect",
    "create",
    "deleteSnapshot",
    "getFullInfo",
    "kill",
    "pause",
  ]) {
    assert.equal(typeof E2bSandbox[method], "function");
  }
  assertMethods(E2bSandbox, ["createSnapshot", "kill"]);
  assert.deepEqual(
    [FileType.FILE, FileType.DIR, FileType.SYMLINK],
    ["file", "dir", "symlink"],
  );
});

test("Modal SDK preserves the adapted bridge API", () => {
  assertMethods(ModalClient, ["close"]);
  assertMethods(AppService, ["fromName"]);
  assertMethods(ImageService, ["delete", "fromId", "fromRegistry"]);
  assertMethods(SandboxService, ["create", "fromId", "fromName"]);
  assertMethods(ModalSandbox, [
    "exec",
    "mountImage",
    "poll",
    "snapshotDirectory",
    "terminate",
    "tunnels",
    "unmountImage",
  ]);
  assertMethods(SandboxFilesystem, ["readBytes", "writeBytes"]);
  assertMethods(ContainerProcess, ["wait"]);
  assert.equal(
    typeof Object.getOwnPropertyDescriptor(
      ModalSandbox.prototype,
      "filesystem",
    )?.get,
    "function",
  );
});

const bridgePath = fileURLToPath(new URL("../remote-bridge.mjs", import.meta.url));

function runMockBridge(provider, request, tempRoot) {
  const payload = Buffer.from(JSON.stringify(request)).toString("base64");
  const result = spawnSync(process.execPath, [bridgePath, provider, payload], {
    encoding: "utf8",
    env: {
      ...process.env,
      AGENTKERNEL_REMOTE_BRIDGE_MODE: "mock",
      AGENTKERNEL_REMOTE_TMPDIR: tempRoot,
    },
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);
  const outputLines = result.stdout.trim().split("\n");
  assert.equal(outputLines.length, 1, "bridge must emit exactly one JSON line");
  return JSON.parse(outputLines[0]);
}

test("provider JSON-over-stdio bridge contract remains stable", async () => {
  const tempRoot = await fs.mkdtemp(
    path.join(os.tmpdir(), "agentkernel-provider-contract-"),
  );
  try {
    for (const provider of ["runloop", "e2b", "modal"]) {
      const sandboxName = `${provider}-sdk-contract`;
      const request = { sandbox_name: sandboxName };
      const created = runMockBridge(
        provider,
        { ...request, operation: "create", ports: [] },
        tempRoot,
      );
      assert.equal(created.success, true);
      assert.equal(created.running, true);

      const content = Buffer.from(`${provider}-ok`).toString("base64");
      runMockBridge(
        provider,
        {
          ...request,
          operation: "write_file",
          path: "/workspace/provider.txt",
          content_base64: content,
        },
        tempRoot,
      );
      const read = runMockBridge(
        provider,
        { ...request, operation: "read_file", path: "/workspace/provider.txt" },
        tempRoot,
      );
      assert.equal(read.content_base64, content);

      const status = runMockBridge(
        provider,
        { ...request, operation: "status" },
        tempRoot,
      );
      assert.equal(status.remote_id, created.remote_id);
      assert.equal(status.running, true);

      const stopped = runMockBridge(
        provider,
        { ...request, operation: "stop" },
        tempRoot,
      );
      assert.equal(stopped.running, false);
      const destroyed = runMockBridge(
        provider,
        { ...request, operation: "destroy" },
        tempRoot,
      );
      assert.equal(destroyed.remote_id, created.remote_id);
    }
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
});
