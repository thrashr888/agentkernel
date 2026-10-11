import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

const scriptsDir = fileURLToPath(new URL("../", import.meta.url));

// Modal reads its TOML configuration at import time. A fresh child per fixture
// checks the real SDK consumer without reading the developer's configuration.
async function readProfile(t, contents, overrides = {}) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "agentkernel-modal-config-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const configPath = path.join(root, "modal.toml");
  if (contents !== null) await fs.writeFile(configPath, contents);
  const env = Object.fromEntries(
    Object.entries(process.env).filter(([key]) => !key.startsWith("MODAL_")),
  );
  const result = spawnSync(process.execPath, ["--input-type=module", "-e", `
    import { ModalClient } from "modal";
    // Inject a local transport stub: profile resolution must not call a service.
    const client = new ModalClient({ cpClient: { close() {} } });
    try { console.log(JSON.stringify(client.profile)); }
    finally { client.close(); }
  `], {
    cwd: scriptsDir,
    env: { ...env, MODAL_CONFIG_PATH: configPath, ...overrides },
    encoding: "utf8",
    timeout: 10_000,
  });
  assert.equal(result.error, undefined);
  assert.equal(result.status, 0, result.stderr);
  return JSON.parse(result.stdout.trim());
}

const profiles = `
[default]
active = true
token_id = "fixture-id"
token_secret = "fixture-secret"
environment = "development"

[production]
token_id = "production-fixture-id"
token_secret = "production-fixture-secret"
environment = "production"
`;

test("Modal reads the active TOML profile with the patched parser", async (t) => {
  const profile = await readProfile(t, profiles);
  assert.equal(profile.tokenId, "fixture-id");
  assert.equal(profile.tokenSecret, "fixture-secret");
  assert.equal(profile.environment, "development");
});

test("Modal selects named profiles and preserves environment overrides", async (t) => {
  const profile = await readProfile(t, profiles, {
    MODAL_PROFILE: "production",
    MODAL_TOKEN_ID: "override-fixture-id",
    MODAL_ENVIRONMENT: "staging",
  });
  assert.equal(profile.tokenId, "override-fixture-id");
  assert.equal(profile.tokenSecret, "production-fixture-secret");
  assert.equal(profile.environment, "staging");
});

test("Modal tolerates a missing TOML configuration", async (t) => {
  const profile = await readProfile(t, null);
  assert.equal(profile.tokenId, undefined);
  assert.equal(profile.tokenSecret, undefined);
});

test("Modal preserves fallback behavior for malformed TOML", async (t) => {
  const profile = await readProfile(t, "[invalid", {
    MODAL_TOKEN_ID: "fallback-fixture-id",
  });
  assert.equal(profile.tokenId, "fallback-fixture-id");
  assert.equal(profile.tokenSecret, undefined);
});
