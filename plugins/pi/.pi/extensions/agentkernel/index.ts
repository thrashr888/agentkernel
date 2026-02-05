/**
 * agentkernel extension for Pi coding agent
 *
 * Overrides Pi's built-in bash tool to route all shell commands through
 * agentkernel microVM sandboxes. Each session gets a persistent sandbox
 * that is automatically cleaned up when the session ends.
 *
 * Install: agentkernel plugin install pi
 * Or manually copy this directory into your project's .pi/extensions/
 */
import type { ExtensionAPI } from "@mariozechner/pi-coding-agent";
import { Type } from "@sinclair/typebox";

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
    "User-Agent": "agentkernel-pi-extension/0.8.0",
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

export default function (pi: ExtensionAPI) {
  let sandboxName: string | undefined;
  let sandboxReady = false;

  // Create a persistent sandbox when the session starts
  pi.on("session_start", async (_event, ctx) => {
    const id =
      Date.now().toString(36) +
      Math.random().toString(36).slice(2, 6);
    const name = `pi-${id}`;
    try {
      await request<SandboxInfo>("POST", "/sandboxes", {
        name,
        image: "node:22-alpine",
      });
      sandboxName = name;
      sandboxReady = true;
      if (ctx.hasUI) {
        ctx.ui.notify(`agentkernel sandbox ready: ${name}`, "info");
      }
    } catch (err) {
      if (ctx.hasUI) {
        ctx.ui.notify(
          `agentkernel not available — bash will run locally. ${err}`,
          "warning",
        );
      }
    }
  });

  // Remove the sandbox when the session ends
  pi.on("session_shutdown", async () => {
    if (sandboxName) {
      try {
        await request<string>("DELETE", `/sandboxes/${sandboxName}`);
      } catch {
        // Best-effort cleanup
      }
      sandboxName = undefined;
      sandboxReady = false;
    }
  });

  // Override the built-in bash tool to run commands in the sandbox.
  // Pi allows replacing built-in tools by registering with the same name.
  // Commands are sent to the agentkernel HTTP API as an argv array,
  // avoiding any shell injection — the sandbox receives ["sh", "-c", command]
  // where `command` is the exact string the LLM produced.
  pi.registerTool({
    name: "bash",
    label: "bash (sandboxed)",
    description:
      "Run a bash command in an agentkernel microVM sandbox. " +
      "Each command runs in a hardware-isolated virtual machine with its own kernel. " +
      "State persists within the session (installed packages, files).",
    parameters: Type.Object({
      command: Type.String({
        description: "The bash command to run",
      }),
    }),

    async execute(_toolCallId, params, signal, onUpdate) {
      const { command } = params as { command: string };

      if (!sandboxReady || !sandboxName) {
        // Fallback: run locally via pi.exec() when sandbox unavailable
        const result = await pi.exec("bash", ["-c", command], { signal });
        return {
          content: [
            { type: "text", text: result.stdout + result.stderr },
          ],
        };
      }

      try {
        if (onUpdate) {
          onUpdate({
            content: [
              {
                type: "text",
                text: `Running in sandbox ${sandboxName}...`,
              },
            ],
          });
        }

        const result = await request<RunOutput>(
          "POST",
          `/sandboxes/${sandboxName}/exec`,
          { command: ["sh", "-c", command] },
        );

        return {
          content: [{ type: "text", text: result.output }],
          details: { sandbox: sandboxName, sandboxed: true },
        };
      } catch (err: unknown) {
        if (signal?.aborted) {
          return {
            content: [{ type: "text", text: "Command cancelled." }],
          };
        }
        const message =
          err instanceof Error ? err.message : String(err);
        return {
          content: [
            {
              type: "text",
              text: `Sandbox error: ${message}`,
            },
          ],
          details: { error: true },
        };
      }
    },
  });

  // Additional tool: one-shot execution in a fresh sandbox
  pi.registerTool({
    name: "sandbox_run",
    label: "sandbox_run",
    description:
      "Run a command in a fresh one-shot agentkernel sandbox. " +
      "Unlike bash, each call gets a clean environment. " +
      "Use for untrusted code or when you need isolation from the session.",
    parameters: Type.Object({
      command: Type.String({ description: "Shell command to run" }),
      image: Type.Optional(
        Type.String({
          description:
            "Container image (default: alpine:3.20). Examples: python:3.12-alpine, node:22-alpine",
        }),
      ),
    }),

    async execute(_toolCallId, params) {
      const { command, image } = params as {
        command: string;
        image?: string;
      };
      try {
        const result = await request<RunOutput>("POST", "/run", {
          command: ["sh", "-c", command],
          image,
          fast: true,
        });
        return {
          content: [{ type: "text", text: result.output }],
          details: { sandbox: "one-shot", image: image ?? "alpine:3.20" },
        };
      } catch (err: unknown) {
        const message =
          err instanceof Error ? err.message : String(err);
        return {
          content: [
            { type: "text", text: `Sandbox error: ${message}` },
          ],
          details: { error: true },
        };
      }
    },
  });

  // /sandbox command — show sandbox status
  pi.registerCommand("sandbox", {
    description: "Show agentkernel sandbox status",
    handler: async (_args, ctx) => {
      if (!sandboxReady || !sandboxName) {
        ctx.ui.notify(
          "No active sandbox. Is agentkernel running?",
          "warning",
        );
        return;
      }
      try {
        const info = await request<SandboxInfo>(
          "GET",
          `/sandboxes/${sandboxName}`,
        );
        ctx.ui.notify(
          `Sandbox: ${info.name}\nStatus: ${info.status}\nBackend: ${info.backend}`,
          "info",
        );
      } catch {
        ctx.ui.notify(
          `Sandbox: ${sandboxName} (status unknown)`,
          "warning",
        );
      }
    },
  });
}
