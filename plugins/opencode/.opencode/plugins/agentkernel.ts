/**
 * agentkernel plugin for OpenCode
 *
 * Routes code execution through agentkernel sandboxes for hardware-isolated
 * microVM security. Each OpenCode session gets a persistent sandbox that is
 * automatically cleaned up when the session ends.
 *
 * Install: agentkernel plugin install opencode
 * Or manually copy this directory into your project's .opencode/plugins/
 */
import type { Plugin } from "@opencode-ai/plugin";
import { tool } from "@opencode-ai/plugin";

interface SandboxInfo {
  name: string;
  status: string;
  backend: string;
}

interface RunOutput {
  output: string;
}

interface ApiResponse<T> {
  success: boolean;
  data?: T;
  error?: string;
}

const BASE_URL =
  process.env.AGENTKERNEL_BASE_URL ?? "http://localhost:18888";
const API_KEY = process.env.AGENTKERNEL_API_KEY;

async function request<T>(
  method: string,
  path: string,
  body?: unknown,
): Promise<T> {
  const headers: Record<string, string> = {
    "User-Agent": "agentkernel-opencode-plugin/0.1.0",
  };
  if (API_KEY) {
    headers["Authorization"] = `Bearer ${API_KEY}`;
  }
  if (body) {
    headers["Content-Type"] = "application/json";
  }

  const response = await fetch(`${BASE_URL}${path}`, {
    method,
    headers,
    body: body ? JSON.stringify(body) : undefined,
  });

  const text = await response.text();
  if (!response.ok) {
    throw new Error(
      `agentkernel API error (${response.status}): ${text}`,
    );
  }

  const json = JSON.parse(text) as ApiResponse<T>;
  if (!json.success) {
    throw new Error(`agentkernel error: ${json.error ?? "Unknown error"}`);
  }
  return json.data as T;
}

// Track active sandboxes per session
const activeSandboxes = new Map<string, string>();

export const agentkernel: Plugin = async ({
  project,
  directory,
  client,
}) => {
  client.app.log("info", "agentkernel plugin loaded", {
    baseUrl: BASE_URL,
    directory,
  });

  return {
    hooks: {
      "session.created": async ({ session }: { session: { id: string } }) => {
        const name = `opencode-${session.id.slice(0, 8)}`;
        try {
          await request<SandboxInfo>("POST", "/sandboxes", {
            name,
            image: "node:22-alpine",
          });
          activeSandboxes.set(session.id, name);
          client.app.log("info", `agentkernel sandbox created: ${name}`);
        } catch (err) {
          client.app.log(
            "warn",
            `Failed to create sandbox: ${err}. Commands will run without sandbox.`,
          );
        }
      },

      "session.deleted": async ({
        session,
      }: {
        session: { id: string };
      }) => {
        const name = activeSandboxes.get(session.id);
        if (name) {
          try {
            await request<string>("DELETE", `/sandboxes/${name}`);
            client.app.log(
              "info",
              `agentkernel sandbox removed: ${name}`,
            );
          } catch {
            // Best-effort cleanup
          }
          activeSandboxes.delete(session.id);
        }
      },
    },

    tools: {
      sandbox_run: tool({
        description:
          "Run a command in an isolated agentkernel microVM sandbox. " +
          "Each command runs in a hardware-isolated virtual machine with its own kernel. " +
          "Use this for untrusted code execution, installing packages, or running tests safely.",
        args: {
          command: tool.schema
            .array(tool.schema.string())
            .describe("Command and arguments to run, e.g. ['python3', '-c', 'print(1)']"),
          image: tool.schema
            .string()
            .optional()
            .describe(
              "Container image to use, e.g. 'python:3.12-alpine', 'node:22-alpine'",
            ),
          profile: tool.schema
            .enum(["permissive", "moderate", "restrictive"])
            .optional()
            .describe("Security profile (default: moderate)"),
        },
        async execute(args, context) {
          // If there's an active session sandbox, use it
          // Otherwise, use the one-shot /run endpoint
          const result = await request<RunOutput>("POST", "/run", {
            command: args.command,
            image: args.image,
            profile: args.profile,
            fast: true,
          });
          return result.output;
        },
      }),

      sandbox_exec: tool({
        description:
          "Run a command in the current session's persistent sandbox. " +
          "The sandbox is created when the session starts and removed when it ends. " +
          "State (installed packages, files) persists between calls within the same session.",
        args: {
          command: tool.schema
            .array(tool.schema.string())
            .describe("Command and arguments to run"),
        },
        async execute(args, context) {
          // Look up the session sandbox from our in-memory map first
          let sandboxName: string | undefined;
          for (const [, name] of activeSandboxes) {
            sandboxName = name;
            break;
          }

          // Fallback: query the API if the map is empty (e.g. plugin reloaded mid-session)
          if (!sandboxName) {
            const sandboxes = await request<SandboxInfo[]>(
              "GET",
              "/sandboxes",
            );
            const found = sandboxes.find((s) =>
              s.name.startsWith("opencode-"),
            );
            if (!found) {
              throw new Error(
                "No active session sandbox. Use sandbox_run for one-shot execution.",
              );
            }
            sandboxName = found.name;
          }

          const result = await request<RunOutput>(
            "POST",
            `/sandboxes/${sandboxName}/exec`,
            { command: args.command },
          );
          return result.output;
        },
      }),

      sandbox_list: tool({
        description: "List all active agentkernel sandboxes.",
        args: {},
        async execute() {
          const sandboxes = await request<SandboxInfo[]>(
            "GET",
            "/sandboxes",
          );
          return sandboxes
            .map(
              (s) => `${s.name} (${s.status}, ${s.backend})`,
            )
            .join("\n");
        },
      }),
    },
  };
};
