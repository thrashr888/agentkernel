#!/usr/bin/env node

import crypto from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { spawn } from "node:child_process";

const [, , provider, payload] = process.argv;

if (!provider || !payload) {
  console.error("usage: remote-bridge.mjs <provider> <base64-request>");
  process.exit(1);
}

// Validate that a string is safe to use as a path component (no separators, no traversal).
function validatePathComponent(value, label) {
  if (!value || /[/\\]|^\.\.?$/.test(value)) {
    throw new Error(`Invalid ${label}: '${value}' must not contain path separators or be a traversal component`);
  }
}

validatePathComponent(provider, "provider");

const request = JSON.parse(Buffer.from(payload, "base64").toString("utf8"));
const mode = process.env.AGENTKERNEL_REMOTE_BRIDGE_MODE ?? "";
try {
  process.loadEnvFile?.();
} catch {
  // Ignore missing or invalid .env files; exported environment wins.
}
let daytonaSdkPromise;
let e2bSdkPromise;
let runloopSdkPromise;

function bridgeTempRoot() {
  if (process.platform !== "win32") {
    return process.env.AGENTKERNEL_REMOTE_TMPDIR || "/tmp";
  }
  return os.tmpdir();
}

const rootDir = path.join(bridgeTempRoot(), "agentkernel-remote-bridge", provider);
await fs.mkdir(rootDir, { recursive: true });
normalizeProviderEnvAliases();

try {
  if (mode === "mock") {
    switch (request.operation) {
      case "create":
        respond(await createSandbox(provider, request));
        break;
      case "resume":
        respond(await resumeSandbox(request));
        break;
      case "status":
        respond(await statusSandbox(request));
        break;
      case "stop":
        respond(await stopSandbox(request));
        break;
      case "destroy":
        respond(await destroySandbox(request));
        break;
      case "exec":
        respond(await execSandbox(request));
        break;
      case "attach":
        process.exit(await attachSandbox(request));
        break;
      case "write_file":
        respond(await writeSandboxFile(request));
        break;
      case "read_file":
        respond(await readSandboxFile(request));
        break;
      case "remove_file":
        respond(await removeSandboxFile(request));
        break;
      case "mkdir":
        respond(await mkdirSandbox(request));
        break;
      case "sync_push":
        respond(await syncPush(request));
        break;
      case "sync_pull":
        respond(await syncPull(request));
        break;
      case "snapshot":
        respond(await takeSnapshot(request));
        break;
      case "delete_snapshot":
        respond(await deleteSnapshot(request));
        break;
      case "restore":
        respond(await restoreSnapshot(request));
        break;
      default:
        respond({
          success: false,
          error: `unsupported remote bridge operation: ${request.operation}`,
        });
    }
  } else {
    await handleProviderRequest(provider, request);
  }
} catch (error) {
  respond({
    success: false,
    error: scrubSecrets(error instanceof Error ? error.message : String(error)),
  });
}

/**
 * Replace known API key values in a string with redacted placeholders so that
 * provider SDK errors (which sometimes include the bearer token in the message)
 * do not leak credentials to the caller.
 */
function scrubSecrets(text) {
  const sensitiveEnvs = [
    "DAYTONA_API_KEY",
    "RUNLOOP_API_KEY",
    "E2B_API_KEY",
    "AGENTKERNEL_API_KEY",
    "AGENTCOMPUTER_API_KEY",
  ];
  let result = text;
  for (const envName of sensitiveEnvs) {
    const value = process.env[envName];
    if (value && value.length >= 8) {
      result = result.replaceAll(value, "[REDACTED]");
    }
  }
  return result;
}

function respond(body) {
  process.stdout.write(`${JSON.stringify(body)}\n`);
  process.exit(body.success === false ? 1 : 0);
}

function promoteEnvAlias(canonical, aliases) {
  if (process.env[canonical]) {
    return;
  }
  for (const alias of aliases) {
    if (process.env[alias]) {
      process.env[canonical] = process.env[alias];
      return;
    }
  }
}

function normalizeProviderEnvAliases() {
  promoteEnvAlias("DAYTONA_API_KEY", ["daytona_api_key"]);
  promoteEnvAlias("DAYTONA_ORGANIZATION_ID", [
    "daytona_organization_key",
    "daytona_org_id",
  ]);
  promoteEnvAlias("E2B_API_KEY", ["E2B_key", "e2b_api_key"]);
  promoteEnvAlias("RUNLOOP_API_KEY", ["runloop_api_key"]);
}

async function handleProviderRequest(providerName, providerRequest) {
  if (providerName === "daytona") {
    switch (providerRequest.operation) {
      case "create":
        respond(await createDaytonaSandbox(providerRequest));
        return;
      case "resume":
        respond(await resumeDaytonaSandbox(providerRequest));
        return;
      case "status":
        respond(await statusDaytonaSandbox(providerRequest));
        return;
      case "stop":
        respond(await stopDaytonaSandbox(providerRequest));
        return;
      case "destroy":
        respond(await destroyDaytonaSandbox(providerRequest));
        return;
      case "exec":
        respond(await execDaytonaSandbox(providerRequest));
        return;
      case "attach":
        process.exit(await attachDaytonaSandbox(providerRequest));
        return;
      case "write_file":
        respond(await writeDaytonaFile(providerRequest));
        return;
      case "read_file":
        respond(await readDaytonaFile(providerRequest));
        return;
      case "remove_file":
        respond(await removeDaytonaFile(providerRequest));
        return;
      case "mkdir":
        respond(await mkdirDaytona(providerRequest));
        return;
      case "sync_push":
        respond(await syncPushDaytona(providerRequest));
        return;
      case "sync_pull":
        respond(await syncPullDaytona(providerRequest));
        return;
      case "snapshot":
        respond(await takeDaytonaSnapshot(providerRequest));
        return;
      case "delete_snapshot":
        respond(await deleteDaytonaSnapshot(providerRequest));
        return;
      case "restore":
        respond(await restoreDaytonaSnapshot(providerRequest));
        return;
      default:
        throw new Error(
          `unsupported Daytona bridge operation: ${providerRequest.operation}`,
        );
    }
  }

  if (providerName === "e2b") {
    switch (providerRequest.operation) {
      case "create":
        respond(await createE2bSandbox(providerRequest));
        return;
      case "resume":
        respond(await resumeE2bSandbox(providerRequest));
        return;
      case "status":
        respond(await statusE2bSandbox(providerRequest));
        return;
      case "stop":
        respond(await stopE2bSandbox(providerRequest));
        return;
      case "destroy":
        respond(await destroyE2bSandbox(providerRequest));
        return;
      case "exec":
        respond(await execE2bSandbox(providerRequest));
        return;
      case "attach":
        process.exit(await attachE2bSandbox(providerRequest));
        return;
      case "write_file":
        respond(await writeE2bFile(providerRequest));
        return;
      case "read_file":
        respond(await readE2bFile(providerRequest));
        return;
      case "remove_file":
        respond(await removeE2bFile(providerRequest));
        return;
      case "mkdir":
        respond(await mkdirE2b(providerRequest));
        return;
      case "sync_push":
        respond(await syncPushE2b(providerRequest));
        return;
      case "sync_pull":
        respond(await syncPullE2b(providerRequest));
        return;
      case "snapshot":
        respond(await takeE2bSnapshot(providerRequest));
        return;
      case "delete_snapshot":
        respond(await deleteE2bSnapshot(providerRequest));
        return;
      case "restore":
        respond(await restoreE2bSnapshot(providerRequest));
        return;
      default:
        throw new Error(
          `unsupported E2B bridge operation: ${providerRequest.operation}`,
        );
    }
  }

  if (providerName === "runloop") {
    switch (providerRequest.operation) {
      case "create":
        respond(await createRunloopSandbox(providerRequest));
        return;
      case "resume":
        respond(await resumeRunloopSandbox(providerRequest));
        return;
      case "status":
        respond(await statusRunloopSandbox(providerRequest));
        return;
      case "stop":
        respond(await stopRunloopSandbox(providerRequest));
        return;
      case "destroy":
        respond(await destroyRunloopSandbox(providerRequest));
        return;
      case "exec":
        respond(await execRunloopSandbox(providerRequest));
        return;
      case "attach":
        process.exit(await attachRunloopSandbox(providerRequest));
        return;
      case "write_file":
        respond(await writeRunloopFile(providerRequest));
        return;
      case "read_file":
        respond(await readRunloopFile(providerRequest));
        return;
      case "remove_file":
        respond(await removeRunloopFile(providerRequest));
        return;
      case "mkdir":
        respond(await mkdirRunloop(providerRequest));
        return;
      case "sync_push":
        respond(await syncPushRunloop(providerRequest));
        return;
      case "sync_pull":
        respond(await syncPullRunloop(providerRequest));
        return;
      case "snapshot":
        respond(await takeRunloopSnapshot(providerRequest));
        return;
      case "delete_snapshot":
        respond(await deleteRunloopSnapshot(providerRequest));
        return;
      case "restore":
        respond(await restoreRunloopSnapshot(providerRequest));
        return;
      default:
        throw new Error(
          `unsupported Runloop bridge operation: ${providerRequest.operation}`,
        );
    }
  }

  respond({
    success: false,
    error:
      `Remote provider bridge for '${providerName}' is not configured. ` +
      `Set AGENTKERNEL_REMOTE_BRIDGE_MODE=mock for local testing or install a provider-aware bridge.`,
  });
}

async function loadDaytonaSdk() {
  if (!daytonaSdkPromise) {
    daytonaSdkPromise = import("@daytonaio/sdk").catch((error) => {
      throw new Error(
        "Daytona support requires '@daytonaio/sdk'. Run 'npm install --prefix scripts' first. " +
          `(${error.message})`,
      );
    });
  }
  return daytonaSdkPromise;
}

async function loadE2bSdk() {
  if (!e2bSdkPromise) {
    e2bSdkPromise = import("e2b").catch((error) => {
      throw new Error(
        "E2B support requires 'e2b'. Run 'npm install --prefix scripts' first. " +
          `(${error.message})`,
      );
    });
  }
  return e2bSdkPromise;
}

async function loadRunloopSdk() {
  if (!runloopSdkPromise) {
    runloopSdkPromise = import("@runloop/api-client").catch((error) => {
      throw new Error(
        "Runloop support requires '@runloop/api-client'. Run 'npm install --prefix scripts' first. " +
          `(${error.message})`,
      );
    });
  }
  return runloopSdkPromise;
}

async function withDaytonaClient(fn) {
  const { Daytona } = await loadDaytonaSdk();
  const client = new Daytona({
    apiKey: process.env.DAYTONA_API_KEY,
    apiUrl: process.env.DAYTONA_API_URL,
    target: process.env.DAYTONA_TARGET || "us",
    organizationId:
      process.env.DAYTONA_ORGANIZATION_ID || process.env.DAYTONA_ORG_ID,
  });

  try {
    return await fn(client);
  } finally {
    if (client[Symbol.asyncDispose]) {
      await client[Symbol.asyncDispose]();
    }
  }
}

async function withRunloopSdk(fn) {
  const { RunloopSDK, toFile } = await loadRunloopSdk();
  const sdk = new RunloopSDK({
    bearerToken: process.env.RUNLOOP_API_KEY,
    baseURL: process.env.RUNLOOP_BASE_URL || undefined,
  });
  return fn(sdk, { toFile });
}

function daytonaEnvMap(values = {}) {
  return Object.fromEntries(
    Object.entries(values).map(([key, value]) => [key, String(value)]),
  );
}

function daytonaShellQuote(value) {
  return `'${String(value).replace(/'/g, `'\\''`)}'`;
}

function daytonaCommand(argv = []) {
  return argv.map(daytonaShellQuote).join(" ");
}

function daytonaMemoryGiB(memoryMb) {
  if (!memoryMb || Number.isNaN(Number(memoryMb))) {
    return undefined;
  }
  return Math.max(1, Math.ceil(Number(memoryMb) / 1024));
}

function daytonaPortSpec(port) {
  return `${port.container_port}/${port.protocol ?? "tcp"}`;
}

function daytonaRunning(state) {
  return state === "started" || state === "running";
}

function daytonaWorkspacePath(providerRequest) {
  return providerRequest.path || "/workspace";
}

function daytonaPosixPath(basePath, relPath = "") {
  const normalizedBase = path.posix.normalize(basePath || "/workspace");
  return relPath ? path.posix.join(normalizedBase, relPath) : normalizedBase;
}

async function ensureDaytonaFolder(sandbox, folderPath) {
  try {
    await sandbox.fs.createFolder(folderPath, "755");
  } catch {
    // Treat existing folders as success.
  }
}

function deleteSort(relPaths) {
  return [...relPaths].sort((left, right) => {
    const leftDepth = left.split("/").length;
    const rightDepth = right.split("/").length;
    if (leftDepth !== rightDepth) {
      return rightDepth - leftDepth;
    }
    return right.length - left.length;
  });
}

async function collectDaytonaEntries(sandbox, basePath, ignoreRules) {
  const entries = new Map();
  await walkDaytona(sandbox, daytonaPosixPath(basePath), "", entries, ignoreRules);
  return entries;
}

async function walkDaytona(sandbox, basePath, relDir, entries, ignoreRules) {
  const currentPath = relDir ? daytonaPosixPath(basePath, relDir) : basePath;
  let children = [];
  try {
    children = await sandbox.fs.listFiles(currentPath);
  } catch {
    return;
  }

  for (const child of children) {
    const relPath = relDir ? path.posix.join(relDir, child.name) : child.name;
    if (shouldIgnore(relPath, ignoreRules)) {
      continue;
    }
    if (child.isDir) {
      entries.set(relPath, { kind: "dir" });
      await walkDaytona(sandbox, basePath, relPath, entries, ignoreRules);
    } else {
      entries.set(relPath, { kind: "file" });
    }
  }
}

async function hashDaytonaTree(sandbox, basePath, ignoreRules = []) {
  const entries = await collectDaytonaEntries(sandbox, basePath, ignoreRules);
  const hasher = crypto.createHash("sha256");
  for (const relPath of [...entries.keys()].sort()) {
    const entry = entries.get(relPath);
    hasher.update(relPath);
    hasher.update(entry.kind);
    if (entry.kind === "file") {
      const content = await sandbox.fs.downloadFile(daytonaPosixPath(basePath, relPath));
      hasher.update(content);
    }
  }
  return hasher.digest("hex");
}

async function mirrorLocalToDaytona(sandbox, sourceDir, remoteBasePath, ignoreRules) {
  const sourceEntries = await collectEntries(sourceDir, ignoreRules);
  const remoteEntries = await collectDaytonaEntries(sandbox, remoteBasePath, ignoreRules);

  for (const relPath of deleteSort(remoteEntries.keys())) {
    if (!sourceEntries.has(relPath)) {
      const entry = remoteEntries.get(relPath);
      await sandbox.fs.deleteFile(
        daytonaPosixPath(remoteBasePath, relPath),
        entry?.kind === "dir",
      );
    }
  }

  await ensureDaytonaFolder(sandbox, daytonaPosixPath(remoteBasePath));

  const directories = [...sourceEntries.entries()]
    .filter(([, entry]) => entry.kind === "dir")
    .map(([relPath]) => relPath)
    .sort();
  for (const relPath of directories) {
    await ensureDaytonaFolder(sandbox, daytonaPosixPath(remoteBasePath, relPath));
  }

  const fileUploads = [...sourceEntries.entries()]
    .filter(([, entry]) => entry.kind === "file")
    .map(([relPath]) => ({
      source: path.join(sourceDir, relPath),
      destination: daytonaPosixPath(remoteBasePath, relPath),
    }));
  for (let index = 0; index < fileUploads.length; index += 32) {
    await sandbox.fs.uploadFiles(fileUploads.slice(index, index + 32));
  }
}

async function mirrorDaytonaToLocal(sandbox, remoteBasePath, targetDir, ignoreRules) {
  await fs.mkdir(targetDir, { recursive: true });
  const remoteEntries = await collectDaytonaEntries(sandbox, remoteBasePath, ignoreRules);
  const localEntries = await collectEntries(targetDir, ignoreRules);

  for (const relPath of deleteSort(localEntries.keys())) {
    if (!remoteEntries.has(relPath)) {
      await fs.rm(path.join(targetDir, relPath), { recursive: true, force: true });
    }
  }

  const directories = [...remoteEntries.entries()]
    .filter(([, entry]) => entry.kind === "dir")
    .map(([relPath]) => relPath)
    .sort();
  for (const relPath of directories) {
    await fs.mkdir(path.join(targetDir, relPath), { recursive: true });
  }

  const downloads = [...remoteEntries.entries()]
    .filter(([, entry]) => entry.kind === "file")
    .map(([relPath]) => ({
      source: daytonaPosixPath(remoteBasePath, relPath),
      destination: path.join(targetDir, relPath),
    }));

  for (let index = 0; index < downloads.length; index += 32) {
    const batch = downloads.slice(index, index + 32);
    const results = await sandbox.fs.downloadFiles(batch);
    const error = results.find((result) => result.error);
    if (error) {
      throw new Error(`failed to download ${error.source}: ${error.error}`);
    }
  }
}

async function getDaytonaSandbox(daytona, providerRequest) {
  const remoteId =
    providerRequest.remote_id || providerRequest.remote_metadata?.daytona_id;
  if (!remoteId) {
    throw new Error("missing remote_id for Daytona sandbox");
  }
  return daytona.get(remoteId);
}

async function daytonaEndpoints(sandbox, providerRequest) {
  const portSpecs =
    providerRequest.remote_metadata?.published_ports?.split(",").filter(Boolean) ??
    (providerRequest.ports || []).map(daytonaPortSpec);

  const endpoints = [];
  for (const spec of portSpecs) {
    const [portString, protocol = "tcp"] = spec.split("/");
    const port = Number.parseInt(portString, 10);
    if (!Number.isFinite(port)) {
      continue;
    }

    try {
      const preview = await sandbox.getPreviewLink(port);
      endpoints.push({
        container_port: port,
        protocol,
        url: preview.url,
      });
    } catch {
      // Leave the endpoint absent if Daytona cannot open a preview yet.
    }
  }
  return endpoints;
}

async function daytonaResponse(sandbox, providerRequest, extra = {}) {
  const endpoints =
    extra.endpoints ?? (await daytonaEndpoints(sandbox, providerRequest));

  return {
    success: true,
    remote_id: sandbox.id,
    remote_metadata: {
      ...(providerRequest.remote_metadata || {}),
      daytona_id: sandbox.id,
      sandbox_state: sandbox.state || "",
      profile_name:
        providerRequest.profile || providerRequest.image || sandbox.image || "default",
      published_ports:
        providerRequest.remote_metadata?.published_ports ||
        (providerRequest.ports || []).map(daytonaPortSpec).join(","),
      last_known_status: daytonaRunning(sandbox.state) ? "running" : "stopped",
    },
    endpoints,
    running: daytonaRunning(sandbox.state),
    ...extra,
  };
}

async function createDaytonaSandbox(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await daytona.create({
      name: providerRequest.sandbox_name,
      image: providerRequest.image || "alpine:3.20",
      envVars: daytonaEnvMap(providerRequest.env),
      resources: {
        cpu: providerRequest.vcpus || 1,
        memory: daytonaMemoryGiB(providerRequest.memory_mb),
      },
      autoStopInterval: 15,
      public: Boolean(providerRequest.ports?.length),
    });
    return daytonaResponse(sandbox, providerRequest);
  });
}

async function resumeDaytonaSandbox(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    if (!daytonaRunning(sandbox.state)) {
      await sandbox.start();
    }
    await sandbox.refreshDataSafe();
    return daytonaResponse(sandbox, providerRequest);
  });
}

async function statusDaytonaSandbox(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    await sandbox.refreshDataSafe();
    return daytonaResponse(sandbox, providerRequest);
  });
}

async function stopDaytonaSandbox(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    if (daytonaRunning(sandbox.state)) {
      await sandbox.stop();
      await sandbox.refreshDataSafe();
    }
    return daytonaResponse(sandbox, providerRequest);
  });
}

async function destroyDaytonaSandbox(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    await sandbox.delete();
    return {
      success: true,
      remote_id: sandbox.id,
      remote_metadata: {
        ...(providerRequest.remote_metadata || {}),
        daytona_id: sandbox.id,
        last_known_status: "stopped",
      },
      running: false,
      endpoints: [],
    };
  });
}

async function execDaytonaSandbox(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const result = await sandbox.process.executeCommand(
      daytonaCommand(providerRequest.command || []),
      providerRequest.workdir,
      daytonaEnvMap(providerRequest.env),
    );

    return daytonaResponse(sandbox, providerRequest, {
      exit_code: result.exitCode ?? 0,
      stdout:
        result.result || result.artifacts?.stdout || result.stdout || "",
      stderr: result.stderr || result.artifacts?.stderr || "",
    });
  });
}

async function writeDaytonaFile(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    await sandbox.fs.uploadFile(
      Buffer.from(providerRequest.content_base64 || "", "base64"),
      providerRequest.path,
    );
    return daytonaResponse(sandbox, providerRequest);
  });
}

async function readDaytonaFile(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const content = await sandbox.fs.downloadFile(providerRequest.path);
    return daytonaResponse(sandbox, providerRequest, {
      content_base64: Buffer.from(content).toString("base64"),
    });
  });
}

async function removeDaytonaFile(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    await sandbox.process.executeCommand(
      `rm -rf -- ${daytonaShellQuote(providerRequest.path)}`,
    );
    return daytonaResponse(sandbox, providerRequest);
  });
}

async function mkdirDaytona(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const flag = providerRequest.recursive ? "-p " : "";
    await sandbox.process.executeCommand(
      `mkdir ${flag}-- ${daytonaShellQuote(providerRequest.path)}`,
    );
    return daytonaResponse(sandbox, providerRequest);
  });
}

async function syncPushDaytona(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const localPath = providerRequest.local_path;
    if (!localPath) {
      throw new Error("sync_push requires local_path");
    }

    const ignoreRules = await loadIgnoreRules(localPath);
    const workspacePath = daytonaWorkspacePath(providerRequest);
    const remoteRevision = await hashDaytonaTree(sandbox, workspacePath, ignoreRules);
    if (providerRequest.workspace_revision && remoteRevision !== providerRequest.workspace_revision) {
      throw new Error("workspace sync conflict: remote workspace changed since last sync");
    }

    await mirrorLocalToDaytona(sandbox, localPath, workspacePath, ignoreRules);
    const workspaceRevision = await hashDaytonaTree(sandbox, workspacePath, ignoreRules);
    return daytonaResponse(sandbox, providerRequest, { workspace_revision: workspaceRevision });
  });
}

async function syncPullDaytona(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const localPath = providerRequest.local_path;
    if (!localPath) {
      throw new Error("sync_pull requires local_path");
    }

    const ignoreRules = await loadIgnoreRules(localPath);
    const localRevision = await hashTree(localPath, ignoreRules);
    if (providerRequest.workspace_revision && localRevision !== providerRequest.workspace_revision) {
      throw new Error("workspace sync conflict: local workspace changed since last sync");
    }

    const workspacePath = daytonaWorkspacePath(providerRequest);
    await mirrorDaytonaToLocal(sandbox, workspacePath, localPath, ignoreRules);
    const workspaceRevision = await hashDaytonaTree(sandbox, workspacePath, ignoreRules);
    return daytonaResponse(sandbox, providerRequest, { workspace_revision: workspaceRevision });
  });
}

async function takeDaytonaSnapshot(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const snapshotName = providerRequest.snapshot_name || `snapshot-${Date.now()}`;
    const destination = snapshotDir(sandbox.id, snapshotName);
    await fs.rm(destination, { recursive: true, force: true });
    await fs.mkdir(destination, { recursive: true });

    const workspacePath = daytonaWorkspacePath(providerRequest);
    await mirrorDaytonaToLocal(sandbox, workspacePath, destination, []);
    const workspaceRevision = await hashDaytonaTree(sandbox, workspacePath, []);
    const response = await daytonaResponse(sandbox, providerRequest, {
      workspace_revision: workspaceRevision,
    });
    response.remote_metadata = {
      ...response.remote_metadata,
      snapshot_handle: encodeSnapshotHandle(sandbox.id, snapshotName),
    };
    return response;
  });
}

async function restoreDaytonaSnapshot(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const snapshotHandle = providerRequest.snapshot_name;
    if (!snapshotHandle) {
      throw new Error("restore requires snapshot_name");
    }

    const { remoteId, snapshotName } = decodeSnapshotHandle(
      snapshotHandle,
      sandbox.id,
    );
    const source = snapshotDir(remoteId, snapshotName);
    await fs.access(source);

    const workspacePath = daytonaWorkspacePath(providerRequest);
    await mirrorLocalToDaytona(sandbox, source, workspacePath, []);
    const workspaceRevision = await hashDaytonaTree(sandbox, workspacePath, []);
    return daytonaResponse(sandbox, providerRequest, { workspace_revision: workspaceRevision });
  });
}

async function deleteDaytonaSnapshot(providerRequest) {
  const snapshotHandle = providerRequest.snapshot_name;
  if (!snapshotHandle) {
    throw new Error("delete_snapshot requires snapshot_name");
  }
  const { remoteId, snapshotName } = decodeSnapshotHandle(snapshotHandle);
  await fs.rm(snapshotDir(remoteId, snapshotName), {
    recursive: true,
    force: true,
  });
  return { success: true };
}

async function attachDaytonaSandbox(providerRequest) {
  return withDaytonaClient(async (daytona) => {
    const sandbox = await getDaytonaSandbox(daytona, providerRequest);
    const pty = await sandbox.process.createPty({
      id: `agentkernel-${Date.now()}`,
      cwd: providerRequest.workdir,
      envs: daytonaEnvMap(providerRequest.env),
      cols: process.stdout.columns || 120,
      rows: process.stdout.rows || 30,
      onData: (data) => {
        process.stdout.write(Buffer.from(data));
      },
    });

    await pty.waitForConnection();

    const resize = async () => {
      if (process.stdout.isTTY) {
        await pty.resize(process.stdout.columns || 120, process.stdout.rows || 30);
      }
    };

    const onData = (chunk) => {
      void pty.sendInput(chunk);
    };

    if (process.stdin.isTTY) {
      process.stdin.setRawMode(true);
    }
    process.stdin.resume();
    process.stdin.on("data", onData);
    process.stdout.on("resize", resize);

    try {
      const result = await pty.wait();
      return result.exitCode ?? 0;
    } finally {
      process.stdin.off("data", onData);
      process.stdout.off("resize", resize);
      if (process.stdin.isTTY) {
        process.stdin.setRawMode(false);
      }
      await pty.disconnect().catch(() => {});
    }
  });
}

function e2bWorkspacePath(providerRequest) {
  return providerRequest.path || "/workspace";
}

function e2bPosixPath(basePath, relPath = "") {
  const normalizedBase = path.posix.normalize(basePath || "/workspace");
  return relPath ? path.posix.join(normalizedBase, relPath) : normalizedBase;
}

function e2bPublishedPorts(providerRequest) {
  return (
    providerRequest.remote_metadata?.published_ports?.split(",").filter(Boolean) ??
    (providerRequest.ports || []).map(daytonaPortSpec)
  );
}

function e2bDebug() {
  return String(process.env.E2B_DEBUG || "").toLowerCase() === "true";
}

function e2bHostUrl(info, port) {
  if (!Number.isFinite(port)) {
    return null;
  }
  if (e2bDebug()) {
    return `http://localhost:${port}`;
  }
  if (!info.sandboxDomain) {
    return null;
  }
  return `https://${port}-${info.sandboxId}.${info.sandboxDomain}`;
}

async function getE2bRemoteId(providerRequest) {
  const remoteId = providerRequest.remote_id || providerRequest.remote_metadata?.e2b_id;
  if (!remoteId) {
    throw new Error("missing remote_id for E2B sandbox");
  }
  return remoteId;
}

async function getE2bSandbox(providerRequest) {
  const { Sandbox } = await loadE2bSdk();
  return Sandbox.connect(await getE2bRemoteId(providerRequest));
}

async function getE2bInfo(providerRequest) {
  const { Sandbox } = await loadE2bSdk();
  return Sandbox.getFullInfo(await getE2bRemoteId(providerRequest));
}

async function e2bEndpoints(info, providerRequest) {
  const endpoints = [];
  for (const spec of e2bPublishedPorts(providerRequest)) {
    const [portString, protocol = "tcp"] = spec.split("/");
    const port = Number.parseInt(portString, 10);
    const url = e2bHostUrl(info, port);
    if (!url) {
      continue;
    }
    endpoints.push({
      container_port: port,
      protocol,
      url,
    });
  }
  return endpoints;
}

async function e2bResponse(providerRequest, info, extra = {}) {
  const endpoints = extra.endpoints ?? (await e2bEndpoints(info, providerRequest));
  return {
    success: true,
    remote_id: info.sandboxId,
    remote_metadata: {
      ...(providerRequest.remote_metadata || {}),
      e2b_id: info.sandboxId,
      sandbox_state: info.state || "",
      sandbox_domain: info.sandboxDomain || "",
      template_id: info.templateId || "",
      profile_name:
        providerRequest.profile || providerRequest.image || info.templateId || "base",
      published_ports: e2bPublishedPorts(providerRequest).join(","),
      last_known_status: info.state === "running" ? "running" : "stopped",
    },
    endpoints,
    running: info.state === "running",
    ...extra,
  };
}

async function collectE2bEntries(sandbox, basePath, ignoreRules, fileType) {
  const entries = new Map();
  await walkE2b(sandbox, e2bPosixPath(basePath), "", entries, ignoreRules, fileType);
  return entries;
}

async function walkE2b(
  sandbox,
  basePath,
  relDir,
  entries,
  ignoreRules,
  fileType,
) {
  const currentPath = relDir ? e2bPosixPath(basePath, relDir) : basePath;
  let children = [];
  try {
    children = await sandbox.files.list(currentPath);
  } catch {
    return;
  }

  for (const child of children) {
    const childPath = child.path || e2bPosixPath(currentPath, child.name || "");
    const relPath = path.posix.relative(basePath, childPath);
    if (!relPath || shouldIgnore(relPath, ignoreRules)) {
      continue;
    }

    if (child.type === fileType.DIR) {
      entries.set(relPath, { kind: "dir" });
      await walkE2b(sandbox, basePath, relPath, entries, ignoreRules, fileType);
    } else if (child.type === fileType.FILE) {
      entries.set(relPath, { kind: "file" });
    }
  }
}

async function hashE2bTree(sandbox, basePath, ignoreRules = []) {
  const { FileType } = await loadE2bSdk();
  const entries = await collectE2bEntries(sandbox, basePath, ignoreRules, FileType);
  const hasher = crypto.createHash("sha256");
  for (const relPath of [...entries.keys()].sort()) {
    const entry = entries.get(relPath);
    hasher.update(relPath);
    hasher.update(entry.kind);
    if (entry.kind === "file") {
      const content = await sandbox.files.read(e2bPosixPath(basePath, relPath), {
        format: "bytes",
      });
      hasher.update(Buffer.from(content));
    }
  }
  return hasher.digest("hex");
}

function bufferToArrayBuffer(buffer) {
  return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}

async function mirrorLocalToE2b(sandbox, sourceDir, remoteBasePath, ignoreRules) {
  const { FileType } = await loadE2bSdk();
  const sourceEntries = await collectEntries(sourceDir, ignoreRules);
  const remoteEntries = await collectE2bEntries(
    sandbox,
    remoteBasePath,
    ignoreRules,
    FileType,
  );

  for (const relPath of deleteSort(remoteEntries.keys())) {
    if (!sourceEntries.has(relPath)) {
      await sandbox.files.remove(e2bPosixPath(remoteBasePath, relPath)).catch(async () => {
        await sandbox.commands.run(
          `rm -rf -- ${daytonaShellQuote(e2bPosixPath(remoteBasePath, relPath))}`,
        );
      });
    }
  }

  await sandbox.files.makeDir(e2bPosixPath(remoteBasePath));

  const directories = [...sourceEntries.entries()]
    .filter(([, entry]) => entry.kind === "dir")
    .map(([relPath]) => relPath)
    .sort();
  for (const relPath of directories) {
    await sandbox.files.makeDir(e2bPosixPath(remoteBasePath, relPath));
  }

  const fileUploads = [];
  for (const [relPath, entry] of sourceEntries.entries()) {
    if (entry.kind !== "file") {
      continue;
    }
    const sourcePath = path.join(sourceDir, relPath);
    fileUploads.push({
      path: e2bPosixPath(remoteBasePath, relPath),
      data: bufferToArrayBuffer(await fs.readFile(sourcePath)),
    });
  }

  for (let index = 0; index < fileUploads.length; index += 32) {
    await sandbox.files.write(fileUploads.slice(index, index + 32));
  }
}

async function mirrorE2bToLocal(sandbox, remoteBasePath, targetDir, ignoreRules) {
  const { FileType } = await loadE2bSdk();
  await fs.mkdir(targetDir, { recursive: true });
  const remoteEntries = await collectE2bEntries(
    sandbox,
    remoteBasePath,
    ignoreRules,
    FileType,
  );
  const localEntries = await collectEntries(targetDir, ignoreRules);

  for (const relPath of deleteSort(localEntries.keys())) {
    if (!remoteEntries.has(relPath)) {
      await fs.rm(path.join(targetDir, relPath), { recursive: true, force: true });
    }
  }

  const directories = [...remoteEntries.entries()]
    .filter(([, entry]) => entry.kind === "dir")
    .map(([relPath]) => relPath)
    .sort();
  for (const relPath of directories) {
    await fs.mkdir(path.join(targetDir, relPath), { recursive: true });
  }

  for (const [relPath, entry] of remoteEntries.entries()) {
    if (entry.kind !== "file") {
      continue;
    }
    const content = await sandbox.files.read(e2bPosixPath(remoteBasePath, relPath), {
      format: "bytes",
    });
    const destination = path.join(targetDir, relPath);
    await fs.mkdir(path.dirname(destination), { recursive: true });
    await fs.writeFile(destination, Buffer.from(content));
  }
}

async function createE2bSandbox(providerRequest) {
  const { Sandbox } = await loadE2bSdk();
  const restoreSnapshot = providerRequest.remote_metadata?.restore_snapshot;
  const templateOrSnapshot =
    restoreSnapshot || providerRequest.profile || providerRequest.image || "base";
  const sandbox = await Sandbox.create(templateOrSnapshot, {
    envs: providerRequest.env || {},
    metadata: {
      agentkernel_name: providerRequest.sandbox_name,
    },
    timeoutMs: 15 * 60 * 1000,
    allowInternetAccess: providerRequest.network !== false,
    lifecycle: {
      onTimeout: "pause",
      autoResume: true,
    },
    secure: true,
  });
  const info = await Sandbox.getFullInfo(sandbox.sandboxId);
  return e2bResponse(providerRequest, info);
}

async function resumeE2bSandbox(providerRequest) {
  const sandbox = await getE2bSandbox(providerRequest);
  const { Sandbox } = await loadE2bSdk();
  const info = await Sandbox.getFullInfo(sandbox.sandboxId);
  return e2bResponse(providerRequest, info);
}

async function statusE2bSandbox(providerRequest) {
  return e2bResponse(providerRequest, await getE2bInfo(providerRequest));
}

async function stopE2bSandbox(providerRequest) {
  const { Sandbox } = await loadE2bSdk();
  await Sandbox.pause(await getE2bRemoteId(providerRequest));
  return e2bResponse(providerRequest, await getE2bInfo(providerRequest));
}

async function destroyE2bSandbox(providerRequest) {
  const { Sandbox } = await loadE2bSdk();
  const remoteId = await getE2bRemoteId(providerRequest);
  await Sandbox.kill(remoteId);
  return {
    success: true,
    remote_id: remoteId,
    remote_metadata: {
      ...(providerRequest.remote_metadata || {}),
      e2b_id: remoteId,
      last_known_status: "stopped",
    },
    running: false,
    endpoints: [],
  };
}

async function execE2bSandbox(providerRequest) {
  const sandbox = await getE2bSandbox(providerRequest);
  const { Sandbox } = await loadE2bSdk();
  let result;
  try {
    result = await sandbox.commands.run(daytonaCommand(providerRequest.command || []), {
      cwd: providerRequest.workdir,
      envs: providerRequest.env || {},
    });
  } catch (error) {
    if (typeof error?.exitCode === "number") {
      result = error;
    } else {
      throw error;
    }
  }
  const info = await Sandbox.getFullInfo(sandbox.sandboxId);
  return e2bResponse(providerRequest, info, {
    exit_code: result.exitCode ?? 0,
    stdout: result.stdout || "",
    stderr: result.stderr || "",
  });
}

async function writeE2bFile(providerRequest) {
  const sandbox = await getE2bSandbox(providerRequest);
  const { Sandbox } = await loadE2bSdk();
  await sandbox.files.write(
    providerRequest.path,
    bufferToArrayBuffer(Buffer.from(providerRequest.content_base64 || "", "base64")),
  );
  const info = await Sandbox.getFullInfo(sandbox.sandboxId);
  return e2bResponse(providerRequest, info);
}

async function readE2bFile(providerRequest) {
  const sandbox = await getE2bSandbox(providerRequest);
  const { Sandbox } = await loadE2bSdk();
  const content = await sandbox.files.read(providerRequest.path, { format: "bytes" });
  const info = await Sandbox.getFullInfo(sandbox.sandboxId);
  return e2bResponse(providerRequest, info, {
    content_base64: Buffer.from(content).toString("base64"),
  });
}

async function removeE2bFile(providerRequest) {
  const sandbox = await getE2bSandbox(providerRequest);
  const { Sandbox } = await loadE2bSdk();
  await sandbox.files.remove(providerRequest.path);
  const info = await Sandbox.getFullInfo(sandbox.sandboxId);
  return e2bResponse(providerRequest, info);
}

async function mkdirE2b(providerRequest) {
  const sandbox = await getE2bSandbox(providerRequest);
  const { Sandbox } = await loadE2bSdk();
  const flag = providerRequest.recursive ? "-p " : "";
  await sandbox.commands.run(`mkdir ${flag}-- ${daytonaShellQuote(providerRequest.path)}`);
  const info = await Sandbox.getFullInfo(sandbox.sandboxId);
  return e2bResponse(providerRequest, info);
}

async function syncPushE2b(providerRequest) {
  const sandbox = await getE2bSandbox(providerRequest);
  const localPath = providerRequest.local_path;
  if (!localPath) {
    throw new Error("sync_push requires local_path");
  }

  const ignoreRules = await loadIgnoreRules(localPath);
  const workspacePath = e2bWorkspacePath(providerRequest);
  const remoteRevision = await hashE2bTree(sandbox, workspacePath, ignoreRules);
  if (providerRequest.workspace_revision && remoteRevision !== providerRequest.workspace_revision) {
    throw new Error("workspace sync conflict: remote workspace changed since last sync");
  }

  await mirrorLocalToE2b(sandbox, localPath, workspacePath, ignoreRules);
  const workspaceRevision = await hashE2bTree(sandbox, workspacePath, ignoreRules);
  const { Sandbox } = await loadE2bSdk();
  const info = await Sandbox.getFullInfo(sandbox.sandboxId);
  return e2bResponse(providerRequest, info, { workspace_revision: workspaceRevision });
}

async function syncPullE2b(providerRequest) {
  const sandbox = await getE2bSandbox(providerRequest);
  const localPath = providerRequest.local_path;
  if (!localPath) {
    throw new Error("sync_pull requires local_path");
  }

  const ignoreRules = await loadIgnoreRules(localPath);
  const localRevision = await hashTree(localPath, ignoreRules);
  if (providerRequest.workspace_revision && localRevision !== providerRequest.workspace_revision) {
    throw new Error("workspace sync conflict: local workspace changed since last sync");
  }

  const workspacePath = e2bWorkspacePath(providerRequest);
  await mirrorE2bToLocal(sandbox, workspacePath, localPath, ignoreRules);
  const workspaceRevision = await hashE2bTree(sandbox, workspacePath, ignoreRules);
  const { Sandbox } = await loadE2bSdk();
  const info = await Sandbox.getFullInfo(sandbox.sandboxId);
  return e2bResponse(providerRequest, info, { workspace_revision: workspaceRevision });
}

async function takeE2bSnapshot(providerRequest) {
  const sandbox = await getE2bSandbox(providerRequest);
  const { Sandbox } = await loadE2bSdk();
  const snapshot = await sandbox.createSnapshot();
  await Sandbox.connect(sandbox.sandboxId);
  const info = await Sandbox.getFullInfo(sandbox.sandboxId);
  const response = await e2bResponse(providerRequest, info);
  response.remote_metadata = {
    ...response.remote_metadata,
    snapshot_handle: snapshot.snapshotId,
  };
  return response;
}

async function deleteE2bSnapshot(providerRequest) {
  const snapshotHandle = providerRequest.snapshot_name;
  if (!snapshotHandle) {
    throw new Error("delete_snapshot requires snapshot_name");
  }
  const { Sandbox } = await loadE2bSdk();
  await Sandbox.deleteSnapshot(snapshotHandle);
  return { success: true };
}

async function restoreE2bSnapshot(providerRequest) {
  const snapshotHandle = providerRequest.snapshot_name;
  if (!snapshotHandle) {
    throw new Error("restore requires snapshot_name");
  }

  const { Sandbox } = await loadE2bSdk();
  const target = await getE2bSandbox(providerRequest);
  const source = await Sandbox.create(snapshotHandle, {
    timeoutMs: 5 * 60 * 1000,
    allowInternetAccess: providerRequest.network !== false,
    lifecycle: {
      onTimeout: "kill",
      autoResume: false,
    },
    secure: true,
  });
  const tmpDir = await fs.mkdtemp(
    path.join(bridgeTempRoot(), "agentkernel-e2b-restore-"),
  );

  try {
    await mirrorE2bToLocal(source, "/workspace", tmpDir, []);
    await mirrorLocalToE2b(target, tmpDir, e2bWorkspacePath(providerRequest), []);
  } finally {
    await fs.rm(tmpDir, { recursive: true, force: true }).catch(() => {});
    await source.kill().catch(() => {});
  }

  const info = await Sandbox.getFullInfo(target.sandboxId);
  const workspaceRevision = await hashE2bTree(
    target,
    e2bWorkspacePath(providerRequest),
    [],
  );
  return e2bResponse(providerRequest, info, { workspace_revision: workspaceRevision });
}

async function attachE2bSandbox(providerRequest) {
  const sandbox = await getE2bSandbox(providerRequest);
  const pty = await sandbox.pty.create({
    cols: process.stdout.columns || 120,
    rows: process.stdout.rows || 30,
    cwd: providerRequest.workdir,
    envs: providerRequest.env || {},
    onData: (data) => {
      process.stdout.write(Buffer.from(data));
    },
  });

  const resize = async () => {
    if (process.stdout.isTTY) {
      await sandbox.pty
        .resize(pty.pid, {
          cols: process.stdout.columns || 120,
          rows: process.stdout.rows || 30,
        })
        .catch(() => {});
    }
  };

  const onData = (chunk) => {
    void sandbox.pty.sendInput(pty.pid, chunk).catch(() => {});
  };

  if (process.stdin.isTTY) {
    process.stdin.setRawMode(true);
  }
  process.stdin.resume();
  process.stdin.on("data", onData);
  process.stdout.on("resize", resize);

  try {
    const result = await pty.wait();
    return result.exitCode ?? 0;
  } catch (error) {
    if (typeof error?.exitCode === "number") {
      return error.exitCode;
    }
    throw error;
  } finally {
    process.stdin.off("data", onData);
    process.stdout.off("resize", resize);
    if (process.stdin.isTTY) {
      process.stdin.setRawMode(false);
    }
    await pty.disconnect().catch(() => {});
  }
}

function runloopRunning(status) {
  return ["provisioning", "initializing", "running", "resuming"].includes(
    status || "",
  );
}

function runloopProfileName(providerRequest, info = null) {
  return (
    providerRequest.profile ||
    providerRequest.image ||
    providerRequest.remote_metadata?.profile_name ||
    info?.blueprint_id ||
    "default"
  );
}

function runloopPublishedPorts(providerRequest) {
  return (
    providerRequest.remote_metadata?.published_ports?.split(",").filter(Boolean) ??
    (providerRequest.ports || []).map(daytonaPortSpec)
  );
}

function escapeForDoubleQuotes(value) {
  return String(value).replace(/["\\$`]/g, "\\$&");
}

function runloopAssignmentValue(requestedPath) {
  const normalized = path.posix.normalize(requestedPath || "/workspace");
  if (normalized === "/workspace") {
    return '"$HOME/workspace"';
  }
  if (normalized.startsWith("/workspace/")) {
    const relPath = normalized.slice("/workspace/".length);
    return `"$HOME/workspace/${escapeForDoubleQuotes(relPath)}"`;
  }
  return daytonaShellQuote(normalized);
}

function runloopRewriteWorkspaceText(value) {
  return String(value).replaceAll("/workspace", "$HOME/workspace");
}

function runloopCommandArg(arg) {
  return runloopAssignmentValue(String(arg));
}

function runloopCommand(argv = []) {
  if (
    argv.length >= 3 &&
    ["sh", "bash", "zsh"].includes(argv[0]) &&
    argv[1] === "-lc"
  ) {
    const shell = daytonaShellQuote(argv[0]);
    const script = daytonaShellQuote(runloopRewriteWorkspaceText(argv[2]));
    const rest = argv.slice(3).map(runloopCommandArg).join(" ");
    return [shell, "-lc", script, rest].filter(Boolean).join(" ");
  }
  return argv.map(runloopCommandArg).join(" ");
}

function runloopArchiveRelativePath(providerRequest) {
  const suffix = (providerRequest.remote_id ||
    providerRequest.sandbox_name ||
    "workspace")
    .replace(/[^A-Za-z0-9_.-]+/g, "-")
    .slice(0, 96);
  return `.agentkernel/agentkernel-${suffix}.tgz`;
}

function runloopWorkspaceRelativePath(providerRequest) {
  const workspacePath = providerRequest.path || "/workspace";
  const normalized = path.posix.normalize(workspacePath);
  if (normalized === "/workspace") {
    return "workspace";
  }
  if (normalized.startsWith("/workspace/")) {
    return path.posix.join("workspace", normalized.slice("/workspace/".length));
  }
  throw new Error(`Runloop workspace sync expects /workspace, got ${workspacePath}`);
}

function runloopValidateEnvName(name) {
  if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) {
    throw new Error(`invalid environment variable name: ${name}`);
  }
}

function runloopWrappedScript(mainScript, options = {}) {
  const env = options.env || {};
  const workdir = options.workdir || "/workspace";
  const lines = [
    "set -e",
    'mkdir -p "$HOME/.agentkernel" "$HOME/workspace"',
    'if [ ! -e /workspace ]; then ln -s "$HOME/workspace" /workspace 2>/dev/null || true; fi',
    `AK_WORKDIR_PRIMARY=${daytonaShellQuote(workdir)}`,
    'case "$AK_WORKDIR_PRIMARY" in',
    '  /workspace*) AK_WORKDIR_FALLBACK="$HOME/workspace${AK_WORKDIR_PRIMARY#/workspace}" ;;',
    '  *) AK_WORKDIR_FALLBACK="" ;;',
    "esac",
    'if [ -n "$AK_WORKDIR_FALLBACK" ]; then mkdir -p "$AK_WORKDIR_FALLBACK"; fi',
    'if [ -n "$AK_WORKDIR_FALLBACK" ]; then',
    '  cd "$AK_WORKDIR_PRIMARY" 2>/dev/null || cd "$AK_WORKDIR_FALLBACK"',
    "else",
    '  cd "$AK_WORKDIR_PRIMARY"',
    "fi",
  ];

  for (const [key, value] of Object.entries(env)) {
    runloopValidateEnvName(key);
    lines.push(`export ${key}=${daytonaShellQuote(value)}`);
  }

  lines.push(mainScript);
  return lines.join("\n");
}

async function runloopExec(devbox, mainScript, options = {}) {
  const result = await devbox.cmd.exec(
    runloopWrappedScript(mainScript, options),
    options.shellName ? { shell_name: options.shellName } : undefined,
  );
  return {
    exitCode: result.exitCode ?? 0,
    stdout: await result.stdout(),
    stderr: await result.stderr(),
  };
}

async function ensureRunloopWorkspace(devbox) {
  await runloopExec(devbox, ":", { workdir: "/workspace" });
}

function runloopMemoryGiB(memoryMb) {
  return daytonaMemoryGiB(memoryMb);
}

function runloopResourceSize(providerRequest) {
  const vcpus = Number(providerRequest.vcpus || 1);
  const memoryGiB = Number(runloopMemoryGiB(providerRequest.memory_mb) || 1);

  if (vcpus <= 1 && memoryGiB <= 1) {
    return { resource_size_request: "X_SMALL" };
  }
  if (vcpus <= 2 && memoryGiB <= 2) {
    return { resource_size_request: "SMALL" };
  }
  if (vcpus <= 4 && memoryGiB <= 4) {
    return { resource_size_request: "MEDIUM" };
  }

  return {
    resource_size_request: "CUSTOM_SIZE",
    custom_cpu_cores: providerRequest.vcpus,
    custom_gb_memory: memoryGiB,
  };
}

function runloopCreateParams(providerRequest) {
  const params = {
    name: providerRequest.sandbox_name,
    environment_variables: providerRequest.env || {},
    metadata: {
      agentkernel_name: providerRequest.sandbox_name,
    },
    launch_parameters: {
      after_idle: {
        idle_time_seconds: 15 * 60,
        on_idle: "suspend",
      },
    },
  };

  Object.assign(
    params.launch_parameters,
    runloopResourceSize(providerRequest),
  );

  const profileName = runloopProfileName(providerRequest);
  if (profileName && !["default", "base"].includes(profileName)) {
    params.blueprint_name = profileName;
  }

  if ((providerRequest.ports || []).length > 0) {
    params.tunnel = {
      auth_mode: "open",
      http_keep_alive: true,
    };
  }

  return params;
}

function getRunloopRemoteId(providerRequest) {
  const remoteId =
    providerRequest.remote_id || providerRequest.remote_metadata?.runloop_id;
  if (!remoteId) {
    throw new Error("missing remote_id for Runloop devbox");
  }
  return remoteId;
}

function getRunloopDevbox(sdk, providerRequest) {
  return sdk.devbox.fromId(getRunloopRemoteId(providerRequest));
}

async function ensureRunloopTunnel(devbox, info, providerRequest) {
  if (runloopPublishedPorts(providerRequest).length === 0) {
    return info;
  }
  if (info.tunnel) {
    return info;
  }
  await devbox.net.enableTunnel({
    auth_mode: "open",
    http_keep_alive: true,
  });
  return devbox.getInfo();
}

async function ensureRunloopOperational(devbox, providerRequest) {
  let info = await devbox.getInfo();

  if (info.status === "shutdown") {
    throw new Error(`Runloop devbox '${info.id}' is shut down and cannot be resumed`);
  }

  if (info.status === "suspended") {
    await devbox.resume();
    info = await devbox.getInfo();
  } else if (!runloopRunning(info.status)) {
    await devbox.awaitRunning();
    info = await devbox.getInfo();
  }

  await ensureRunloopWorkspace(devbox);
  return ensureRunloopTunnel(devbox, await devbox.getInfo(), providerRequest);
}

async function runloopEndpoints(devbox, info, providerRequest) {
  if (!info.tunnel) {
    return [];
  }

  const endpoints = [];
  for (const spec of runloopPublishedPorts(providerRequest)) {
    const [portString, protocol = "tcp"] = spec.split("/");
    const port = Number.parseInt(portString, 10);
    if (!Number.isFinite(port)) {
      continue;
    }
    try {
      endpoints.push({
        container_port: port,
        protocol,
        url: await devbox.getTunnelUrl(port),
      });
    } catch {
      // Skip ports without a resolved tunnel URL.
    }
  }
  return endpoints;
}

async function runloopResponse(devbox, providerRequest, info, extra = {}) {
  const endpoints =
    extra.endpoints ?? (await runloopEndpoints(devbox, info, providerRequest));

  return {
    success: true,
    remote_id: info.id,
    remote_metadata: {
      ...(providerRequest.remote_metadata || {}),
      runloop_id: info.id,
      devbox_status: info.status || "",
      profile_name: runloopProfileName(providerRequest, info),
      published_ports: runloopPublishedPorts(providerRequest).join(","),
      tunnel_key:
        info.tunnel?.tunnel_key ||
        providerRequest.remote_metadata?.tunnel_key ||
        "",
      last_known_status: runloopRunning(info.status) ? "running" : "stopped",
    },
    endpoints,
    running: runloopRunning(info.status),
    ...extra,
  };
}

async function createLocalArchiveFromWorkspace(sourceDir, ignoreRules) {
  const tmpRoot = await fs.mkdtemp(
    path.join(bridgeTempRoot(), "agentkernel-runloop-workspace-"),
  );
  const stageDir = path.join(tmpRoot, "stage");
  const archivePath = path.join(tmpRoot, "workspace.tgz");
  await fs.mkdir(stageDir, { recursive: true });
  await mirrorDirectory(sourceDir, stageDir, ignoreRules);
  const result = await spawnCommand(["tar", "-czf", archivePath, "-C", stageDir, "."], {
    stdio: "pipe",
  });
  if (result.exitCode !== 0) {
    throw new Error(`failed to create workspace archive: ${result.stderr || result.stdout}`);
  }
  return { tmpRoot, archivePath };
}

async function extractLocalArchiveToDir(archivePath, targetDir) {
  await fs.mkdir(targetDir, { recursive: true });
  const result = await spawnCommand(["tar", "-xzf", archivePath, "-C", targetDir], {
    stdio: "pipe",
  });
  if (result.exitCode !== 0) {
    throw new Error(`failed to extract workspace archive: ${result.stderr || result.stdout}`);
  }
}

async function uploadRunloopArchive(devbox, providerRequest, archivePath) {
  const { toFile } = await loadRunloopSdk();
  const archive = await fs.readFile(archivePath);
  await devbox.file.upload({
    path: runloopArchiveRelativePath(providerRequest),
    file: await toFile(archive, "workspace.tgz", {
      type: "application/gzip",
    }),
  });
}

async function createRunloopWorkspaceArchive(devbox, providerRequest) {
  const archiveRel = runloopArchiveRelativePath(providerRequest);
  const workspaceRel = runloopWorkspaceRelativePath(providerRequest);
  await runloopExec(
    devbox,
    [
      `AK_ARCHIVE="$HOME/${archiveRel}"`,
      `AK_WORKSPACE="$HOME/${workspaceRel}"`,
      'mkdir -p "$(dirname "$AK_ARCHIVE")" "$AK_WORKSPACE"',
      'tar -czf "$AK_ARCHIVE" -C "$AK_WORKSPACE" .',
    ].join("\n"),
    { workdir: "/workspace" },
  );
  return archiveRel;
}

async function downloadRunloopArchiveBuffer(devbox, providerRequest) {
  const response = await devbox.file.download({
    path: runloopArchiveRelativePath(providerRequest),
  });
  return Buffer.from(await response.arrayBuffer());
}

async function hashRunloopWorkspace(devbox, providerRequest, ignoreRules = []) {
  const tmpRoot = await fs.mkdtemp(
    path.join(bridgeTempRoot(), "agentkernel-runloop-hash-"),
  );
  const archivePath = path.join(tmpRoot, "workspace.tgz");
  const extractDir = path.join(tmpRoot, "workspace");
  try {
    await createRunloopWorkspaceArchive(devbox, providerRequest);
    await fs.writeFile(archivePath, await downloadRunloopArchiveBuffer(devbox, providerRequest));
    await extractLocalArchiveToDir(archivePath, extractDir);
    return hashTree(extractDir, ignoreRules);
  } finally {
    await fs.rm(tmpRoot, { recursive: true, force: true }).catch(() => {});
  }
}

async function replaceRunloopWorkspace(devbox, providerRequest) {
  const archiveRel = runloopArchiveRelativePath(providerRequest);
  const workspaceRel = runloopWorkspaceRelativePath(providerRequest);
  await runloopExec(
    devbox,
    [
      `AK_ARCHIVE="$HOME/${archiveRel}"`,
      `AK_WORKSPACE="$HOME/${workspaceRel}"`,
      'mkdir -p "$(dirname "$AK_ARCHIVE")" "$AK_WORKSPACE"',
      '(cd "$AK_WORKSPACE" && find . -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +)',
      'tar -xzf "$AK_ARCHIVE" -C "$AK_WORKSPACE"',
      'rm -f "$AK_ARCHIVE"',
    ].join("\n"),
    { workdir: "/workspace" },
  );
}

async function createRunloopSandbox(providerRequest) {
  return withRunloopSdk(async (sdk) => {
    const devbox = await sdk.devbox.create(runloopCreateParams(providerRequest));
    await ensureRunloopWorkspace(devbox);
    const info = await ensureRunloopTunnel(devbox, await devbox.getInfo(), providerRequest);
    return runloopResponse(devbox, providerRequest, info);
  });
}

async function resumeRunloopSandbox(providerRequest) {
  return withRunloopSdk(async (sdk) => {
    const devbox = getRunloopDevbox(sdk, providerRequest);
    const info = await ensureRunloopOperational(devbox, providerRequest);
    return runloopResponse(devbox, providerRequest, info);
  });
}

async function statusRunloopSandbox(providerRequest) {
  return withRunloopSdk(async (sdk) => {
    const devbox = getRunloopDevbox(sdk, providerRequest);
    const info = await devbox.getInfo();
    return runloopResponse(devbox, providerRequest, info);
  });
}

async function stopRunloopSandbox(providerRequest) {
  return withRunloopSdk(async (sdk) => {
    const devbox = getRunloopDevbox(sdk, providerRequest);
    const info = await devbox.getInfo();
    if (info.status === "running") {
      await devbox.suspend();
    }
    const updated = await devbox.getInfo();
    return runloopResponse(devbox, providerRequest, updated);
  });
}

async function destroyRunloopSandbox(providerRequest) {
  return withRunloopSdk(async (sdk) => {
    const devbox = getRunloopDevbox(sdk, providerRequest);
    const remoteId = getRunloopRemoteId(providerRequest);
    try {
      await devbox.shutdown();
    } catch {
      // Treat an already-shutdown devbox as removed.
    }
    return {
      success: true,
      remote_id: remoteId,
      remote_metadata: {
        ...(providerRequest.remote_metadata || {}),
        runloop_id: remoteId,
        last_known_status: "stopped",
      },
      running: false,
      endpoints: [],
    };
  });
}

async function execRunloopSandbox(providerRequest) {
  return withRunloopSdk(async (sdk) => {
    const devbox = getRunloopDevbox(sdk, providerRequest);
    const info = await ensureRunloopOperational(devbox, providerRequest);
    const command = providerRequest.command || [];
    if (command.length === 0) {
      throw new Error("exec requires a command");
    }

    const result = await runloopExec(devbox, runloopCommand(command), {
      env: providerRequest.env || {},
      workdir: providerRequest.workdir || "/workspace",
    });
    const updated = await devbox.getInfo();
    return runloopResponse(devbox, providerRequest, updated, {
      exit_code: result.exitCode,
      stdout: result.stdout,
      stderr: result.stderr,
    });
  });
}

async function writeRunloopFile(providerRequest) {
  return withRunloopSdk(async (sdk) => {
    const devbox = getRunloopDevbox(sdk, providerRequest);
    await ensureRunloopOperational(devbox, providerRequest);
    const target = providerRequest.path;
    if (!target) {
      throw new Error("write_file requires path");
    }
    const encoded = providerRequest.content_base64 || "";
    await runloopExec(
      devbox,
      [
        `AK_TARGET=${runloopAssignmentValue(target)}`,
        'mkdir -p "$(dirname "$AK_TARGET")"',
        `printf '%s' ${daytonaShellQuote(encoded)} | base64 -d > "$AK_TARGET"`,
      ].join("\n"),
      { workdir: "/workspace" },
    );
    return runloopResponse(devbox, providerRequest, await devbox.getInfo());
  });
}

async function readRunloopFile(providerRequest) {
  return withRunloopSdk(async (sdk) => {
    const devbox = getRunloopDevbox(sdk, providerRequest);
    await ensureRunloopOperational(devbox, providerRequest);
    const target = providerRequest.path;
    if (!target) {
      throw new Error("read_file requires path");
    }
    const result = await runloopExec(
      devbox,
      [
        `AK_TARGET=${runloopAssignmentValue(target)}`,
        'base64 < "$AK_TARGET" | tr -d "\\n"',
      ].join("\n"),
      { workdir: "/workspace" },
    );
    return runloopResponse(devbox, providerRequest, await devbox.getInfo(), {
      content_base64: result.stdout.trim(),
    });
  });
}

async function removeRunloopFile(providerRequest) {
  return withRunloopSdk(async (sdk) => {
    const devbox = getRunloopDevbox(sdk, providerRequest);
    await ensureRunloopOperational(devbox, providerRequest);
    const target = providerRequest.path;
    if (!target) {
      throw new Error("remove_file requires path");
    }
    await runloopExec(
      devbox,
      `AK_TARGET=${runloopAssignmentValue(target)}\nrm -rf -- "$AK_TARGET"`,
      { workdir: "/workspace" },
    );
    return runloopResponse(devbox, providerRequest, await devbox.getInfo());
  });
}

async function mkdirRunloop(providerRequest) {
  return withRunloopSdk(async (sdk) => {
    const devbox = getRunloopDevbox(sdk, providerRequest);
    await ensureRunloopOperational(devbox, providerRequest);
    const target = providerRequest.path;
    if (!target) {
      throw new Error("mkdir requires path");
    }
    const flag = providerRequest.recursive ? "-p " : "";
    await runloopExec(
      devbox,
      `AK_TARGET=${runloopAssignmentValue(target)}\nmkdir ${flag}-- "$AK_TARGET"`,
      { workdir: "/workspace" },
    );
    return runloopResponse(devbox, providerRequest, await devbox.getInfo());
  });
}

async function syncPushRunloop(providerRequest) {
  return withRunloopSdk(async (sdk) => {
    const devbox = getRunloopDevbox(sdk, providerRequest);
    await ensureRunloopOperational(devbox, providerRequest);
    const localPath = providerRequest.local_path;
    if (!localPath) {
      throw new Error("sync_push requires local_path");
    }

    const ignoreRules = await loadIgnoreRules(localPath);
    const { tmpRoot, archivePath } = await createLocalArchiveFromWorkspace(
      localPath,
      ignoreRules,
    );

    try {
      await uploadRunloopArchive(devbox, providerRequest, archivePath);
      await replaceRunloopWorkspace(devbox, providerRequest);
    } finally {
      await fs.rm(tmpRoot, { recursive: true, force: true }).catch(() => {});
    }

    const workspaceRevision = await hashRunloopWorkspace(
      devbox,
      providerRequest,
      ignoreRules,
    );
    return runloopResponse(devbox, providerRequest, await devbox.getInfo(), {
      workspace_revision: workspaceRevision,
    });
  });
}

async function syncPullRunloop(providerRequest) {
  return withRunloopSdk(async (sdk) => {
    const devbox = getRunloopDevbox(sdk, providerRequest);
    await ensureRunloopOperational(devbox, providerRequest);
    const localPath = providerRequest.local_path;
    if (!localPath) {
      throw new Error("sync_pull requires local_path");
    }

    const ignoreRules = await loadIgnoreRules(localPath);

    const tmpRoot = await fs.mkdtemp(
      path.join(bridgeTempRoot(), "agentkernel-runloop-pull-"),
    );
    const archivePath = path.join(tmpRoot, "workspace.tgz");
    const extractDir = path.join(tmpRoot, "workspace");

    try {
      await createRunloopWorkspaceArchive(devbox, providerRequest);
      await fs.writeFile(archivePath, await downloadRunloopArchiveBuffer(devbox, providerRequest));
      await fs.mkdir(localPath, { recursive: true });
      const localEntries = await collectEntries(localPath, ignoreRules);
      for (const relPath of deleteSort(localEntries.keys())) {
        await fs.rm(path.join(localPath, relPath), {
          recursive: true,
          force: true,
        });
      }
      await extractLocalArchiveToDir(archivePath, localPath);
    } finally {
      await fs.rm(tmpRoot, { recursive: true, force: true }).catch(() => {});
    }

    const workspaceRevision = await hashTree(localPath, ignoreRules);
    return runloopResponse(devbox, providerRequest, await devbox.getInfo(), {
      workspace_revision: workspaceRevision,
    });
  });
}

async function takeRunloopSnapshot(providerRequest) {
  return withRunloopSdk(async (sdk) => {
    const devbox = getRunloopDevbox(sdk, providerRequest);
    await ensureRunloopOperational(devbox, providerRequest);
    const snapshot = await devbox.snapshotDisk({
      name: providerRequest.snapshot_name || `snapshot-${Date.now()}`,
      metadata: {
        agentkernel_name: providerRequest.sandbox_name,
      },
    });
    const response = await runloopResponse(
      devbox,
      providerRequest,
      await devbox.getInfo(),
    );
    response.remote_metadata = {
      ...response.remote_metadata,
      snapshot_handle: snapshot.id,
    };
    return response;
  });
}

async function deleteRunloopSnapshot(providerRequest) {
  const snapshotHandle = providerRequest.snapshot_name;
  if (!snapshotHandle) {
    throw new Error("delete_snapshot requires snapshot_name");
  }
  return withRunloopSdk(async (sdk) => {
    const snapshot = sdk.snapshot.fromId(snapshotHandle);
    await snapshot.delete();
    return { success: true };
  });
}

async function restoreRunloopSnapshot(providerRequest) {
  const snapshotHandle = providerRequest.snapshot_name;
  if (!snapshotHandle) {
    throw new Error("restore requires snapshot_name");
  }

  return withRunloopSdk(async (sdk) => {
    const target = getRunloopDevbox(sdk, providerRequest);
    await ensureRunloopOperational(target, providerRequest);

    const source = await sdk.snapshot.fromId(snapshotHandle).createDevbox({
      name: `${providerRequest.sandbox_name}-restore-${Date.now()}`,
    });
    const tmpRoot = await fs.mkdtemp(
      path.join(bridgeTempRoot(), "agentkernel-runloop-restore-"),
    );
    const archivePath = path.join(tmpRoot, "workspace.tgz");
    const extractDir = path.join(tmpRoot, "workspace");
    let workspaceRevision = null;

    try {
      await ensureRunloopWorkspace(source);
      await createRunloopWorkspaceArchive(source, {
        ...providerRequest,
        remote_id: source.id,
        remote_metadata: {},
      });
      await fs.writeFile(
        archivePath,
        await downloadRunloopArchiveBuffer(source, {
          ...providerRequest,
          remote_id: source.id,
          remote_metadata: {},
        }),
      );
      await extractLocalArchiveToDir(archivePath, extractDir);
      workspaceRevision = await hashTree(extractDir, []);
      await uploadRunloopArchive(target, providerRequest, archivePath);
      await replaceRunloopWorkspace(target, providerRequest);
    } finally {
      await fs.rm(tmpRoot, { recursive: true, force: true }).catch(() => {});
      await source.shutdown().catch(() => {});
    }

    return runloopResponse(target, providerRequest, await target.getInfo(), {
      workspace_revision: workspaceRevision,
    });
  });
}

async function attachRunloopSandbox(providerRequest) {
  return withRunloopSdk(async (sdk) => {
    const devbox = getRunloopDevbox(sdk, providerRequest);
    await ensureRunloopOperational(devbox, providerRequest);

    const shellProgram =
      providerRequest.shell || process.env.SHELL || "/bin/bash";
    const execution = await devbox.cmd.execAsync(
      runloopWrappedScript(`exec ${daytonaShellQuote(shellProgram)} -li`, {
        env: providerRequest.env || {},
        workdir: providerRequest.workdir || "/workspace",
      }),
      {
        attach_stdin: true,
        stdout: (line) => process.stdout.write(line),
        stderr: (line) => process.stderr.write(line),
      },
    );

    const onData = (chunk) => {
      const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
      if (buffer.length === 1 && buffer[0] === 0x03) {
        void sdk.api.devboxes.executions
          .sendStdIn(devbox.id, execution.executionId, { signal: "INTERRUPT" })
          .catch(() => {});
        return;
      }
      if (buffer.length === 1 && buffer[0] === 0x04) {
        void sdk.api.devboxes.executions
          .sendStdIn(devbox.id, execution.executionId, { signal: "EOF" })
          .catch(() => {});
        return;
      }
      void sdk.api.devboxes.executions
        .sendStdIn(devbox.id, execution.executionId, {
          text: buffer.toString("utf8"),
        })
        .catch(() => {});
    };

    if (process.stdin.isTTY) {
      process.stdin.setRawMode(true);
    }
    process.stdin.resume();
    process.stdin.on("data", onData);

    try {
      const result = await execution.result();
      return result.exitCode ?? 0;
    } finally {
      process.stdin.off("data", onData);
      if (process.stdin.isTTY) {
        process.stdin.setRawMode(false);
      }
    }
  });
}

function sandboxesDir() {
  return path.join(rootDir, "sandboxes");
}

function sandboxStatePath(remoteId) {
  validatePathComponent(remoteId, "remoteId");
  return path.join(sandboxesDir(), `${remoteId}.json`);
}

function nameMapPath(name) {
  validatePathComponent(name, "sandbox name");
  return path.join(rootDir, "names", `${name}.json`);
}

function snapshotDir(remoteId, snapshotName) {
  validatePathComponent(remoteId, "remoteId");
  validatePathComponent(snapshotName, "snapshot name");
  return path.join(rootDir, "snapshots", remoteId, snapshotName);
}

function encodeSnapshotHandle(remoteId, snapshotName) {
  return `${remoteId}:${snapshotName}`;
}

function decodeSnapshotHandle(handle, fallbackRemoteId) {
  const separator = handle.indexOf(":");
  if (separator === -1) {
    if (!fallbackRemoteId) {
      throw new Error("snapshot handle is missing its remote identifier");
    }
    return { remoteId: fallbackRemoteId, snapshotName: handle };
  }

  return {
    remoteId: handle.slice(0, separator),
    snapshotName: handle.slice(separator + 1),
  };
}

async function loadSandbox(request) {
  const remoteId = request.remote_id ?? (await lookupRemoteId(request.sandbox_name));
  if (!remoteId) {
    throw new Error(`remote sandbox '${request.sandbox_name}' not found`);
  }

  const file = sandboxStatePath(remoteId);
  const raw = await fs.readFile(file, "utf8");
  return JSON.parse(raw);
}

async function saveSandbox(state) {
  await fs.mkdir(sandboxesDir(), { recursive: true });
  await fs.mkdir(path.dirname(nameMapPath(state.name)), { recursive: true });
  await fs.writeFile(sandboxStatePath(state.remoteId), JSON.stringify(state, null, 2));
  await fs.writeFile(nameMapPath(state.name), JSON.stringify({ remoteId: state.remoteId }));
}

async function lookupRemoteId(name) {
  try {
    const raw = await fs.readFile(nameMapPath(name), "utf8");
    const parsed = JSON.parse(raw);
    return parsed.remoteId;
  } catch {
    return null;
  }
}

function endpointUrl(providerName, remoteId, port) {
  return `https://${providerName}-${remoteId}-${port}.agentkernel.invalid`;
}

function toResponse(state, extra = {}) {
  return {
    success: true,
    remote_id: state.remoteId,
    remote_namespace: state.namespace ?? null,
    remote_metadata: {
      ...state.remoteMetadata,
      last_known_status: state.running ? "running" : "stopped",
    },
    workspace_revision: state.workspaceRevision ?? null,
    endpoints: state.endpoints ?? [],
    running: Boolean(state.running),
    ...extra,
  };
}

async function createSandbox(providerName, request) {
  const remoteId = crypto.randomUUID();
  const fsRoot = path.join(rootDir, "filesystems", remoteId, "rootfs");
  const workspaceDir = path.join(fsRoot, "workspace");
  await fs.mkdir(workspaceDir, { recursive: true });

  const state = {
    provider: providerName,
    remoteId,
    name: request.sandbox_name,
    namespace: request.remote_namespace ?? null,
    profile: request.profile ?? "default",
    image: request.image ?? null,
    vcpus: request.vcpus ?? 1,
    memoryMb: request.memory_mb ?? 512,
    running: true,
    fsRoot,
    workspaceDir,
    workspaceRevision: hashString(""),
    remoteMetadata: {
      ...(request.remote_metadata ?? {}),
      profile_name: request.profile ?? request.image ?? "default",
    },
    endpoints: (request.ports ?? []).map((port) => ({
      container_port: port.container_port,
      protocol: port.protocol ?? "tcp",
      url: endpointUrl(providerName, remoteId, port.container_port),
    })),
  };

  await saveSandbox(state);
  return toResponse(state);
}

async function resumeSandbox(request) {
  const state = await loadSandbox(request);
  state.running = true;
  await saveSandbox(state);
  return toResponse(state);
}

async function statusSandbox(request) {
  const state = await loadSandbox(request);
  return toResponse(state);
}

async function stopSandbox(request) {
  const state = await loadSandbox(request);
  state.running = false;
  await saveSandbox(state);
  return toResponse(state);
}

async function destroySandbox(request) {
  const state = await loadSandbox(request);
  await fs.rm(path.join(rootDir, "filesystems", state.remoteId), {
    recursive: true,
    force: true,
  });
  await fs.rm(path.dirname(snapshotDir(state.remoteId, "placeholder")), {
    recursive: true,
    force: true,
  });
  await fs.rm(sandboxStatePath(state.remoteId), { force: true });
  await fs.rm(nameMapPath(state.name), { force: true });
  return { success: true, remote_id: state.remoteId, running: false };
}

async function execSandbox(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const command = request.command ?? [];
  if (command.length === 0) {
    throw new Error("exec requires a command");
  }

  const result = await spawnCommand(command, {
    cwd: resolveWorkdir(state, request.workdir),
    env: { ...process.env, ...Object.fromEntries(Object.entries(request.env ?? {})) },
    stdio: "pipe",
  });

  state.workspaceRevision = await hashTree(state.workspaceDir);
  await saveSandbox(state);
  return toResponse(state, {
    exit_code: result.exitCode,
    stdout: result.stdout,
    stderr: result.stderr,
  });
}

async function attachSandbox(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const shell = request.shell || process.env.SHELL || "/bin/sh";
  const env = { ...process.env, ...Object.fromEntries(Object.entries(request.env ?? {})) };
  const child = spawn(shell, {
    cwd: resolveWorkdir(state, "/workspace"),
    env,
    stdio: "inherit",
    shell: false,
  });

  const exitCode = await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", (code) => resolve(code ?? -1));
  });

  state.workspaceRevision = await hashTree(state.workspaceDir);
  await saveSandbox(state);
  return exitCode;
}

async function writeSandboxFile(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const target = resolveSandboxPath(state, request.path);
  await fs.mkdir(path.dirname(target), { recursive: true });
  await fs.writeFile(target, Buffer.from(request.content_base64 ?? "", "base64"));
  state.workspaceRevision = await hashTree(state.workspaceDir);
  await saveSandbox(state);
  return toResponse(state);
}

async function readSandboxFile(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const target = resolveSandboxPath(state, request.path);
  const content = await fs.readFile(target);
  return toResponse(state, { content_base64: content.toString("base64") });
}

async function removeSandboxFile(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const target = resolveSandboxPath(state, request.path);
  await fs.rm(target, { force: true });
  state.workspaceRevision = await hashTree(state.workspaceDir);
  await saveSandbox(state);
  return toResponse(state);
}

async function mkdirSandbox(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const target = resolveSandboxPath(state, request.path);
  await fs.mkdir(target, { recursive: Boolean(request.recursive) });
  state.workspaceRevision = await hashTree(state.workspaceDir);
  await saveSandbox(state);
  return toResponse(state);
}

async function syncPush(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const localPath = request.local_path;
  if (!localPath) {
    throw new Error("sync_push requires local_path");
  }

  const ignoreRules = await loadIgnoreRules(localPath);
  const remoteRevision = await hashTree(state.workspaceDir, ignoreRules);
  if (request.workspace_revision && remoteRevision !== request.workspace_revision) {
    throw new Error("workspace sync conflict: remote workspace changed since last sync");
  }

  await mirrorDirectory(localPath, state.workspaceDir, ignoreRules);
  state.workspaceRevision = await hashTree(state.workspaceDir, ignoreRules);
  await saveSandbox(state);
  return toResponse(state);
}

async function syncPull(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const localPath = request.local_path;
  if (!localPath) {
    throw new Error("sync_pull requires local_path");
  }

  const ignoreRules = await loadIgnoreRules(localPath);
  const localRevision = await hashTree(localPath, ignoreRules);
  if (request.workspace_revision && localRevision !== request.workspace_revision) {
    throw new Error("workspace sync conflict: local workspace changed since last sync");
  }

  await mirrorDirectory(state.workspaceDir, localPath, ignoreRules);
  state.workspaceRevision = await hashTree(state.workspaceDir, ignoreRules);
  await saveSandbox(state);
  return toResponse(state);
}

async function takeSnapshot(request) {
  const state = await loadSandbox(request);
  ensureRunning(state);
  const snapshotName = request.snapshot_name || `snapshot-${Date.now()}`;
  const destination = snapshotDir(state.remoteId, snapshotName);
  await fs.rm(destination, { recursive: true, force: true });
  await mirrorDirectory(state.fsRoot, destination, []);
  return toResponse(state, {
    remote_metadata: {
      ...state.remoteMetadata,
      snapshot_handle: encodeSnapshotHandle(state.remoteId, snapshotName),
      last_known_status: state.running ? "running" : "stopped",
    },
  });
}

async function restoreSnapshot(request) {
  const state = await loadSandbox(request);
  const snapshotHandle = request.snapshot_name;
  if (!snapshotHandle) {
    throw new Error("restore requires snapshot_name");
  }
  const { remoteId, snapshotName } = decodeSnapshotHandle(
    snapshotHandle,
    state.remoteId,
  );
  const source = snapshotDir(remoteId, snapshotName);
  await fs.access(source);
  await mirrorDirectory(source, state.fsRoot, []);
  state.workspaceRevision = await hashTree(state.workspaceDir);
  await saveSandbox(state);
  return toResponse(state);
}

async function deleteSnapshot(request) {
  const snapshotHandle = request.snapshot_name;
  if (!snapshotHandle) {
    throw new Error("delete_snapshot requires snapshot_name");
  }
  const { remoteId, snapshotName } = decodeSnapshotHandle(snapshotHandle);
  await fs.rm(snapshotDir(remoteId, snapshotName), {
    recursive: true,
    force: true,
  });
  return { success: true };
}

function ensureRunning(state) {
  if (!state.running) {
    throw new Error(`remote sandbox '${state.name}' is not running`);
  }
}

function resolveWorkdir(state, requested) {
  if (!requested) {
    return state.workspaceDir;
  }
  return resolveSandboxPath(state, requested);
}

function resolveSandboxPath(state, requestedPath) {
  if (!requestedPath || !requestedPath.startsWith("/")) {
    throw new Error(`sandbox path must be absolute: ${requestedPath}`);
  }
  const rel = path.normalize(requestedPath).replace(/^(\.\.(\/|\\|$))+/, "");
  const target = path.resolve(state.fsRoot, `.${rel}`);
  if (!target.startsWith(path.resolve(state.fsRoot))) {
    throw new Error(`path escapes sandbox root: ${requestedPath}`);
  }
  return target;
}

async function spawnCommand(command, options) {
  return await new Promise((resolve, reject) => {
    const [program, ...args] = command;
    const child = spawn(program, args, options);
    let stdout = "";
    let stderr = "";
    child.stdout?.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr?.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    child.on("error", reject);
    child.on("exit", (exitCode) => resolve({ exitCode: exitCode ?? -1, stdout, stderr }));
  });
}

async function loadIgnoreRules(baseDir) {
  const builtins = [".git", ".jj", "node_modules", "target", "dist", "build", ".DS_Store"];
  const rules = [...builtins];
  for (const file of [".agentkernelignore", ".gitignore"]) {
    try {
      const raw = await fs.readFile(path.join(baseDir, file), "utf8");
      for (const line of raw.split(/\r?\n/)) {
        const trimmed = line.trim();
        if (trimmed && !trimmed.startsWith("#")) {
          rules.push(trimmed.replace(/^\.\//, ""));
        }
      }
    } catch {
      // ignore missing files
    }
  }
  return rules;
}

function shouldIgnore(relPath, rules) {
  return rules.some((rule) => {
    if (rule.endsWith("/")) {
      const prefix = rule.slice(0, -1);
      return relPath === prefix || relPath.startsWith(`${prefix}/`);
    }
    return relPath === rule || relPath.startsWith(`${rule}/`);
  });
}

async function mirrorDirectory(sourceDir, targetDir, ignoreRules) {
  await fs.mkdir(targetDir, { recursive: true });
  const sourceEntries = await collectEntries(sourceDir, ignoreRules);
  const targetEntries = await collectEntries(targetDir, ignoreRules);

  for (const relPath of targetEntries.keys()) {
    if (!sourceEntries.has(relPath)) {
      await fs.rm(path.join(targetDir, relPath), { recursive: true, force: true });
    }
  }

  for (const [relPath, entry] of sourceEntries.entries()) {
    const sourcePath = path.join(sourceDir, relPath);
    const targetPath = path.join(targetDir, relPath);
    if (entry.kind === "dir") {
      await fs.mkdir(targetPath, { recursive: true });
      continue;
    }
    await fs.mkdir(path.dirname(targetPath), { recursive: true });
    await fs.copyFile(sourcePath, targetPath);
  }
}

async function collectEntries(baseDir, ignoreRules) {
  const entries = new Map();
  await walk(baseDir, "", entries, ignoreRules);
  return entries;
}

async function walk(baseDir, relDir, entries, ignoreRules) {
  let children = [];
  try {
    children = await fs.readdir(path.join(baseDir, relDir), { withFileTypes: true });
  } catch {
    return;
  }

  for (const child of children) {
    const relPath = relDir ? path.join(relDir, child.name) : child.name;
    const normalized = relPath.split(path.sep).join("/");
    if (shouldIgnore(normalized, ignoreRules)) {
      continue;
    }
    if (child.isDirectory()) {
      entries.set(normalized, { kind: "dir" });
      await walk(baseDir, relPath, entries, ignoreRules);
    } else if (child.isFile()) {
      entries.set(normalized, { kind: "file" });
    }
  }
}

async function hashTree(baseDir, ignoreRules = []) {
  const entries = await collectEntries(baseDir, ignoreRules);
  const hasher = crypto.createHash("sha256");
  for (const relPath of [...entries.keys()].sort()) {
    const entry = entries.get(relPath);
    hasher.update(relPath);
    hasher.update(entry.kind);
    if (entry.kind === "file") {
      try {
        hasher.update(await fs.readFile(path.join(baseDir, relPath)));
      } catch {
        // ignore transient reads
      }
    }
  }
  return hasher.digest("hex");
}

function hashString(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}
