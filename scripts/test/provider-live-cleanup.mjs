import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const provider = process.env.AGENTKERNEL_LIVE_PROVIDER;
const liveRoot = path.resolve(process.env.AGENTKERNEL_LIVE_ROOT || os.tmpdir());
const bridgePath = fileURLToPath(new URL("../remote-bridge.mjs", import.meta.url));

function destroy(resource, tempRoot) {
  const payload = Buffer.from(
    JSON.stringify({ ...resource, operation: "destroy" }),
  ).toString("base64");
  const result = spawnSync(process.execPath, [bridgePath, provider, payload], {
    encoding: "utf8",
    env: { ...process.env, AGENTKERNEL_REMOTE_TMPDIR: tempRoot },
  });
  return result.status === 0;
}

let failed = false;
let entries = [];
try {
  entries = await fs.readdir(liveRoot, { withFileTypes: true });
} catch {
  process.exit(0);
}

for (const entry of entries) {
  if (!entry.isDirectory() || !entry.name.startsWith(`agentkernel-${provider}-live-`)) {
    continue;
  }
  const tempRoot = path.join(liveRoot, entry.name);
  const resourcePath = path.join(tempRoot, "live-resource.json");
  let keepManifest = false;
  try {
    const resource = JSON.parse(await fs.readFile(resourcePath, "utf8"));
    if (resource.remote_id) {
      if (!destroy(resource, tempRoot)) {
        failed = true;
        keepManifest = true;
        console.error(`provider cleanup failed for ${provider} resource; manifest retained for retry`);
      }
    } else {
      await fs.rm(tempRoot, { recursive: true, force: true });
    }
  } catch (error) {
    // A process killed before create completed has no provider resource to delete.
    // Preserve a malformed or unreadable manifest for operator inspection.
    if (error?.code !== "ENOENT") {
      failed = true;
      keepManifest = true;
      console.error(`provider cleanup could not read ${provider} resource manifest`);
    }
  }
  if (!keepManifest) {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
}

process.exitCode = failed ? 1 : 0;
