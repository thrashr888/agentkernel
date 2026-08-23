/**
 * Compare the bundled desktop version with a local AgentKernel server.
 *
 * A version mismatch on a remote server is often intentional, so only local
 * endpoints are classified as a possible stale Homebrew formula install.
 */
export type LocalServerVersionStatus =
  | "current"
  | "older"
  | "newer"
  | "unknown"
  | "remote";

function parseVersion(version: string): number[] | null {
  const match = version
    .trim()
    .replace(/^v/i, "")
    .match(/^(\d+)(?:\.(\d+))?(?:\.(\d+))?/);
  if (!match) return null;
  return [Number(match[1]), Number(match[2] ?? 0), Number(match[3] ?? 0)];
}

function isLocalServer(url: string): boolean {
  try {
    const hostname = new URL(url).hostname.toLowerCase();
    return (
      hostname === "localhost" ||
      hostname === "127.0.0.1" ||
      hostname === "[::1]" ||
      hostname === "::1"
    );
  } catch {
    return false;
  }
}

/** Classify a server version, restricting stale-install warnings to localhost. */
export function classifyLocalServerVersion(
  appVersion: string,
  serverVersion: string,
  serverUrl: string,
): LocalServerVersionStatus {
  if (!isLocalServer(serverUrl)) return "remote";

  const app = parseVersion(appVersion);
  const server = parseVersion(serverVersion);
  if (!app || !server) return "unknown";

  for (let index = 0; index < 3; index += 1) {
    if (app[index] > server[index]) return "older";
    if (app[index] < server[index]) return "newer";
  }
  return "current";
}

export function localServerVersionMessage(
  status: LocalServerVersionStatus,
  appVersion: string,
  serverVersion: string,
): string | null {
  if (status === "older") {
    return `This desktop app is newer (v${appVersion}) than the local server (v${serverVersion}). A stale Homebrew formula may be running; run brew upgrade agentkernel, then restart the server.`;
  }
  if (status === "newer") {
    return `The local server (v${serverVersion}) is newer than this desktop app (v${appVersion}). Update the desktop app for the matching control plane.`;
  }
  return null;
}
