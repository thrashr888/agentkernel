import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";

const markerPrefix = "# >>> agentkernel-vscode ";
const markerSuffix = "# <<< agentkernel-vscode";

function markerKey(sandboxName: string): string {
  return encodeURIComponent(sandboxName);
}

function markerStart(sandboxName: string): string {
  return `${markerPrefix}${markerKey(sandboxName)}`;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function hostAlias(sandboxName: string): string {
  return `agentkernel-${sandboxName}`;
}

export function mergeManagedSshConfig(
  existing: string,
  sandboxName: string,
  generatedConfig: string,
): string {
  const start = markerStart(sandboxName);
  const block = [start, generatedConfig.trim(), markerSuffix].join("\n");
  const managedBlock = new RegExp(
    `(?:^|\\n)${escapeRegExp(start)}\\n[\\s\\S]*?\\n${escapeRegExp(markerSuffix)}(?=\\n|$)`,
    "g",
  );
  const withoutPrevious = existing.replace(managedBlock, "").trimEnd();
  return withoutPrevious ? `${withoutPrevious}\n\n${block}\n` : `${block}\n`;
}

function expandHome(filePath: string): string {
  if (filePath === "~") {
    return os.homedir();
  }
  if (filePath.startsWith("~/")) {
    return path.join(os.homedir(), filePath.slice(2));
  }
  return filePath;
}

export async function installManagedSshConfig(
  configPath: string,
  sandboxName: string,
  generatedConfig: string,
): Promise<string> {
  if (!generatedConfig.includes(`Host ${hostAlias(sandboxName)}`)) {
    throw new Error(
      `AgentKernel did not generate an SSH host entry for sandbox "${sandboxName}".`,
    );
  }

  const requestedPath = configPath.trim();
  if (!requestedPath) {
    throw new Error("SSH config path cannot be empty.");
  }
  const resolvedPath = path.resolve(expandHome(requestedPath));
  await fs.mkdir(path.dirname(resolvedPath), { recursive: true, mode: 0o700 });

  let existing = "";
  try {
    existing = await fs.readFile(resolvedPath, "utf8");
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code !== "ENOENT") {
      throw error;
    }
  }

  const merged = mergeManagedSshConfig(existing, sandboxName, generatedConfig);
  await fs.writeFile(resolvedPath, merged, { encoding: "utf8", mode: 0o600 });
  // Preserve a secure mode when the config already existed with permissive bits.
  try {
    await fs.chmod(resolvedPath, 0o600);
  } catch {
    // chmod is not available on every supported platform/filesystem.
  }
  return resolvedPath;
}
