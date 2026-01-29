import { resolveConfig } from "./config.js";
import {
  AgentKernelError,
  NetworkError,
  errorFromStatus,
} from "./errors.js";
import { SandboxSession } from "./sandbox.js";
import { parseSSE } from "./sse.js";
import type {
  AgentKernelOptions,
  ApiResponse,
  CreateSandboxOptions,
  RunOptions,
  RunOutput,
  SandboxInfo,
  StreamEvent,
} from "./types.js";

const SDK_VERSION = "0.1.0";

/**
 * Client for the agentkernel HTTP API.
 *
 * @example
 * ```ts
 * const client = new AgentKernel();
 * const result = await client.run(["echo", "hello"]);
 * console.log(result.output); // "hello\n"
 * ```
 */
export class AgentKernel {
  private readonly baseUrl: string;
  private readonly apiKey: string | undefined;
  private readonly timeout: number;

  constructor(opts?: AgentKernelOptions) {
    const config = resolveConfig(opts);
    this.baseUrl = config.baseUrl;
    this.apiKey = config.apiKey;
    this.timeout = config.timeout;
  }

  // -- Core API methods --

  /** Health check. Returns "ok" if the server is running. */
  async health(): Promise<string> {
    const res = await this.request<string>("GET", "/health");
    return res;
  }

  /** Run a command in a temporary sandbox. */
  async run(command: string[], opts?: RunOptions): Promise<RunOutput> {
    return this.request<RunOutput>("POST", "/run", {
      command,
      image: opts?.image,
      profile: opts?.profile,
      fast: opts?.fast ?? true,
    });
  }

  /**
   * Run a command with SSE streaming output.
   *
   * @example
   * ```ts
   * for await (const event of client.runStream(["python3", "script.py"])) {
   *   if (event.type === "output") process.stdout.write(String(event.data.data));
   * }
   * ```
   */
  async *runStream(
    command: string[],
    opts?: RunOptions,
  ): AsyncGenerator<StreamEvent> {
    const body = JSON.stringify({
      command,
      image: opts?.image,
      profile: opts?.profile,
      fast: opts?.fast ?? true,
    });

    const response = await this.fetch("/run/stream", {
      method: "POST",
      headers: this.headers("application/json"),
      body,
    });

    if (!response.ok) {
      const text = await response.text();
      throw errorFromStatus(response.status, text);
    }

    if (!response.body) {
      throw new AgentKernelError("No response body for SSE stream");
    }

    yield* parseSSE(response.body);
  }

  /** List all sandboxes. */
  async listSandboxes(): Promise<SandboxInfo[]> {
    return this.request<SandboxInfo[]>("GET", "/sandboxes");
  }

  /** Create a new sandbox. */
  async createSandbox(
    name: string,
    opts?: CreateSandboxOptions,
  ): Promise<SandboxInfo> {
    return this.request<SandboxInfo>("POST", "/sandboxes", {
      name,
      image: opts?.image,
    });
  }

  /** Get info about a sandbox. */
  async getSandbox(name: string): Promise<SandboxInfo> {
    return this.request<SandboxInfo>("GET", `/sandboxes/${encodeURIComponent(name)}`);
  }

  /** Remove a sandbox. */
  async removeSandbox(name: string): Promise<void> {
    await this.request<string>("DELETE", `/sandboxes/${encodeURIComponent(name)}`);
  }

  /** Run a command in an existing sandbox. */
  async execInSandbox(name: string, command: string[]): Promise<RunOutput> {
    return this.request<RunOutput>(
      "POST",
      `/sandboxes/${encodeURIComponent(name)}/exec`,
      { command },
    );
  }

  /**
   * Create a sandbox session with automatic cleanup.
   *
   * The returned SandboxSession implements AsyncDisposable,
   * so it works with `await using` (TS 5.2+):
   *
   * @example
   * ```ts
   * await using sb = await client.sandbox("test", { image: "python:3.12-alpine" });
   * await sb.run(["pip", "install", "numpy"]);
   * // sandbox auto-removed when scope exits
   * ```
   */
  async sandbox(
    name: string,
    opts?: CreateSandboxOptions,
  ): Promise<SandboxSession> {
    await this.createSandbox(name, opts);
    return new SandboxSession(
      name,
      (n, cmd) => this.execInSandbox(n, cmd),
      (n) => this.removeSandbox(n),
      (n) => this.getSandbox(n),
    );
  }

  // -- Internal helpers --

  private headers(contentType?: string): Record<string, string> {
    const h: Record<string, string> = {
      "User-Agent": `agentkernel-nodejs-sdk/${SDK_VERSION}`,
    };
    if (contentType) h["Content-Type"] = contentType;
    if (this.apiKey) h["Authorization"] = `Bearer ${this.apiKey}`;
    return h;
  }

  private async fetch(path: string, init: RequestInit): Promise<Response> {
    const url = `${this.baseUrl}${path}`;
    try {
      return await fetch(url, {
        ...init,
        signal: AbortSignal.timeout(this.timeout),
      });
    } catch (err) {
      if (err instanceof DOMException && err.name === "TimeoutError") {
        throw new NetworkError(`Request timed out after ${this.timeout}ms`);
      }
      if (err instanceof TypeError) {
        throw new NetworkError(`Failed to connect to ${this.baseUrl}: ${err.message}`);
      }
      throw new NetworkError(
        err instanceof Error ? err.message : String(err),
      );
    }
  }

  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<T> {
    const init: RequestInit = {
      method,
      headers: this.headers(body ? "application/json" : undefined),
    };
    if (body) init.body = JSON.stringify(body);

    const response = await this.fetch(path, init);

    const text = await response.text();
    if (!response.ok) {
      throw errorFromStatus(response.status, text);
    }

    const parsed: ApiResponse<T> = JSON.parse(text);
    if (!parsed.success) {
      throw new AgentKernelError(parsed.error ?? "Unknown error");
    }
    return parsed.data as T;
  }
}
