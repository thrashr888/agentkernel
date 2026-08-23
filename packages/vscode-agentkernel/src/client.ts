import type { Sandbox } from "./types";

export type FetchLike = (
  input: string,
  init?: RequestInit,
) => Promise<Response>;

export class AgentKernelError extends Error {
  public readonly statusCode?: number;

  public constructor(message: string, statusCode?: number) {
    super(message);
    this.name = "AgentKernelError";
    this.statusCode = statusCode;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isSandbox(value: unknown): value is Sandbox {
  return (
    isRecord(value) &&
    typeof value.name === "string" &&
    typeof value.uuid === "string" &&
    typeof value.status === "string" &&
    typeof value.backend === "string"
  );
}

function defaultFetch(): FetchLike {
  if (typeof globalThis.fetch !== "function") {
    throw new AgentKernelError(
      "The VS Code runtime does not provide fetch; update VS Code or report this compatibility issue.",
    );
  }
  return globalThis.fetch.bind(globalThis) as FetchLike;
}

export class AgentKernelClient {
  private readonly baseUrl: string;
  private readonly apiKey?: string;
  private readonly fetchImpl: FetchLike;

  public constructor(
    baseUrl: string,
    apiKey?: string,
    fetchImpl: FetchLike = defaultFetch(),
  ) {
    const trimmed = baseUrl.trim();
    if (!trimmed) {
      throw new AgentKernelError("AgentKernel API URL cannot be empty.");
    }
    try {
      const url = new URL(trimmed);
      if (url.protocol !== "http:" && url.protocol !== "https:") {
        throw new Error("unsupported protocol");
      }
    } catch {
      throw new AgentKernelError(
        `AgentKernel API URL must use http:// or https://: ${baseUrl}`,
      );
    }
    this.baseUrl = trimmed.replace(/\/+$/, "");
    this.apiKey = apiKey?.trim() || undefined;
    this.fetchImpl = fetchImpl;
  }

  public async listSandboxes(): Promise<Sandbox[]> {
    const response = await this.fetchImpl(`${this.baseUrl}/sandboxes`, {
      method: "GET",
      headers: {
        Accept: "application/json",
        ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}),
      },
    });

    const body = await this.readBody(response);
    if (!response.ok) {
      const message =
        isRecord(body) && typeof body.error === "string"
          ? body.error
          : `AgentKernel API returned HTTP ${response.status}.`;
      throw new AgentKernelError(message, response.status);
    }

    if (!isRecord(body) || body.success !== true || !Array.isArray(body.data)) {
      throw new AgentKernelError(
        "AgentKernel API returned an unexpected sandbox list response.",
      );
    }

    const invalid = body.data.find((item) => !isSandbox(item));
    if (invalid !== undefined) {
      throw new AgentKernelError(
        "AgentKernel API returned an invalid sandbox entry.",
      );
    }
    return body.data as Sandbox[];
  }

  private async readBody(response: Response): Promise<unknown> {
    const text = await response.text();
    if (!text.trim()) {
      return undefined;
    }
    try {
      return JSON.parse(text) as unknown;
    } catch {
      throw new AgentKernelError(
        `AgentKernel API returned invalid JSON (HTTP ${response.status}).`,
        response.status,
      );
    }
  }
}
